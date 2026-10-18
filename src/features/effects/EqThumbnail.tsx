import { useEffect, useRef } from "react";
import { effectsIpc } from "../../lib/effects";

/** The strip's equaliser curve in miniature, computed by the core. */
export function EqThumbnail({ params, bypass }: { params: Record<string, number>; bypass: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const key = JSON.stringify(params);
  useEffect(() => {
    let alive = true;
    effectsIpc
      .eqResponse(JSON.parse(key) as Record<string, number>, 64)
      .then((r) => {
        const el = ref.current;
        if (!alive || !el) return;
        const rect = el.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        el.width = Math.max(1, Math.round(rect.width * dpr));
        el.height = Math.max(1, Math.round(rect.height * dpr));
        const ctx = el.getContext("2d");
        if (!ctx) return;
        ctx.scale(dpr, dpr);
        const w = rect.width;
        const h = rect.height;
        ctx.fillStyle = "#16212d";
        ctx.fillRect(0, 0, w, h);
        ctx.strokeStyle = "#2b3e52";
        ctx.beginPath();
        ctx.moveTo(0, h / 2);
        ctx.lineTo(w, h / 2);
        ctx.stroke();
        ctx.beginPath();
        r.freqs.forEach((f, i) => {
          const x = (Math.log(f / 20) / Math.log(1000)) * w;
          const y = h / 2 - (Math.max(-18, Math.min(18, r.db[i] ?? 0)) / 18) * (h / 2 - 2);
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        });
        ctx.strokeStyle = bypass ? "#6b7c8c" : "#7cc4ff";
        ctx.lineWidth = 1.5;
        ctx.stroke();
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [key, bypass]);
  return <canvas ref={ref} className="eq-mini" aria-hidden />;
}
