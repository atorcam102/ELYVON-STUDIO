/** Keep a stop at the minimum; a deliberate extra drag collapses the panel. */
export function panelResize(raw: number, min: number, max: number, collapseDistance?: number) {
  return {
    size: Math.round(Math.min(Math.max(min, max), Math.max(min, raw))),
    collapsed: collapseDistance !== undefined && raw <= min - collapseDistance,
  };
}
