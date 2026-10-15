import { useEffect, useRef, useState, type KeyboardEvent } from "react";

interface InlineEditProps {
  value: string;
  onCommit: (value: string) => void;
  /** Starts in edit mode (e.g. after "Rename…"). */
  editing?: boolean;
  onEditingChange?: (editing: boolean) => void;
  className?: string;
  label: string;
  maxLength?: number;
}

/** Text that turns into an input on double-click. Enter commits, Escape cancels. */
export function InlineEdit({ value, onCommit, editing: editingProp, onEditingChange, className, label, maxLength = 128 }: InlineEditProps) {
  const [localEditing, setLocalEditing] = useState(false);
  const editing = editingProp ?? localEditing;
  const setEditing = (v: boolean) => {
    setLocalEditing(v);
    onEditingChange?.(v);
  };

  if (!editing) {
    return (
      <span className={`inline-edit ${className ?? ""}`} title={`${value} — double-click to rename`} onDoubleClick={() => setEditing(true)}>
        {value}
      </span>
    );
  }
  return (
    <InlineEditInput
      initial={value}
      label={label}
      maxLength={maxLength}
      className={className}
      onDone={(next) => {
        setEditing(false);
        if (next !== null && next !== value) onCommit(next);
      }}
    />
  );
}

function InlineEditInput({
  initial,
  label,
  maxLength,
  className,
  onDone,
}: {
  initial: string;
  label: string;
  maxLength: number;
  className?: string;
  onDone: (value: string | null) => void;
}) {
  const [draft, setDraft] = useState(initial);
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    ref.current?.select();
  }, []);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      ref={ref}
      className={`inline-edit__input ${className ?? ""}`}
      aria-label={label}
      value={draft}
      maxLength={maxLength}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => finish(draft.trim() || null)}
      onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(draft.trim() || null);
        if (e.key === "Escape") finish(null);
      }}
      onClick={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
    />
  );
}

interface NumberFieldProps {
  value: number;
  onCommit: (value: number) => void;
  min: number;
  max: number;
  step?: number;
  /** Decimal places shown. */
  precision?: number;
  label: string;
  suffix?: string;
  className?: string;
  disabled?: boolean;
}

/**
 * Compact numeric field. Click to type; ArrowUp/Down (Shift ×10) or vertical
 * drag to adjust. Commits once per gesture so each change is one undo step.
 */
export function NumberField({ value, onCommit, min, max, step = 1, precision = 0, label, suffix, className, disabled }: NumberFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [dragValue, setDragValue] = useState<number | null>(null);
  const drag = useRef<{ y: number; start: number; moved: boolean } | null>(null);
  const clamp = (v: number) => Math.min(max, Math.max(min, Math.round(v / step) * step));
  const shown = dragValue ?? value;

  const commit = (v: number) => {
    const c = clamp(v);
    if (Number.isFinite(c) && c !== value) onCommit(Number(c.toFixed(precision)));
  };

  if (draft !== null) {
    return (
      <input
        className={`number-field number-field--editing ${className ?? ""}`}
        aria-label={label}
        autoFocus
        inputMode="decimal"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onFocus={(e) => e.target.select()}
        onBlur={() => {
          const v = Number(draft.replace(",", "."));
          setDraft(null);
          if (draft.trim() !== "" && !Number.isNaN(v)) commit(v);
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setDraft(null);
        }}
      />
    );
  }

  return (
    <button
      type="button"
      className={`number-field ${className ?? ""}`}
      aria-label={`${label}: ${shown.toFixed(precision)}${suffix ? ` ${suffix}` : ""}`}
      title={`${label} — click to type, drag or use arrow keys to adjust`}
      disabled={disabled}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, start: value, moved: false };
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (!d) return;
        const dy = d.y - e.clientY;
        if (Math.abs(dy) > 2) d.moved = true;
        if (d.moved) setDragValue(clamp(d.start + Math.round(dy / 3) * step * (e.shiftKey ? 0.1 : 1)));
      }}
      onPointerUp={() => {
        const d = drag.current;
        drag.current = null;
        if (d?.moved && dragValue !== null) commit(dragValue);
        else setDraft(value.toFixed(precision));
        setDragValue(null);
      }}
      onKeyDown={(e) => {
        const delta = e.key === "ArrowUp" ? 1 : e.key === "ArrowDown" ? -1 : 0;
        if (delta) {
          e.preventDefault();
          commit(value + delta * step * (e.shiftKey ? 10 : 1));
        } else if (e.key === "Enter") {
          setDraft(value.toFixed(precision));
        }
      }}
    >
      <span className="number-field__value">{shown.toFixed(precision)}</span>
      {suffix && <span className="number-field__suffix">{suffix}</span>}
    </button>
  );
}
