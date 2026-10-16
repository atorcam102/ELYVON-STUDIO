import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "../bindings/Project";
import type { SessionView } from "../bindings/SessionView";

// The native core is replaced by an in-memory fake for store tests.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invoke(cmd, args),
  Channel: class {
    onmessage: ((m: unknown) => void) | null = null;
  },
}));

const { enqueue, useProject } = await import("./project");
const { nextTrackSelection, useUi } = await import("./ui");
const { useNotifications } = await import("./notifications");
const { useUpdates } = await import("./updates");

function project(trackIds: string[], regionIds: string[] = []): Project {
  return {
    id: "p",
    name: "Song",
    lengthBars: 64,
    transport: {
      tempo: 120,
      timeSignature: { numerator: 4, denominator: 4 },
      key: { tonic: "C", mode: "major" },
      loopRange: { enabled: false, start: 0, end: 15360 },
      tempoChanges: [],
      meterChanges: [],
      metronome: { enabled: false, volumeDb: -6, accent: true },
      countInBars: 0,
      preRollBars: 0,
      punch: { enabled: false, start: 0, end: 15360 },
    },
    sampleRate: 48000,
    media: [],
    markers: [],
    tracks: trackIds.map((id, i) => ({
      id,
      name: id,
      kind: "midi",
      color: "#5B8DEF",
      muted: false,
      soloed: false,
      volumeDb: 0,
      pan: 0,
      regions: i === 0 ? regionIds.map((r) => ({ id: r, name: r, start: 0, length: 960, muted: false, content: { type: "midi", notes: [] } })) : [],
    })),
  };
}

function view(p: Project, extra: Partial<SessionView> = {}): SessionView {
  return { project: p, path: null, origin: "new", dirty: false, undoLabel: null, redoLabel: null, revision: 0, ...extra };
}

beforeEach(() => {
  invoke.mockReset();
  useNotifications.setState({ toasts: [] });
  useUpdates.setState({ state: { phase: "idle" }, background: false });
});

describe("track selection", () => {
  const order = ["a", "b", "c", "d"];
  const sel = { trackIds: ["b"], regionIds: [], anchorTrackId: "b" };

  it("selects one track on plain click", () => {
    expect(nextTrackSelection(order, sel, "d", "single")).toEqual({ ids: ["d"], anchor: "d" });
  });

  it("toggles with the primary modifier", () => {
    expect(nextTrackSelection(order, sel, "d", "toggle").ids).toEqual(["b", "d"]);
    expect(nextTrackSelection(order, sel, "b", "toggle").ids).toEqual([]);
  });

  it("selects ranges from the anchor with shift", () => {
    expect(nextTrackSelection(order, sel, "d", "range").ids).toEqual(["b", "c", "d"]);
    expect(nextTrackSelection(order, sel, "a", "range").ids).toEqual(["a", "b"]);
  });

  it("drops deleted tracks and regions from the selection", () => {
    useUi.setState({ selection: { trackIds: ["a", "gone"], regionIds: ["r1", "r-gone"], anchorTrackId: "gone" } });
    useUi.getState().pruneSelection(project(["a"], ["r1"]));
    expect(useUi.getState().selection).toEqual({ trackIds: ["a"], regionIds: ["r1"], anchorTrackId: null });
  });
});

describe("project store", () => {
  it("serialises IPC calls in submission order", async () => {
    const order: string[] = [];
    const slow = enqueue(async () => {
      await new Promise((r) => setTimeout(r, 20));
      order.push("slow");
    });
    const fast = enqueue(async () => {
      order.push("fast");
    });
    await Promise.all([slow, fast]);
    expect(order).toEqual(["slow", "fast"]);
  });

  it("keeps the queue alive after a failure", async () => {
    await expect(enqueue(() => Promise.reject(new Error("x")))).rejects.toThrow("x");
    await expect(enqueue(async () => 42)).resolves.toBe(42);
  });

  it("renders the view returned by the core after an edit", async () => {
    const next = view(project(["t1"]), { dirty: true, revision: 1 });
    invoke.mockResolvedValueOnce({ view: next, outcome: { createdTracks: ["t1"], createdRegions: [] } });
    const outcome = await useProject.getState().apply({ type: "addTrack", kind: "midi" });
    expect(invoke).toHaveBeenCalledWith("project_apply", { command: { type: "addTrack", kind: "midi" } });
    expect(outcome?.createdTracks).toEqual(["t1"]);
    expect(useProject.getState().view).toBe(next);
    expect(useProject.getState().pending).toBe(0);
  });

  it("reports rejected edits without changing the view", async () => {
    const before = view(project([]));
    useProject.setState({ view: before });
    invoke.mockRejectedValueOnce({ code: "invalidEdit", message: "tempo must be between 20 and 999 BPM" });
    expect(await useProject.getState().apply({ type: "setTempo", bpm: 5 })).toBeNull();
    expect(useProject.getState().view).toBe(before);
    expect(useNotifications.getState().toasts[0]?.message).toMatch(/tempo/);
  });
});

describe("update workflow", () => {
  const info = { currentVersion: "0.1.0", version: "0.2.0", notes: "Fixes", date: null, platformLabel: "ubuntu-amd64" };

  it("maps every check result to a phase", async () => {
    invoke.mockResolvedValueOnce({ status: "upToDate", currentVersion: "0.1.0" });
    await useUpdates.getState().check();
    expect(useUpdates.getState().state).toEqual({ phase: "upToDate", currentVersion: "0.1.0" });

    invoke.mockResolvedValueOnce({ status: "prereleaseSkipped", currentVersion: "0.1.0", version: "0.2.0-beta.1" });
    await useUpdates.getState().check();
    expect(useUpdates.getState().state.phase).toBe("prereleaseSkipped");

    invoke.mockResolvedValueOnce({ status: "unavailable", reason: "dev build" });
    await useUpdates.getState().check();
    expect(useUpdates.getState().state).toEqual({ phase: "unavailable", reason: "dev build" });

    invoke.mockRejectedValueOnce({ code: "network", message: "offline" });
    await useUpdates.getState().check({ background: true });
    expect(useUpdates.getState().state).toMatchObject({ phase: "error", error: { code: "network" } });
    expect(useUpdates.getState().background).toBe(true);
  });

  it("downloads only after an available update and returns to it on cancel", async () => {
    await useUpdates.getState().download();
    expect(invoke).not.toHaveBeenCalled();

    useUpdates.setState({ state: { phase: "available", info } });
    invoke.mockRejectedValueOnce({ code: "cancelled", message: "The download was cancelled." });
    await useUpdates.getState().download();
    expect(useUpdates.getState().state).toEqual({ phase: "available", info });
  });

  it("reaches ready after a verified download", async () => {
    useUpdates.setState({ state: { phase: "available", info } });
    invoke.mockResolvedValueOnce(undefined);
    await useUpdates.getState().download();
    expect(useUpdates.getState().state).toEqual({ phase: "ready", info });
  });

  it("keeps the verified package when installation is blocked by unsaved work", async () => {
    useUpdates.setState({ state: { phase: "ready", info } });
    invoke.mockRejectedValueOnce({ code: "unsavedChanges", message: "unsaved" });
    await expect(useUpdates.getState().install()).rejects.toMatchObject({ code: "unsavedChanges" });
    expect(useUpdates.getState().state).toEqual({ phase: "ready", info });

    invoke.mockResolvedValueOnce({ restartRequired: true });
    await useUpdates.getState().install();
    expect(useUpdates.getState().state).toEqual({ phase: "installed", info, restartRequired: true });
  });

  it("never installs without a downloaded package", async () => {
    useUpdates.setState({ state: { phase: "available", info } });
    await useUpdates.getState().install();
    expect(invoke).not.toHaveBeenCalled();
  });
});
