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

let d;

async function shot(name) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  writeFileSync(join(shots, `${name}.png`), await d.screenshot());
}

const trackCount = () => d.count(".track-header");

/** 16-bit mono PCM WAV: `seconds` of a 0.5-amplitude sine. */
function sineWav(rate, seconds, freq) {
  const n = rate * seconds;
  const buf = Buffer.alloc(44 + n * 2);
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + n * 2, 4);
  buf.write("WAVEfmt ", 8);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(rate, 24);
  buf.writeUInt32LE(rate * 2, 28);
  buf.writeUInt16LE(2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write("data", 36);
  buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) buf.writeInt16LE(Math.round(0.5 * Math.sin((2 * Math.PI * freq * i) / rate) * 32767), 44 + i * 2);
  return buf;
}

describe("Elyvon Studio", { timeout: 120_000 }, () => {
  before(async () => {
    d = await Driver.start({ application: app });
    await d.find(".controlbar", { timeout: 20_000 });
  });

  after(async () => {
    // Close a clean test-owned document so the unsaved-work guard is respected.
    try {
      await d?.invoke("project_save", { path: join(work, "final-session.elyvon") });
    } finally {
      assert.deepEqual(await d?.stop(), [], "final test process exits without a forced kill");
    }
  });

  it("starts with an empty, clean project and an honest engine status", async () => {
    assert.equal(await trackCount(), 0);
    await d.find(".timeline-empty__card");
    // The engine opens a device, or falls back to its silent internal clock
    // (CI has no sound card) and says so.
    const engine = await d.waitFor(
      async () => {
        const s = (await d.invoke("audio_status")).ok;
        return s.state === "running" ? s : null;
      },
      { message: "engine running", timeout: 20_000 },
    );
    const status = await d.textOf(".status__pill");
    if (engine.output === "internalClock") assert.match(status, /no audio output/i);
    else assert.match(status, /kHz/);
    const play = await d.find('[aria-label^="Play"]');
    assert.notEqual(await d.attr(play, "disabled"), "true", "play is enabled with a running engine");
    assert.notEqual(await d.attr(await d.find('[aria-label^="Record"]'),"disabled"),"true","native recording can be started after arming inputs");
    const missing=await d.invoke("recording_start",{fromTick:0,monitorOnly:false});assert.equal(missing.err.code,"audioUnavailable");
    const view = await d.invoke("project_get");
    assert.equal(view.ok.dirty, false);
    const contracts = await d.invoke("app_contracts");
    assert.deepEqual(contracts.ok, { ipc: 2, project: 3, audio: 1, jobs: 1 });
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
    assert.equal(file.schemaVersion, 3);
    assert.equal(file.project.tracks[0].name, "Keys");

    const fresh = await d.invoke("project_new", { discardChanges: false });
    assert.equal(fresh.ok.project.tracks.length, 0, "clean project can be replaced");
    const reopened = await d.invoke("project_open", { path: saved.ok.path, discardChanges: false });
    assert.deepEqual(reopened.ok.project, saved.ok.project);
    // End the native process, not just the webview, before reading the save.
    await d.stop();
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
    // Layout is persisted in preferences: select the mixer tab explicitly.
    await d.clickCss(".bottom-panel .tabs__tab:nth-child(1)");
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

  it("imports real audio, plays it on the engine clock and exports a render", async () => {
    await d.invoke("project_new", { discardChanges: true });
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    const wav = join(work, "tone.wav");
    writeFileSync(wav, sineWav(44_100, 2, 440));
    const before = readFileSync(wav);
    const imported = await d.invoke("media_import", { paths: [wav], trackId: null, start: 0, layout: "newTracks" });
    assert.ok(imported.ok, JSON.stringify(imported.err));
    assert.equal(imported.ok.failures.length, 0);
    const project = imported.ok.view.project;
    assert.equal(project.media[0].info.sampleRate, 44_100);
    assert.equal(project.media[0].info.frames, 88_200);
    await d.exec("location.reload()");
    await d.find(".region--audio", { timeout: 10_000 });
    await d.find('[data-testid="waveform"][data-state="ready"]', { timeout: 15_000 });
    await shot("06-audio-waveform");

    await d.invoke("transport_command", { command: { type: "locate", tick: 0 } });
    await d.clickCss('[aria-label^="Play"]');
    const playing = await d.waitFor(
      async () => {
        const s = (await d.invoke("transport_state")).ok;
        return s && s.state === "playing" && s.seconds > 0.3 ? s : null;
      },
      { message: "engine playhead advancing", timeout: 10_000 },
    );
    assert.ok(playing.ticks > 0);
    const moved = await d.waitFor(async () => {
      const t = await d.exec('return document.querySelector("[data-testid=playhead]").style.transform');
      return t && t !== "translateX(0px)" ? t : null;
    }, { message: "UI playhead follows engine events" });
    assert.match(moved, /translateX/);
    await shot("07-playing");
    await d.keys(" ");
    await d.waitFor(async () => (await d.invoke("transport_state")).ok.state === "stopped", { message: "space stops" });

    const out = join(work, "render.wav");
    const report = await d.invoke("project_render", { path: out, startTick: null, endTick: null, metronome: false });
    assert.ok(report.ok, JSON.stringify(report.err));
    assert.equal(report.ok.clips, 1);
    // Mono source, centre pan: constant-power law gives 0.5 · √½ per side.
    assert.ok(Math.abs(report.ok.peak[0] - 0.5 * Math.SQRT1_2) < 0.002, `peak ${report.ok.peak[0]}`);
    assert.ok(readFileSync(out).length > 44);
    assert.deepEqual(readFileSync(wav), before, "the source file is never modified");
  });

  it("validates audio input routing and releases the recording lock after startup failure", async () => {
    const project=(await d.invoke("project_get")).ok.project;
    const track=project.tracks.find(t=>t.kind==="audio");
    const invalid=await d.invoke("project_apply",{command:{type:"setTrackIo",id:track.id,io:{...track.io,armed:true,inputChannel:31,inputChannels:2}}});
    assert.equal(invalid.err.code,"invalidEdit");
    const armed=await d.invoke("project_apply",{command:{type:"setTrackIo",id:track.id,io:{...track.io,armed:true,inputChannel:0,inputChannels:1}}});assert.ok(armed.ok);
    const devices=await d.invoke("recording_devices");assert.ok(Array.isArray(devices.ok));
    const started=await d.invoke("recording_start",{fromTick:0,monitorOnly:false});
    if(started.ok){const stopped=await d.invoke("recording_stop",{cancel:true});assert.equal(stopped.ok.report.state,"cancelled");}
    else{assert.equal(started.err.code,"audioUnavailable");assert.equal((await d.invoke("recording_status")).ok.active,false);}
    const edited=await d.invoke("project_apply",{command:{type:"renameProject",name:"Input checks"}});assert.ok(edited.ok,"input startup failure must release the project lock");
  });

  it("records the virtual MIDI keyboard through native timing, saves, restarts and exports SMF", async () => {
    assert.ok((await d.invoke("project_new",{discardChanges:true})).ok);
    const added=await d.invoke("project_apply",{command:{type:"addTrack",kind:"midi"}});assert.ok(added.ok);
    await d.exec("location.reload()");await d.find(".controlbar",{timeout:10000});
    await d.clickCss('[aria-label="MIDI devices and monitor"]');
    await d.find('[role="dialog"]');
    const switches=await d.findAll('[role="dialog"] .toggle__track');await d.click(switches[1]);
    await d.waitFor(async()=>(await d.invoke("project_get")).ok.project.tracks[0].io.armed,{message:"MIDI track armed through the UI"});
    await d.clickCss('[aria-label="Record armed MIDI tracks"]');
    await d.waitFor(async()=>(await d.invoke("midi_status")).ok.recording,{message:"native MIDI recording started"});
    await d.waitFor(async()=>(await d.invoke("transport_state")).ok.seconds>0.1,{message:"MIDI engine clock running"});
    const key=await d.find('[aria-label="MIDI C4"]');const rect=await d.rect(key);
    await d.cmd("POST","/actions",{actions:[{type:"pointer",id:"held-midi-key",parameters:{pointerType:"mouse"},actions:[{type:"pointerMove",duration:0,origin:"viewport",x:Math.round(rect.x+rect.width/2),y:Math.round(rect.y+rect.height/2)},{type:"pointerDown",button:0}]}]});
    await d.waitFor(async()=>{const s=(await d.invoke("midi_status")).ok;return s.monitor.some(m=>m.bytes.join(",")==="144,60,100");},{message:"native MIDI monitor receives the virtual key"});
    await shot("08-midi-record-monitor");
    const noteOnSeconds=(await d.invoke("transport_state")).ok.seconds;
    await d.waitFor(async()=>(await d.invoke("transport_state")).ok.seconds>noteOnSeconds+0.2,{message:"held note duration comes from the engine"});
    const stopped=await d.invoke("midi_record_stop",{cancel:false});assert.ok(stopped.ok,JSON.stringify(stopped.err));
    await d.cmd("DELETE","/actions");
    const recorded=stopped.ok.project.tracks[0].regions[0];assert.equal(recorded.content.type,"midi");assert.ok(recorded.content.notes[0].length>=200,"stopping a held note preserves its duration");
    assert.ok(recorded.midiEvents.some(e=>e.bytes[0]===0x80&&e.bytes[1]===60),"stop closes held MIDI keys");
    const file=join(work,"captured-midi.elyvon");assert.ok((await d.invoke("project_save",{path:file})).ok);
    const midiFile=join(work,"captured.mid");assert.equal((await d.invoke("midi_export",{path:midiFile})).ok,midiFile);assert.equal(readFileSync(midiFile).subarray(0,4).toString(),"MThd");
    assert.deepEqual(await d.stop(),[],"recording process exits without a forced kill");
    d=await Driver.start({application:app});await d.find(".controlbar",{timeout:20000});
    const opened=await d.invoke("project_open",{path:file,discardChanges:false});assert.ok(opened.ok,JSON.stringify(opened.err));
    assert.deepEqual(opened.ok.project.tracks[0].regions[0].midiEvents,recorded.midiEvents);assert.equal(opened.ok.project.tracks[0].io.armed,true);
    await d.exec("location.reload()");await d.find(".region--midi",{timeout:10000});await shot("09-recorded-midi-reopened");
    await d.invoke("project_new",{discardChanges:true});
    const imported=await d.invoke("midi_import",{path:midiFile,startTick:0,useFileTempo:true});assert.ok(imported.ok,JSON.stringify(imported.err));
    assert.deepEqual(imported.ok.project.tracks[0].regions[0].midiEvents,recorded.midiEvents,"SMF import/export preserves the recorded sequence");
    await d.exec("location.reload()");await d.find(".controlbar",{timeout:10000});
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
