import { useRef, type PointerEvent } from "react";

interface SplitterProps {
  orientation: "vertical" | "horizontal";
  /** Current size of the panel being resized. */
  value: number;
  min: number;
  max: number;
  /** +1 when dragging right/down grows the panel, −1 when it shrinks it. */
  direction: 1 | -1;
  onChange: (value: number) => void;
  label: string;
}

/** Drag handle that resizes an adjacent panel. Keyboard: arrows (Shift = ×4). */
export function Splitter({ orientation, value, min, max, direction, onChange, label }: SplitterProps) {
  const start = useRef<{ pos: number; value: number } | null>(null);
  const clamp = (v: number) => Math.round(Math.min(max, Math.max(min, v)));

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    start.current = { pos: orientation === "vertical" ? e.clientX : e.clientY, value };
    document.body.classList.add(orientation === "vertical" ? "is-resizing-x" : "is-resizing-y");
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    const pos = orientation === "vertical" ? e.clientX : e.clientY;
    onChange(clamp(start.current.value + (pos - start.current.pos) * direction));
  };
  const end = () => {
    start.current = null;
    document.body.classList.remove("is-resizing-x", "is-resizing-y");
  };

  return (
    <div
      className={`splitter splitter--${orientation}`}
      role="separator"
      aria-orientation={orientation}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 40 : 10;
        const grow = orientation === "vertical" ? ["ArrowRight"] : ["ArrowDown"];
        const shrink = orientation === "vertical" ? ["ArrowLeft"] : ["ArrowUp"];
        if (grow.includes(e.key)) onChange(clamp(value + step * direction));
        else if (shrink.includes(e.key)) onChange(clamp(value - step * direction));
        else return;
        e.preventDefault();
      }}
    />
  );
}
