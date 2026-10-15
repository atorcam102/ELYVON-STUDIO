import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

export interface TabDef<T extends string> {
  id: T;
  label: string;
  icon?: IconName;
  /** Small trailing marker, e.g. "Preview" or "Soon". */
  note?: string;
}

interface TabsProps<T extends string> {
  tabs: TabDef<T>[];
  value: T;
  onChange: (id: T) => void;
  label: string;
  trailing?: ReactNode;
  variant?: "underline" | "pill";
}

export function Tabs<T extends string>({ tabs, value, onChange, label, trailing, variant = "underline" }: TabsProps<T>) {
  return (
    <div className={`tabs tabs--${variant}`}>
      <div role="tablist" aria-label={label} className="tabs__list">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={t.id === value}
            className={`tabs__tab${t.id === value ? " is-active" : ""}`}
            onClick={() => onChange(t.id)}
            onKeyDown={(e) => {
              const i = tabs.findIndex((x) => x.id === value);
              const delta = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
              if (!delta) return;
              const next = tabs[(i + delta + tabs.length) % tabs.length];
              if (next) onChange(next.id);
            }}
          >
            {t.icon && <Icon name={t.icon} size={14} />}
            <span>{t.label}</span>
            {t.note && <span className="tabs__note">{t.note}</span>}
          </button>
        ))}
      </div>
      {trailing && <div className="tabs__trailing">{trailing}</div>}
    </div>
  );
}
