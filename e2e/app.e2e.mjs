// End-to-end regression tests against the real Elyvon Studio binary.
//
//   npx tauri build --debug --no-bundle
//   xvfb-run -a node --test e2e/          (Linux; needs tauri-driver + WebKitWebDriver)
//
// Native file dialogs cannot be scripted through WebDriver, so persistence is
// exercised through the same IPC commands the dialogs feed into.

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { after, before, describe, it } from "node:test";
import { Driver } from "./webdriver.mjs";

const app = resolve(process.env.ELYVON_APP ?? "target/debug/elyvon-studio");
const shots = process.env.E2E_SCREENSHOTS;
const work = mkdtempSync(join(tmpdir(), "elyvon-e2e-"));
// Isolated preferences, recovery and logs: audio device choices made by the
// tests never leak into the machine's real settings.
for (const name of ["CONFIG", "DATA", "CACHE"]) process.env[`XDG_${name}_HOME`] = join(work, name.toLowerCase());

let d;

async function shot(name) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  writeFileSync(join(shots, `${name}.png`), await d.screenshot());
}

const trackCount = () => d.count(".track-header");

describe("Elyvon Studio", { timeout: 120_000 }, () => {
  before(async () => {
    d = await Driver.start({ application: app });
    await d.find(".controlbar", { timeout: 20_000 });
  });

  after(async () => {
    // The app must exit by itself when the WebDriver session closes its window
    // (this also closes the audio streams); a forced kill is a failure.
    const forced = await d?.stop();
    assert.deepEqual(forced ?? [], [], "application exited on its own");
  });

  it("starts with an empty, clean project and an honest engine status", async () => {
    assert.equal(await trackCount(), 0);
    await d.find(".timeline-empty__card");
    // CI runners have no sound card: whatever the machine has, the UI must
    // match the native status and only enable playback when a stream runs.
    const engine = await d.waitFor(
      async () => {
        const s = (await d.invoke("audio_status")).ok;
        return s.state === "starting" ? null : s;
      },
      { timeout: 15_000, message: "audio engine settles" },
    );
    const pill = await d.textOf(".status__pill");
    const play = await d.find('[aria-label^="Play"]');
    if (engine.state === "running") {
      assert.match(pill, /Engine|Headless/);
      assert.equal(await d.attr(play, "disabled"), null);
    } else {
      assert.ok(engine.message, "a non-running engine always explains itself");
      assert.doesNotMatch(pill, /^Engine ·/);
      assert.equal(await d.attr(play, "disabled"), "true", "play must be disabled without a running engine");
    }
    const record = await d.find('[aria-label^="Record"]');
    assert.equal(await d.attr(record, "disabled"), "true", "recording is not implemented");
    const view = await d.invoke("project_get");
    assert.equal(view.ok.dirty, false);
    const contracts = await d.invoke("app_contracts");
    assert.deepEqual(contracts.ok, { ipc: 2, project: 1, audio: 1, jobs: 1 });
    await shot("01-empty");
  });

  it("creates, renames, mutes and solos tracks through the UI", async () => {
    const buttons = await d.findAll(".timeline-empty__actions .btn");
    await d.click(buttons[1]); // Instrument
    await d.waitFor(async () => (await trackCount()) === 1, { message: "first track" });
    await d.clickCss('[aria-label="New track"]');
    await d.clickCss('.menu__item:first-of-type'); // Audio
    await d.waitFor(async () => (await trackCount()) === 2, { message: "second track" });

    const name = await d.find(".track-header .inline-edit");
    await d.doubleClick(name);
    const input = await d.find(".track-header .inline-edit__input");
    await d.type(input, "Keys\uE007");
    await d.waitFor(async () => (await d.textOf(".track-header .inline-edit")) === "Keys", { message: "rename" });

    await d.clickCss(".track-header .ms-btn--mute");
    await d.clickCss(".track-header:nth-child(2) .ms-btn--solo");
    const project = (await d.invoke("project_get")).ok.project;
    assert.equal(project.tracks[0].name, "Keys");
    assert.equal(project.tracks[0].muted, true);
    assert.equal(project.tracks[1].soloed, true);
    assert.equal((await d.invoke("project_get")).ok.dirty, true);
    await d.find(".dirty-dot");
  });

  it("undoes and redoes edits", async () => {
    await d.clickCss('[aria-label^="Undo"]');
    await d.waitFor(async () => !(await d.invoke("project_get")).ok.project.tracks[1].soloed, { message: "undo solo" });
    await d.clickCss('[aria-label^="Redo"]');
    await d.waitFor(async () => (await d.invoke("project_get")).ok.project.tracks[1].soloed, { message: "redo solo" });
  });

  it("refuses to discard unsaved work without consent", async () => {
    const r = await d.invoke("project_new", { discardChanges: false });
    assert.equal(r.err.code, "unsavedChanges");
    const quit = await d.invoke("app_quit", { discardChanges: false });
    assert.equal(quit.err.code, "unsavedChanges");
    const restart = await d.invoke("app_restart");
    assert.equal(restart.err.code, "unsavedChanges");
  });

  it("saves atomically and reopens with every change preserved", async () => {
    const path = join(work, "session");
    const saved = await d.invoke("project_save", { path });
    assert.ok(saved.ok, JSON.stringify(saved.err));
    assert.equal(saved.ok.dirty, false);
    assert.ok(saved.ok.path.endsWith("session.elyvon"));
    const file = JSON.parse(readFileSync(saved.ok.path, "utf8"));
    assert.equal(file.format, "elyvon-studio-project");
    assert.equal(file.schemaVersion, 1);
    assert.equal(file.project.tracks[0].name, "Keys");

    const fresh = await d.invoke("project_new", { discardChanges: false });
    assert.equal(fresh.ok.project.tracks.length, 0, "clean project can be replaced");
    const reopened = await d.invoke("project_open", { path: saved.ok.path, discardChanges: false });
    assert.deepEqual(reopened.ok.project, saved.ok.project);
    // End the native process, not just the webview, before reading the save.
    assert.deepEqual(await d.stop(), [], "application exited on its own");
    d = await Driver.start({ application: app });
    await d.find(".controlbar", { timeout: 20_000 });
    const restarted = await d.invoke("project_open", { path: saved.ok.path, discardChanges: false });
    assert.deepEqual(restarted.ok.project, saved.ok.project);
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    await d.waitFor(async () => (await trackCount()) === 2, { message: "tracks after native restart" });
  });

  it("reports unreadable and incompatible project files clearly", async () => {
    const broken = join(work, "broken.elyvon");
    writeFileSync(broken, "{ not json");
    const r = await d.invoke("project_open", { path: broken, discardChanges: true });
    assert.equal(r.err.code, "invalidProject");
    const future = join(work, "future.elyvon");
    writeFileSync(future, JSON.stringify({ format: "elyvon-studio-project", schemaVersion: 99, project: {} }));
    const f = await d.invoke("project_open", { path: future, discardChanges: true });
    assert.equal(f.err.code, "newerVersion");
    const missing = await d.invoke("project_open", { path: join(work, "nope.elyvon"), discardChanges: true });
    assert.equal(missing.err.code, "notFound");
  });

  it("opens the demo session and moves a region with snapping", async () => {
    await d.invoke("project_open_demo", { discardChanges: true });
    await d.exec("location.reload()");
    await d.find(".region", { timeout: 10_000 });
    assert.equal(await trackCount(), 6);
    await d.find(".badge--demo");
    await shot("02-demo");

    const before = (await d.invoke("project_get")).ok.project;
    const region = await d.find('.region[data-region-id]');
    const id = await d.attr(region, "data-region-id");
    const startBefore = before.tracks.flatMap((t) => t.regions).find((r) => r.id === id).start;
    await d.drag(region, 120, 0);
    const after = await d.waitFor(
      async () => {
        const p = (await d.invoke("project_get")).ok.project;
        const r = p.tracks.flatMap((t) => t.regions).find((x) => x.id === id);
        return r.start !== startBefore ? r : null;
      },
      { message: "region move" },
    );
    assert.equal(after.start % 240, 0, "moved region snaps to the grid");
    await shot("03-region-moved");
    await d.clickCss('[aria-label^="Undo"]');
    await d.waitFor(
      async () =>
        (await d.invoke("project_get")).ok.project.tracks.flatMap((t) => t.regions).find((r) => r.id === id).start === startBefore,
      { message: "undo move" },
    );
  });

  it("rejects edits that would break the project", async () => {
    const p = (await d.invoke("project_get")).ok.project;
    const audio = p.tracks.find((t) => t.kind === "audio");
    const r = await d.invoke("project_apply", { command: { type: "createMidiRegion", trackId: audio.id, start: 0, length: 960 } });
    assert.equal(r.err.code, "invalidEdit");
    const t = await d.invoke("project_apply", { command: { type: "setTempo", bpm: 5000 } });
    assert.equal(t.err.code, "invalidEdit");
  });

  it("shows the mixer, piano roll and browser panels", async () => {
    await d.find(".strip");
    assert.equal(await d.count(".strip:not(.strip--master)"), 6);
    const midi = await d.find(".region--midi");
    await d.click(midi);
    await d.clickCss(".bottom-panel .tabs__tab:nth-child(2)"); // Piano Roll
    await d.find(".pr-note");
    await shot("04-piano-roll");
    await d.clickCss(".bottom-panel .tabs__tab:nth-child(1)");
    await d.find(".file-list");
  });

  it("runs the real engine on the headless clock: transport, playhead, tone and meters", async () => {
    const before = (await d.invoke("project_get")).ok;
    const devices = (await d.invoke("audio_devices")).ok;
    assert.ok(devices.hosts.some((h) => h.id === "headless" && h.headless));
    const configured = await d.invoke("audio_configure", {
      setup: {
        enabled: true,
        host: "headless",
        outputDevice: "headless:clock",
        outputDeviceName: null,
        inputDevice: null,
        sampleRate: 48000,
        bufferFrames: 256,
      },
    });
    assert.ok(configured.ok, JSON.stringify(configured.err));
    assert.equal(configured.ok.state, "running");
    assert.equal(configured.ok.output.headless, true);
    await d.waitFor(async () => /Headless/.test(await d.textOf(".status__pill")), { message: "status pill" });

    // Play from bar 1 through the real transport button.
    await d.clickCss('[aria-label^="Go to beginning"]');
    await d.clickCss('[aria-label^="Play"]');
    const moving = await d.waitFor(
      async () => {
        const t = (await d.invoke("audio_telemetry")).ok;
        return t.playing && t.positionTicks > 960 ? t : null;
      },
      { timeout: 5000, message: "transport advances" },
    );
    assert.ok(moving.clockFrames >= moving.transportFrames);
    const lcd = await d.textOf(".lcd");
    await d.waitFor(async () => (await d.textOf(".lcd")) !== lcd, { message: "LCD follows the playhead" });

    // Opt-in test tone: real meters read the rendered level.
    const tone = await d.invoke("audio_test_tone", {
      tone: { enabled: true, frequencyHz: 440, levelDb: -18, channels: { type: "all" } },
    });
    assert.equal(tone.ok.levelDb, -18);
    const metered = await d.waitFor(
      async () => {
        const t = (await d.invoke("audio_telemetry")).ok;
        return t.outputPeakDb.length === 2 && t.outputPeakDb.every((db) => Math.abs(db + 18) < 0.5) ? t : null;
      },
      { timeout: 5000, message: "meters at -18 dBFS" },
    );
    assert.equal(metered.stats.clippedSamples, 0);
    await d.waitFor(async () => /18\.0/.test(await d.textOf('[data-testid="output-peak"]')), { message: "output strip peak" });
    await shot("06-engine-headless-playing");

    // Loud requests are clamped to the safety ceiling.
    const loud = await d.invoke("audio_test_tone", {
      tone: { enabled: true, frequencyHz: 440, levelDb: 0, channels: { type: "all" } },
    });
    assert.equal(loud.ok.levelDb, -6);
    await d.invoke("audio_test_tone", { tone: { enabled: false, frequencyHz: 440, levelDb: -18, channels: { type: "all" } } });

    await d.clickCss('[aria-label^="Stop"]');
    const stopped = await d.waitFor(
      async () => {
        const t = (await d.invoke("audio_telemetry")).ok;
        return t.playing ? null : t;
      },
      { message: "transport stops" },
    );
    await new Promise((r) => setTimeout(r, 200));
    assert.equal((await d.invoke("audio_telemetry")).ok.transportFrames, stopped.transportFrames, "stop holds position");

    // Settings › Audio shows the stream, device and health.
    await d.clickCss(".status__pill");
    await d.find('[data-testid="audio-stream-facts"]');
    assert.match(await d.textOf('[data-testid="audio-stream-facts"]'), /48000 Hz/);
    // The form reflects the natively saved choice, not a stale UI copy.
    await d.waitFor(
      async () => (await d.exec("return document.querySelectorAll('.audio-form select')[1]?.value")) === "headless:clock",
      { message: "output select shows the saved device" },
    );
    await d.find('[data-testid="audio-health"]');
    await shot("07-settings-audio");
    await d.keys("\uE00C"); // Escape

    // Engine activity never touches the project.
    const after = (await d.invoke("project_get")).ok;
    assert.deepEqual(after.project, before.project);
    assert.equal(after.dirty, before.dirty);

    // Preferences persist natively; restore the system default for later runs.
    const settings = (await d.invoke("settings_get")).ok;
    assert.equal(settings.audio.outputDevice, "headless:clock");
    assert.equal(settings.audio.profiles[0].bufferFrames, 256);
    const restored = await d.invoke("audio_configure", {
      setup: { enabled: true, host: null, outputDevice: null, outputDeviceName: null, inputDevice: null, sampleRate: null, bufferFrames: null },
    });
    assert.ok(restored.ok);
  });

  it("refuses transport commands with an explanation when the engine is off", async () => {
    const off = await d.invoke("audio_configure", {
      setup: { enabled: false, host: null, outputDevice: null, outputDeviceName: null, inputDevice: null, sampleRate: null, bufferFrames: null },
    });
    assert.equal(off.ok.state, "disabled");
    const play = await d.invoke("audio_play", { fromTicks: 0 });
    assert.equal(play.err.code, "audioUnavailable");
    assert.match(play.err.message, /Settings › Audio/);
    await d.waitFor(async () => (await d.attr(await d.find('[aria-label^="Play"]'), "disabled")) === "true", { message: "play disabled" });
    await d.invoke("audio_configure", {
      setup: { enabled: true, host: null, outputDevice: null, outputDeviceName: null, inputDevice: null, sampleRate: null, bufferFrames: null },
    });
  });

  it("explains why updates are unavailable instead of pretending", async () => {
    await d.clickCss('[aria-label^="Settings"]');
    await d.clickCss(".settings__nav-item:nth-child(2)");
    const page = await d.textOf(".settings__content");
    assert.match(page, /development builds|signing key|installer/i);
    const r = (await d.invoke("update_check")).ok;
    assert.equal(r.status, "unavailable");
    await shot("05-settings-updates");
    await d.keys("\uE00C"); // Escape
  });
});
