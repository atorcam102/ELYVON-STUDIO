import { useState } from "react";

/** State helper for menus opened from a click position or anchor element. */
export function useMenu() {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  return {
    at,
    openAt: (x: number, y: number) => setAt({ x, y }),
    openBelow: (el: HTMLElement) => {
      const r = el.getBoundingClientRect();
      setAt({ x: r.left, y: r.bottom + 4 });
    },
    close: () => setAt(null),
  };
}
