# Phases 04 and 05 — temporal engine, transport and tempo; clips and audio regions

**Date:** 2026-10-16. **Branch:** `claude/gallant-fermat-h1epb3`. **Overall: PARTIAL** — every
software criterion below has automated evidence, but no test ran on physical audio hardware and
incremental disk streaming is not implemented.

Entry state (audited): the engine reported `Unavailable`; `elyvon-audio` held contracts only. The
"native audio backend" prerequisite (Phase 03) did not exist, so the minimum engine both phases
need (lock-free callback engine, device output, internal clock) was built here — see
[ADR 0005](../ADR/0005-lock-free-plan-engine-and-in-memory-media.md).

## Acceptance criteria

| Criterion | Result | Evidence |
| --- | --- | --- |
| 04: playhead/transport reflect real engine events, not JS timers | **Met** | `audio://position` emitted from engine atomics; UI has no timer advancing time; native E2E clicks Play and observes engine seconds and the UI playhead move (`e2e/app.e2e.mjs` "imports real audio, plays it…") |
| 04: loop and metronome keep frame/bar accuracy | **Met** | `timing.rs::cycle_jumps_are_sample_accurate_at_block_boundaries` (blocks 1, 25, 175, 512, loop length, 4096); `ten_minute_tempo_map_render_has_exact_length_and_click_train` (1200 clicks, max error 0.0 vs independent reference); `metronome_follows_odd_and_compound_metres` (3/4, 6/8, 7/8) |
| 04: tempo/locator changes saved and reopened without drift | **Met** | Schema 2 + `v2-timeline.elyvon` fixture round-trip; `persistence.rs`; frames derived from ticks on load (no stored frames) |
| 05: ≥ 12 real clips play correctly and export a diagnostic render | **Met** | `session.rs::twelve_clip_project_renders_exports_and_reopens_identically`: 13 audible clips from float/16-bit WAV, AIFF, FLAC, MP3, Vorbis (44.1→48 kHz), exact sample comparison (< 1e-6) for 8 clips, RMS ±0.5 dB for lossy, exported WAV decodes bit-identical, JSON report |
| 05: region edits never modify source files; reopen plays the same sequence | **Met** | Same test: split/trim/fades/duplicate, save, reopen, re-decode → identical checksum and samples; every source file byte-identical; native E2E checks the imported WAV is unchanged |
| 05: waveforms/positions from real data | **Met** | `waveform_columns_come_from_the_real_samples`, `large_sessions_plan_and_zoom_within_budget` (20-min file, 2000 columns < 50 ms at 4 zoom levels, brute-force bounds); E2E waits for `data-state="ready"` canvas |

## Implemented vs pending

| Item | State |
| --- | --- |
| Ticks/beats ↔ seconds ↔ frames, bars, tempo changes, metre changes | Complete (`core/tempo.rs`) |
| Play/stop/pause/locate/cycle/count-in/pre-roll/metronome/tap tempo/markers | Complete (engine + UI) |
| Punch markers | Partial: stored, edited, shown; recording does not exist |
| Tempo change during playback | Complete: playhead remapped by musical position at plan swap (`tempo_change_during_playback_keeps_the_musical_position`) |
| Block scheduler, no drift, no double trigger, no stuck notes | Complete for scheduling (`midi_notes_fire_once…`, `cycle_wrap_and_stop_release…`); no instrument consumes MIDI yet; notes started before Play are not chased |
| Snap/grid/triplets/swing, zoom, scroll, ruler, playhead sync, undo/redo | Complete (Rust golden vectors mirrored in `src/lib/tempoMap.test.ts`) |
| Decoding WAV/AIFF/FLAC/MP3/Vorbis off the callback, licences checked | Complete (Symphonia MPL-2.0 unmodified; cargo deny ok) |
| Immutable source, regions by id, offset/gain/fades, mono/stereo, analysis cache | Complete; cache is in memory (content-keyed, survives reopen within a run) |
| Streaming blocks from disk prepared off RT | **Partial**: decoding/resampling happen off RT, but the decoded file is held in memory (budget 4 GiB / 512 MiB on 32-bit); no incremental disk streaming |
| Multi-track mix, sample-accurate sync, overlaps, crossfades, loops | Complete (`playback.rs`) |
| Import by dialog, drag and drop, multiple files, differing rates | Complete (rubato FFT, tested 22.05/44.1/48/96 kHz) |
| Move/cut/duplicate/split/trim, fades, crossfade, mute/solo, undo/redo, save | Complete (region solo = track solo; no per-region solo) |
| RT safety | Complete in tests: zero allocations/frees in `Engine::process` under play, locate, pause, count-in, cycle wraps, plan swap, odd callback sizes (`rt_alloc.rs`) |

## Commands run (this container, Ubuntu 24.04 x64, 4 vCPU)

| Command | Result |
| --- | --- |
| `npm run check` (versions, typecheck, eslint, vitest, vite build) | Pass — 37 frontend tests |
| `npm run check:rust` (fmt, clippy -D warnings, workspace tests, binding drift) | Pass — 237 Rust tests, 57 s |
| `cargo deny --locked check` (0.20.2) | advisories ok, bans ok, licenses ok, sources ok |
| `actionlint -shellcheck= -pyflakes=` (1.7.12) | Pass |
| `npx tauri build --debug --no-bundle` + `xvfb-run … node --test e2e/app.e2e.mjs` | 11/11 pass (new: import, play, Space-stop, render) |
| `xvfb-run … node e2e/measure.mjs` | Launch→ready 664–730 ms, native RSS ~190 MB (debug), timeline median 17 ms / p95 34 ms ([raw](native-metrics-phase-04-05-local.json)) |

GitHub Actions results for the pushed commits are recorded in the final report of the PR/merge.

## Platform matrix

| Target | Compiled here | Automated tests | Audio hardware |
| --- | --- | --- | --- |
| Ubuntu x64 | Yes (debug) | Rust, frontend, native E2E under Xvfb (internal clock) | **Not tested** — no sound card in the container |
| Windows x64 / x86 | CI (`windows-latest`) | Installer build only | Not tested |
| Windows ARM64, macOS Intel/Apple Silicon, Ubuntu ARM64 | Not built in regular CI | — | Not tested; no support claim |

cpal backends: WASAPI (Windows), CoreAudio (macOS), ALSA (Linux; PipeWire/Pulse through the ALSA
plug-in). JACK/ASIO are not enabled. Plug-ins are not part of these phases.

## Bugs, limitations, risks

- No physical device test: latency, xruns, device hot-plug and sample-rate negotiation on real
  hardware are unverified. A failed stream switches to the internal clock and is reported.
- Debug builds: DSP crates are compiled with opt-level 2 in the dev profile so `tauri dev` keeps up.
- Decoded audio lives in memory; very long sessions hit the budget with a clear `tooLarge` error.
- Relative media paths resolve against the project folder; imports store absolute paths.
- MIDI notes that started before the play position are not chased; MIDI is not audible (no instrument).
- Editing during playback releases sounding MIDI notes (by design, avoids stuck notes).
- Metronome clicks every beat of the denominator (eighths in 6/8 and 7/8).
- Schema 2 files cannot be opened by 0.1.x builds (refused, not misread).

## Next phase

Recording (input streams, punch in/out, latency measurement and compensation), incremental disk
streaming, device validation on every target, then instruments so scheduled MIDI becomes audible.


## Follow-up on this branch (2026-10-16)

The incremental-disk-streaming limitation above is resolved in the subsequent implementation:
large sources decode in bounded packets to a PCM cache, rubato runs chunk by chunk, and a worker
feeds a bounded associative page cache. `streaming.rs` compares resampled samples to the resident
path and renders twelve differently offset clips from one streamed source without hash-slot
collisions. Diagnostic export writes blocks to a staged WAV; source/read failures preserve the
previous export. Cache underruns are counted and failed diagnostic renders cannot be published
as successful files. MIDI cuts now retain crossing-note tails, and cycle note releases are queued
into the next valid callback when they land exactly at the boundary.

See [Phases 06–07](PHASE-06-07.md) for the new recording/MIDI implementation, reproducible checks
and remaining physical-device/platform acceptance. The historical measurements above describe
the earlier commit, not a claim about the follow-up's hardware performance.
