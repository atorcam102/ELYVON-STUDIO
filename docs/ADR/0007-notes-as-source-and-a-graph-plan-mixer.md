# 0007 — Notes as the editable MIDI source and a graph-plan mixer

Date: 2026-10-16. Status: accepted (Phases 08–09).

## Context

Schema 3 played recorded/imported MIDI from raw byte events and kept
`content.notes` only as a piano-roll projection, so editing notes could not
change playback. The engine summed tracks straight into the output with no
buses, sends, inserts or master strip, and every plan swap sent a MIDI panic,
which would have cut held notes on every fader or note edit.

## Decision

* **Notes are authoritative.** `MidiNote` gains a per-region `id`, `channel`,
  `muted` and `quantizeOffset`. `Region::midiEvents` keeps only non-note
  events (controllers, bend, pressure, program, SysEx); validation rejects note
  on/off there. Imports, recordings and schema ≤ 3 files are normalised by
  first-in-first-out pairing. Note-off velocity is not preserved.
* **Piano-roll edits are native commands** addressed by note id (`addNotes`,
  `moveNotes`, `resizeNotes`, `updateNotes`, `transformNotes`, lane edits) and
  `batch` makes a multi-region gesture one undo step.
* **The playback plan is the mixer graph.** Each track and auxiliary channel
  is a strip (format → inserts → pre-fader sends → fader/pan → post-fader
  sends → output gate → path-compensation delay). Auxes are processed in
  topological order, then the master strip. Routing cycles are rejected by
  project validation before any edit applies; there is no feedback mode.
* **Graph swaps stay off the callback's allocation path**: the control thread
  builds the new plan; the callback adopts running state (fader ramps, filter
  memory, limiter and delay buffers, instrument voices) by swapping buffers and
  returns the old plan to be freed on the control thread. Newly inserted or
  re-enabled processors fade in over one block. Notes still covered in the new
  plan keep sounding; the others get note-offs (no global panic).
* **Instrument tracks use a small built-in test instrument** ("Elyvon Test
  Tone") so the virtual keyboard, input monitoring and previews are audible
  without plug-in hosting. It is labelled as a test instrument.
* **Meters** are lock-free per-strip accumulators (peak via `fetch_max`,
  monotonic sums for RMS, sticky clip) read by the publisher at ~30 Hz.

## Consequences

Schema bumps to 4 (older applications refuse newer files). Insert latency is
compensated relative between parallel paths; the absolute output latency is
reported but the timeline is not shifted to hide it. Third-party plug-in
formats remain absent from the typed `Processor` enum.
