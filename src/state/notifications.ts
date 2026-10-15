import { create } from "zustand";
import type { IpcError } from "../lib/ipc";

export type ToastKind = "info" | "success" | "warning" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

interface NotificationState {
  toasts: Toast[];
  push: (kind: ToastKind, message: string, timeoutMs?: number) => void;
  dismiss: (id: number) => void;
}

let nextId = 1;

export const useNotifications = create<NotificationState>((set, get) => ({
  toasts: [],
  push: (kind, message, timeoutMs = kind === "error" ? 8000 : 4000) => {
    // Collapse identical messages instead of stacking duplicates.
    if (get().toasts.some((t) => t.message === message)) return;
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, kind, message }] }));
    if (timeoutMs > 0) setTimeout(() => get().dismiss(id), timeoutMs);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export function notify(kind: ToastKind, message: string) {
  useNotifications.getState().push(kind, message);
}

export function notifyError(e: IpcError | Error | unknown, prefix?: string) {
  const message =
    typeof e === "object" && e !== null && "message" in e ? String((e as { message: unknown }).message) : String(e);
  notify("error", prefix ? `${prefix}: ${message}` : message);
}
