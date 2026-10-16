# Roadmap

Phases describe real capabilities, not completion inferred from source configuration.
[STATUS](STATUS.md) and [FEATURE_MATRIX](FEATURE_MATRIX.md) are the evidence-based baseline.

## Phase 1 — Foundation (0.1.x, partial distribution validation)

Native project model, .elyvon schema 1, undo/redo, recovery, editing UI, menus and update policy
exist. Windows x64/x86 unsigned preview installers are published. Real audio, production signing,
hardware installation checks and verified distribution for five other targets remain absent.
The old “seven-platform CI complete” claim was inaccurate; see the support matrix.

## Phase 02 — Repository audit and technical contracts (partial)

Reproducible inventory, feature/support matrices, numbered ADRs, versioned MIDI/graph/job/UI
boundaries, compatibility fixtures, shared quality commands, dependency/licence checks and native
measurement runner. No audio engine or new production UI functionality is claimed. Hardware,
production-key upgrade, complete packaged notices and release-mode performance work remain pending.

## Phases 04–05 — Tempo, transport, audio clips (partial)

Tempo/metre map, block scheduler, transport (cycle, count-in, pre-roll, metronome, tap tempo,
locators/markers, punch storage), lock-free engine with cpal output or internal clock, MIDI note
scheduling with stuck-note protection; decoding (WAV/AIFF/FLAC/MP3/Vorbis), resampling, media pool,
real waveforms, non-destructive region editing, crossfades, loops, offline render/export. Evidence:
[audit/PHASE-04-05.md](audit/PHASE-04-05.md). **Partial**: no physical-device validation, no
disk streaming (in-memory budget), no recording.

## Next functional phase — Recording, latency and streaming

Input streams and recording into the media pool (punch in/out already stored), latency
measurement/compensation, incremental disk streaming for long sessions, device tests on each
target with measured callback deadlines and xruns.

## Later — MIDI, instruments and editing

MIDI ports/recording, full piano-roll editing, native synth/sampler and playable loop browser.
Versioned contracts alone do not implement these features.

## Later — Plug-ins and production

Isolated CLAP/VST3 (AU on macOS where supported), FX, buses, automation, audio editing, bounce,
export and large-session optimisation. Any licensed/certified format requires an explicit decision
and tests; no Atmos or proprietary compatibility is promised.

## Other platforms and suite

Enable and test targets independently when authorised; preserve separate installers and signed
updates. Extract shared UI/core components only when a second actual product needs them.
