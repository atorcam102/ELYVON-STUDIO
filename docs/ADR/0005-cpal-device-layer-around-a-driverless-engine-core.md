# 5. CPAL device layer around a driver-free engine core

Date: 2026-10-15. Status: accepted (Phase 03).

**Context.** The engine must run on WASAPI, CoreAudio and ALSA/PipeWire/JACK, be testable in CI
without sound cards, and keep the audio callback free of allocation, locks, I/O and logging.

**Decision.** Split the engine in two crates. `elyvon-audio` holds everything the callback runs
(`EngineCore`, wait-free rtrb command queue, atomic telemetry, format conversion) with no device
dependency, plus a deterministic offline renderer. `elyvon-audio-device` wraps CPAL 0.18 (Apache-2.0,
`realtime` feature for MMCSS/RT priority) behind a small `Backend` trait, adds an explicit headless
clock, and a manager thread that owns streams and handles fallback, faults and reconnection.
Default CPAL hosts only: WASAPI, CoreAudio, ALSA (which reaches PipeWire/PulseAudio via plug-ins).

**Alternatives.** Direct per-OS APIs (more code, three times the RT review); PortAudio (C build
and licence review, weaker Rust integration); enabling CPAL's JACK/PipeWire/PulseAudio/ASIO
features now (extra system libraries or the Steinberg SDK licence, and no device to test).

**Consequences.** Core behaviour is proven headlessly on every CI run; device code is small and
replaceable. CPAL limits exclusive-mode WASAPI and offers no ASIO without the SDK. Every target
still needs a physical device test before support is declared ([ADR 3](0003-separate-intended-targets-from-verified-distribution.md)).
`audio_thread_priority` is MPL-2.0: its notice and source availability must be honoured when shipping.
