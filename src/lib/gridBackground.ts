// Grid lines as tiled 1px gradients. Tiling a small `linear-gradient` with
// `background-size` renders crisply in every webview; `repeating-linear-gradient`
// smears in WebKitGTK's compositing mode.

export interface GridLayer {
  /** Spacing in CSS pixels; layers with spacing < 2 are skipped. */
  every: number;
  color: string;
  axis: "x" | "y";
  /** Line length for x-axis layers (e.g. short ruler ticks); default full height. */
  length?: string;
}

export function gridBackground(layers: GridLayer[]) {
  const visible = layers.filter((l) => l.every >= 2);
  return {
    backgroundImage: visible
      .map((l) =>
        l.axis === "x"
          ? `linear-gradient(to right, ${l.color} 0 1px, transparent 1px)`
          : `linear-gradient(to bottom, transparent 0 calc(100% - 1px), ${l.color} calc(100% - 1px))`,
      )
      .join(", "),
    backgroundSize: visible.map((l) => (l.axis === "x" ? `${l.every}px ${l.length ?? "100%"}` : `100% ${l.every}px`)).join(", "),
    backgroundRepeat: visible.map((l) => (l.axis === "x" ? "repeat-x" : "repeat-y")).join(", "),
    backgroundPosition: visible.map(() => "left bottom").join(", "),
  };
}
