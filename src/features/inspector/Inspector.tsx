import { useCapture } from "../../state/capture";
import { InspectorChannels } from "../mixer/Mixer";
import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import type { Track } from "../../bindings/Track";
import { Button, Toggle } from "../../components/Button";
import { InlineEdit, NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { formatDb, formatPan, MAX_DB, MIN_DB } from "../../lib/format";
import { formatKey, formatLength, ticksPerBar } from "../../lib/music";
import { useProject } from "../../state/project";
import { TempoMap } from "../../lib/tempoMap";
import { useTransport } from "../../state/transport";
import { useUi } from "../../state/ui";
import { relinkMedia } from "../project/regionActions";
import { deleteSelectedTracks, duplicateSelectedTracks, setMuted, setSoloed } from "../project/editActions";

import { KIND_ICON, KIND_LABEL, TRACK_COLORS } from "../tracks/trackMeta";
import { useEffect, useState } from "react";
import type { AutomationMode } from "../../bindings/AutomationMode";
import type { FadeCurve } from "../../bindings/FadeCurve";
import type { GridDivision } from "../../bindings/GridDivision";
import { MODES } from "../../lib/automation";
import { ipc } from "../../lib/ipc";
import {
  bounceTrack,
  crossfadeWithNext,
  freezeTrack,
  markersAtTransients,
  normalizeSelected,
  sliceAtTransients,
  unfreezeTrack,
} from "../project/editingActions";

/** Tracks whose freeze no longer matches their material (from the core). */
function useStaleFreezes(project: Project): string[] {
  const [stale, setStale] = useState<string[]>([]);
  const anyFrozen = project.tracks.some((t) => t.freeze);
  useEffect(() => {
    if (!anyFrozen) return;
    void ipc.mixer
      .status()
      .then((s) => setStale(s.staleFreezes))
      .catch(() => undefined);
  }, [project, anyFrozen]);
  return anyFrozen ? stale : [];
}

function TrackTools({ track, project }: { track: Track; project: Project }) {
  const apply = useProject((s) => s.apply);
  const stale = useStaleFreezes(project).includes(track.id);
  if (track.kind === "midi") return null;
  return (
    <>
      <Row label="Automation">
        <select
          aria-label="Automation mode"
          value={track.automation.mode}
          onChange={(e) => void apply({ type: "setAutomationMode", channel: { type: "track", id: track.id }, mode: e.target.value as AutomationMode })}
        >
          {MODES.map((m) => (
            <option key={m.mode} value={m.mode} title={m.title}>
              {m.label}
            </option>
          ))}
        </select>
        <span className="insp-hint">{track.automation.lanes.filter((l) => l.points.length).length} lane(s)</span>
      </Row>
      <div className="insp-buttons">
        {track.freeze ? (
          <Button size="sm" onClick={() => void unfreezeTrack(track.id)}>
            Unfreeze
          </Button>
        ) : (
          <Button size="sm" disabled={!track.regions.length} title="Render the regions through the inserts and play that file instead" onClick={() => void freezeTrack(track.id)}>
            Freeze
          </Button>
        )}
        <Button size="sm" disabled={!track.regions.length || !!track.freeze} title="Render this track to a new file on a new track (sources are muted, not deleted)" onClick={() => void bounceTrack(track.id)}>
          Bounce in place
        </Button>
      </div>
      {track.freeze && (
        <p className={`insp-note${stale ? "" : " insp-note--muted"}`}>
          {stale
            ? "The freeze is out of date (tempo, files or material changed): the track plays live. Unfreeze and freeze again."
            : "Frozen: the track plays its rendered file. Regions and inserts are locked until you unfreeze."}
        </p>
      )}
    </>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="insp-row">
      <span className="insp-row__label">{label}</span>
      <span className="insp-row__value">{children}</span>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="insp-section">
      <h3 className="insp-section__title">{title}</h3>
      {children}
    </section>
  );
}

function TrackInspector({ track, anySolo, project }: { track: Track; anySolo: boolean; project: Project }) {
  const apply = useProject((s) => s.apply);
  const captureActive=useCapture(s=>!!s.status?.active);
  const silencedBySolo = anySolo && !track.soloed && !track.muted;
  return (
    <Section title="Track">
      <div className="insp-track-title" style={{ ["--track-color" as string]: track.color }}>
        <Icon name={KIND_ICON[track.kind]} size={16} />
        <InlineEdit
          value={track.name}
          label="Track name"
          onCommit={(name) => void apply({ type: "renameTrack", id: track.id, name })}
          className="insp-track-title__name"
        />
      </div>
      <Row label="Type">{KIND_LABEL[track.kind]}</Row>
      <Toggle checked={track.io.armed} disabled={captureActive} label="Arm track for recording" onChange={armed=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,armed}})}/>
      {track.kind==="audio"&&<>
       <Row label="Input channel"><NumberField label="Input channel" value={track.io.inputChannel+1} min={1} max={32} onCommit={v=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,inputChannel:v-1}})}/></Row>
       <Row label="Channels"><select aria-label="Input channels" disabled={captureActive} value={track.io.inputChannels} onChange={e=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,inputChannels:Number(e.target.value)}})}><option value={1}>Mono</option><option value={2}>Stereo</option></select></Row>
       <Toggle checked={track.io.monitor} disabled={captureActive} label="Monitor this input" onChange={monitor=>void apply({type:"setTrackIo",id:track.id,io:{...track.io,monitor}})}/>
      </>}

      <Row label="Colour">
        <span className="swatches swatches--inline">
          {TRACK_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              className={`swatch swatch--sm${c === track.color ? " is-active" : ""}`}
              style={{ background: c }}
              aria-label={`Colour ${c}`}
              onClick={() => void apply({ type: "setTrackColor", id: track.id, color: c })}
            />
          ))}
        </span>
      </Row>
      <Row label="Volume">
        <NumberField
          label="Volume"
          value={Math.max(MIN_DB, track.volumeDb)}
          min={MIN_DB}
          max={MAX_DB}
          step={0.5}
          precision={1}
          suffix="dB"
          onCommit={(volumeDb) => void apply({ type: "setTrackVolume", id: track.id, volumeDb })}
        />
        <span className="insp-hint">{formatDb(track.volumeDb)}</span>
      </Row>
      <Row label="Pan">
        <NumberField
          label="Pan"
          value={Math.round(track.pan * 64)}
          min={-64}
          max={64}
          onCommit={(v) => void apply({ type: "setTrackPan", id: track.id, pan: v / 64 })}
        />
        <span className="insp-hint">{formatPan(track.pan)}</span>
      </Row>
      <div className="insp-buttons">
        <button
          type="button"
          className={`ms-btn ms-btn--wide ms-btn--mute${track.muted ? " is-on" : ""}`}
          aria-pressed={track.muted}
          onClick={() => void setMuted([track.id], !track.muted)}
        >
          Mute
        </button>
        <button
          type="button"
          className={`ms-btn ms-btn--wide ms-btn--solo${track.soloed ? " is-on" : ""}`}
          aria-pressed={track.soloed}
          onClick={() => void setSoloed([track.id], !track.soloed)}
        >
          Solo
        </button>
      </div>
      {silencedBySolo && <p className="insp-note">Silenced because another track is soloed.</p>}
      <TrackTools track={track} project={project} />
      <p className="insp-note insp-note--muted">
        Mute, solo, volume and pan are saved with the project and applied by the native engine.
      </p>
    </Section>
  );
}

function AudioRegionFields({ region, project }: { region: Region; project: Project }) {
  const apply = useProject((s) => s.apply);
  const mediaState = useTransport((s) => (region.content.type === "audio" && region.content.media ? s.media[region.content.media] : undefined));
  if (region.content.type !== "audio") return null;
  const c = region.content;
  const asset = c.media ? project.media.find((m) => m.id === c.media) : undefined;
  if (!asset) return <Row label="Source">None (placeholder, silent)</Row>;
  const rate = asset.info?.sampleRate ?? project.sampleRate;
  const ms = (frames: number) => Math.round((frames / rate) * 10000) / 10;
  const frames = (millis: number) => Math.round((millis / 1000) * rate);
  const set = (patch: Partial<{ gainDb: number; fadeIn: number; fadeOut: number; looped: boolean }>) =>
    void apply({ type: "setAudioRegion", id: region.id, gainDb: c.gainDb, fadeIn: c.fadeIn, fadeOut: c.fadeOut, looped: c.looped, ...patch });
  const info = asset.info;
  const status =
    mediaState?.state === "ready"
      ? mediaState.changed
        ? "Loaded — the file changed since import"
        : "Loaded"
      : mediaState?.state === "failed"
        ? mediaState.message
        : "Loading…";
  return (
    <>
      <Row label="File">
        <span title={asset.path}>{asset.name}</span>
      </Row>
      <Row label="Status">{status}</Row>
      {info && (
        <Row label="Format">{`${info.format.toUpperCase()} · ${info.sampleRate / 1000} kHz · ${info.channels === 1 ? "mono" : info.channels === 2 ? "stereo" : `${info.channels} ch (first 2 used)`} · ${(info.frames / info.sampleRate).toFixed(2)} s`}</Row>
      )}
      <Row label="Offset">{`${ms(c.offset)} ms into the file`}</Row>
      <Row label="Gain">
        <NumberField label="Region gain" value={c.gainDb} min={-96} max={24} step={0.5} precision={1} suffix="dB" onCommit={(gainDb) => set({ gainDb })} />
      </Row>
      <Row label="Fade in">
        <NumberField label="Fade in" value={ms(c.fadeIn)} min={0} max={60000} step={1} precision={1} suffix="ms" onCommit={(v) => set({ fadeIn: frames(v) })} />
      </Row>
      <Row label="Fade out">
        <NumberField label="Fade out" value={ms(c.fadeOut)} min={0} max={60000} step={1} precision={1} suffix="ms" onCommit={(v) => set({ fadeOut: frames(v) })} />
      </Row>
      <Toggle checked={c.looped} disabled={c.reversed} onChange={(looped) => set({ looped })} label="Loop" description="Repeat the file to fill the region" />
      <Toggle
        checked={c.reversed}
        disabled={c.looped}
        onChange={(reversed) => void apply({ type: "setRegionsReversed", ids: [region.id], reversed })}
        label="Reverse"
        description="Play the region's audio backwards"
      />
      <Row label="Fade shape">
        <select aria-label="Fade shape" value={c.fadeCurve} onChange={(e) => void apply({ type: "setFadeCurve", ids: [region.id], curve: e.target.value as FadeCurve })}>
          <option value="equalPower">Equal power</option>
          <option value="linear">Linear (equal gain)</option>
          <option value="sCurve">S-curve</option>
        </select>
      </Row>
      <div className="insp-buttons">
        <Button size="sm" title="Set the region gain so its peak is −1 dBFS" onClick={() => void normalizeSelected(-1)}>
          Normalize
        </Button>
        <Button size="sm" title="Overlap with the next touching region and crossfade over 20 ms" onClick={() => void crossfadeWithNext(20)}>
          Crossfade next
        </Button>
      </div>
      <div className="insp-buttons">
        <Button size="sm" title="Split at detected transients" onClick={() => void sliceAtTransients(0.5)}>
          Slice at hits
        </Button>
        <Button size="sm" title="Add a marker at each detected transient" onClick={() => void markersAtTransients(0.5)}>
          Mark hits
        </Button>
      </div>
      {mediaState?.state === "failed" && (
        <Button size="sm" icon="folder" onClick={() => void relinkMedia(asset.id)}>
          Locate file…
        </Button>
      )}
      <p className="insp-note insp-note--muted">Edits are non-destructive: the file on disk is never modified.</p>
    </>
  );
}

/** Active take of a take folder, or its comp (edited in Takes & Audio). */
function TakeFolderRow({ region, project }: { region: Region; project: Project }) {
  const apply = useProject((s) => s.apply);
  const folder = region.capture!.folder;
  const track = project.tracks.find((t) => t.regions.some((r) => r.id === region.id));
  const takes = (track?.regions ?? []).filter((r) => r.capture?.folder === folder).sort((a, b) => a.capture!.take - b.capture!.take);
  const comp = track?.comps.find((c) => c.folder === folder);
  const active = takes.find((r) => !r.muted)?.capture?.take;
  const value = comp && !comp.flattened ? "comp" : comp?.flattened ? "flat" : String(active ?? "");
  return (
    <Row label="Take folder">
      <select
        aria-label="Active take"
        value={value}
        disabled={comp?.flattened}
        onChange={(e) => {
          if (track && /^\d+$/.test(e.target.value)) void apply({ type: "selectTake", trackId: track.id, folder, take: Number(e.target.value) });
        }}
      >
        {comp && !comp.flattened && (
          <option value="comp" disabled>
            {`Comp of ${new Set(comp.segments.map((s) => s.take)).size} takes`}
          </option>
        )}
        {comp?.flattened && (
          <option value="flat" disabled>
            Flattened comp
          </option>
        )}
        {takes.map((r) => (
          <option key={r.id} value={r.capture!.take}>
            {`Take ${r.capture!.take}${!comp && !r.muted ? " · active" : ""}`}
          </option>
        ))}
      </select>
    </Row>
  );
}

function RegionInspector({ region, project }: { region: Region; project: Project }) {
  const apply = useProject((s) => s.apply);
  const map = TempoMap.fromTransport(project.transport);
  const ts = map.timeSignatureAt(region.start);
  return (
    <Section title="Region">
      <div className="insp-track-title">
        <Icon name={region.content.type === "midi" ? "piano" : "waveform"} size={16} />
        <InlineEdit
          value={region.name}
          label="Region name"
          onCommit={(name) => void apply({ type: "renameRegion", id: region.id, name })}
          className="insp-track-title__name"
        />
      </div>
      <Row label="Type">{region.content.type === "midi" ? "MIDI" : "Audio"}</Row>
      {region.capture && <TakeFolderRow region={region} project={project} />}
      <Row label="Position">{map.formatPosition(region.start)}</Row>
      <Row label="Length">{`${formatLength(region.length, ts)} · ${(map.secondsAt(region.start + region.length) - map.secondsAt(region.start)).toFixed(3)} s`}</Row>
      <Toggle checked={region.muted} onChange={(muted) => void apply({ type: "setRegionsMuted", ids: [region.id], muted })} label="Mute region" />
      {region.content.type === "midi" ? <Row label="Notes">{region.content.notes.length}</Row> : <AudioRegionFields region={region} project={project} />}
    </Section>
  );
}

function ProjectInspector({ project }: { project: Project }) {
  const t = project.transport;
  const perBar = ticksPerBar(t.timeSignature);
  return (
    <Section title="Project">
      <Row label="Name">{project.name}</Row>
      <Row label="Tempo">{`${t.tempo} BPM${t.tempoChanges.length ? ` + ${t.tempoChanges.length} change(s)` : ""}`}</Row>
      <Row label="Sample rate">{`${project.sampleRate / 1000} kHz`}</Row>
      <Row label="Audio files">{project.media.length}</Row>
      <Row label="Markers">{project.markers.length}</Row>
      <Row label="Signature">{`${t.timeSignature.numerator}/${t.timeSignature.denominator}`}</Row>
      <Row label="Key">{formatKey(t.key)}</Row>
      <Row label="Length">{`${project.lengthBars} bars`}</Row>
      <Row label="Loop">
        {`${t.loopRange.enabled ? "On" : "Off"} · ${t.loopRange.start / perBar + 1}–${t.loopRange.end / perBar + 1}`}
      </Row>
      <Row label="Tracks">{project.tracks.length}</Row>
    </Section>
  );
}

export function Inspector() {
  const project = useProject((s) => s.view?.project);
  const selection = useUi((s) => s.selection);
  if (!project) return <aside className="inspector" />;

  const anySolo = project.tracks.some((t) => t.soloed);
  const tracks = project.tracks.filter((t) => selection.trackIds.includes(t.id));
  const lastRegionId = selection.regionIds[selection.regionIds.length - 1];
  const region = lastRegionId ? project.tracks.flatMap((t) => t.regions).find((r) => r.id === lastRegionId) : undefined;
  const focusTrack = tracks[tracks.length - 1] ?? project.tracks[0];
  const audio = region?.content.type === "audio" ? region.content : undefined;
  const midi = region?.content.type === "midi";
  const quantizeRegion = (value: string) => {
    if (!region || !midi) return;
    void useProject.getState().apply({
      type: "transformNotes",
      targets: [{ regionId: region.id }],
      transform: value === "off" ? { type: "unquantize" } : { type: "quantize", grid: { division: value as GridDivision, triplet: false, swing: 0 }, strength: 100 },
    });
  };
  const transposeRegion = (semitones: number) => {
    if (region && midi && semitones) void useProject.getState().apply({ type: "transformNotes", targets: [{ regionId: region.id }], transform: { type: "transpose", semitones } });
  };

  return (
    <aside className="inspector" aria-label="Inspector">
      <div className="inspector__scroll">
        <details className="inspector-disclosure">
          <summary><Icon name="chevronRight" size={15} /><span>Pasaje: <strong>{region?.name ?? "Valores de…por omisión"}</strong></span></summary>
          <div className="inspector-region-options">
            <label>Silenciar: <input type="checkbox" checked={region?.muted ?? false} onChange={e => { if (region) void useProject.getState().apply({ type: "setRegionsMuted", ids: [region.id], muted: e.target.checked }); }} /></label>
            <label title={audio ? (audio.reversed ? "Un pasaje invertido no puede repetirse" : "Repite el archivo hasta el final del pasaje") : "Bucle de pasajes MIDI no disponible"}>Bucle: <input type="checkbox" aria-label="Bucle del pasaje" disabled={!audio || audio.reversed} checked={audio?.looped ?? false} onChange={e => { if (region && audio) void useProject.getState().apply({ type: "setAudioRegion", id: region.id, gainDb: audio.gainDb, fadeIn: audio.fadeIn, fadeOut: audio.fadeOut, looped: e.target.checked }); }} /></label>
            <label title={midi ? "Cuantiza todas las notas del pasaje (100 %)" : "Solo para pasajes MIDI"}>Cuantizar <select aria-label="Cuantizar pasaje" disabled={!midi} value="" onChange={e => quantizeRegion(e.target.value)}><option value="" disabled hidden>—</option><option value="off">Desactivado</option><option value="sixteenth">Semicorchea</option><option value="eighth">Corchea</option></select></label>
            <label>Q-Swing: <span>—</span></label>
            <label title={midi ? "Transpone todas las notas del pasaje" : "Solo para pasajes MIDI"}>Transponer: <select aria-label="Transponer pasaje" disabled={!midi} value="" onChange={e => transposeRegion(Number(e.target.value))}><option value="" disabled hidden>—</option>{[-12, -1, 1, 12].map(n => <option key={n} value={n}>{n > 0 ? `+${n}` : n}</option>)}</select></label>
            <label>Fuente del tono: <span>Desactivado</span></label>
            <label>Desplaz. de velo… <span>—</span></label>
          </div>
        {region && <RegionInspector region={region} project={project} />}
        </details>
        <details className="inspector-disclosure">
          <summary><Icon name="chevronRight" size={15} /><span>Pista: <strong>{focusTrack?.name ?? "Sin selección"}</strong></span></summary>
        {tracks.length > 1 ? (
          <Section title={`${tracks.length} tracks selected`}>
            <div className="insp-buttons">
              <button type="button" className="ms-btn ms-btn--wide ms-btn--mute" onClick={() => void setMuted(selection.trackIds, !tracks.every((t) => t.muted))}>
                Mute all
              </button>
              <button type="button" className="ms-btn ms-btn--wide ms-btn--solo" onClick={() => void setSoloed(selection.trackIds, !tracks.every((t) => t.soloed))}>
                Solo all
              </button>
            </div>
            <div className="insp-buttons">
              <Button size="sm" icon="duplicate" onClick={() => void duplicateSelectedTracks()}>
                Duplicate
              </Button>
              <Button size="sm" icon="trash" variant="danger" onClick={() => void deleteSelectedTracks()}>
                Delete
              </Button>
            </div>
          </Section>
        ) : focusTrack ? (
          <TrackInspector track={focusTrack} anySolo={anySolo} project={project} />
        ) : (
          <p className="insp-empty">Select a track or region to see its properties.</p>
        )}
          <details className="inspector-project"><summary>Proyecto</summary><ProjectInspector project={project} /></details>
        </details>
        <InspectorChannels trackId={focusTrack?.id} />
      </div>
    </aside>
  );
}
