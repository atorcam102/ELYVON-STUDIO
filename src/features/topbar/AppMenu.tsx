import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { menuData } from "./menuData";
import { activateMenuAction } from "./menuActions";

export function AppMenu() {
  const [open, setOpen] = useState<string | null>(null);
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const menu = ref.current?.querySelector<HTMLElement>(".desktop-menu");
    if (!menu) return;
    const keepInside = () => {
      menu.style.transform = "";
      const bounds = menu.getBoundingClientRect();
      const shift = Math.max(8 - bounds.left, Math.min(0, window.innerWidth - bounds.right - 8));
      menu.style.transform = `translateX(${shift}px)`;
    };
    keepInside();
    window.addEventListener("resize", keepInside);
    return () => window.removeEventListener("resize", keepInside);
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(null); };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") { setOpen(null); ref.current?.querySelector<HTMLButtonElement>(`[aria-expanded="true"]`)?.focus(); }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const rows = [...ref.current!.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')];
        const index = rows.indexOf(document.activeElement as HTMLButtonElement);
        rows[(index + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length]?.focus();
      }
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", key, true);
    return () => { window.removeEventListener("pointerdown", close); window.removeEventListener("keydown", key, true); };
  }, [open]);
  return <nav className="app-menubar" aria-label="Menú de aplicación" ref={ref}>
    <Icon name="project" size={16} />
    {Object.entries(menuData).map(([title, contents]) => <div className="app-menu-anchor" key={title}>
      <button type="button" className={open === title ? "is-open" : ""} aria-haspopup="menu" aria-expanded={open === title}
        onClick={() => setOpen(open === title ? null : title)} onPointerEnter={() => { if (open) setOpen(title); }}>{title}</button>
      {open === title && <div className="desktop-menu" role="menu" aria-label={title}>
        {contents.split("\n").map((row, i) => {
          if (row === "---") return <div className="desktop-menu__separator" role="separator" key={i} />;
          const [raw = "", action, shortcut] = row.split("|");
          const disabled = raw.startsWith("!");
          const checked = raw.startsWith("✓ ");
          const label = raw.replace(/^[!]|^✓ /g, "");
          return <button key={i} type="button" role="menuitem" disabled={disabled} onClick={() => { setOpen(null); activateMenuAction(action, label); }}>
            <span className="desktop-menu__check">{checked && <Icon name="check" size={14} />}</span>
            <span>{label}</span><span className="desktop-menu__shortcut">{shortcut === ">" ? <Icon name="chevronRight" size={13} /> : shortcut}</span>
          </button>;
        })}
      </div>}
    </div>)}
  </nav>;
}
