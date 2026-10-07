import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MouseSensor,
  TouchSensor,
  pointerWithin,
  rectIntersection,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type CollisionDetection,
  type DragEndEvent,
  type DragMoveEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { snapCenterToCursor } from "@dnd-kit/modifiers";
import {
  SortableContext,
  arrayMove,
  rectSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ArrowRight, Plus } from "lucide-react";
import { useRef, useState } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Router } from "../../api/types";
import { routerName } from "../../lib/market-format";
import { truncateMiddle } from "../../lib/wallet-format";

const LIST_ZONE = "list-zone";
/** How far past the route box a drop still counts, so a near miss isn't lost. */
const DROP_MARGIN = 32;
const SPRING = { type: "spring", stiffness: 520, damping: 34, mass: 0.6 } as const;

// Ids carry where the item lives, so a drop knows whether it came from the list or the route.
const routeId = (address: string) => `route:${address}`;
const listId = (address: string) => `list:${address}`;
const addressOf = (id: string) => id.slice(id.indexOf(":") + 1);

// The pointer decides, and a box under it beats the zone around it, so a drop lands between
// the two boxes the user aimed at rather than at the end.
const collision: CollisionDetection = (args) => {
  const within = pointerWithin(args);
  const boxes = within.filter((hit) => String(hit.id).startsWith("route:"));
  if (boxes.length) return boxes;
  return within.length ? within : rectIntersection(args);
};

function displayName(address: string, byAddress: Map<string, Router>) {
  return byAddress.get(address)?.offer?.name || routerName(address);
}

function Endpoint({ label, detail }: { label: string; detail?: string }) {
  return (
    <span className="inline-flex flex-none items-center gap-1.5 rounded-full border border-primary/40 bg-primary/[0.08] px-3 py-1.5 font-mono text-[11px] font-semibold text-primary">
      {label}
      {detail && <span className="font-normal text-primary/70">{detail}</span>}
    </span>
  );
}

function Arrow() {
  return <ArrowRight size={14} strokeWidth={2} className="mx-1.5 flex-none text-subtle" />;
}

function BoxFace({ hop, name, lifted = false }: { hop: number | null; name: string; lifted?: boolean }) {
  return (
    <span
      className={`inline-flex items-center gap-2 rounded-control border bg-surface px-2.5 py-1.5 ${
        lifted
          ? "border-primary/60 shadow-[0_8px_24px_rgba(0,0,0,0.45)]"
          : "border-line-strong hover:border-primary/50"
      }`}
    >
      <span className="font-mono text-[10px] font-semibold text-primary">
        {hop === null ? <Plus size={11} strokeWidth={2.4} /> : `#${hop}`}
      </span>
      <span className="max-w-[150px] truncate text-[12px] font-semibold text-foreground">{name}</span>
    </span>
  );
}

function RouteBox({
  address,
  hop,
  name,
  nudged,
  reduceMotion,
  boxes,
  onRemove,
}: {
  address: string;
  hop: number;
  name: string;
  /** A router being dragged in would land before this box. */
  nudged: boolean;
  reduceMotion: boolean;
  boxes: Map<string, HTMLElement>;
  onRemove: (address: string) => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: routeId(address),
    // Neighbours glide to their new place on every reorder, insert and removal, not only
    // while a drag is running.
    animateLayoutChanges: () => !reduceMotion,
    transition: reduceMotion ? null : { duration: 220, easing: "cubic-bezier(0.2, 0, 0, 1)" },
  });
  return (
    <div
      ref={(el) => {
        setNodeRef(el);
        if (el) boxes.set(address, el);
        else boxes.delete(address);
      }}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      className={`flex items-center ${isDragging ? "opacity-35" : ""}`}
    >
      {/* The slide is on an inner layer, so the measured outer box never moves under the
          pointer and the drop slot can't flicker between two neighbours. */}
      <motion.div
        animate={{ x: nudged ? 14 : 0 }}
        transition={reduceMotion ? { duration: 0 } : SPRING}
        className="flex items-center"
      >
      <Arrow />
      <motion.button
        type="button"
        initial={reduceMotion ? false : { opacity: 0, scale: 0.85 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={SPRING}
        onClick={() => onRemove(address)}
        aria-label={`Remove ${name} from the route`}
        className="cursor-grab touch-none rounded-control outline-none focus-visible:shadow-ring active:cursor-grabbing"
        {...attributes}
        {...listeners}
      >
        <BoxFace hop={hop} name={name} />
      </motion.button>
      </motion.div>
    </div>
  );
}

function ListRow({
  router,
  hop,
  meta,
  onToggle,
}: {
  router: Router;
  hop: number | null;
  meta: ReactNode;
  onToggle: (address: string) => void;
}) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: listId(router.address) });
  const selected = hop !== null;
  return (
    <div
      ref={setNodeRef}
      {...attributes}
      {...listeners}
      aria-pressed={selected}
      onClick={() => onToggle(router.address)}
      // Enter adds or removes; the drag sensor still gets every key, for Space and Escape.
      onKeyDown={(e) => {
        if (e.key === "Enter") onToggle(router.address);
        listeners?.onKeyDown?.(e);
      }}
      className={`flex cursor-grab touch-manipulation items-center justify-between gap-3 rounded-control border px-3 py-2 outline-none transition-[border-color,background-color,opacity] duration-150 focus-visible:shadow-ring active:cursor-grabbing ${
        selected ? "border-primary/45 bg-primary/[0.06]" : "border-line bg-surface-raised hover:border-line-strong"
      } ${isDragging ? "opacity-40" : ""}`}
    >
      <span className="flex min-w-0 items-center gap-2.5 font-mono text-[11px] text-muted">
        <span
          className={`grid h-5 min-w-5 flex-none place-items-center rounded-full px-1 text-[10px] font-semibold ${
            selected ? "bg-primary text-[var(--color-bg)]" : "border border-line text-subtle"
          }`}
        >
          {selected ? hop : <Plus size={11} strokeWidth={2.4} />}
        </span>
        {router.offer?.name ? (
          <span className="flex min-w-0 flex-col leading-[1.35]">
            <span className="truncate text-[12px] font-semibold text-foreground">{router.offer.name}</span>
            <span className="truncate font-mono text-[10.5px] text-subtle">{routerName(router.address)}</span>
          </span>
        ) : (
          <span className="font-mono text-[11px] leading-[1.45] text-muted">{routerName(router.address)}</span>
        )}
      </span>
      {meta}
    </div>
  );
}

/**
 * The route a manual pick builds: You → routers in order → You or the receiver. Click or drag a
 * router in to add it, click its box or drag it back to remove it, drag boxes to reorder. The
 * order is the route: the crate uses preferred routers in exactly this order.
 */
export function RouteBuilder({
  routers,
  selected,
  onChange,
  receiver,
  rowMeta,
  emptyMessage,
}: {
  /** The list, already filtered and sorted. */
  routers: Router[];
  selected: string[];
  onChange: (next: string[]) => void;
  /** The PaySwap address; absent when the swap pays back to this wallet. */
  receiver?: string;
  rowMeta: (router: Router) => ReactNode;
  emptyMessage: ReactNode;
}) {
  const reduceMotion = useReducedMotion() ?? false;
  const [activeId, setActiveId] = useState<string | null>(null);
  // Where a router dragged in from the list would land, and where to draw the bar showing it.
  const [drop, setDrop] = useState<{ slot: number; x: number; y: number; h: number } | null>(null);
  const zoneEl = useRef<HTMLDivElement | null>(null);
  const startEl = useRef<HTMLSpanElement | null>(null);
  const boxes = useRef(new Map<string, HTMLElement>()).current;
  const sensors = useSensors(
    // A short travel before a drag starts, so a click stays a click.
    useSensor(MouseSensor, { activationConstraint: { distance: 5 } }),
    // A press-and-hold on touch, so a swipe still scrolls the page.
    useSensor(TouchSensor, { activationConstraint: { delay: 160, tolerance: 6 } }),
    // Space lifts and drops; Enter is left to the click that adds or removes.
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
      keyboardCodes: { start: ["Space"], cancel: ["Escape"], end: ["Space"] },
    }),
  );

  const byAddress = new Map(routers.map((router) => [router.address, router]));
  const hopOf = new Map(selected.map((address, i) => [address, i + 1]));
  const draggingFromList = activeId?.startsWith("list:") ?? false;
  const showRoute = selected.length > 0 || draggingFromList;

  const { setNodeRef: listZone, isOver: overList } = useDroppable({ id: LIST_ZONE });

  const toggle = (address: string) =>
    onChange(selected.includes(address) ? selected.filter((a) => a !== address) : [...selected, address]);
  const remove = (address: string) => onChange(selected.filter((a) => a !== address));

  function onDragStart({ active }: DragStartEvent) {
    setActiveId(String(active.id));
    setDrop(null);
  }

  // The slot comes from the pointer itself: the dragged element's own rect is the wide list row,
  // whose centre can be far from where the user is pointing.
  function onDragMove({ active, activatorEvent, delta }: DragMoveEvent) {
    const id = String(active.id);
    const zone = zoneEl.current;
    if (!id.startsWith("list:") || !zone) return;
    const start =
      "touches" in activatorEvent
        ? (activatorEvent as TouchEvent).touches[0]
        : "clientX" in activatorEvent
          ? (activatorEvent as MouseEvent)
          : null;
    const rect = active.rect.current.translated;
    const px = start ? start.clientX + delta.x : rect ? rect.left + rect.width / 2 : 0;
    const py = start ? start.clientY + delta.y : rect ? rect.top + rect.height / 2 : 0;
    const z = zone.getBoundingClientRect();
    if (
      px < z.left - DROP_MARGIN ||
      px > z.right + DROP_MARGIN ||
      py < z.top - DROP_MARGIN ||
      py > z.bottom + DROP_MARGIN
    ) {
      setDrop((current) => (current ? null : current));
      return;
    }
    const order = selected.filter((a) => a !== addressOf(id));
    const rects = order.map((a) => boxes.get(a)?.getBoundingClientRect() ?? null);
    // The row the pointer is on, then the first box in it whose middle is right of the pointer.
    let row = rects.filter((r): r is DOMRect => r !== null && py >= r.top && py <= r.bottom);
    if (!row.length && rects.length) {
      const nearest = rects.reduce((best, r) =>
        r && (!best || Math.abs(r.top + r.height / 2 - py) < Math.abs(best.top + best.height / 2 - py)) ? r : best,
      );
      row = rects.filter((r): r is DOMRect => r !== null && nearest !== null && r.top === nearest.top);
    }
    // No box to place it against counts as the end, so a drop on the route never does nothing.
    const ahead = row.find((r) => r.left + r.width / 2 > px);
    const slot = ahead ? rects.indexOf(ahead) : row.length ? rects.indexOf(row[row.length - 1]) + 1 : order.length;
    const anchor = slot < rects.length ? rects[slot] : null;
    const before = slot > 0 ? rects[slot - 1] : startEl.current?.getBoundingClientRect() ?? null;
    const at = anchor
      ? { x: anchor.left + 6, y: anchor.top, h: anchor.height }
      : before
        ? { x: before.right + 6, y: before.top, h: before.height }
        : { x: z.left + 12, y: z.top + 8, h: 28 };
    const next = { slot, x: at.x - z.left, y: at.y - z.top, h: at.h };
    setDrop((current) =>
      current && current.slot === next.slot && current.x === next.x && current.y === next.y ? current : next,
    );
  }

  function onDragEnd({ active, over }: DragEndEvent) {
    const id = String(active.id);
    const address = addressOf(id);
    const slot = drop?.slot;
    setActiveId(null);
    setDrop(null);

    if (id.startsWith("list:")) {
      if (slot === undefined) return;
      const order = selected.filter((a) => a !== address);
      order.splice(slot, 0, address);
      onChange(order);
      return;
    }
    if (!over) return;
    const overId = String(over.id);
    const target = overId.startsWith("route:") ? selected.indexOf(addressOf(overId)) : -1;
    if (target >= 0) {
      const from = selected.indexOf(address);
      if (from !== target) onChange(arrayMove(selected, from, target));
    } else if (overId === LIST_ZONE || overId.startsWith("list:")) {
      remove(address);
    }
  }

  const activeAddress = activeId ? addressOf(activeId) : null;

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={collision}
      onDragStart={onDragStart}
      onDragMove={onDragMove}
      onDragEnd={onDragEnd}
      onDragCancel={() => {
        setActiveId(null);
        setDrop(null);
      }}
    >
      <AnimatePresence initial={false}>
        {showRoute && (
          <motion.div
            key="route"
            initial={reduceMotion ? false : { opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={reduceMotion ? { opacity: 0 } : { opacity: 0, height: 0 }}
            transition={{ duration: reduceMotion ? 0 : 0.22, ease: [0.2, 0, 0, 1] }}
            className="overflow-hidden"
          >
            <div
              ref={zoneEl}
              className={`relative flex flex-wrap items-center gap-y-2 rounded-control border px-3 py-2.5 transition-[border-color,background-color] duration-150 ${
                drop ? "border-primary/60 bg-primary/[0.05]" : "border-line bg-surface"
              }`}
            >
              <AnimatePresence>
                {drop && (
                  <motion.span
                    key="slot"
                    aria-hidden
                    initial={{ opacity: 0, x: drop.x, y: drop.y, height: drop.h }}
                    animate={{ opacity: 1, x: drop.x, y: drop.y, height: drop.h }}
                    exit={{ opacity: 0 }}
                    transition={reduceMotion ? { duration: 0 } : SPRING}
                    className="pointer-events-none absolute left-0 top-0 z-10 w-[3px] rounded-full bg-primary shadow-[0_0_10px_var(--color-primary)]"
                  />
                )}
              </AnimatePresence>
              <span ref={startEl} className="flex">
                <Endpoint label="You" />
              </span>
              <SortableContext items={selected.map(routeId)} strategy={rectSortingStrategy}>
                {selected.map((address, i) => (
                  <RouteBox
                    key={address}
                    address={address}
                    hop={i + 1}
                    name={displayName(address, byAddress)}
                    nudged={
                      drop !== null &&
                      address !== activeAddress &&
                      selected.filter((a) => a !== activeAddress).indexOf(address) >= drop.slot
                    }
                    reduceMotion={reduceMotion}
                    boxes={boxes}
                    onRemove={remove}
                  />
                ))}
              </SortableContext>
              {selected.length === 0 && (
                <span className="ml-2 text-[11.5px] text-subtle">Drop a router here</span>
              )}
              <motion.span layout={reduceMotion ? false : "position"} transition={SPRING} className="flex items-center">
                <Arrow />
                {receiver ? (
                  <Endpoint label="Receiver" detail={truncateMiddle(receiver, 6, 4)} />
                ) : (
                  <Endpoint label="You" />
                )}
              </motion.span>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div
        ref={listZone}
        className={`flex max-h-45 flex-col gap-1.5 overflow-y-auto rounded-control transition-[box-shadow] duration-150 ${
          overList && activeId?.startsWith("route:") ? "shadow-[inset_0_0_0_1px_var(--color-danger)]" : ""
        }`}
      >
        {routers.length === 0 && emptyMessage}
        {routers.map((router) => (
          <ListRow
            key={router.address}
            router={router}
            hop={hopOf.get(router.address) ?? null}
            meta={rowMeta(router)}
            onToggle={toggle}
          />
        ))}
      </div>

      {/* Portalled: the page wrapper keeps a transform, which would pin a fixed overlay to it. */}
      {createPortal(
        <DragOverlay
          // Centred on the pointer: the overlay is a small box, the row it was lifted from is wide.
          modifiers={[snapCenterToCursor]}
          dropAnimation={reduceMotion ? null : { duration: 200, easing: "cubic-bezier(0.2, 0, 0, 1)" }}
        >
          {activeAddress && (
            <BoxFace
              // The number it would get where it is now; a plus while it isn't over the route.
              hop={
                draggingFromList
                  ? drop
                    ? drop.slot + 1
                    : null
                  : (hopOf.get(activeAddress) ?? null)
              }
              name={displayName(activeAddress, byAddress)}
              lifted
            />
          )}
        </DragOverlay>,
        document.body,
      )}
    </DndContext>
  );
}
