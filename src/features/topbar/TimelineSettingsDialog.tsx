import type { EditCommand } from "../../bindings/EditCommand";
import type { MeterChange } from "../../bindings/MeterChange";
import type { TempoChange } from "../../bindings/TempoChange";
import { Button, IconButton, Toggle } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { InlineEdit, NumberField } from "../../components/Fields";
import { DENOMINATORS } from "../../lib/music";
import { TempoMap } from "../../lib/tempoMap";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { locate } from "../transport/transportActions";

const SAMPLE_RATES = [44100, 48000, 88200, 96000, 176400, 192000];

/**
 * Persistent timeline settings: every change is one undoable native edit.
 * Ranges (cycle/punch) are edited in whole bars here; the ruler allows finer
 * cycle drags.
 */
export function TimelineSettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const project = useProject((s) => s.view?.project);
  const apply = useProject((s) => s.apply);
  const cursor = useUi((s) => s.cursorTicks);
  if (!open || !project) return null;
  const t = project.transport;
  const map = TempoMap.fromTransport(t);
  const run = (c: EditCommand) => void apply(c);

  const setTempoChanges = (changes: TempoChange[]) =>
    run({ type: "setTempoChanges", changes: [...changes].sort((a, b) => a.tick - b.tick) });
  const setMeterChanges = (changes: MeterChange[]) =>
    run({ type: "setMeterChanges", changes: [...changes].sort((a, b) => a.bar - b.bar) });
  const barOf = (tick: number) => map.barAt(tick).bar + 1;

  return (
    <Dialog open title="Tempo, metre and locators" onClose={onClose} width={620} className="timeline-settings">
      <section className="ts-section">
        <h3>Tempo</h3>
        <div className="ts-row">
          <span>Start tempo</span>
          <NumberField label="Start tempo" value={t.tempo} min={20} max={999} precision={2} step={0.01} onCommit={(bpm) => run({ type: "setTempo", bpm })} suffix="BPM" />
        </div>
        {t.tempoChanges.map((c, i) => (
          <div className="ts-row" key={c.tick}>
            <span>at {map.formatPosition(c.tick)}</span>
            <NumberField
              label={`Tempo change ${i + 1}`}
              value={c.bpm}
              min={20}
              max={999}
              precision={2}
              step={0.01}
              suffix="BPM"
              onCommit={(bpm) => setTempoChanges(t.tempoChanges.map((x, j) => (j === i ? { ...x, bpm } : x)))}
            />
            <IconButton icon="trash" label="Remove tempo change" size={13} onClick={() => setTempoChanges(t.tempoChanges.filter((_, j) => j !== i))} />
          </div>
        ))}
        <Button
          size="sm"
          icon="plus"
          disabled={cursor === 0 || t.tempoChanges.some((c) => c.tick === cursor)}
          onClick={() => setTempoChanges([...t.tempoChanges, { tick: cursor, bpm: Math.round(map.bpmAt(cursor) * 100) / 100 }])}
        >
          Add tempo change at cursor ({map.formatPosition(cursor)})
        </Button>
      </section>

      <section className="ts-section">
        <h3>Time signature</h3>
        <div className="ts-row">
          <span>Bar 1</span>
          <span>{`${t.timeSignature.numerator}/${t.timeSignature.denominator}`}</span>
        </div>
        {t.meterChanges.map((m, i) => (
          <div className="ts-row" key={m.bar}>
            <span>Bar</span>
            <NumberField
              label={`Metre change ${i + 1} bar`}
              value={m.bar + 1}
              min={2}
              max={9999}
              onCommit={(bar) => setMeterChanges(t.meterChanges.map((x, j) => (j === i ? { ...x, bar: bar - 1 } : x)))}
            />
            <NumberField
              label="Beats per bar"
              value={m.timeSignature.numerator}
              min={1}
              max={32}
              onCommit={(numerator) =>
                setMeterChanges(t.meterChanges.map((x, j) => (j === i ? { ...x, timeSignature: { ...x.timeSignature, numerator } } : x)))
              }
            />
            <select
              aria-label="Beat unit"
              value={m.timeSignature.denominator}
              onChange={(e) =>
                setMeterChanges(
                  t.meterChanges.map((x, j) => (j === i ? { ...x, timeSignature: { ...x.timeSignature, denominator: Number(e.target.value) } } : x)),
                )
              }
            >
              {DENOMINATORS.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
            <IconButton icon="trash" label="Remove metre change" size={13} onClick={() => setMeterChanges(t.meterChanges.filter((_, j) => j !== i))} />
          </div>
        ))}
        <Button
          size="sm"
          icon="plus"
          disabled={barOf(cursor) < 2 || t.meterChanges.some((m) => m.bar === barOf(cursor) - 1)}
          onClick={() => setMeterChanges([...t.meterChanges, { bar: barOf(cursor) - 1, timeSignature: map.timeSignatureAt(cursor) }])}
        >
          Add metre change at bar {barOf(cursor)}
        </Button>
      </section>

      <section className="ts-section">
        <h3>Markers</h3>
        {project.markers.length === 0 && <p className="insp-note">No markers yet.</p>}
        {project.markers.map((m) => (
          <div className="ts-row" key={m.id}>
            <InlineEdit value={m.name} label="Marker name" onCommit={(name) => run({ type: "updateMarker", id: m.id, name, tick: m.tick })} />
            <button type="button" className="link-btn" onClick={() => locate(m.tick)} title="Move the playhead here">
              {map.formatPosition(m.tick)}
            </button>
            <IconButton icon="trash" label="Delete marker" size={13} onClick={() => run({ type: "removeMarkers", ids: [m.id] })} />
          </div>
        ))}
        <Button size="sm" icon="marker" onClick={() => run({ type: "addMarker", tick: cursor })}>
          Add marker at cursor
        </Button>
      </section>

      <section className="ts-section">
        <h3>Cycle and punch</h3>
        {(
          [
            ["Cycle", t.loopRange, (enabled: boolean, start: number, end: number) => run({ type: "setLoop", enabled, start, end })],
            ["Punch (stored; recording not available yet)", t.punch, (enabled: boolean, start: number, end: number) => run({ type: "setPunch", enabled, start, end })],
          ] as const
        ).map(([name, range, set]) => (
          <div className="ts-row" key={name}>
            <Toggle checked={range.enabled} onChange={(on) => set(on, range.start, range.end)} label={name} />
            <span>bars</span>
            <NumberField label={`${name} start bar`} value={barOf(range.start)} min={1} max={9999} onCommit={(b) => set(range.enabled, map.barStart(b - 1), Math.max(range.end, map.barStart(b)))} />
            <span>to</span>
            <NumberField label={`${name} end bar`} value={barOf(Math.max(0, range.end - 1)) + 1} min={2} max={10000} onCommit={(b) => set(range.enabled, Math.min(range.start, map.barStart(b - 2)), map.barStart(b - 1))} />
          </div>
        ))}
      </section>

      <section className="ts-section">
        <h3>Count-in, pre-roll and metronome</h3>
        <div className="ts-row">
          <span>Count-in bars</span>
          <NumberField label="Count-in bars" value={t.countInBars} min={0} max={4} onCommit={(bars) => run({ type: "setCountIn", bars })} />
          <span>Pre-roll bars</span>
          <NumberField label="Pre-roll bars" value={t.preRollBars} min={0} max={4} onCommit={(bars) => run({ type: "setPreRoll", bars })} />
        </div>
        <div className="ts-row">
          <Toggle checked={t.metronome.enabled} onChange={(enabled) => run({ type: "setMetronome", settings: { ...t.metronome, enabled } })} label="Click during playback" />
          <Toggle checked={t.metronome.accent} onChange={(accent) => run({ type: "setMetronome", settings: { ...t.metronome, accent } })} label="Accent bar downbeats" />
          <NumberField label="Click level" value={t.metronome.volumeDb} min={-60} max={6} step={0.5} precision={1} suffix="dB" onCommit={(volumeDb) => run({ type: "setMetronome", settings: { ...t.metronome, volumeDb } })} />
        </div>
      </section>

      <section className="ts-section">
        <h3>Sample rate</h3>
        <div className="ts-row">
          <select aria-label="Project sample rate" value={project.sampleRate} onChange={(e) => run({ type: "setSampleRate", sampleRate: Number(e.target.value) })}>
            {SAMPLE_RATES.map((r) => (
              <option key={r} value={r}>
                {r / 1000} kHz
              </option>
            ))}
          </select>
          <span className="insp-note">The engine opens the device at this rate when it supports it; files are resampled.</span>
        </div>
      </section>
    </Dialog>
  );
}
