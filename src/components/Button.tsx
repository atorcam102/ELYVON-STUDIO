import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

type Variant = "default" | "primary" | "danger" | "ghost";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  icon?: IconName;
  size?: "sm" | "md";
  children?: ReactNode;
}

export function Button({ variant = "default", icon, size = "md", className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={["btn", `btn--${variant}`, `btn--${size}`, className].filter(Boolean).join(" ")}
      {...rest}
    >
      {icon && <Icon name={icon} size={14} />}
      {children && <span>{children}</span>}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: IconName;
  /** Accessible label, also used as tooltip. */
  label: string;
  active?: boolean;
  size?: number;
  tone?: "default" | "record" | "accent";
}

export function IconButton({ icon, label, active, size = 16, tone = "default", className, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={rest.title ?? label}
      aria-pressed={active === undefined ? undefined : active}
      className={["icon-btn", `icon-btn--${tone}`, active && "is-active", className].filter(Boolean).join(" ")}
      {...rest}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}

export function Toggle({ checked, onChange, label, description, disabled }: ToggleProps) {
  return (
    <label className={`toggle${disabled ? " is-disabled" : ""}`}>
      <span className="toggle__text">
        <span className="toggle__label">{label}</span>
        {description && <span className="toggle__desc">{description}</span>}
      </span>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle__track" aria-hidden />
    </label>
  );
}

export function Badge({ tone = "neutral", children }: { tone?: "neutral" | "warning" | "accent" | "demo"; children: ReactNode }) {
  return <span className={`badge badge--${tone}`}>{children}</span>;
}
