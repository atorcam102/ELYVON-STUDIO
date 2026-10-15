// Platform conventions for shortcuts and labels.

const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";

export const isMac = /Mac OS X|Macintosh/.test(ua);
export const isWindows = /Windows/.test(ua);

/** The primary modifier: ⌘ on macOS, Ctrl elsewhere. */
export function hasPrimaryModifier(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/** Formats a shortcut such as "Mod+Shift+S" for display on this platform. */
export function shortcutLabel(spec: string): string {
  const parts = spec.split("+");
  if (isMac) {
    const map: Record<string, string> = { Mod: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃" };
    return parts.map((p) => map[p] ?? p).join("");
  }
  return parts.map((p) => (p === "Mod" ? "Ctrl" : p)).join("+");
}

/** Whether keyboard input currently targets a text field. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "range", "color"].includes(target.type);
  }
  return false;
}
