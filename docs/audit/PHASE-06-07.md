# Phases 06–07 — audio capture and native MIDI

Date: 2026-10-16. Branch: `claude/gallant-fermat-h1epb3`, based on
`d3ae87a6928533d185b40e148249bf50a6a73c7f`. **Overall: PARTIAL.** Native functionality
and deterministic software checks exist; physical-device and cross-platform acceptance remain
open. The supplied phase documents are requirements, not evidence of completion.

## Changes to phases 04–05

Large audio sources decode/resample incrementally to a temporary PCM disk cache. A worker
fills 32 associative stereo pages of 4096 frames per source (about 1 MiB). The callback reads
atomic samples without disk I/O, locks or allocations. Offline export prepares each block,
disables speculative read-ahead and fails explicitly on missing samples instead of publishing
a silently damaged render. WAV and its JSON diagnostic report are staged before replacement.
The tests compare streaming against resident decoding, including compressed audio, resampling,
seeks, loops and twelve independently offset clips sharing one source under a 2 MiB budget.

MIDI splits retain the portion of crossing notes in both resulting regions. A cycle release
exactly at the callback boundary is deferred to offset zero of the next callback.

## Phase 06

| Requirement | Implementation and evidence | Outstanding acceptance |
| --- | --- | --- |
| Native input, arming, mono/stereo channel routing | `input.rs`, `recording.rs`; CPAL F32/I16/I32/U16 input; persisted `TrackIo`; inspector and Input dialog; invalid routes rejected before capture | Real interfaces, rate negotiation, hot-plug and platform drivers |
| Multitrack writing away from RT | Fixed capture packets and bounded SPSC to writer; immutable float WAV/BWF takes with UTC/time reference; `multitrack_record_stop_decode_restart_preserves_samples_and_bwf` | Sustained physical input under load |
| Monitor, clipping and latency | Bounded monitor queue, input peak/clipped frames, CPAL timestamp estimates, signed manual recording offset; pre-origin samples trimmed | Actual round-trip latency and direct hardware monitoring |
| Count-in, pre-roll, punch and cycle | Native clock history maps callbacks across transport boundaries; punch gating and per-lap takes; `countin_preroll_punch_cycle_and_compensation_gate_capture` | Voice/guitar punch-in and cycle recording on hardware |
| Take preservation and recovery | Checkpointed `.partial.wav` and JSON journal; cancellation, overflow, disconnect and write failure explicit; recover from aligned bytes; take selection and one-step import undo | Actual disk exhaustion/device removal while recording |
| Delay compensation | Validated DAG delay planner and bounded RT delay line; aligned short-file correlation with ambiguous data rejected | Active hosted plug-in graph/PDC, multiple independent device clocks, live input SRC |

One input device at the engine sample rate is supported, up to 32 channels. A failed device
start releases editing locks. Audio and MIDI currently record in separate sessions. Recording
blocks destructive edits and restart; a finished take remains available if project insertion
fails. Project schema 3 retains capture metadata, take selection and I/O routing; versions 1 and
2 migrate on open, and their frozen fixtures remain unchanged.

## Phase 07

| Requirement | Implementation and evidence | Outstanding acceptance |
| --- | --- | --- |
| Input/output, routing, filtering, monitor | `midi/ports.rs`, native MIDI owner, persisted per-track ports/channel filter/override; missing destinations reported without silently rerouting; bounded 256-byte SysEx | Real keyboard/interface, port hot-plug on each OS |
| Timeline scheduling | Frame offsets validated across callback sizes; packets/deadlines queued to OS worker; lateness and overflows reported | Physical MIDI timing/jitter; OS sends do not guarantee sample accuracy |
| Recording and virtual keyboard | Native clock timestamps, raw channel events plus piano-roll note projection, held notes and sustain closed at stop; recording backup before project commit; keyboard monitor uses the same native route | External keyboard recording, audible internal instruments |
| Panic and disconnect | Tracked note releases plus CC64/120/123; stop/seek/cycle/port changes; bounded queues fail closed | Physical stuck-note and cable removal tests |
| Standard MIDI files | Type 0/1 import, PPQ conversion, tempo/metre/channel/SysEx preservation, type-1 export; strict truncation checks and round-trip tests | Unsupported SMPTE, type 2, fragmented SysEx and mid-bar metre changes fail explicitly |
| Virtual ports | Backend APIs on Linux/macOS, platform gated; own virtual ports excluded from enumeration | OS/hardware validation; Windows virtual ports need an external driver |

No internal synthesizer, hosted instrument, MIDI 2.0/UMP or playback chase for notes beginning
before a seek is implemented. MIDI can drive an external destination; the virtual keyboard by
itself does not generate audio. Crop/split state chase is separate from playback chase.

## Reproducible validation

This run uses Debian 13 x86-64, Rust 1.99.1, Node 24.19, a rootless native-library sysroot,
Xvfb and D-Bus. No ALSA audio or sequencer device is exposed. Source the environment's
`/workspace/.elyvon-cloud/activate.sh` before native commands.

| Check | Result |
| --- | --- |
| `npm run check` | Pass: versions, TypeScript, ESLint, 37 tests, production frontend build |
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked` | Pass: 271 tests, no ignored/failing tests |
| Generated TypeScript binding comparison | Pass; obsolete `AudioSource.ts` removed |
| `cargo deny --locked check` | All policies pass; existing duplicate-dependency/yanked `zip 4.6.1` warnings remain |
| `actionlint -shellcheck= -pyflakes=` | Pass |
| `npm run tauri -- build --debug --no-bundle -- --locked` | Pass on Linux x86-64 |
| Native binary E2E under Xvfb/D-Bus | Pass: 13/13, including virtual MIDI capture, clean process exit/restart and SMF round-trip |

The suite exercises real IPC,
input failure/unlocking and virtual MIDI recording, save, process restart, reopening and SMF
round-trip. File dialogs are bypassed through their actual native commands. Synthetic capture
signals test the writer and renderer; they are not microphone or loopback hardware evidence.

This container requires a local WebKit helper-path binding through `proot` and headless software
compositing (`WEBKIT_DISABLE_COMPOSITING_MODE=1`); without the latter, WebKit's page process
crashes after the demo screenshot. These environment settings are confined to the test wrapper,
not application distribution. The WebDriver helper now consumes driver output and waits for
its owned application process to exit; recording/restart tests reject forced termination.

Linux software validation does not certify Windows/macOS builds, installers, ARM targets,
hardware latency, xruns or long-duration recording. No new release or deployment is implied.
