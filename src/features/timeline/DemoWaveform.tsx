import { useEffect, useRef } from "react";

/** An explicitly labelled illustration for demo regions with no audio file. */
export function DemoWaveform({ id }: { id: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const seed = Array.from(id).reduce(
      (sum, letter) => sum + letter.charCodeAt(0),
      0,
    );
    const draw = () => {
      const { width, height } = element.getBoundingClientRect();
      if (!width || !height) return;
      const ratio = window.devicePixelRatio || 1;
      element.width = Math.ceil(width * ratio);
      element.height = Math.ceil(height * ratio);
      const context = element.getContext("2d");
      if (!context) return;
      context.scale(ratio, ratio);
      context.fillStyle = "#c1e7fa";
      let state = seed;
      const random = () => {
        state = (state * 1664525 + 1013904223) >>> 0;
        return state / 4294967296;
      };
      const syllables = Array.from({ length: 26 }, (_, i) => ({
        at: (i + random() * 0.8) / 26,
        level: 0.25 + random() * 0.7,
        decay: 0.004 + random() * 0.014,
      }));
      for (let x = 0; x < width; x++) {
        const t = x / width;
        let envelope = 0.02;
        for (const syllable of syllables) {
          const elapsed = t - syllable.at;
          if (elapsed >= 0 && elapsed < 0.1)
            envelope +=
              syllable.level *
              (1 - Math.exp(-elapsed / 0.002)) *
              Math.exp(-elapsed / syllable.decay);
        }
        const amplitude = Math.min(
          height * 0.46,
          0.3 + envelope * (0.4 + random() * 0.6) * height,
        );
        context.fillRect(x, height / 2 - amplitude, 1, amplitude * 2);
      }
    };
    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(element);
    return () => observer.disconnect();
  }, [id]);
  return (
    <canvas ref={canvas} className="region__demo-waveform" aria-hidden="true" />
  );
}
