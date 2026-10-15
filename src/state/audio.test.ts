import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "../bindings/EngineStatus";
import type { TelemetrySnapshot } from "../bindings/TelemetrySnapshot";

// The native engine is replaced by a fake for store tests; no audio runs.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invoke(cmd, args),
  Channel: class {
    onmessage: ((m: unknown) => void) | null = null;
  },
}));

const { engineRunning, useAudio } = await import("./audio");
const { useUi } = await import("./ui");
const { useNotifications } = await import("./notifications");

function status(state: EngineStatus["state"], extra: Partial<EngineStatus> = {}): EngineStatus {
  return {
    state,
    revision: 1,
    message: state === "running" ? null : "No audio output device was found.",
    errorCode: null,
    notices: [],
    output:
      state === "running"
        ? {
            host: "ALSA",
            deviceId: "alsa:hw:0",
            deviceName: "Speakers",
            sampleRate: 48000,
            requestedBufferFrames: 256,
            reportedBufferFrames: 256,
            channels: 2,
            layout: "stereo",
            bufferLayout: "interleaved",
            sampleFormat: "f32",
            bitsPerSample: 32,
            nominalLatencyMs: 5.3,
            headless: false,
          }
        : null,
    input: null,
    ...extra,
  };
}

function telemetry(extra: Partial<TelemetrySnapshot> = {}): TelemetrySnapshot {
  return {
    clockFrames: 48000,
    transportFrames: 24000,
    positionTicks: 960,
    playing: true,
    toneActive: false,
    sampleRate: 48000,
    outputPeakDb: [-18, -20],
    inputPeakDb: [],
    stats: {
      callbacks: 10,
      inputCallbacks: 0,
      lateCallbacks: 0,
      timingGaps: 0,
      backendXruns: 0,
      streamErrors: 0,
      clippedSamples: 0,
      maxCallbackLoad: 0.1,
      lastBlockFrames: 256,
      maxBlockFrames: 256,
      measuredOutputLatencyMs: null,
      realtimeDenied: false,
    },
    ...extra,
  };
}

beforeEach(() => {
  invoke.mockReset();
  useNotifications.setState({ toasts: [] });
  useUi.setState({ cursorTicks: 0 });
  useAudio.setState({ status: null, telemetry: null, tone: null, meterViews: 0, outputHoldDb: -96, transportIntent: "stopped" });
});

describe("audio store", () => {
  it("only treats a running engine as available", () => {
    expect(engineRunning(null)).toBe(false);
    expect(engineRunning(status("noDevice"))).toBe(false);
    expect(engineRunning(status("deviceLost"))).toBe(false);
    expect(engineRunning(status("running"))).toBe(true);
  });

  it("does not call the engine for play/stop when it is not running", async () => {
    useAudio.setState({ status: status("noDevice") });
    await useAudio.getState().play();
    await useAudio.getState().stop();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("plays from the cursor and follows the engine playhead", async () => {
    useAudio.setState({ status: status("running") });
    useUi.setState({ cursorTicks: 480 });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "audio_telemetry") return Promise.resolve(telemetry({ positionTicks: 1920 }));
      return Promise.resolve(undefined);
    });
    await useAudio.getState().play();
    expect(invoke).toHaveBeenCalledWith("audio_play", { fromTicks: 480 });
    // Polling starts from the play request even before the engine reports it.
    expect(useAudio.getState().transportIntent).toBe("playing");
    await vi.waitFor(() => expect(useUi.getState().cursorTicks).toBe(1920));
    expect(useAudio.getState().outputHoldDb).toBe(-18);
    // Stop the polling loop.
    useAudio.setState({ status: status("disabled"), transportIntent: "stopped" });
  });

  it("clears telemetry and the tone when the engine stops running", async () => {
    useAudio.setState({
      status: status("running"),
      telemetry: telemetry({ playing: false }),
      tone: { enabled: true, frequencyHz: 440, levelDb: -18, channels: { type: "all" } },
    });
    // The native side reports a hot-unplug.
    invoke.mockResolvedValue(status("deviceLost", { revision: 2 }));
    await useAudio.getState().load();
    const s = useAudio.getState();
    expect(s.status?.state).toBe("deviceLost");
    expect(s.telemetry).toBeNull();
    expect(s.tone).toBeNull();
  });

  it("reports engine errors as notifications instead of failing silently", async () => {
    useAudio.setState({ status: status("running") });
    invoke.mockRejectedValue({ code: "audioUnavailable", message: "The audio engine is not running." });
    await useAudio.getState().stop();
    expect(useNotifications.getState().toasts.at(-1)?.message).toContain("not running");
  });
});
