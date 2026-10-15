// Promise-based modal prompts rendered by <PromptHost />.

import { create } from "zustand";

export interface PromptButton<T extends string> {
  id: T;
  label: string;
  kind?: "primary" | "danger" | "default";
}

export interface PromptRequest {
  title: string;
  message: string;
  detail?: string;
  buttons: PromptButton<string>[];
  /** Button id returned on Escape / backdrop click. */
  cancelId: string;
  resolve: (id: string) => void;
}

interface DialogState {
  prompt: PromptRequest | null;
  show: (req: PromptRequest) => void;
  close: (id: string) => void;
}

export const useDialogs = create<DialogState>((set, get) => ({
  prompt: null,
  show: (prompt) => {
    // A new prompt supersedes any open one (which resolves as cancelled).
    const open = get().prompt;
    if (open) open.resolve(open.cancelId);
    set({ prompt });
  },
  close: (id) => {
    const open = get().prompt;
    set({ prompt: null });
    open?.resolve(id);
  },
}));

export function ask<T extends string>(opts: {
  title: string;
  message: string;
  detail?: string;
  buttons: PromptButton<T>[];
  cancelId: T;
}): Promise<T> {
  return new Promise((resolve) => {
    useDialogs.getState().show({ ...opts, resolve: (id) => resolve(id as T) });
  });
}
