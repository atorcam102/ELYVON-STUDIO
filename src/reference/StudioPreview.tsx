import React, { useEffect, useRef, useState } from "react";
import {
  Archive,
  ArrowCounterClockwise,
  ArrowUUpLeft,
  ArrowsHorizontal,
  CaretDown,
  CaretLeft,
  CaretRight,
  Circle,
  Clock,
  Copy,
  Faders,
  FastForward,
  GearSix,
  GridFour,
  Guitar,
  Headphones,
  Info,
  Keyboard,
  List,
  MagnifyingGlass,
  Metronome,
  Microphone,
  MusicNotes,
  Play,
  Plus,
  Repeat,
  Scissors,
  SkipBack,
  SlidersHorizontal,
  SquaresFour,
  Stop,
  Users,
  Waveform as WaveIcon,
  X,
} from "@phosphor-icons/react";
import type { Icon } from "@phosphor-icons/react";
import "./styles.css";
import { ChordArrangement, LiveLoops } from "./VisualPanels";

type IconButtonProps = {
  icon: Icon;
  label: string;
  active?: boolean;
  onClick?: () => void;
  className?: string;
};
function IconButton({
  icon: Icon,
  label,
  active,
  onClick,
  className = "",
}: IconButtonProps) {
  return (
    <button
      className={`icon-button ${active ? "active" : ""} ${className}`}
      title={label}
      aria-label={label}
      aria-pressed={onClick ? !!active : undefined}
      onClick={onClick}
    >
      <Icon size={16} weight="fill" />
    </button>
  );
}
function Menu({
  children,
  className = "",
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <button className={`menu-button ${className}`}>
      {children}
      <CaretDown size={9} />
    </button>
  );
}
const colors = [
  "#14a6cc",
  "#138bbd",
  "#2674bc",
  "#344ab8",
  "#5340c7",
  "#345cc5",
  "#227ec0",
  "#178bb8",
  "#1f96b5",
  "#06b5bd",
  "#039fbb",
  "#167da8",
  "#367cbe",
  "#4848c4",
  "#5640bf",
  "#355ac0",
  "#1786b9",
  "#0b9aba",
  "#1aafba",
  "#159bb5",
];
const channelNames = [
  "Vocal Textures",
  "Distant Vocals",
  "Near Vocals",
  "Distant Harmonies",
  "Main Vocal",
  "Backing Vocal",
  "Harmony Vocal",
  "Choir",
  "Room Mic",
  "Top Line",
  "Tenor",
  "Vocoder",
  "Sample",
  "Dark Synth Pad",
  "Custom Soft Piano",
  "Night of Avalon",
  "Lost Reverse",
  "String Vox",
  "Moonlight Ark",
  "Ocean Bass",
];
const instruments = [
  "Alchemy",
  "Sampler",
  "RetroSyn",
  "Alchemy",
  "Q-Sampler",
  "RetroSyn",
  "Alchemy",
  "Sampler",
];
const fxLists = [
  ["Cnsl EQ", "Channel EQ", "Space D"],
  ["Noise Gate", "Channel EQ", "Compressor", "Space D"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Cnsl EQ", "Compressor", "Delay D"],
  ["Noise Gate", "Graph EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Tube EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Cnsl EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay", "Tremolo"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay"],
  ["Noise Gate", "Noise Gate", "Channel EQ", "Compressor"],
  ["Noise Gate", "Channel EQ", "Compressor", "St-Delay"],
  ["Cnsl EQ", "Beat Break"],
  ["Cnsl EQ", "Compressor", "Bitcrusher"],
  ["Cnsl EQ", "SilverVerb", "Channel EQ"],
  ["Cnsl EQ", "Channel EQ", "Step FX"],
  ["Cnsl EQ", "Multipr"],
  ["Cnsl EQ", "Ensemble", "St-Delay"],
  ["Cnsl EQ", "Tube EQ", "Compressor"],
];
const tracks = [
  {
    name: "Verse Vocals",
    region: "Verse Vocals",
    color: "#3b51c1",
    icon: Microphone,
    start: 0,
    width: 65,
    seed: 5,
  },
  {
    name: "VerseHarmony",
    region: "VerseHarmony",
    color: "#2a75c5",
    icon: Microphone,
    start: 0,
    width: 84,
    seed: 11,
  },
  {
    name: "Vocal Stack",
    region: "Vocal Stack",
    color: "#2d474c",
    icon: Users,
    start: 25,
    width: 62,
    seed: 0,
  },
  {
    name: "Lead Vocal",
    region: "Lead Vocal",
    color: "#129ab9",
    icon: Microphone,
    start: 0,
    width: 78,
    seed: 17,
  },
  {
    name: "Vocal Double",
    region: "Vocal Double",
    color: "#0db5b8",
    icon: Microphone,
    start: 0,
    width: 79,
    seed: 24,
  },
];
function Waveform({
  seed,
  color = "#b4e5ff",
  small = false,
}: {
  seed: number;
  color?: string;
  small?: boolean;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const paint = () => {
      const { width, height } = canvas.getBoundingClientRect();
      const ratio = window.devicePixelRatio || 1;
      canvas.width = width * ratio;
      canvas.height = height * ratio;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.scale(ratio, ratio);
      ctx.fillStyle = color;
      const center = height / 2;
      let randomState = seed * 127;
      const random = () => {
        randomState = (randomState * 1664525 + 1013904223) >>> 0;
        return randomState / 4294967296;
      };
      const syllables = Array.from({ length: 29 }, (_, i) => ({
        start: (i + random() * 0.7) / 29,
        attack: 0.001 + random() * 0.003,
        decay: 0.003 + random() * 0.014,
        level: 0.13 + random() * 0.85,
      }));
      for (let x = 0; x < width; x += 1) {
        const t = x / width;
        let envelope = 0.012;
        for (const syllable of syllables) {
          const d = t - syllable.start;
          if (d >= 0 && d < syllable.decay * 5)
            envelope +=
              syllable.level *
              (1 - Math.exp(-d / syllable.attack)) *
              Math.exp(-d / syllable.decay);
        }
        const texture = 0.38 + random() * 0.62;
        const amp = Math.min(
          center - 1,
          0.4 + envelope * texture * height * (small ? 0.26 : 1.15),
        );
        ctx.fillRect(x, center - amp, 1, amp * 2);
      }
    };
    paint();
    const observer = new ResizeObserver(paint);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [seed, color, small]);
  return (
    <canvas
      ref={ref}
      className="waveform"
      aria-label="Audio waveform visualisation"
    />
  );
}
function Transport({
  library,
  setLibrary,
  mixer,
  setMixer,
  sequencer,
  setSequencer,
}: {
  library: boolean;
  setLibrary: (x: boolean) => void;
  mixer: boolean;
  setMixer: (x: boolean) => void;
  sequencer: boolean;
  setSequencer: (x: boolean) => void;
}) {
  return (
    <>
      <div className="window-title">
        <div className="traffic-lights">
          <i />
          <i />
          <i />
        </div>
        <span>
          ELYVON STUDIO <span className="title-divider">—</span> Untitled
        </span>
        <div className="window-detail">
          <MusicNotes size={11} /> Tracks
        </div>
      </div>
      <header className="transport">
        <div className="transport-tools">
          <div className="button-segment">
            <IconButton
              icon={Archive}
              label="Library"
              active={library}
              onClick={() => setLibrary(!library)}
            />
            <IconButton icon={Info} label="Inspector" />
            <IconButton icon={Clock} label="Quick Help" />
            <IconButton icon={Copy} label="Toolbar" />
          </div>
          <div className="button-segment second">
            <IconButton icon={GearSix} label="Smart Controls" />
            <IconButton
              icon={Faders}
              label="Mixer"
              active={mixer}
              onClick={() => setMixer(!mixer)}
            />
            <IconButton
              icon={Scissors}
              label="Editors"
              active={sequencer}
              onClick={() => setSequencer(!sequencer)}
            />
          </div>
        </div>
        <div className="transport-center">
          <div className="playback">
            <IconButton icon={SkipBack} label="Rewind" />
            <IconButton icon={FastForward} label="Forward" />
            <IconButton icon={Stop} label="Stop" />
            <IconButton icon={Play} label="Play" className="play-button" />
            <IconButton
              icon={Circle}
              label="Record"
              className="record-button"
            />
            <IconButton icon={Repeat} label="Cycle" />
          </div>
          <div className="lcd">
            <div className="position">
              <strong>01:00:52:11.52</strong>
              <span>032 3 4 42</span>
            </div>
            <div className="lcd-location">
              <span>060 1 1 001</span>
              <span>062 1 1 001</span>
            </div>
            <div className="tempo">
              <strong>145.0000</strong>
              <span>Keep Tempo</span>
            </div>
            <div className="signature">
              <strong>4/4</strong>
              <span>/16</span>
            </div>
            <div className="midi">
              <span>No In</span>
              <span>No Out</span>
            </div>
            <CaretDown size={10} />
          </div>
          <div className="button-segment timing">
            <IconButton icon={Metronome} label="Metronome" />
            <span className="count-in">1234</span>
          </div>
        </div>
        <div className="transport-right">
          <div className="master-output">
            <div />
            <div />
          </div>
          <div className="button-segment">
            <IconButton icon={List} label="Lists" />
            <IconButton icon={Keyboard} label="Musical Typing" />
            <IconButton icon={Headphones} label="Loop Browser" />
            <IconButton icon={MusicNotes} label="Media Browser" />
          </div>
        </div>
      </header>
    </>
  );
}
function TrackHeader({
  track,
  index,
}: {
  track: (typeof tracks)[number];
  index: number;
}) {
  const Icon = track.icon;
  return (
    <div className={`track-header ${index === 0 ? "selected" : ""}`}>
      <span className="track-number">{index + 1}</span>
      <Icon
        className="track-instrument"
        size={24}
        weight="fill"
        style={{ color: track.color }}
      />
      <div className="track-label">
        <strong>{track.name}</strong>
        <div className="track-switches">
          <button>M</button>
          <button>S</button>
          {index === 0 && <button className="record-arm">R</button>}
          <div className="track-volume">
            <i />
          </div>
        </div>
      </div>
      <i className="track-indicator" />
    </div>
  );
}
function Arrangement() {
  return (
    <section className="arrangement" aria-label="Track arrangement">
      <div className="arrangement-side">
        <div className="track-list-toolbar">
          <IconButton icon={Plus} label="Add track" />
          <IconButton icon={Copy} label="Duplicate track" />
          <span />
          <IconButton icon={SquaresFour} label="Track settings" />
        </div>
        {tracks.map((t, i) => (
          <TrackHeader key={t.name} track={t} index={i} />
        ))}
      </div>
      <div className="arrangement-scroll">
        <div className="ruler">
          <div className="bar-numbers">
            {Array.from({ length: 12 }, (_, i) => (
              <span key={i}>{27 + i * 2}</span>
            ))}
          </div>
          <div className="section-markers">
            <span>Chorus 1</span>
            <span>Verse 2</span>
          </div>
        </div>
        <div className="track-regions">
          {tracks.map((track, i) => (
            <div className="track-lane" key={track.name}>
              {track.seed !== 0 ? (
                <>
                  <div
                    className="audio-region"
                    style={{
                      background: track.color,
                      left: `${track.start}%`,
                      width: `${track.width}%`,
                    }}
                  >
                    <div className="region-title">
                      {track.region}
                      <span>
                        {i === 1
                          ? "VerseHarmony  ◦"
                          : i === 3
                            ? "Lead Vocal  ◦"
                            : ""}
                      </span>
                    </div>
                    <Waveform
                      seed={track.seed}
                      color={i > 2 ? "#a4f5f1" : "#b6d9ff"}
                    />
                  </div>
                  {i === 0 && (
                    <div
                      className="audio-region secondary-region"
                      style={{
                        background: track.color,
                        left: "80%",
                        width: "35%",
                      }}
                    >
                      <div className="region-title">
                        A Verse Vocals: 11 - Comp A
                      </div>
                      <Waveform seed={13} />
                    </div>
                  )}
                </>
              ) : (
                <>
                  <span className="empty-stack-label">Vocal Stack</span>
                  <span className="empty-stack-label second-label">
                    Vocal Stack
                  </span>
                </>
              )}
            </div>
          ))}
          <div className="playhead">
            <i />
          </div>
        </div>
      </div>
    </section>
  );
}
function Channel({ index }: { index: number }) {
  const [muted, setMuted] = useState(false);
  const [solo, setSolo] = useState(false);
  return (
    <div
      className={`channel ${index === 0 ? "channel-selected" : ""}`}
      style={{ "--channel-color": colors[index] } as React.CSSProperties}
    >
      <div className="channel-input">
        {index < 13 ? (
          <span>
            <span className="input-icon">⊙</span>
            {index === 0 ? "Bus 7" : "Input"}
          </span>
        ) : (
          <span className="instrument-input">{instruments[index - 13]}</span>
        )}
      </div>
      <div className="effects-rack">
        {fxLists[index]?.map((fx, i) => (
          <button key={i} className="effect-slot" title={fx}>
            {fx}
          </button>
        ))}
      </div>
      <div className="routing">
        <button>
          {index === 16 ? "Bus 7" : index % 6 === 4 ? "Object" : "Surround"}
        </button>
        <button className="group-name">
          {index < 13 ? "1: Vocals" : "2: Keyboards"}
        </button>
      </div>
      <div className="pan-section">
        <div className={index % 6 === 4 ? "surround-pan" : "pan-knob"}>
          <i style={{ transform: `rotate(${((index % 5) - 2) * 18}deg)` }} />
        </div>
      </div>
      <div className="channel-values">
        <span>{(-14.3 - index * 0.63).toFixed(1)}</span>
        <span>{(-2.3 - index * 0.41).toFixed(1)}</span>
      </div>
      <div className="fader-section">
        <div className="db-scale">
          {[
            "6",
            "0",
            "−6",
            "−12",
            "−18",
            "−24",
            "−30",
            "−40",
            "−50",
            "−60",
          ].map((n) => (
            <span key={n}>{n}</span>
          ))}
        </div>
        <div className="level-meter">
          <div
            className="meter-fill"
            style={{ height: `${22 + ((index * 17) % 66)}%` }}
          />
          <div className="meter-ticks" />
        </div>
        <div className="fader-track">
          <div className="fader-ticks" />
          <input
            aria-label={`${channelNames[index]} volume`}
            type="range"
            min="0"
            max="100"
            defaultValue={70 - ((index * 7) % 35)}
          />
        </div>
      </div>
      <div className="automation">
        <button>R</button>
        <button>I</button>
      </div>
      <div className="channel-switches">
        <button
          className={muted ? "on" : ""}
          aria-pressed={muted}
          onClick={() => setMuted(!muted)}
        >
          M
        </button>
        <button
          className={solo ? "solo" : ""}
          aria-pressed={solo}
          onClick={() => setSolo(!solo)}
        >
          S
        </button>
      </div>
      <button className="channel-name">
        {channelNames[index]?.split(" ").map((word, i) => (
          <React.Fragment key={i}>
            {word}
            {i === 0 ? <br /> : " "}
          </React.Fragment>
        ))}
      </button>
    </div>
  );
}
function Mixer() {
  const [filter, setFilter] = useState("Tracks");
  return (
    <section className="mixer" aria-label="Mixer">
      <div className="mixer-toolbar">
        <div className="mixer-menu">
          <IconButton icon={ArrowUUpLeft} label="Back" />
          <Menu>Edit</Menu>
          <Menu>Options</Menu>
          <Menu>View</Menu>
          <span className="sends-label">Sends on Faders:</span>
          <button className="power-button">⏻</button>
          <Menu>Off</Menu>
        </div>
        <div className="segmented mixer-scope">
          {["Single", "Tracks", "All"].map((t) => (
            <button
              className={filter === t ? "selected" : ""}
              onClick={() => setFilter(t)}
              key={t}
            >
              {t}
            </button>
          ))}
        </div>
        <div className="mixer-filters">
          {[
            "Audio",
            "Inst",
            "Aux",
            "Bus",
            "Input",
            "Output",
            "Master/VCA",
            "MIDI",
          ].map((t) => (
            <button key={t}>{t}</button>
          ))}
        </div>
        <IconButton icon={SquaresFour} label="Channel strip components" />
      </div>
      <div className="mixer-scroll">
        <div className="mixer-labels">
          <span>Input</span>
          <span>Audio FX</span>
          <span>Output</span>
          <span>Group</span>
          <span>Pan</span>
          <span>dB</span>
        </div>
        <div className="channels">
          {channelNames.map((name, i) => (
            <Channel key={name} index={i} />
          ))}
        </div>
      </div>
    </section>
  );
}
const libraryCategories = [
  "User Channel Strip Settings",
  "Bass",
  "Acoustic Drums",
  "Electronic Drum Kit",
  "Percussion",
  "Guitar",
  "Clavinet",
  "Electric Piano",
  "Mellotron",
  "Organ",
  "Piano",
  "Studio Horns",
  "Studio Strings",
  "Synthesizer",
  "Orchestral",
  "World",
];
function Library({ close }: { close: () => void }) {
  return (
    <aside className="library">
      <div className="library-heading">
        Library <Menu>All Sounds</Menu>
        <IconButton icon={X} label="Close library" onClick={close} />
      </div>
      <div className="library-patch">
        <Guitar size={94} weight="fill" />
        <strong>Phasing Guitar Plucks</strong>
      </div>
      <label className="library-search">
        <MagnifyingGlass size={13} />
        <input placeholder="Search Sounds" aria-label="Search Sounds" />
      </label>
      <div className="library-columns">
        <div>
          {libraryCategories.map((name) => (
            <button className={name === "Guitar" ? "selected" : ""} key={name}>
              {name}
              <CaretRight size={10} />
            </button>
          ))}
        </div>
        <div>
          {[
            "Acoustic Guitar",
            "Classical Acoustic Guitar",
            "Hard Rock",
            "Processed",
          ].map((name) => (
            <button key={name}>
              {name}
              <CaretRight size={10} />
            </button>
          ))}
        </div>
      </div>
      <div className="library-bottom">
        <button>
          <Archive size={12} /> Get More Sounds
        </button>
        <div>
          <button>Revert</button>
          <button>Delete</button>
          <button>Save…</button>
        </div>
      </div>
    </aside>
  );
}
function StepSequencer() {
  const names = [
    "Kick 1",
    "Kick 2",
    "Snare 1",
    "Rim",
    "Clap 1",
    "Hi-Hat 1",
    "Hi-Hat 2",
    "Velocity",
    "Note Repeat",
    "Chance",
  ];
  return (
    <section className="sequencer">
      <div className="step-toolbar">
        <Menu>Edit</Menu>
        <Menu>Functions</Menu>
        <Menu>View</Menu>
        <div className="segmented">
          <button className="selected">On/Off</button>
          <button>Velocity / Value</button>
        </div>
        <span />
        <Menu>32 Steps</Menu>
      </div>
      <div className="step-heading">Trap Beat 1</div>
      <div className="step-rows">
        {names.map((name, row) => (
          <div
            className="step-row"
            key={name}
            style={
              {
                "--step-color": [
                  "#4669de",
                  "#04a2c6",
                  "#9420c8",
                  "#a022db",
                  "#a12be4",
                  "#00c5a1",
                  "#00a8be",
                ][row % 7],
              } as React.CSSProperties
            }
          >
            <div className="step-name">
              <WaveIcon size={16} />
              <span>
                {name}
                <small>M S</small>
              </span>
              <span className="step-options">⌄</span>
            </div>
            <div className="step-cells">
              {Array.from({ length: 32 }, (_, col) => (
                <div
                  className={`step-cell ${(col * (row + 3) + row * 7) % 11 < 3 ? "lit" : ""} ${col === 10 ? "current" : ""}`}
                  key={col}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
export default function StudioPreview() {
  const [library, setLibrary] = useState(false);
  const [mixer, setMixer] = useState(true);
  const [sequencer, setSequencer] = useState(false);
  const [view, setView] = useState("tracks");
  return (
    <main className="studio">
      <Transport
        {...{ library, setLibrary, mixer, setMixer, sequencer, setSequencer }}
      />
      <div className="studio-body">
        {library && <Library close={() => setLibrary(false)} />}
        <div className="main-workspace">
          <nav className="workspace-toolbar" aria-label="Arrangement controls">
            <IconButton icon={ArrowCounterClockwise} label="Undo" />
            <Menu>Edit</Menu>
            <Menu>Functions</Menu>
            <Menu>View</Menu>
            <div className="view-buttons">
              <IconButton
                icon={GridFour}
                label="Live Loops"
                active={view === "loops"}
                onClick={() => {
                  setView("loops");
                  setSequencer(true);
                  setMixer(false);
                }}
              />
              <IconButton
                icon={List}
                label="Tracks view"
                active={view === "tracks"}
                onClick={() => {
                  setView("tracks");
                  setSequencer(false);
                  setMixer(true);
                }}
              />
              <IconButton icon={ArrowsHorizontal} label="Automation" />
              <IconButton
                icon={MusicNotes}
                label="Chord track"
                active={view === "chords"}
                onClick={() => {
                  setView(view === "chords" ? "tracks" : "chords");
                  setSequencer(false);
                  setMixer(view === "chords");
                }}
              />
            </div>
            <div className="tool-selector">
              <Menu>
                <span>↖</span>
              </Menu>
              <Menu>
                <Plus size={12} />
              </Menu>
            </div>
            <div className="snap-controls">
              <span>Snap:</span>
              <Menu>Smart</Menu>
              <span>Drag:</span>
              <Menu>No Overlap</Menu>
              <IconButton
                icon={WaveIcon}
                label="Snap to zero crossings"
                active
              />
            </div>
            <div className="zoom-controls">
              <SlidersHorizontal size={13} />
              <input
                type="range"
                aria-label="Horizontal zoom"
                defaultValue={38}
              />
              <MagnifyingGlass size={12} />
              <input
                type="range"
                aria-label="Vertical zoom"
                defaultValue={45}
              />
            </div>
          </nav>
          {view === "loops" ? (
            <LiveLoops wave={Waveform} />
          ) : view === "chords" ? (
            <ChordArrangement wave={Waveform} />
          ) : (
            <Arrangement />
          )}
          {view === "chords" ? null : sequencer ? (
            <StepSequencer />
          ) : mixer ? (
            <Mixer />
          ) : (
            <div className="empty-workspace">
              <div className="empty-grid" />
            </div>
          )}
          <footer className="workspace-footer">
            <span>
              <CaretLeft size={10} /> <CaretRight size={10} />
            </span>
            <div className="horizontal-scroll" />
            <span>
              <MagnifyingGlass size={10} />
            </span>
          </footer>
        </div>
      </div>
    </main>
  );
}
