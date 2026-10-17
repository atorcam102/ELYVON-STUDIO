// End-to-end regression tests against the real Elyvon Studio binary.
//
//   npx tauri build --debug --no-bundle
//   xvfb-run -a node --test e2e/          (Linux; needs tauri-driver + WebKitWebDriver)
//
// Native file dialogs cannot be scripted through WebDriver, so persistence is
// exercised through the same IPC commands the dialogs feed into.

import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, renameSync, writeFileSync } from "node:fs";
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

/** Opens the control bar's "Más controles" drop-down (file buttons, status, settings). */
async function openExtras() {
  await d.exec("document.querySelector('.controlbar__extras').open = true");
}

/** Clicks a control that lives in the "Más controles" drop-down. */
async function clickExtra(css) {
  await openExtras();
  await d.clickCss(css);
  await d.exec("document.querySelector('.controlbar__extras').open = false");
}

/** Clicks the "Más controles" button whose text is `text`. */
async function clickExtraText(text) {
  await openExtras();
  await d.exec(`[...document.querySelectorAll('.controlbar__extras button')].find((b) => b.textContent === ${JSON.stringify(text)}).click()`);
  await d.exec("document.querySelector('.controlbar__extras').open = false");
}

/** Opens the mixer from the control bar (layout is persisted, so check first). */
async function showMixer() {
  if (!(await d.count(".bottom-panel--mixer"))) await d.clickCss('[aria-label="Mezclador (X)"]');
  await d.find(".bottom-panel .strip");
}

/** Hides the bottom panel so it does not cover the arrangement. */
async function hideBottom() {
  if (await d.count(".bottom-panel--mixer")) await d.clickCss('[aria-label="Mezclador (X)"]');
  else if (await d.count(".bottom-panel__bar")) await d.clickCss('[aria-label="Editor (E)"]');
}

/** Opens the editors panel on the tab titled `title`. */
async function showEditor(title) {
  if (!(await d.count(".bottom-panel__bar"))) await d.clickCss('[aria-label="Editor (E)"]');
  await d.clickCss(`.bottom-panel [role=tab][title="${title}"]`);
}

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

/** Interleaved samples of a 32-bit float WAV (render export). */
function readFloatWav(path) {
  const b = readFileSync(path);
  let p = 12;
  while (p + 8 <= b.length) {
    const id = b.toString("ascii", p, p + 4);
    const n = b.readUInt32LE(p + 4);
    if (id === "data") return new Float32Array(b.buffer.slice(b.byteOffset + p + 8, b.byteOffset + p + 8 + n));
    p += 8 + n + (n % 2);
  }
  throw new Error("no data chunk in " + path);
}

describe("Elyvon Studio", { timeout: 120_000 }, () => {
  before(async () => {
    d = await Driver.start({ application: app });
    await d.find(".controlbar", { timeout: 20_000 });
    // The layout is persisted in preferences: start from the arrangement alone.
    await hideBottom();
    if (await d.count(".right-panel__close")) await d.clickCss(".right-panel__close");
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
    await openExtras();
    const status = await d.textOf(".status__pill");
    await d.exec("document.querySelector('.controlbar__extras').open = false");
    if (engine.output === "internalClock") assert.match(status, /no audio output/i);
    else assert.match(status, /kHz/);
    const play = await d.find('[aria-label^="Play"]');
    assert.notEqual(await d.attr(play, "disabled"), "true", "play is enabled with a running engine");
    assert.notEqual(await d.attr(await d.find('[aria-label^="Record"]'),"disabled"),"true","native recording can be started after arming inputs");
    const missing=await d.invoke("recording_start",{fromTick:0,monitorOnly:false});assert.equal(missing.err.code,"audioUnavailable");
    const view = await d.invoke("project_get");
    assert.equal(view.ok.dirty, false);
    const contracts = await d.invoke("app_contracts");
    assert.deepEqual(contracts.ok, { ipc: 2, project: 5, audio: 1, jobs: 1 });
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
    const project = await d.waitFor(
      async () => {
        const p = (await d.invoke("project_get")).ok.project;
        return p.tracks[0].muted && p.tracks[1].soloed ? p : null;
      },
      { message: "mute and solo applied" },
    );
    assert.equal(project.tracks[0].name, "Keys");
    assert.equal(project.tracks[0].muted, true);
    assert.equal(project.tracks[1].soloed, true);
    assert.equal((await d.invoke("project_get")).ok.dirty, true);
    await d.find(".dirty-dot");
  });

  it("undoes and redoes edits", async () => {
    await clickExtra('[aria-label^="Undo"]');
    await d.waitFor(async () => !(await d.invoke("project_get")).ok.project.tracks[1].soloed, { message: "undo solo" });
    await clickExtra('[aria-label^="Redo"]');
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
    assert.equal(file.schemaVersion, 5);
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
    await hideBottom(); // the reload restores the persisted layout
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
    await clickExtra('[aria-label^="Undo"]');
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
    await showMixer();
    const strips = await d.exec("return [...document.querySelectorAll('.bottom-panel .strip:not(.strip--master)')].map((e) => e.getAttribute('data-channel') + ' ' + e.className)");
    assert.equal(strips.length, 6, JSON.stringify(strips));
    await hideBottom(); // the mixer opens tall and covers the arrangement
    const midi = await d.find(".region--midi");
    await d.click(midi);
    await showEditor("Editor de teclado");
    await d.find('[data-testid="piano-roll"] .pr__canvas');
    const count = await d.exec("return document.querySelector('.pr__sel')?.textContent ?? ''");
    assert.match(count, /^0\/[1-9]\d* notes$/, "the demo region's notes are loaded into the editor");
    await shot("04-piano-roll");
    await d.clickCss('[aria-label="Exploradores (F)"]');
    await d.clickCss('.right-panel [role=tab]:nth-child(2)'); // Todos los archivos
    await d.find(".file-list");
    await d.clickCss('[aria-label="Exploradores (F)"]');
  });

  it("imports real audio, plays it on the engine clock and exports a render", async () => {
    await d.invoke("project_new", { discardChanges: true });
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    const wav = join(work, "tone.wav");
    writeFileSync(wav, sineWav(44_100, 2, 440));
    const before = readFileSync(wav);
    // Observe the event before import; reloading below must not mask missing
    // ready notifications from files that bypass the background media loader.
    const handler = await d.exec(`
      window.__testMediaStatuses = [];
      return window.__TAURI_INTERNALS__.transformCallback(event => {
        window.__testMediaStatuses.push(...event.payload);
      });
    `);
    const listener = await d.invoke("plugin:event|listen", { event: "media://status", target: { kind: "Any" }, handler });
    assert.ok(listener.ok !== undefined, JSON.stringify(listener.err));
    const imported = await d.invoke("media_import", { paths: [wav], trackId: null, start: 0, layout: "newTracks" });
    assert.ok(imported.ok, JSON.stringify(imported.err));
    assert.equal(imported.ok.failures.length, 0);
    const project = imported.ok.view.project;
    assert.equal(project.media[0].info.sampleRate, 44_100);
    assert.equal(project.media[0].info.frames, 88_200);
    const readyEvent = () => d.waitFor(
      () => d.exec("return window.__testMediaStatuses.some(s => s.id === arguments[0] && s.state.state === 'ready')", [project.media[0].id]),
      { message: "ready media event without reloading", timeout: 5000 },
    );
    await readyEvent();
    await d.exec("window.__testMediaStatuses = []");
    const relinked = await d.invoke("media_relink", { id: project.media[0].id, path: wav });
    assert.ok(relinked.ok, JSON.stringify(relinked.err));
    await readyEvent();
    await d.invoke("plugin:event|unlisten", { event: "media://status", eventId: listener.ok });
    await d.exec("window.__TAURI_INTERNALS__.unregisterCallback(arguments[0])", [handler]);
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
    await clickExtraText("Dispositivos MIDI…");
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
    // Schema 4: the held key becomes an editable note closed at stop.
    assert.ok(recorded.content.notes.some(n=>n.pitch===60&&n.velocity===100),"stop closes held MIDI keys into a note");
    assert.ok(recorded.midiEvents.every(e=>(e.bytes[0]&0xf0)!==0x80&&(e.bytes[0]&0xf0)!==0x90),"note messages are not duplicated as raw events");
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
    const shape=(r)=>r.content.notes.map(n=>[n.pitch,n.velocity,n.channel,n.start,n.length]);
    assert.deepEqual(shape(imported.ok.project.tracks[0].regions[0]),shape(recorded),"SMF round trip keeps the notes");
    await d.exec("location.reload()");await d.find(".controlbar",{timeout:10000});
  });

  it("draws notes with the piano-roll pencil and undoes them as one step each", async () => {
    assert.ok((await d.invoke("project_new", { discardChanges: true })).ok);
    const added = await d.invoke("project_apply", { command: { type: "addTrack", kind: "instrument" } });
    const track = added.ok.outcome.createdTracks[0];
    const region = await d.invoke("project_apply", { command: { type: "createMidiRegion", trackId: track, start: 0, length: 15360 } });
    assert.ok(region.ok, JSON.stringify(region.err));
    await d.exec("location.reload()");
    await d.find(".region--midi", { timeout: 10_000 });
    await hideBottom();
    await d.click(await d.find(".region--midi"));
    await showEditor("Editor de teclado");
    const canvas = await d.find(".pr__canvas");
    const rect = await d.rect(canvas);
    await d.clickCss('.piano-tools [title="pencil"]');
    const click = (dx, dy) =>
      d.cmd("POST", "/actions", {
        actions: [{ type: "pointer", id: "pencil", parameters: { pointerType: "mouse" }, actions: [
          { type: "pointerMove", duration: 0, origin: "viewport", x: Math.round(rect.x + dx), y: Math.round(rect.y + dy) },
          { type: "pointerDown", button: 0 },
          { type: "pointerUp", button: 0 },
        ] }],
      });
    await click(140, Math.round(22 + (rect.height - 22) * 0.3));
    await click(260, Math.round(22 + (rect.height - 22) * 0.8));
    const notes = async () => (await d.invoke("project_get")).ok.project.tracks[0].regions[0].content.notes;
    await d.waitFor(async () => (await notes()).length === 2, { message: "two notes drawn through the real canvas" });
    const drawn = await notes();
    assert.ok(drawn.every((n) => n.length > 0 && n.velocity === 80 && n.start % 240 === 0), "snapped to the 1/16 grid: " + JSON.stringify(drawn));
    assert.ok(drawn[0].pitch > drawn[1].pitch, "lower on screen is lower in pitch");
    await shot("10-piano-roll-drawn");
    assert.ok((await d.invoke("project_undo")).ok);
    assert.equal((await notes()).length, 1, "each drawn note is one undo step");
    assert.ok((await d.invoke("project_redo")).ok);
    assert.equal((await notes()).length, 2);
    const smf = join(work, "drawn.mid");
    assert.equal((await d.invoke("midi_export", { path: smf })).ok, smf);
    assert.equal(readFileSync(smf).subarray(0, 4).toString(), "MThd");
  });

  it("routes a track through an aux bus and meters the real signal", async () => {
    assert.ok((await d.invoke("project_new", { discardChanges: true })).ok);
    const wav = join(work, "bus-tone.wav");
    writeFileSync(wav, sineWav(48_000, 3, 440));
    const imported = await d.invoke("media_import", { paths: [wav], trackId: null, start: 0, layout: "newTracks" });
    assert.ok(imported.ok, JSON.stringify(imported.err));
    const track = imported.ok.view.project.tracks[0].id;
    const aux = await d.invoke("project_apply", { command: { type: "addAux", name: "Bus", sources: [] } });
    const auxId = aux.ok.outcome.createdAuxes[0];
    const routed = await d.invoke("project_apply", { command: { type: "setStrip", channel: { type: "track", id: track }, patch: { output: { type: "aux", id: auxId } } } });
    assert.ok(routed.ok, JSON.stringify(routed.err));
    // A cycle is refused before it applies.
    const cycle = await d.invoke("project_apply", { command: { type: "setSend", channel: { type: "aux", id: auxId }, target: auxId, send: { target: auxId, levelDb: 0, preFader: false, muted: false } } });
    assert.equal(cycle.err.code, "invalidEdit");
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    await showMixer();
    await d.find(`[data-channel="aux:${auxId}"]`);
    await d.invoke("transport_command", { command: { type: "play", fromTick: 0, countIn: false, preRoll: false } });
    const peak = (key) => d.exec(`return document.querySelector('.bottom-panel [data-channel="${key}"] .strip__db-box--peak')?.textContent ?? ''`);
    await d.waitFor(async () => (await peak(`aux:${auxId}`)) !== "−∞" && (await peak("master")) !== "−∞", { message: "bus and master meters show the engine signal", timeout: 10_000 });
    const busDb = Number((await peak(`aux:${auxId}`)).replace("−", "-"));
    // Mono file on an auto (mono) strip: 0.5 × −3 dB centre pan law ≈ −9 dBFS.
    assert.ok(busDb > -10.5 && busDb < -7.5, `0.5-amplitude mono sine through the bus reads about −9 dBFS, got ${busDb}`);
    await shot("11-mixer-bus-meters");
    await d.invoke("transport_command", { command: { type: "stop" } });
    const file = join(work, "routing.elyvon");
    assert.ok((await d.invoke("project_save", { path: file })).ok);
    const reopened = await d.invoke("project_open", { path: file, discardChanges: false });
    assert.deepEqual(reopened.ok.project.tracks[0].strip.output, { type: "aux", id: auxId }, "routing is preserved on reopen");
  });

  it("writes automation from the mixer fader in Touch mode and undoes it as one step", async () => {
    await d.invoke("project_new", { discardChanges: true });
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    const wav = join(work, "lead.wav");
    writeFileSync(wav, sineWav(48_000, 8, 440));
    const imported = await d.invoke("media_import", { paths: [wav], trackId: null, start: 0, layout: "newTracks" });
    assert.ok(imported.ok, JSON.stringify(imported.err));
    const track = imported.ok.view.project.tracks[0];
    const channel = { type: "track", id: track.id };
    // Commands sent straight over IPC do not refresh the UI mirror.
    await d.exec("location.reload()");
    await d.find(".region--audio", { timeout: 10_000 });
    const vol = { channel, param: { type: "volume" } };
    const lane = async () => (await d.invoke("project_get")).ok.project.tracks[0].automation.lanes.find((l) => l.param.type === "volume");
    // Read mode: a lane at −96 dB silences the render; Off ignores it.
    assert.ok((await d.invoke("project_apply", { command: { type: "addAutomationPoints", target: vol, points: [{ tick: 0, value: -96 }] } })).ok);
    const silent = await d.invoke("project_render", { path: join(work, "auto-silent.wav"), startTick: 0, endTick: 3840, metronome: false });
    assert.ok(silent.ok, JSON.stringify(silent.err));
    assert.ok(silent.ok.peak[0] < 1e-4, "the lane silences the track: " + silent.ok.peak[0]);
    assert.ok((await d.invoke("project_apply", { command: { type: "setAutomationMode", channel, mode: "off" } })).ok);
    const off = await d.invoke("project_render", { path: join(work, "auto-off.wav"), startTick: 0, endTick: 3840, metronome: false });
    assert.ok(off.ok.peak[0] > 0.3, "Off plays the static fader");

    // The lane editor adds a point where it is clicked; undo removes it.
    await showEditor("Automatización");
    const canvas = await d.find("[data-testid=automation-editor] canvas", { timeout: 5000 });
    await d.click(canvas);
    await d.waitFor(async () => (await lane())?.points.length === 2, { message: "point added from the canvas" });
    await shot("12-automation-lane");
    await clickExtra('[aria-label^="Undo"]');
    await d.waitFor(async () => (await lane())?.points.length === 1, { message: "undo removes the point" });

    // Touch: dragging the fader while playing writes automation, not the fader.
    assert.ok((await d.invoke("project_apply", { command: { type: "removeAutomationLane", target: vol } })).ok);
    assert.ok((await d.invoke("project_apply", { command: { type: "setAutomationMode", channel, mode: "touch" } })).ok);
    await d.exec("location.reload()");
    await d.find(".region--audio", { timeout: 10_000 });
    await showMixer();
    assert.equal(await d.exec("return document.querySelector('.bottom-panel .strip__auto').value"), "touch", "the strip shows the mode");
    await d.invoke("transport_command", { command: { type: "locate", tick: 0 } });
    await d.invoke("transport_command", { command: { type: "play", fromTick: 0, countIn: false, preRoll: false } });
    await d.waitFor(async () => {
      const s = (await d.invoke("transport_state")).ok;
      return s && s.state === "playing" && s.seconds > 0.5;
    }, { message: "playing", timeout: 10_000 });
    const faderCss = '.bottom-panel .fader[aria-label="' + track.name + ' volume"]';
    await d.exec("document.querySelector(arguments[0]).scrollIntoView({ block: 'center' })", [faderCss]);
    const fader = await d.find(faderCss);
    await d.drag(fader, 0, -40, 12);
    await new Promise((r) => setTimeout(r, 700));
    await d.invoke("transport_command", { command: { type: "stop" } });
    const written = await d.waitFor(async () => {
      const v = (await d.invoke("project_get")).ok;
      return v.undoLabel === "Write Automation" ? v : null;
    }, { message: "written automation becomes one undo step", timeout: 10_000 });
    const t = written.project.tracks[0];
    assert.equal(t.volumeDb, 0, "the static fader is unchanged in Touch mode");
    const w = t.automation.lanes.find((l) => l.param.type === "volume");
    assert.ok(w && w.points.length >= 2, "points written: " + JSON.stringify(w));
    assert.ok(w.points.some((p) => p.value > 0.5), "the fader moved up while playing: " + JSON.stringify(w.points));
    assert.deepEqual(t.regions.map((r) => r.automation), [[]], "region lanes untouched");
    await clickExtra('[aria-label^="Undo"]');
    await d.waitFor(async () => !(await lane()), { message: "undo removes the written lane" });
  });

  it("comps three takes in the takes editor, exports them exactly and repairs moved media", async () => {
    await d.invoke("project_new", { discardChanges: true });
    await d.exec("location.reload()");
    await d.find(".controlbar", { timeout: 10_000 });
    const freqs = [220, 330, 495];
    const files = freqs.map((f, i) => {
      const p = join(work, "take" + (i + 1) + ".wav");
      writeFileSync(p, sineWav(48_000, 2, f));
      return p;
    });
    // Measure the files with the importer, then commit them as one cycle
    // recording (the edit the recorder issues after three passes).
    const imported = await d.invoke("media_import", { paths: files, trackId: null, start: 0, layout: "newTracks" });
    assert.ok(imported.ok, JSON.stringify(imported.err));
    const p0 = imported.ok.view.project;
    const track = p0.tracks[0];
    const assets = p0.media.map((m) => ({ ...m, id: randomUUID(), name: m.name.replace(".wav", "") }));
    const commit = await d.invoke("project_apply", {
      command: {
        type: "batch",
        commands: [
          { type: "removeTracks", ids: p0.tracks.slice(1).map((t) => t.id) },
          { type: "removeRegions", ids: [track.regions[0].id] },
          { type: "removeUnusedMedia" },
          { type: "importTakes", assets, placements: assets.map((a) => ({ mediaId: a.id, track: { type: "existing", id: track.id }, start: 0 })), recordedAt: 1 },
        ],
      },
    });
    assert.ok(commit.ok, JSON.stringify(commit.err));
    const takes = commit.ok.view.project.tracks[0].regions;
    assert.equal(takes.length, 3);
    const folder = takes[0].capture.folder;
    await d.exec("location.reload()");
    await d.find(".region--audio", { timeout: 10_000 });

    // Select a take (the three overlap; the last one is on top) and comp
    // with the mouse in the takes editor.
    await hideBottom();
    const regions = await d.findAll(".region--audio");
    await d.drag(regions[regions.length - 1], 0, 0, 1);
    await showEditor("Tomas y audio");
    await d.find("[data-testid=takes-editor]", { timeout: 5000 });
    assert.equal(await d.count("[data-testid^=take-]"), 3);
    await d.find('[data-testid="take-1"] [data-testid="waveform"][data-state="ready"]', { timeout: 15_000 });
    const lane1 = await d.find('[data-testid="take-1"]');
    await d.drag(lane1, 160, 0, 10);
    const comped = await d.waitFor(async () => {
      const t = (await d.invoke("project_get")).ok.project.tracks[0];
      return t.comps.length && t.comps[0].segments.some((s) => s.take === 1) ? t : null;
    }, { message: "dragging across take 1 comps it in", timeout: 5000 });
    assert.ok(comped.regions.every((r) => r.muted), "takes are kept and muted");
    await shot("13-takes-comp");

    // Exact thirds from takes 1, 2 and 3, then verify the exported samples.
    for (const [take, start, end] of [[1, 0, 1280], [2, 1280, 2560], [3, 2560, 3840]]) {
      const r = await d.invoke("project_apply", { command: { type: "compTake", trackId: track.id, folder, take, start, end } });
      assert.ok(r.ok, JSON.stringify(r.err));
    }
    const out = join(work, "comp.wav");
    const rep = await d.invoke("project_render", { path: out, startTick: 0, endTick: 3840, metronome: false });
    assert.ok(rep.ok, JSON.stringify(rep.err));
    const rate = rep.ok.sampleRate;
    const left = readFloatWav(out).filter((_, i) => i % 2 === 0);
    for (const [take, seconds] of [[0, 0.33], [1, 1.0], [2, 1.66]]) {
      for (let k = 0; k < 40; k++) {
        const f = Math.round(seconds * rate) + k * 7;
        const expected = 0.5 * Math.SQRT1_2 * Math.sin((2 * Math.PI * freqs[take] * f) / rate);
        assert.ok(Math.abs(left[f] - expected) < 0.01, "take " + (take + 1) + " at frame " + f + ": " + left[f] + " vs " + expected);
      }
    }
    // Flatten keeps the takes and is reversible.
    const flat = await d.invoke("project_apply", { command: { type: "flattenComp", trackId: track.id, folder } });
    assert.equal(flat.ok.view.project.tracks[0].regions.length, 6);
    const unflat = await d.invoke("project_apply", { command: { type: "unflattenComp", trackId: track.id, folder } });
    assert.equal(unflat.ok.view.project.tracks[0].regions.length, 3);

    // Normalise (region gain) and freeze (rendered file, inserts locked).
    const norm = await d.invoke("region_normalize", { ids: takes.map((r) => r.id), targetDb: -1 });
    assert.ok(norm.ok, JSON.stringify(norm.err));
    assert.ok(norm.ok.view.project.tracks[0].regions.every((r) => Math.abs(r.content.gainDb - 5.02) < 0.05));
    const frozen = await d.invoke("track_freeze", { id: track.id });
    assert.ok(frozen.ok, JSON.stringify(frozen.err));
    assert.ok(frozen.ok.view.project.tracks[0].freeze);
    assert.deepEqual((await d.invoke("mixer_status")).ok.staleFreezes, []);
    const refused = await d.invoke("project_apply", { command: { type: "compTake", trackId: track.id, folder, take: 2, start: 0, end: 100 } });
    assert.equal(refused.err.code, "invalidEdit", "frozen material cannot be edited");
    assert.ok((await d.invoke("project_apply", { command: { type: "unfreezeTrack", id: track.id } })).ok);

    // Save, move a take away, repair it by content, collect media, reopen.
    const dir = join(work, "comp-song");
    mkdirSync(dir, { recursive: true });
    const song = join(dir, "song.elyvon");
    assert.ok((await d.invoke("project_save", { path: song })).ok);
    const moved = join(work, "moved");
    mkdirSync(moved, { recursive: true });
    renameSync(files[1], join(moved, "take2.wav"));
    await d.invoke("media_rescan");
    const repair = await d.invoke("media_repair", { directory: moved });
    assert.ok(repair.ok, JSON.stringify(repair.err));
    assert.equal(repair.ok.report.relinked.length, 1);
    const collected = await d.invoke("media_collect");
    assert.ok(collected.ok, JSON.stringify(collected.err));
    assert.ok(collected.ok.project.media.every((m) => m.path.startsWith("Media/")), JSON.stringify(collected.ok.project.media.map((m) => m.path)));
    assert.ok((await d.invoke("project_save", { path: null })).ok);
    const cancelled = await d.invoke("media_backup_unused", { confirm: false });
    assert.equal(cancelled.err.code, "cancelled", "destructive clean-up needs confirmation");
    const reopened = await d.invoke("project_open", { path: song, discardChanges: false });
    assert.ok(reopened.ok, JSON.stringify(reopened.err));
    assert.equal(reopened.ok.project.tracks[0].comps[0].segments.length, 3);
    assert.deepEqual(readFileSync(join(moved, "take2.wav")), sineWav(48_000, 2, 330), "sources never modified");
  });

  it("explains why updates are unavailable instead of pretending", async () => {
    await clickExtraText("Ajustes");
    await d.clickCss(".settings__nav-item:nth-child(2)");
    const page = await d.textOf(".settings__content");
    assert.match(page, /development builds|signing key|installer/i);
    const r = (await d.invoke("update_check")).ok;
    assert.equal(r.status, "unavailable");
    await shot("05-settings-updates");
    await d.keys("\uE00C"); // Escape
  });
});
