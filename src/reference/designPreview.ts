/** Development-only, in-memory visual fixture. Never used by the native app. */
import type { Project } from "../bindings/Project";
import { defaultAutomation, defaultStrip, emptyMixer } from "../lib/mixerDefaults";
import { useProject } from "../state/project";
import { useUi, DEFAULT_LAYOUT } from "../state/ui";
import { notify } from "../state/notifications";

export function initializeDesignPreview() {
  if (!import.meta.env.DEV) return;
  const project: Project = {
    id: "design-preview", name: "Sin título", sampleRate: 48000, lengthBars: 64,
    transport: {
      tempo: 120, timeSignature: { numerator: 4, denominator: 4 }, key: { tonic: "C", mode: "major" },
      loopRange: { enabled: false, start: 0, end: 15360 }, tempoChanges: [], meterChanges: [],
      metronome: { enabled: false, volumeDb: -12, accent: true }, countInBars: 1, preRollBars: 0,
      punch: { enabled: false, start: 0, end: 3840 },
    },
    tracks: [{ id: "design-track", name: "Deluxe Classic", kind: "instrument", color: "#17a42c", muted: false, soloed: false, volumeDb: 0, pan: 0, folder: null, strip: defaultStrip(),
      io: { armed: false, inputChannel: 0, inputChannels: 2, monitor: false, midiInput: null, midiOutput: null, midiChannel: null, outputChannel: null, midiMonitor: false },
      regions: [], automation: defaultAutomation(), freeze: null, comps: [],
    }], media: [], markers: [], mixer: emptyMixer(),
  };
  if (new URLSearchParams(location.search).has("region")) project.tracks[0]!.regions.push({ id: "design-region", name: "Deluxe Classic", start: 0, length: 3840, muted: false, capture: null, midiEvents: [], content: { type: "midi", notes: [] }, automation: [] });
  useProject.getState().setView({ project, path: null, origin: "new", dirty: false, undoLabel: null, redoLabel: null, revision: 0 });
  useUi.getState().replaceLayout({ ...DEFAULT_LAYOUT, browserVisible: false, inspectorVisible: false });
  if (project.tracks[0]!.regions.length) useUi.getState().selectRegions(["design-region"], ["design-track"]);
  else useUi.getState().selectTracks(["design-track"]);
  useProject.setState({ apply: async command => {
    const view = useProject.getState().view!;
    const next = structuredClone(view.project);
    const tracks = next.tracks;
    const track = "id" in command ? tracks.find(t => t.id === command.id) : undefined;
    const createdRegions: string[] = [];
    const createdTracks: string[] = [];
    switch (command.type) {
      case "setStrip": {
        const channel = command.channel;
        const target = channel.type === "master" ? next.mixer.master : channel.type === "track" ? tracks.find(t => t.id === channel.id) : next.mixer.auxes.find(a => a.id === channel.id);
        if (target) {
          if (command.patch.volumeDb !== undefined) target.volumeDb = command.patch.volumeDb;
          if (command.patch.muted !== undefined) target.muted = command.patch.muted;
          if ("pan" in target && command.patch.pan !== undefined) target.pan = command.patch.pan;
          if ("soloed" in target && command.patch.soloed !== undefined) target.soloed = command.patch.soloed;
        }
        break;
      }
      case "setTrackVolume": if (track) track.volumeDb = command.volumeDb; break;
      case "setTrackPan": if (track) track.pan = command.pan; break;
      case "setTrackIo": if (track) track.io = command.io; break;
      case "renameTrack": if (track) track.name = command.name; break;
      case "setTrackColor": if (track) track.color = command.color; break;
      case "renameProject": next.name = command.name; break;
      case "setTracksMuted": tracks.filter(t => command.ids.includes(t.id)).forEach(t => { t.muted = command.muted; }); break;
      case "setTracksSoloed": tracks.filter(t => command.ids.includes(t.id)).forEach(t => { t.soloed = command.soloed; }); break;
      case "setTempo": next.transport.tempo = command.bpm; break;
      case "setTimeSignature": next.transport.timeSignature = { numerator: command.numerator, denominator: command.denominator }; break;
      case "setKey": next.transport.key = command.key; break;
      case "setRegionsMuted": tracks.flatMap(t => t.regions).filter(r => command.ids.includes(r.id)).forEach(r => { r.muted = command.muted; }); break;
      case "createMidiRegion": {
        const t = tracks.find(t => t.id === command.trackId);
        const id = crypto.randomUUID();
        t?.regions.push({ id, name: t.name, start: command.start, length: command.length, muted: false, capture: null, midiEvents: [], content: { type: "midi", notes: [] }, automation: [] });
        createdRegions.push(id); break;
      }
      default: notify("info", "La vista de diseño no tiene motor nativo. Abre Elyvon Studio para esta operación."); return null;
    }
    useProject.getState().setView({ ...view, project: next, revision: view.revision + 1 });
    return { createdRegions, createdTracks, createdMarkers: [], createdNotes: [], createdAuxes: [], createdInserts: [], createdGroups: [], createdFolders: [], createdPoints: [], selectedRegions: [] };
  } });
}
