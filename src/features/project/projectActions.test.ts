import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionView } from "../../bindings/SessionView";

const invoke = vi.fn();
const saveDialog = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (c: string, a?: unknown) => invoke(c, a), Channel: class {} }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: (o: unknown) => saveDialog(o), open: vi.fn() }));

const { useProject } = await import("../../state/project");
const { useDialogs } = await import("../../state/dialogs");
const actions = await import("./projectActions");

function view(extra: Partial<SessionView> = {}): SessionView {
  return {
    project: {
      id: "p",
      name: "My Song",
      lengthBars: 64,
      transport: {
        tempo: 120,
        timeSignature: { numerator: 4, denominator: 4 },
        key: { tonic: "C", mode: "major" },
        loopRange: { enabled: false, start: 0, end: 15360 },
      },
      tracks: [],
    },
    path: "/music/my-song.elyvon",
    origin: "file",
    dirty: true,
    undoLabel: null,
    redoLabel: null,
    revision: 3,
    ...extra,
  };
}

/** Answers the next prompt with `choice` as soon as it opens. */
function answerPrompt(choice: string) {
  const unsub = useDialogs.subscribe((s) => {
    if (s.prompt) {
      unsub();
      queueMicrotask(() => useDialogs.getState().close(choice));
    }
  });
}

beforeEach(() => {
  invoke.mockReset();
  saveDialog.mockReset();
  useDialogs.setState({ prompt: null });
});

describe("unsaved changes", () => {
  it("proceeds without asking when nothing would be lost", async () => {
    useProject.setState({ view: view({ dirty: false }) });
    expect(await actions.resolveUnsaved("quit")).toBe("proceed");
    expect(useDialogs.getState().prompt).toBeNull();
  });

  it("returns cancel and does nothing else when the user cancels", async () => {
    useProject.setState({ view: view() });
    answerPrompt("cancel");
    expect(await actions.resolveUnsaved("quit")).toBe("cancel");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("saves to the existing file when the user chooses Save", async () => {
    useProject.setState({ view: view() });
    invoke.mockResolvedValueOnce(view({ dirty: false }));
    answerPrompt("save");
    expect(await actions.resolveUnsaved("quit")).toBe("proceed");
    expect(invoke).toHaveBeenCalledWith("project_save", { path: null });
    expect(saveDialog).not.toHaveBeenCalled();
  });

  it("asks for a file name when the project was never saved", async () => {
    useProject.setState({ view: view({ path: null, origin: "new" }) });
    saveDialog.mockResolvedValueOnce("/music/new.elyvon");
    invoke.mockResolvedValueOnce(view({ dirty: false }));
    answerPrompt("save");
    expect(await actions.resolveUnsaved("quit")).toBe("proceed");
    expect(invoke).toHaveBeenCalledWith("project_save", { path: "/music/new.elyvon" });
  });

  it("treats a cancelled Save As as cancel", async () => {
    useProject.setState({ view: view({ path: null, origin: "demo" }) });
    saveDialog.mockResolvedValueOnce(null);
    answerPrompt("save");
    expect(await actions.resolveUnsaved("quit")).toBe("cancel");
  });

  it("quits with explicit discard only when the user chooses Don't Save", async () => {
    useProject.setState({ view: view() });
    invoke.mockResolvedValueOnce(undefined);
    answerPrompt("discard");
    await actions.quitApp();
    expect(invoke).toHaveBeenCalledWith("app_quit", { discardChanges: true });
  });

  it("creates a new project without discarding when the project is clean", async () => {
    useProject.setState({ view: view({ dirty: false }) });
    const fresh = view({ dirty: false, path: null, origin: "new" });
    invoke.mockResolvedValueOnce(fresh);
    await actions.newProject();
    expect(invoke).toHaveBeenCalledWith("project_new", { discardChanges: false });
    expect(useProject.getState().view).toBe(fresh);
  });
});
