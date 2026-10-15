# Native audio engine and devices (Phase 03)

**State: PARTIAL.** The engine, device layer, transport, sample clock, test tone and real
meters are implemented and tested headlessly, through the real ALSA path in a container and
through the native GUI. **No physical speaker, interface or hot-unplug test has been performed
yet**, so device support is not declared for any platform (see the matrix below).

## What exists

| Piece | Where | What it does |
| --- | --- | --- |
| Engine core | `crates/elyvon-audio/src/render.rs` | `EngineCore`: the code the audio callback runs. Transport play/stop/locate, monotonic sample clock, silence rendering, opt-in sine test tone with 10 ms click-free ramps, per-channel peak meters, clamping/clip counting, planar→interleaved conversion. |
| Command queue | `command.rs` | Wait-free SPSC ring buffer ([rtrb](https://crates.io/crates/rtrb)) of `Copy` commands, max 64 applied per block. |
| Telemetry | `telemetry.rs` | Lock-free atomics written by the callback (peaks held until read, clocks, xrun/late/gap/error counters, latency) and read by a control thread. |
| Formats | `format.rs` | f32/f64/i8/i16/i32/i64/u8/u16/u32/u64 output and input conversion, symmetric integer scaling, NaN→silence, clipping counts, mono/stereo/multichannel layouts. |
| Headless renderer | `offline.rs` | Deterministic driver-free rendering in callback-sized blocks, plus signal analysis (peak, RMS, frequency). Used by CI. |
| Device backend | `crates/elyvon-audio-device/src/cpal_backend.rs` | [CPAL 0.18](https://crates.io/crates/cpal) with the `realtime` feature: enumeration (hosts, inputs/outputs, configuration ranges, sample rates, buffer ranges, formats, bit depth, defaults) and stream opening with format negotiation. |
| Headless clock | `headless.rs` | An explicitly selectable "device" that runs the real core paced by the system clock and discards the audio. Never chosen automatically; labelled "no sound output". |
| Device manager | `manager.rs` | One control thread that owns streams, applies preferences, falls back, detects faults, reconnects and logs statistics. |
| Desktop integration | `src-tauri/src/commands/audio.rs` | IPC: status, devices, configure, reconnect, telemetry, play/stop/locate (ticks↔frames from project tempo), test tone. Status changes are pushed as `audio://status` events. |
| UI | `src/state/audio.ts`, `src/features/settings/AudioSettings.tsx`, `TopBar.tsx`, `Mixer.tsx` | Play/stop (Space), playhead follows the engine, status pill, Settings › Audio, real output meter strip in the mixer. |

The core crate has no device dependency, so `cargo test -p elyvon-audio` runs anywhere.
The engine is independent of Tauri: `elyvon-audio-device` is used by the desktop app and by
the `audio-probe` example without any webview.

## Threads and real-time rules

```
UI (webview) ──IPC──▶ Tauri commands ──▶ AudioManager handle ──mpsc──▶ manager thread
                                             │  rtrb commands (wait-free)      │ owns streams
                                             ▼                                 ▼
                                      audio callback (CPAL / headless) ── atomics ──▶ Telemetry
```

* The callback does only `EngineCore::process_interleaved` plus `CallbackMonitor::observe`:
  no allocation/deallocation, no lock, no I/O, no logging, no blocking call. `Instant::now`
  (vDSO / QueryPerformanceCounter) is the only time source used inside it.
* The error callback only sets atomics (fault, xrun, error, real-time-denied flags).
* All buffers are allocated before the stream starts (`EngineCore::new`). Device blocks of any
  size are split into ≤1024-frame chunks, so output is bit-identical for every buffer size.
* Dropping a stream joins its callback thread before a new stream is created.
* Statistics are logged by the manager thread every 10 s when counters changed, never in the
  callback. Repeated open failures are logged once per distinct message.
* The UI polls telemetry at 20 Hz only while playing, while the tone sounds or while Settings ›
  Audio is open. An idle engine generates no IPC.

## Transport and clock

`clockFrames` counts every rendered frame since start-up and stays monotonic across device
changes. `transportFrames` advances only while playing; stop keeps the position. Play starts
from the edit cursor; the UI cursor follows `positionTicks` (frames → ticks with the project
tempo in Rust). Moving the cursor while playing relocates the engine. Changing sample rate
converts the position so the musical time is preserved. Tracks and regions are **not rendered
yet**: playing produces silence plus the optional test tone. Recording is disabled.

## Measuring xruns and dropouts

Three independent counters, all published from the callback with relaxed atomics:

1. **Backend xruns** — reported by the backend itself (`cpal::ErrorKind::Xrun`, e.g. ALSA EPIPE).
2. **Late callbacks** — processing time exceeded the block's real-time budget.
3. **Timing gaps** — heuristic: the interval between two callbacks exceeded 1.75× the previous
   block's duration + 2 ms. A probable dropout; jitter can also trigger it, so it is labelled
   "probable" in the UI.

Plus stream errors, clipped samples, maximum callback load and the driver-predicted output delay.

## Devices, preferences and failures

* Preferences live in `settings.json` (`audio`): enabled, host, output device id and name,
  optional input (metering only), and **per-device profiles** (sample rate, buffer). Older
  settings files load with defaults. `settings_set` never overwrites them; only
  `audio_configure` does.
* Negotiation prefers the device's default channel count and f32, then i32, i16, f64, u16, …
  Unsupported rates/buffers are adjusted with a `configAdjusted` notice; a fixed buffer the
  backend rejects is retried with the device default. 24-bit packed and DSD formats are listed
  but not streamed (`unsupportedFormat` if nothing else exists).
* Missing preferred device → the system default is used with a `preferredDeviceMissing` notice;
  the preference is kept, and the manager switches back automatically when it reappears **and**
  nothing is playing. "Use preferred device" forces a retry.
* No device → `noDevice`, retried every 2 s. Busy device (exclusive mode / another app) →
  `error` with `deviceBusy`, retried. Permission denied → actionable privacy-settings message.
* Hot-unplug → the error callback sets a fault; the manager closes the stream, reports
  `deviceLost`, and reconnects to the preferred or default output when available. After any
  reconnection the transport is stopped and the tone is off. A lost input never stops output.
* None of this touches project data: audio state is not part of `.elyvon` schema 1.

## Platform matrix (audio)

"Compiled" = built in CI or here. "Type-checked" = `cargo check` only (no link, no run).
"Device-verified" requires a physical device test and is **not claimed anywhere yet**.

| Platform | Backend in this build | Evidence today | Device-verified |
| --- | --- | --- | --- |
| Windows x64 | WASAPI (shared mode) | Type-checked here; CI builds the app and runs the audio crate tests + device listing on `windows-latest` | No |
| Windows x86 | WASAPI (shared mode) | Type-checked here; same CI job for `i686-pc-windows-msvc` | No |
| Windows ARM64 | WASAPI | Type-checked (here and CI `audio-targets`) | No |
| macOS Intel | CoreAudio | Type-checked (here and CI `audio-targets`) | No; microphone usage text/entitlement added, untested |
| macOS Apple Silicon | CoreAudio | Type-checked; CI `audio-macos` runs the audio crate tests on `macos-latest` (PRs/main) | No |
| Ubuntu x64 | ALSA (PipeWire/PulseAudio through their ALSA plug-ins) | All tests here; real CPAL→ALSA stream on the `null` PCM; native GUI E2E | No (container has no sound card) |
| Ubuntu ARM64 | ALSA | Not built (no cross sysroot / regular runner) | No |

Not compiled in: **ASIO** (Steinberg SDK licence; no device to test), **JACK** and native
**PipeWire/PulseAudio** hosts (extra system libraries at build and run time). WASAPI exclusive
mode is not offered. These are deliberate, documented gaps, not hidden features.

## Verification

Automated (see [Phase 03 validation](audit/PHASE03_VALIDATION.md) for commands and results):

* `crates/elyvon-audio/tests/realtime.rs`: buffers 64/128/256/512 × 44.1/48/96 kHz; **zero heap
  operations** in the callback path for f32/f64/i16/i32/u16/u8 (counting global allocator with a
  self-test), bounded command bursts, exact silence, sample-accurate transport, tone frequency
  (±0.05 Hz) and level (±0.001), meter readings, bit-identical output across buffer sizes,
  click-free ramps, single-channel tone, mono/8-channel/odd buffers, integer output.
* `crates/elyvon-audio-device/tests/manager.rs`: scripted backend for hot-unplug with fallback and
  switch-back, unplugging the only device, no hardware at start, busy device, per-device profiles,
  position across rate changes, input loss; plus the real headless clock end to end.
* `e2e/app.e2e.mjs`: the real desktop binary plays on the headless clock from the transport button,
  the LCD follows, tone meters read −18 dBFS in IPC and the mixer, Settings › Audio shows the stream,
  the project stays untouched, disabled engine refuses transport with an explanation.

Manual hardware procedure (to be run on each platform before declaring support):

```bash
cargo run --release -p elyvon-audio-device --example audio-probe -- --list
cargo run --release -p elyvon-audio-device --example audio-probe -- \
    --device "<id from --list>" --rate 48000 --buffer 256 --seconds 20 --json report.json
```

Expected: an audible 440 Hz tone at −18 dBFS, meters near −18, zero backend xruns at a sane
buffer size. Unplug the device during the run: the status must become `deviceLost`/fallback
without a crash; replug: it must recover. Repeat at 64/128/512 frames and 44.1/96 kHz, and in
the desktop app (Settings › Audio → Play test tone; Space to play/stop).
