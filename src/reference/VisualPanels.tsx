import { useEffect, useRef, useState, type ComponentType } from "react";
import {
  CaretDown,
  Check,
  Guitar,
  Keyboard,
  PianoKeys,
  Play,
  SquaresFour,
  Waveform,
} from "@phosphor-icons/react";

type WaveProps = { seed: number; color?: string; small?: boolean };
type WaveComponent = ComponentType<WaveProps>;
const loopTracks = [
  { name: "Trap Door", icon: SquaresFour, color: "#b015e8" },
  { name: "Pumping Acid Bass", icon: Keyboard, color: "#9629d5" },
  { name: "Computations Topper", icon: Waveform, color: "#4e5bd4" },
];
function LoopNotes({ seed }: { seed: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const paint = () => {
      const { width, height } = canvas.getBoundingClientRect();
      const size = Math.min(width, height);
      const scale = window.devicePixelRatio || 1;
      canvas.width = width * scale;
      canvas.height = height * scale;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.scale(scale, scale);
      ctx.translate((width - size) / 2, (height - size) / 2);
      ctx.strokeStyle = "#e9b4ff";
      ctx.fillStyle = "#edb7ff";
      ctx.lineWidth = 0.8;
      const center = size / 2;
      for (let i = 0; i < 8; i++) {
        const radius = size * (0.14 + i * 0.035);
        ctx.globalAlpha = 0.12;
        ctx.beginPath();
        ctx.arc(center, center, radius, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = 0.8;
        for (let j = 0; j < 32; j++) {
          if ((j * (i + seed) + i * 3) % 11 > 5) continue;
          const angle = (j / 32) * Math.PI * 2;
          ctx.beginPath();
          ctx.arc(
            center,
            center,
            radius,
            angle,
            angle + 0.045 + (j % 3) * 0.015,
          );
          ctx.stroke();
        }
      }
    };
    paint();
    const observer = new ResizeObserver(paint);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [seed]);
  return (
    <canvas
      className="loop-notes"
      ref={ref}
      aria-label="Circular MIDI note preview"
    />
  );
}
export function LiveLoops({ wave: Wave }: { wave: WaveComponent }) {
  return (
    <section className="live-loops" aria-label="Live Loops">
      <div className="loop-track-list">
        <div className="loop-top-label">Chord</div>
        {loopTracks.map((t) => (
          <div className="loop-track" key={t.name}>
            <t.icon size={28} style={{ color: t.color }} weight="fill" />
            <div>
              <strong>{t.name}</strong>
              <small>
                M&nbsp; S&nbsp; R <span className="loop-volume" />
              </small>
            </div>
          </div>
        ))}
      </div>
      <div className="loop-matrix">
        <div className="loop-matrix-heading">
          Quantize Start:{" "}
          <span>
            1 Bar <CaretDown size={8} />
          </span>
        </div>
        <div className="loop-tiles">
          {loopTracks.map((t, row) => (
            <div className="loop-tile-row" key={t.name}>
              {Array.from({ length: 4 }, (_, col) =>
                row + col < 5 ? (
                  <div
                    key={col}
                    className={`loop-tile row-${row}`}
                    style={{
                      background:
                        row === 0
                          ? col === 0
                            ? "#ae37d8"
                            : "#a000cd"
                          : row === 1
                            ? "#8220c6"
                            : "#3f4ac6",
                    }}
                  >
                    <span>
                      {row === 0
                        ? `Trap Beat ${col + 1}`
                        : row === 1
                          ? `Pumping Acid Bass 0${col + 1}`
                          : "Computations Topper"}
                    </span>
                    {row < 2 ? (
                      <LoopNotes seed={col + 3 + row} />
                    ) : (
                      <Wave seed={col + 26} />
                    )}
                  </div>
                ) : (
                  <div className="loop-tile vacant" key={col} />
                ),
              )}
            </div>
          ))}
        </div>
        <div className="scene-launchers">
          {[1, 2, 3, 4].map((n) => (
            <span key={n}>
              <Play size={8} weight="fill" />{" "}
              {n === 1
                ? "Intro"
                : n === 2
                  ? "Verse"
                  : n === 3
                    ? "Hook"
                    : "Breakdown"}
            </span>
          ))}
        </div>
      </div>
      <div className="loop-arrangement">
        <div className="loop-time">
          <span>1</span>
          <span>1 3</span>
          <span>2</span>
          <span>2 3</span>
        </div>
        <div className="loop-chords">
          Dm <span>C</span>
        </div>
        <div className="loop-midi-notes">
          {Array.from({ length: 32 }, (_, n) => (
            <i
              key={n}
              style={{ left: `${n * 3}%`, top: `${12 + ((n * 37) % 65)}%` }}
            />
          ))}
        </div>
        <div className="loop-bass">
          <span>Bass Player - Pump Bass</span>
          {Array.from({ length: 13 }, (_, n) => (
            <i
              key={n}
              style={{ left: `${n * 8}%`, top: `${30 + ((n * 31) % 50)}%` }}
            />
          ))}
        </div>
        <div className="loop-audio">
          <span>Computations Topper</span>
          <Wave seed={42} />
        </div>
      </div>
    </section>
  );
}
const chordTracks = [
  {
    name: "Piano",
    region: "Piano",
    color: "#5738a8",
    icon: PianoKeys,
    seed: 31,
  },
  {
    name: "Platinum Cuts",
    region: "Drummer - Modern Hip Hop",
    color: "#385eac",
    icon: SquaresFour,
    seed: 22,
  },
  {
    name: "Simple Foundation",
    region: "Bass Player - Pop Songwriter",
    color: "#2188ab",
    icon: Guitar,
    seed: 17,
  },
  {
    name: "Majestic Rise Pad",
    region: "Keyboard Player - Modulated Pad",
    color: "#00aaa8",
    icon: Keyboard,
    seed: 12,
  },
];
export function ChordArrangement({ wave: Wave }: { wave: WaveComponent }) {
  const [popup, setPopup] = useState(true);
  return (
    <section className="chord-arrangement" aria-label="Instrument arrangement">
      <div className="chord-sidebar">
        <div className="chord-sidebar-heading">Chord</div>
        {chordTracks.map((t) => (
          <div className="instrument-track" key={t.name}>
            <div>
              <strong>{t.name}</strong>
              <small>
                M&nbsp; S&nbsp; R <span className="loop-volume" />
              </small>
            </div>
            <t.icon size={68} style={{ color: t.color }} weight="fill" />
          </div>
        ))}
      </div>
      <div className="chord-timeline">
        <div className="chord-time">
          {[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => (
            <span key={n}>{n}</span>
          ))}
        </div>
        <div className="chord-regions">
          {["Bm7", "G", "Gmaj7", "F#m7", "Bm7", "G", "Gmaj7", "F#m7"].map(
            (c, i) => (
              <button
                className={i === 2 ? "selected" : ""}
                key={i}
                onClick={() => setPopup(!popup)}
              >
                {c}
              </button>
            ),
          )}
        </div>
        <div className="instrument-regions">
          {chordTracks.map((t, i) => (
            <div
              className={`instrument-region instrument-${i}`}
              key={t.name}
              style={{ background: t.color }}
            >
              <span>
                {t.region}
                <span>{t.region}</span>
              </span>
              {i === 2 ? (
                <div className="bass-note-chart">
                  {Array.from({ length: 35 }, (_, n) => (
                    <i
                      key={n}
                      style={{
                        left: `${n * 3}%`,
                        top: `${15 + ((n * 31) % 60)}%`,
                        width: `${1 + (n % 3)}%`,
                      }}
                    />
                  ))}
                </div>
              ) : (
                <>
                  <Wave
                    seed={t.seed}
                    small={i === 0}
                    color={i === 3 ? "#8af2e3" : "#c6d5ff"}
                  />
                  {i === 0 && <Wave seed={t.seed} small color="#c6d5ff" />}
                </>
              )}
            </div>
          ))}
        </div>
        {popup && (
          <div className="chord-popup">
            <div className="chord-field">
              Chord <button>Bm7</button>
            </div>
            <label>
              Root Note{" "}
              <span>
                B <CaretDown size={8} />
              </span>
            </label>
            <div className="chord-buttons">
              {["Maj", "Min", "Sus2", "Sus4", "5", "Aug", "Dim"].map((c) => (
                <button key={c} className={c === "Min" ? "selected" : ""}>
                  {c}
                </button>
              ))}
            </div>
            <div className="chord-buttons extensions">
              {[
                "♭5",
                "#5",
                "6",
                "7",
                "maj7",
                "♭9",
                "9",
                "#9",
                "11",
                "#11",
                "♭13",
                "13",
              ].map((c) => (
                <button key={c} className={c === "7" ? "selected" : ""}>
                  {c}
                </button>
              ))}
            </div>
            <label>
              Bass Note{" "}
              <span>
                None <CaretDown size={8} />
              </span>
            </label>
            <label>
              Scale{" "}
              <span>
                Dorian <CaretDown size={8} />
              </span>
            </label>
            <div className="scale-options">
              {[
                "Auto (Natural Minor / Aeolian)",
                "Natural Minor (Aeolian)",
                "Dorian",
                "Phrygian",
                "Dorian ♭2 (MM2)",
                "Minor Blues",
                "Minor Pentatonic",
                "Bebop Harmonic Minor",
              ].map((c) => (
                <button key={c}>
                  {c === "Dorian" ? <Check size={9} /> : <span />}
                  {c}
                </button>
              ))}
            </div>
            <label>MIDI Input</label>
            <label>Preview</label>
          </div>
        )}
      </div>
    </section>
  );
}
