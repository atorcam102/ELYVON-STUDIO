# 0008 — Sample-accurate automation and non-destructive comping

Date: 2026-10-17. Status: accepted (Phases 10–11).

## Context

The mixer graph (ADR 0007) had static parameters only. Mix automation must be
sample-accurate, identical offline and live, writable from the UI during
playback without locks on the callback, and must survive tempo changes.
Phase 11 needs takes, comping, freeze and bounce without ever rewriting the
user's source files, and an edit history that stays usable on large projects.

## Decision

* **Lanes are project data addressed by stable ids.** `AutomationParam`
  names volume, pan, mute, a send by its aux id and an insert parameter by its
  insert id. Values are stored in the parameter's unit; curves (linear, step,
  one-dimensional cubic Bézier) are evaluated on a normalised scale (dB-linear
  levels, logarithmic frequency/Q/release) over **musical time**. Region lanes
  replace the track lane inside their region and move with it. Lanes of
  removed sends/inserts are pruned in the same undo step.
* **Envelopes are compiled, not interpolated per block.** The control thread
  turns lanes into frame pieces split at tempo changes, so evaluating a curve
  at frame `t` is exact in ticks. Discontinuities (steps, jumps, region edges)
  ramp over 2 ms ending on the point's frame; level parameters ramp linearly in
  gain. Output is a pure function of the playhead: renders are independent of
  the block size and live playback equals export (tested bit-exact).
  Insert coefficients step on an absolute 32-frame grid; trim gain and bypass
  are per frame. Automated bypass blends dry and processed signal (the limiter
  blends its gain so the dry path keeps the look-ahead delay).
* **Writing.** Modes Off/Read/Touch/Latch/Write per channel. Control gestures
  reach the callback as `ParamOverride`s through an SPSC queue (no locks) and a
  pure `AutomationRecorder` (project crate) collects them with the engine
  position. Cycle wraps and locates start new passes; Write overwrites every
  lane of the channel. When playback stops the recording becomes one
  `WriteAutomation` edit (one undo step), thinned with Ramer–Douglas–Peucker.
  Nothing is written while stopped, in Off/Read, or into region lanes.
* **Comps are descriptions.** A `TakeComp` lists `(range, take)` segments of
  a take folder. The takes stay untouched and muted; the engine plays the
  comp's virtual regions (trimmed copies of the takes with crossfades centred
  on each boundary). `FlattenComp` inserts exactly those regions — the same
  function — so flattening is sample-identical and reversible.
* **Freeze is checked.** A frozen track plays a pre-fader render (regions,
  comps, inserts and insert automation; the look-ahead latency is removed).
  A fingerprint of that material, the tempo map, the sample rate and the media
  is stored; edits that would change it are refused; if it stops matching the
  track plays live again and the UI reports the freeze as stale.
* **Media stay original.** Reverse, normalise (region gain), slices, slip,
  glue of contiguous pieces and crossfades only change references. Freeze and
  bounce write *new* files. Repair relinks by name + size + content
  fingerprint; collect copies (verified, never overwriting) into the project's
  `Media` folder; the only destructive clean-up moves unused project files to
  `.elyvon-backup/<time>/` and requires explicit confirmation.
* **History has a memory budget** (512 MiB of snapshots on 64-bit, 96 MiB on
  32-bit) in addition to the 200-step limit; at least one step is always kept.

## Consequences

Schema 5 (all new fields have neutral defaults; schema 4 files migrate on
open; older applications refuse schema 5). Touch/Latch capture timing is the
IPC round trip of a UI gesture (milliseconds), not sample-accurate; playback of
written data is. Gluing unrelated audio needs a bounce. Region automation is
not written by gestures. Snapshot undo remains O(project) per edit (≈26 ms per
edit for 7 680 regions and 128 000 points in release).
