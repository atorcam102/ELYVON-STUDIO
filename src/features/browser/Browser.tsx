import { useCallback, useEffect, useState } from "react";
import type { DirEntry } from "../../bindings/DirEntry";
import type { DirListing } from "../../bindings/DirListing";
import type { EntryKind } from "../../bindings/EntryKind";
import type { Place } from "../../bindings/Place";
import { Badge, IconButton } from "../../components/Button";
import { Icon, type IconName } from "../../components/Icon";
import { Tabs } from "../../components/Tabs";
import { fileName, formatBytes } from "../../lib/format";
import { ipc, toIpcError, type IpcError } from "../../lib/ipc";
import { notify } from "../../state/notifications";
import { useProject } from "../../state/project";
import { useUi, type BrowserTab } from "../../state/ui";
import { openProject } from "../project/projectActions";

const ENTRY_ICON: Record<EntryKind, IconName> = {
  directory: "folder",
  project: "project",
  audio: "waveform",
  midi: "piano",
  other: "file",
};

function FileBrowser() {
  const [listing, setListing] = useState<DirListing | null>(null);
  const [places, setPlaces] = useState<Place[]>([]);
  const [error, setError] = useState<IpcError | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async (path?: string) => {
    setLoading(true);
    try {
      const l = await ipc.browser.list(path);
      setListing(l);
      setError(null);
      setSelected(null);
    } catch (e) {
      setError(toIpcError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let alive = true;
    ipc.browser.list().then(
      (l) => alive && setListing(l),
      (e) => alive && setError(toIpcError(e)),
    );
    ipc.browser.places().then(
      (p) => alive && setPlaces(p),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, []);

  const activate = (entry: DirEntry) => {
    if (entry.kind === "directory") void load(entry.path);
    else if (entry.kind === "project") void openProject(entry.path);
    else if (entry.kind === "audio" || entry.kind === "midi")
      notify(
        "info",
        `Importing ${entry.kind === "audio" ? "audio" : "MIDI"} files arrives in a later phase.`,
      );
  };

  return (
    <div className="file-browser">
      <div className="file-browser__nav">
        <select
          className="select select--sm"
          aria-label="Location"
          value=""
          onChange={(e) => e.target.value && void load(e.target.value)}
        >
          <option value="">Locations…</option>
          {places.map((p) => (
            <option key={p.path} value={p.path}>
              {p.label}
            </option>
          ))}
        </select>
        <IconButton
          icon="arrowUp"
          label="Parent folder"
          size={14}
          disabled={!listing?.parent}
          onClick={() => listing?.parent && void load(listing.parent)}
        />
        <IconButton
          icon="refresh"
          label="Refresh"
          size={14}
          onClick={() => void load(listing?.path)}
        />
      </div>
      {listing && (
        <div className="file-browser__path" title={listing.path}>
          <Icon name="folder" size={12} />
          <span>{fileName(listing.path) || listing.path}</span>
        </div>
      )}
      {error ? (
        <div className="file-browser__error">
          <Icon name="warning" size={14} />
          <span>{error.message}</span>
        </div>
      ) : (
        <ul
          className={`file-list${loading ? " is-loading" : ""}`}
          role="listbox"
          aria-label="Files"
        >
          {listing?.entries.length === 0 && (
            <li className="file-list__empty">This folder is empty.</li>
          )}
          {listing?.entries.map((e) => (
            <li
              key={e.path}
              role="option"
              aria-selected={selected === e.path}
              tabIndex={0}
              className={`file-list__item kind-${e.kind}${selected === e.path ? " is-selected" : ""}`}
              onClick={() => setSelected(e.path)}
              onDoubleClick={() => activate(e)}
              onKeyDown={(ev) => {
                if (ev.key === "Enter") activate(e);
              }}
              title={
                e.kind === "project"
                  ? "Double-click to open"
                  : e.kind === "directory"
                    ? "Double-click to open folder"
                    : e.name
              }
            >
              <Icon name={ENTRY_ICON[e.kind]} size={14} />
              <span className="file-list__name">{e.name}</span>
              {e.size !== null && (
                <span className="file-list__size">{formatBytes(e.size)}</span>
              )}
            </li>
          ))}
          {listing?.truncated && (
            <li className="file-list__empty">Showing the first 5000 items.</li>
          )}
        </ul>
      )}
    </div>
  );
}

function Planned({
  icon,
  title,
  text,
}: {
  icon: IconName;
  title: string;
  text: string;
}) {
  return (
    <div className="planned">
      <Icon name={icon} size={22} />
      <h4>
        {title} <Badge>Planned</Badge>
      </h4>
      <p>{text}</p>
    </div>
  );
}

export function Browser() {
  const tab = useUi((s) => s.layout.browserTab) as BrowserTab;
  const setLayout = useUi((s) => s.setLayout);
  const selectedTrack = useUi((s) => s.selection.trackIds[0]);
  const track = useProject((s) =>
    s.view?.project.tracks.find((t) => t.id === selectedTrack),
  );
  return (
    <aside className="browser" aria-label="Library">
      <div className="panel-header">
        <span className="panel-title">Library</span>
        <IconButton
          icon="close"
          label="Hide library (B)"
          size={13}
          onClick={() => setLayout({ browserVisible: false })}
        />
      </div>
      <div
        className="library-patch"
        style={{ ["--patch-color" as string]: track?.color ?? "#a1db28" }}
      >
        <Icon
          name={
            track?.kind === "audio"
              ? "audio"
              : track?.kind === "midi"
                ? "midi"
                : "instrument"
          }
          size={76}
        />
        <span>{track?.name ?? "All Sounds"}</span>
      </div>
      <Tabs<BrowserTab>
        label="Browser sections"
        variant="pill"
        value={tab}
        onChange={(browserTab) => setLayout({ browserTab })}
        tabs={[
          { id: "files", label: "Files" },
          { id: "loops", label: "Loops" },
          { id: "instruments", label: "Instr." },
          { id: "plugins", label: "Plugins" },
        ]}
      />
      <div className="browser__content">
        {tab === "files" && <FileBrowser />}
        {tab === "loops" && (
          <Planned
            icon="loops"
            title="Loop library"
            text="Tempo- and key-aware loop browsing with preview requires the audio engine and is planned for Phase 3."
          />
        )}
        {tab === "instruments" && (
          <Planned
            icon="instrument"
            title="Instruments"
            text="Built-in Elyvon instruments are planned after the audio engine. Instrument tracks already store their MIDI regions."
          />
        )}
        {tab === "plugins" && (
          <Planned
            icon="plug"
            title="Plug-ins"
            text="CLAP and VST3 hosting (scanning, sandboxed loading, latency compensation) is planned for Phase 4."
          />
        )}
      </div>
    </aside>
  );
}
