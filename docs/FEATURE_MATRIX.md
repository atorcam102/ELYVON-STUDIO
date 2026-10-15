# Feature implementation matrix

States: **Implemented** = real code and referenced checks; **Partial** = a working subset;
**Simulated** = explicitly labelled visual/test data; **Absent** = no backend. A passing build
is not device verification. Platform evidence and outstanding acceptance are in [STATUS](STATUS.md).

| Area | State | Actual behaviour | Evidence / missing part |
| --- | --- | --- | --- |
| Session, tracks, regions, timeline data | Implemented | Rust model, stable IDs, integer PPQ 960, validated edits, musical settings | project/model.rs, edit.rs, validate.rs; Rust tests and generated bindings |
| Save/open/save-as, undo/redo | Implemented | Atomic .elyvon schema-1 files, dirty tracking, two-phase save | tests/persistence.rs; native e2e/app.e2e.mjs |
| Recovery and unsaved guards | Implemented | Recovery snapshots, backend refuses destructive operations without explicit discard | recovery/session tests; native E2E |
| GUI process restart + reload saved file | Implemented automated check | Stops native process, starts another, opens saved project and compares model | Native E2E; file dialogs are bypassed via real native IPC |
| Arrangement | Partial | Region move, snap, zoom, MIDI region creation, cursor/cycle editing | src/features/timeline; native E2E; no audio import/waveform editing |
| Piano roll | Partial | Read-only note display | src/features/editors; native screenshot E2E; editing absent |
| Mixer | Partial | Saved fader/pan/mute/solo project parameters | Rust edit tests, frontend store and native UI E2E; no audio processing |
| Audio waveforms in demo | Simulated | Labelled static illustration for demo audio with source:null | DemoWaveform.tsx; no decoder or audio file |
| Audio engine core (RT callback) | Implemented, headless-verified | EngineCore: silence, transport, monotonic clock, test tone, peak meters, clamping, format conversion; no allocation in callback | audio/tests/realtime.rs (64–512 frames × 44.1/48/96 kHz, allocation counter); [AUDIO_ENGINE](AUDIO_ENGINE.md) |
| Transport play/stop/locate | Partial | Real engine transport from the UI (Space, buttons), playhead follows engine clock | native E2E on headless clock; tracks are **not rendered** (silence + optional tone) |
| Device enumeration/selection | Partial | Hosts, inputs/outputs, ranges, rates, buffers, formats, bit depth, defaults; Settings › Audio selection | CPAL backend; listing tested here (ALSA) and in Windows/macOS CI jobs; no physical-device validation |
| Test tone + output meters | Implemented, no hardware check | Opt-in sine (≤ −6 dBFS, ramped), real output meters in mixer and Settings | realtime.rs, E2E reads −18 dBFS; audible output on real speakers untested |
| Track meters | Simulated | Explicit opt-in simulation, off by default; tracks produce no signal yet | src/features/mixer |
| Input metering | Partial | Optional input stream for level meters only | manager tests (fake backend); no capture/recording; untested on hardware |
| Device preferences/fallback | Implemented, no hardware check | Per-device rate/buffer profiles in settings.json; fallback to default with actionable notice; switch-back | types.rs + manager.rs tests; settings migration test |
| Hot-unplug / missing / busy device | Partial | Fault → deviceLost → reconnect; noDevice retry; busy error; project untouched | manager tests with scripted backend; **no physical unplug test** |
| Xrun/dropout statistics | Implemented | Backend xruns, late callbacks, timing gaps, errors, clipping, load; logged off the callback | telemetry tests; container probe evidence |
| Headless clock | Implemented | Explicit "no sound output" device for CI and machines without hardware | manager + E2E tests |
| Recording | Absent | Record button disabled with explanation | No capture-to-disk implementation |
| MIDI event contract | Partial | Version-1 MIDI 1.0 event types and validation of channel/data/block offsets | audio/tests/contracts.rs + midi-v1.json; no port/scheduler/MIDI 2.0/SysEx |
| Audio graph contract | Partial | Versioned acyclic channel-routing proposal, bounds and cycle checks | graph-v1.json and Rust tests; no compiled graph/latency compensation/execution |
| AudioProcessor/AtomicF32/gain-pan | Partial | Thread contract, atomic scalar and mathematical laws | audio Rust tests; graph executor/graph swap not implemented |
| Background jobs contract | Partial | Versioned snapshots/errors/progress/cancellation states | core/tests/jobs.rs + jobs-v1.json; no job scheduler or worker |
| UI state | Implemented | Typed presentation state, layout persisted through Rust settings; selection not saved in project | uiContract.ts, ui.ts; settings/store tests |
| File browser | Partial | Folder navigation and project open | browser.rs; no playable loop/instrument library |
| Instruments, plug-ins, FX, bounce/export | Absent | Unavailable controls/disabled slots | No native implementation |
| Live Loops, step sequencer, chord popup | Simulated | Separate static reference.html | src/reference; not native production features |
| Update policy/signature validation | Partial | Platform/installer/SemVer rules, signature fixtures, unsaved guard | update crate tests + updater.rs; production signed installation not tested |
| Stable/beta channels | Partial | Stable rejects prereleases | version.rs; no independent beta endpoint or channel preference |
| Windows x64/x86 distribution | Partial | Separate unsigned EXE/MSI + hashes in an actual Release | Release preview 3 (current) and preview 2; no physical host install/launch test |
| Other five distribution targets | Absent verification | Target naming and legacy workflow configuration only | STATUS support table; no support claim |
| Quality/dependency checks | Implemented with explicit debt | Shared local/CI commands, bindings, npm audit, cargo deny, actionlint, inventories | ci.yml; B-01 maintenance exception and licence obligations remain |
| Native performance baseline | Partial | Reproducible launch/RSS/demo timeline measurement runner | e2e/measure.mjs; diagnostic debug/Xvfb, no audio or hardware budget claim |
