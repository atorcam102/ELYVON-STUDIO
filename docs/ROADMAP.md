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

## Next functional phase — Native audio backend and transport

Device negotiation, sample-clock transport, preallocated RT graph/queues and real meters; then
media decoding/import, streaming and recording with latency compensation. Require device tests,
callback allocation/locking checks and measured deadlines before enabling playback/recording UI.

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
