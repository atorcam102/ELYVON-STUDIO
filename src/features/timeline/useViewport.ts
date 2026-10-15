import { useEffect, useState, type RefObject } from "react";

export interface Viewport {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Tracks the scroll position and size of a scroll container, coalescing
 * updates to one per animation frame.
 */
export function useViewport(ref: RefObject<HTMLElement | null>): Viewport {
  const [vp, setVp] = useState<Viewport>({ left: 0, top: 0, width: 0, height: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let frame = 0;
    const read = () => {
      frame = 0;
      setVp((prev) => {
        const next = { left: el.scrollLeft, top: el.scrollTop, width: el.clientWidth, height: el.clientHeight };
        return prev.left === next.left && prev.top === next.top && prev.width === next.width && prev.height === next.height
          ? prev
          : next;
      });
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(read);
    };
    read();
    el.addEventListener("scroll", schedule, { passive: true });
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", schedule);
      ro.disconnect();
      if (frame) cancelAnimationFrame(frame);
    };
  }, [ref]);
  return vp;
}
