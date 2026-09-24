/**
 * Copy that also works in the browser build.
 *
 * `navigator.clipboard` is exposed only in a secure context. The web target is served over
 * plain HTTP behind a proxy, and on a LAN address that is not one — the whole API is absent,
 * so a bare `navigator.clipboard.writeText(...)` threw on the first property access and every
 * copy button on the page silently did nothing. The hidden-textarea path is the only copy a
 * non-secure page has left.
 *
 * Returns whether the text actually reached the clipboard, so callers can say so rather than
 * claiming success they never confirmed.
 */
export async function copyText(text: string): Promise<boolean> {
  // Checked before awaiting anything: on the fallback path the `execCommand` below must still
  // be inside the click that triggered it, and an await of a rejected promise can spend it.
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Present but refused — a denied permission, or a document without focus.
    }
  }

  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  // Off-screen rather than hidden: neither `display:none` nor `visibility:hidden` can hold a
  // selection, and anything on-screen would scroll the page under the user.
  area.style.position = "fixed";
  area.style.top = "-9999px";
  document.body.appendChild(area);
  area.select();
  let copied = false;
  try {
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  }
  area.remove();
  return copied;
}
