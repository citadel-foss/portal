import { AlertTriangle, Ban, Trash2, Upload } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  importRouterBlocklist,
  importWalletBlocklist,
  listRouterBlocklist,
  listWalletBlocklist,
  removeRouterBlocklist,
  removeWalletBlocklist,
} from "../../api/commands";
import type { BlocklistEntry, BlocklistImport } from "../../api/types";
import { Card, IconButton, Identifier, Notice } from "../ui/display";
import { Button } from "../ui/inputs";
import { useToastStore } from "../../store/toast";

/** Rejected rows listed before the rest are summed up as a count. */
const SHOWN_REJECTS = 5;

/**
 * Addresses whose coins a swap refuses to take in. The crate does the screening; this only
 * manages the list, which every wallet shares, and every router shares another.
 */
export function BlocklistCard({ routerId }: { routerId?: string }) {
  const [entries, setEntries] = useState<BlocklistEntry[] | null>(null);
  const [importing, setImporting] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [result, setResult] = useState<BlocklistImport | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const pushFailure = useToastStore((s) => s.pushFailure);

  const load = useCallback(async () => {
    try {
      setEntries(await (routerId === undefined ? listWalletBlocklist() : listRouterBlocklist(routerId)));
    } catch (e) {
      pushFailure(e, "Could not read the blocked addresses.");
    }
  }, [routerId, pushFailure]);

  useEffect(() => {
    void load();
  }, [load]);

  async function onFile(file: File) {
    setImporting(true);
    setResult(null);
    try {
      const csv = await file.text();
      setResult(
        await (routerId === undefined ? importWalletBlocklist(csv) : importRouterBlocklist(routerId, csv)),
      );
      await load();
    } catch (e) {
      pushFailure(e, "Could not import that file.");
    } finally {
      setImporting(false);
    }
  }

  async function onRemove(address: string) {
    setRemoving(address);
    try {
      await (routerId === undefined
        ? removeWalletBlocklist([address])
        : removeRouterBlocklist(routerId, [address]));
      await load();
    } catch (e) {
      pushFailure(e, "Could not remove that address.");
    } finally {
      setRemoving(null);
    }
  }

  return (
    <Card className="border-line-strong">
      <div className="flex flex-wrap items-start justify-between gap-4 border-b border-line px-5 py-4">
        <div>
          <h2 className="font-header text-[14px] font-bold text-foreground">Blocked addresses</h2>
          <p className="mt-1 max-w-xl text-[11px] text-muted">
            Swaps funded from these addresses are refused. Applies to every {routerId === undefined ? "wallet" : "router"} in Portal.
          </p>
        </div>
        <div className="flex items-center gap-3">
          {entries !== null && (
            <span className="font-mono text-[11px] text-subtle">{entries.length} blocked</span>
          )}
          <input
            ref={fileInput}
            type="file"
            accept=".csv,.txt,text/csv,text/plain"
            className="hidden"
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) void onFile(file);
            }}
          />
          <Button size="sm" variant="secondary" loading={importing} onClick={() => fileInput.current?.click()}>
            <Upload size={14} strokeWidth={1.8} />
            Upload CSV
          </Button>
        </div>
      </div>

      <div className="flex flex-col gap-3 px-5 py-4">
        <p className="text-[11px] text-subtle">
          One address per line, optionally followed by a comma and a label.
        </p>

        {result && (
          <Notice
            tone={result.rejected.length ? "warning" : "success"}
            icon={result.rejected.length ? <AlertTriangle size={16} strokeWidth={2} /> : undefined}
          >
            <p>
              {result.added} added
              {result.updated > 0 && `, ${result.updated} already listed`}
              {result.rejected.length > 0 && `, ${result.rejected.length} skipped`}.
            </p>
            {result.rejected.slice(0, SHOWN_REJECTS).map((row) => (
              <p key={row.line} className="mt-1 break-all font-mono text-[10.5px] text-muted">
                Line {row.line}: {row.address || "(empty)"} — {row.reason}
              </p>
            ))}
            {result.rejected.length > SHOWN_REJECTS && (
              <p className="mt-1 text-[10.5px] text-muted">
                and {result.rejected.length - SHOWN_REJECTS} more.
              </p>
            )}
          </Notice>
        )}

        {entries !== null && entries.length === 0 && (
          <p className="flex items-center gap-2 text-[12px] text-muted">
            <Ban size={14} strokeWidth={1.8} className="text-subtle" />
            No addresses are blocked.
          </p>
        )}

        {entries !== null && entries.length > 0 && (
          <ul className="flex max-h-72 flex-col divide-y divide-line overflow-y-auto rounded-control border border-line">
            {entries.map((entry) => (
              <li key={entry.address} className="flex items-center justify-between gap-3 px-3.5 py-2.5">
                <span className="flex min-w-0 flex-col gap-0.5">
                  <Identifier value={entry.address} className="text-[11.5px] text-foreground" />
                  {entry.label && <span className="truncate text-[11px] text-subtle">{entry.label}</span>}
                </span>
                <IconButton
                  size="sm"
                  label="Remove"
                  tooltipAlign="right"
                  disabled={removing !== null}
                  onClick={() => void onRemove(entry.address)}
                  icon={<Trash2 size={14} strokeWidth={1.8} />}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}
