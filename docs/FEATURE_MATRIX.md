# Feature implementation matrix

States: **Implemented** = real code and referenced checks; **Partial** = a working subset;
**Simulated** = explicitly labelled visual/test data; **Absent** = no backend. A passing build
is not device verification. Platform evidence and outstanding acceptance are in [STATUS](STATUS.md).

| Area | State | Actual behaviour | Evidence / missing part |
| --- | --- | --- | --- |
| Session, tracks, regions, timeline data | Implemented | Rust model, stable IDs, integer PPQ 960, validated edits, musical settings | project/model.rs, edit.rs, validate.rs; Rust tests and generated bindings |
| Save/open/save-as, undo/redo | Implemented | Atomic .elyvon schema-2 files (schema 1 migrated on open), dirty tracking, two-phase save; every timeline/audio edit is one undo step | tests/persistence.rs (v1 + v2 fixtures); native e2e/app.e2e.mjs |
| Recovery and unsaved guards | Implemented | Recovery snapshots, backend refuses destructive operations without explicit discard | recovery/session tests; native E2E |
| GUI process restart + reload saved file | Implemented automated check | Stops native process, starts another, opens saved project and compares model | Native E2E; file dialogs are bypassed via real native IPC |
| Arrangement | Implemented (Phase 05 scope) | Move, trim (non-destructive), split, duplicate, cut/copy/paste, region mute, gain, fades, loop; metre-aware ruler/grid; snap with triplets/swing; markers; engine-driven playhead with follow | timeline_edit.rs tests; src/lib/tempoMap.test.ts golden vectors; native E2E. No comping/flex/sample editor |
| Piano roll | Partial | Read-only note display | src/features/editors; native screenshot E2E; editing absent |
| Mixer | Partial | Volume (dB), constant-power pan (mono)/balance (stereo), mute/solo applied by the engine; real post-fader peak meters | playback.rs tests; no sends, buses, inserts, automation or master fader processing |
| Waveforms | Implemented | Real min/max columns from a peak pyramid of the decoded file, through tempo map/offset/loop; LRU cache, only the visible part is requested. Demo placeholders keep their labelled illustration | media/peaks.rs, waveform.rs, session.rs (zoom test, 20-min file); RegionWaveform.tsx |
| Meters | Implemented (peak) | Track peaks from the callback via atomics, ~30 Hz events; simulation only offered when the engine is offline | transport.rs, Mixer.tsx; no RMS/LUFS/master meter |
| Audio engine and transport | Implemented, hardware unverified | Lock-free RT engine; play/stop/pause/locate, cycle, count-in, pre-roll, metronome, tap tempo; tempo/metre map; cpal device output with internal-clock fallback | timing.rs, playback.rs, rt_alloc.rs, device.rs tests; native E2E plays under Xvfb on the internal clock. **No physical device test** |
| Audio files (import, decode, resample) | Implemented, in-memory | WAV/AIFF/FLAC/MP3/Ogg Vorbis via Symphonia, any rate resampled (rubato FFT); dialog, OS drag-and-drop, browser; missing/changed files reported; relink | media.rs, session.rs tests; native E2E import. **Disk streaming absent** (memory budget) |
| Offline render / diagnostic export | Implemented | Same engine path, 32-bit float WAV + JSON report (levels per channel/track, checksum) | session.rs (12 clips, export decode-back), timing.rs (10-min render); native E2E |
| Recording, punch recording, input monitoring | Absent | Record disabled; punch range stored/shown only | No input stream |
| MIDI scheduling | Partial | Sample-accurate note on/off per block, ordered, no double triggers, released on stop/locate/cycle/edit | timing.rs; events are not sent to any instrument or port yet; no chase of notes started earlier |
| Audio graph contract | Partial | Versioned acyclic channel-routing proposal, bounds and cycle checks | graph-v1.json and Rust tests; no compiled graph/latency compensation/execution |
| AudioProcessor/AtomicF32/gain-pan | Partial | Laws used by the engine; plan swap via SPSC queues | No processor graph/plug-in hosting yet |
| Background jobs contract | Partial | Versioned snapshots/errors/progress/cancellation states | core/tests/jobs.rs + jobs-v1.json; no job scheduler or worker |
| UI state | Implemented | Typed presentation state, layout persisted through Rust settings; selection not saved in project | uiContract.ts, ui.ts; settings/store tests |
| File browser | Partial | Folder navigation, project open, double-click audio import | browser.rs; no preview playback or loop library |
| Instruments, plug-ins, FX, final mixdown formats | Absent | Unavailable controls/disabled slots | Only the diagnostic float WAV render exists |
| Live Loops, step sequencer, chord popup | Simulated | Separate static reference.html | src/reference; not native production features |
| Update policy/signature validation | Partial | Platform/installer/SemVer rules, signature fixtures, unsaved guard | update crate tests + updater.rs; production signed installation not tested |
| Stable/beta channels | Partial | Stable rejects prereleases | version.rs; no independent beta endpoint or channel preference |
| Windows x64/x86 distribution | Partial | Separate unsigned EXE/MSI + hashes in an actual Release | Release preview 2; no physical host install/launch test |
| Other five distribution targets | Absent verification | Target naming and legacy workflow configuration only | STATUS support table; no support claim |
| Quality/dependency checks | Implemented with explicit debt | Shared local/CI commands, bindings, npm audit, cargo deny, actionlint, inventories | ci.yml; B-01 maintenance exception and licence obligations remain |
| Native performance baseline | Partial | Reproducible launch/RSS/demo timeline measurement runner | e2e/measure.mjs; diagnostic debug/Xvfb, no audio or hardware budget claim |
