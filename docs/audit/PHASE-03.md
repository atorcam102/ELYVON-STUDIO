# Historical Phase 03 evidence

This report describes commits `a141b5e`/`9f60871`, before integration of the
arrangement and recording engine. It does not describe the current IPC or device
diagnostic controls. See [ADR 0006](../ADR/0006-converge-phase03-and-arrangement-engines.md).


# Phase 03 validation evidence — native audio engine and devices

Date: 2026-10-15. Entry: main `22634ed`. Branch `claude/elegant-feynman-biwgpv`.
Container: Ubuntu 24.04.5 x86_64, 4 logical CPUs, Rust 1.97.0 stable (+1.88/1.90 for MSRV),
Node 22.22.0. **No sound card** in the container; ALSA exposes only its `null` PCM.
Nothing below is a physical-device result.

## Commands and observed results (this container)

| Command | Result |
| --- | --- |
| `npm run check` | Passed: versions, typecheck, ESLint (0 warnings), **36 Vitest tests** (was 31; +5 audio store), production build |
| `npm run check:rust` | Passed: fmt, clippy `-D warnings`, **221 Rust tests** (Phase 02 entry: 154), binding drift clean |
| `cargo +1.88 check --workspace --locked` (on entry main) | **Failed**: `tauri-utils@2.10.2 requires rustc 1.90` (Phase 02 MSRV claim was wrong) |
| `cargo +1.90 check --workspace --all-targets --locked` | Passed (entry main and Phase 03 tree) |
| `cargo check --locked --target <t> -p elyvon-audio -p elyvon-audio-device --all-targets` for x86_64/i686/aarch64-pc-windows-msvc, x86_64/aarch64-apple-darwin | Passed — **type-check only**, nothing linked or run |
| `cargo-deny 0.20.2 --locked check` | Passed (advisories, bans, licences, sources); duplicate-version warnings only |
| `actionlint 1.7.12 -shellcheck= -pyflakes=` | Passed |
| `npx tauri build --debug --no-bundle` | Passed (native Linux binary) |
| `xvfb-run … node --test e2e/app.e2e.mjs` | **12/12 passed in 10 of 10 consecutive runs** on the real binary (WebKitWebDriver 2.52, Xvfb 1280×800), incl. 2 new engine tests, after the two fixes below |
| `xvfb-run … node e2e/measure.mjs` | Passed (`status: measured`); the engine starts on its own thread without blocking start-up |

New Rust tests by suite: `elyvon-audio` unit 36 (format, command queue, telemetry, offline),
`realtime.rs` 13, `elyvon-audio-device` unit 17 (negotiation, error mapping, preferences,
listing real devices), `manager.rs` 9, `src-tauri` +2 (settings migration, ticks↔frames),
`elyvon-core` tempo inverse.

### Real-time checks (`crates/elyvon-audio/tests/realtime.rs`)

* `allocator_detector_is_live` proves the counting allocator sees allocations, so the zero
  results below are meaningful.
* 0 heap operations over 200 callbacks for each of 4 buffer sizes × 3 rates × 6 sample types
  (f32, f64, i16, i32, u16, u8), including applying queued commands; also for planar output,
  device blocks larger than the internal chunk, and a 200-command burst (bounded at 64/block).
* Tone at 1 kHz: frequency error < 0.05 Hz, peak within 0.001 of −12 dBFS, RMS = peak/√2,
  meter −12.00 ± 0.01 dBFS, 0 clipped samples, for every rate/buffer combination.
* Output bit-identical for buffers 64/128/256/512 vs. one 8269-frame block at each rate.
* Transport/clock sample-exact; stop keeps position; exact silence when stopped.

### Device manager (`crates/elyvon-audio-device/tests/manager.rs`)

Scripted backend: hot-unplug → fallback to built-in with notice → transport stopped, position kept,
tone off → preferred device returns → switch back while idle; only device unplugged →
`deviceLost`/`noDevice` without event flooding → recovery; no hardware at start; busy device
(`deviceBusy`, retried); per-device profile at 96 kHz/256 with position converted 3 s → 3 s;
input missing/lost never stops output. Real headless clock: plays, tone meters −18 ± 0.5 dBFS,
stop holds position, thread joins on drop. All 9 passed in 3 consecutive runs.

### Real CPAL → ALSA stream ([raw JSON](data/phase03-probe-alsa-null.json))

SHA-256 `4cd971a024566aa7c21b7112a9f4c41886ab409070c22b84c8ea2cf4ff758b34`.

`audio-probe --device alsa:null --rate 48000 --buffer 256 --seconds 4` (release): stream opened as
ALSA, f32, 2 ch, 256 frames (reported 256), meters −18.0/−18.0 dBFS, 508 864 callbacks,
0 backend xruns, 0 late callbacks, 0 timing gaps, 0 errors. **Caveat:** ALSA's `null` PCM has no
clock and consumes samples as fast as possible (≈ 2696 s of audio in 4 s), so this validates the
backend/callback/conversion path, not real-time pacing or audibility. The system "default" device
has no backing card here and correctly reports `noDevice` with an actionable message.

### Headless clock ([raw JSON](data/phase03-probe-headless.json))

SHA-256 `65f39b563b1af18a28ecf578de92b9e567f86bd861347d77be9cdc126b6f5916`.

`audio-probe --host headless --device headless:clock --rate 48000 --buffer 256 --seconds 5` (release):
239 872 frames in 4.98 s (≈ 48 000 frames/s), 937 callbacks, meters −18.0 dBFS, 0 late callbacks,
**1 timing gap** (sleep jitter in a shared container; recorded, not discarded), max callback load
24.7 % (start-up spike). This clock is software-paced; it is not a device latency measurement.

### Native GUI (`e2e/app.e2e.mjs`)

The binary configured the headless clock through IPC, played from the transport button, the
transport passed 960 ticks, the LCD changed, the test tone read −18 ± 0.5 dBFS through IPC and
−18.0 in the mixer output strip, a 0 dBFS request was clamped to −6, Stop held the position,
Settings › Audio showed 48000 Hz and the saved device, the project and dirty flag were unchanged,
preferences persisted with a 256-frame profile, and a disabled engine refused `audio_play` with
an actionable `audioUnavailable` error. Screenshots: [`docs/screenshots/phase-04-05-linux/`](../screenshots/phase-04-05-linux/).

## Defects found and fixed while validating

1. **Play/telemetry race (Phase 03 UI).** The UI polled telemetry right after `audio_play`; the
   audio thread applies commands on its next callback, so the first poll could still read
   `playing=false`, polling never started and the playhead stayed still (2 of 3 runs failed
   "LCD follows the playhead"). Fixed with an explicit transport intent in `src/state/audio.ts`;
   covered by a Vitest case.
2. **E2E harness exit hang (pre-existing since Phase 02).** `e2e/webdriver.mjs` spawned
   tauri-driver with an unread stdout pipe that the app inherits and logs to, and killed the
   driver immediately after `DELETE /session`. Measured with 10 consecutive runs each:
   Phase 02 `main` left orphaned app processes in 6/10 runs and hung once; this branch (which
   logs more) hung 3/10. After the fix (no unread pipes; wait up to 10 s for the app to exit,
   then kill and report it), 10/10 runs were clean with no orphans, and the suite now asserts
   that the app exits on its own when its window closes.
3. **Bounded shutdown.** `AudioManager` drop waits at most 3 s for the manager thread, so a
   driver hung inside the OS cannot block quitting (`a_hung_driver_never_blocks_quitting`).

## CI added in this phase (results appear on the pushed branch)

* `rust`, `e2e`, `msrv` and Linux release builds install `libasound2-dev`.
* `msrv`: `cargo check --workspace --all-targets --locked` on Rust 1.90.
* `audio-targets`: type-check of Windows ARM64 and macOS x64/ARM64 audio paths.
* `audio-macos` (PRs, main, dispatch): audio crate tests + device listing on `macos-latest`.
* Windows x64/x86 build jobs: audio crate tests + WASAPI device listing before bundling.
* `e2e`: device inventory and a 5 s headless engine run uploaded in `native-evidence`.

## Not verified (pending hardware)

Audible tone on any speaker/interface; WASAPI/CoreAudio/ALSA streams on physical devices;
physical hot-unplug and busy-device behaviour; buffer sizes reached on real drivers and real
latency; input metering on a microphone; macOS microphone permission flow; Ubuntu ARM64 build;
ASIO (not implemented: SDK licence and no device), JACK/native PipeWire (not compiled in).
