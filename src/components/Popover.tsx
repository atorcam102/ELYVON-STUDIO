import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "./Icon";

export interface MenuItem {
  id: string;
  label: string;
  icon?: IconName;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  onSelect: () => void;
}

export type MenuEntry = MenuItem | "separator" | { heading: string } | { custom: ReactNode; key: string };

interface MenuProps {
  /** Screen position (e.g. from a click or the anchor's bounding box). */
  at: { x: number; y: number };
  entries: MenuEntry[];
  onClose: () => void;
}

/** Context/dropdown menu rendered in a portal, kept inside the viewport. */
export function Menu({ at, entries, onClose }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.min(at.x, window.innerWidth - r.width - 8),
      y: at.y + r.height > window.innerHeight - 8 ? Math.max(8, at.y - r.height) : at.y,
    });
    el.querySelector<HTMLElement>("[role=menuitem]:not(:disabled)")?.focus();
  }, [at]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const items = Array.from(ref.current?.querySelectorAll<HTMLElement>("[role=menuitem]:not(:disabled)") ?? []);
        const i = items.indexOf(document.activeElement as HTMLElement);
        const next = items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length];
        next?.focus();
      }
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div ref={ref} className="menu" role="menu" style={{ left: pos.x, top: pos.y }}>
      {entries.map((entry, i) => {
        if (entry === "separator") return <div key={`sep-${i}`} className="menu__sep" role="separator" />;
        if ("heading" in entry) return <div key={`h-${entry.heading}`} className="menu__heading">{entry.heading}</div>;
        if ("custom" in entry) return <div key={entry.key} className="menu__custom">{entry.custom}</div>;
        return (
          <button
            key={entry.id}
            type="button"
            role="menuitem"
            className={`menu__item${entry.danger ? " is-danger" : ""}`}
            disabled={entry.disabled}
            onClick={() => {
              onClose();
              entry.onSelect();
            }}
          >
            <span className="menu__icon">{entry.icon && <Icon name={entry.icon} size={14} />}</span>
            <span className="menu__label">{entry.label}</span>
            {entry.shortcut && <span className="menu__shortcut">{entry.shortcut}</span>}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}
