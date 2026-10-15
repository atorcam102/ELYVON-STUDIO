import { useNotifications } from "../state/notifications";
import { Icon } from "./Icon";

const ICONS = { info: "info", success: "check", warning: "warning", error: "warning" } as const;

export function Toasts() {
  const toasts = useNotifications((s) => s.toasts);
  const dismiss = useNotifications((s) => s.dismiss);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.kind}`}>
          <Icon name={ICONS[t.kind]} size={14} />
          <span className="toast__msg">{t.message}</span>
          <button type="button" className="toast__close" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
            <Icon name="close" size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
