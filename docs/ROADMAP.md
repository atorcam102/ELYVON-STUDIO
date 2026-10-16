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
[audit/PHASE-04-05.md](audit/PHASE-04-05.md). **Partial**: physical-device validation remains
pending. Bounded PCM disk streaming and recording are now implemented in the follow-up below.

## Phases 06–07 — Input capture and MIDI (partial)

CPAL input streams, per-track channel selection/arming/monitoring, bounded multitrack disk
writing, WAV/BWF takes, punch/pre-roll/count-in/cycle, take selection, interrupted-take recovery,
manual latency offset and file-based loopback measurement. The graph latency planner and delay
lines are tested primitives; active plug-in compensation awaits a hosted graph.

Native MIDI 1.0 input/output, timestamps, filtering and routing, a virtual keyboard/monitor,
MIDI region recording, panic, missing-port recovery and SMF 0/1 import/type-1 export are implemented.
Linux/macOS virtual-port APIs are gated by platform/backend availability. No internal instrument
or MIDI 2.0/UMP support is claimed.

Remaining acceptance: physical voice/guitar/loopback and MIDI-keyboard tests on every intended
platform; measured hardware latency/jitter, independent-device drift handling and live input SRC.
Audio and MIDI recording currently use separate sessions. See [validation](audit/PHASE-06-07.md).

## Later — Instruments and editing

Full piano-roll editing, playback note chase, native synth/sampler and playable loop browser.

## Later — Plug-ins and production

Isolated CLAP/VST3 (AU on macOS where supported), FX, buses, automation, audio editing, bounce,
export and large-session optimisation. Any licensed/certified format requires an explicit decision
and tests; no Atmos or proprietary compatibility is promised.

## Other platforms and suite

Enable and test targets independently when authorised; preserve separate installers and signed
updates. Extract shared UI/core components only when a second actual product needs them.
