import type { Project } from "../../bindings/Project";
import type { Region } from "../../bindings/Region";
import type { Track } from "../../bindings/Track";
import { Button } from "../../components/Button";
import { InlineEdit, NumberField } from "../../components/Fields";
import { Icon } from "../../components/Icon";
import { formatDb, formatPan, MAX_DB, MIN_DB } from "../../lib/format";
import { formatBarBeat, formatKey, formatLength, ticksPerBar } from "../../lib/music";
import { useProject } from "../../state/project";
import { useUi } from "../../state/ui";
import { deleteSelectedTracks, duplicateSelectedTracks, setMuted, setSoloed } from "../project/editActions";

import { KIND_ICON, KIND_LABEL, TRACK_COLORS } from "../tracks/trackMeta";

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

function TrackInspector({ track, anySolo }: { track: Track; anySolo: boolean }) {
  const apply = useProject((s) => s.apply);
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
      <p className="insp-note insp-note--muted">
        Mute, solo, volume and pan are saved with the project and will drive the mixer once the audio engine ships.
      </p>
    </Section>
  );
}

function RegionInspector({ region, project }: { region: Region; project: Project }) {
  const apply = useProject((s) => s.apply);
  const ts = project.transport.timeSignature;
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
      <Row label="Position">{formatBarBeat(region.start, ts)}</Row>
      <Row label="Length">{formatLength(region.length, ts)}</Row>
      {region.content.type === "midi" ? (
        <Row label="Notes">{region.content.notes.length}</Row>
      ) : (
        <Row label="Source">{region.content.source?.path ?? "None (placeholder)"}</Row>
      )}
    </Section>
  );
}

function ProjectInspector({ project }: { project: Project }) {
  const t = project.transport;
  const perBar = ticksPerBar(t.timeSignature);
  return (
    <Section title="Project">
      <Row label="Name">{project.name}</Row>
      <Row label="Tempo">{`${t.tempo} BPM`}</Row>
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
  const focusTrack = tracks[tracks.length - 1];

  return (
    <aside className="inspector" aria-label="Inspector">
      <div className="panel-header">
        <span className="panel-title">Inspector</span>
      </div>
      <div className="inspector__scroll">
        {region && <RegionInspector region={region} project={project} />}
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
          <TrackInspector track={focusTrack} anySolo={anySolo} />
        ) : (
          <p className="insp-empty">Select a track or region to see its properties.</p>
        )}
        <ProjectInspector project={project} />
      </div>
    </aside>
  );
}
