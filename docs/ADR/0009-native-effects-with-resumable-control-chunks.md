# 0009 — Native effects as typed parameter tables with resumable control chunks

Date: 2026-10-18. Status: accepted (Phases 12–13).

## Context

The mixer graph (ADR 0007) had four hand-written inserts (trim, utility,
filter, limiter) with one `InsertParam` variant per automatable parameter.
Phases 12–13 add sixteen effects with five to forty-nine parameters each,
presets, A/B comparison, sidechains, tempo-synchronised times and LFOs,
latency-reporting convolution and oversampling, gain-reduction meters and an
analyser. The engine guarantees that a bounce is identical to live playback
whatever the callback size (ADR 0008); effects must keep that guarantee and
the real-time rules (no allocation, locks or IO in the callback).

## Decision

* **Parameters are plain numbers described by an append-only table.**
  `elyvon_project::effects` defines `EffectKind` and, per kind, a `ParamDef`
  table (key, Spanish label, unit, range, default, scale: linear, log, choice
  or toggle, automatable). An `EffectSlot` stores `[f64; 64]` values (so
  `Processor` stays `Copy`) and is serialised **by key**: unknown keys are
  ignored, missing keys take the default and values are clamped, so adding a
  parameter never breaks a project or a preset. A parameter's index is its
  automation id (`InsertParam::Effect { kind, index }`); indices are never
  reused or reordered. Parameters whose change rebuilds the processor
  (impulse-response space and length, oversampling factor) are editable but
  not automatable.
* **One generic path for edits, presets and A/B.** Every change is a
  `setInsert` edit (one undo step). Presets are versioned JSON files
  (`elyvon-effect-preset`, version 1) stored by key; factory presets are
  original settings in code. A/B keeps the alternative setting inside the
  slot (`alternate`) and swapping is an ordinary edit.
* **Resumable control chunks.** Effects update smoothed parameters,
  coefficients, ramps and LFO phases once per 16-frame control chunk aligned
  to the absolute timeline frame. A chunk cut by the end of a callback
  continues in the next call with the same per-chunk values (`Bank::enter`),
  so the output is bit-identical for any block size — also while parameters
  glide. LFOs synchronised to the tempo take their phase from the musical
  position (they follow the tempo map); free LFOs accumulate and re-anchor to
  the timeline after a locate.
* **State survives graph swaps.** A rebuilt effect adopts the running state
  (delay lines, filter memories, envelopes) and the heard parameter values of
  its predecessor by swapping buffers — no allocation on the audio thread —
  then glides to the new values (~20 ms).
* **Sidechain keys are part of the plan.** Compressor and gate may be keyed
  by a track's post-insert, pre-fader signal. The plan assigns key slots
  (≤ 16 sources), processes key sources before the tracks they key and keeps
  muted sources running; validation rejects self-keys and track cycles.
* **Graceful overload and honest latency.** Latency (convolution partition,
  oversampling FIR) is reported per insert and compensated on parallel paths;
  the effect delays its own dry signal to match. When real-time callbacks
  overrun (load > 0.9 three times), the engine switches convolution to an
  economy path (first second of the IR) until 50 calm callbacks; offline
  renders never do. Strips whose effects still ring keep processing while the
  transport is stopped so tails decay instead of freezing.
* **Analysis off the audio thread.** The callback only pushes the selected
  strip's post-fader samples into an SPSC queue; the FFT runs on the control
  side. Equaliser curves are computed in Rust from the same biquad sections
  the engine runs.

## Consequences

* New effects are added by appending a `ParamDef` table and a DSP struct; the
  UI, presets, automation and IPC need no new types.
* `Processor` is ~1 KiB; acceptable because processors are control-thread
  data (a clippy size lint is allowed with this justification).
* Impulse responses are synthesised from fixed seeds (original and
  redistributable). Loading user IR files, a production pitch shifter and
  third-party plug-ins remain future work.
