# ADR 0005: Lock-free plan engine, tempo map in Rust and in-memory media

Status: Accepted for Phases 04–05 · 2026-10-16.

Context: the transport, scheduler, clips and metronome must be sample-accurate, repeatable offline
and real-time safe, while the UI only presents. The repository had contracts but no engine.

Decision:

* **Time** is integer ticks (PPQ 960) in the project and integer frames in the engine. A validated
  `TempoMap` (tempo steps at ticks, metre changes on bar lines) converts every position from the
  start of its segment; nothing is accumulated per block, so output is a pure function of the
  playhead and therefore independent of block size (tested with 1…4096-frame blocks).
* The control thread builds an immutable `PlaybackPlan` after every edit (clips resolved into
  non-overlapping segments with equal-power crossfades, MIDI notes, cycle, metronome). Plans and
  transport commands reach the callback through pre-allocated `rtrb` SPSC queues; replaced plans
  return through a third queue and are freed on the control thread. The callback never allocates,
  frees, locks, logs or performs IO (counting-allocator test).
* A tempo-map change during playback remaps the playhead by musical position at the swap.
  Stop/locate/cycle wrap/plan swap release every sounding note (stuck-note protection).
* Media: Symphonia (MPL-2.0, unmodified) decodes WAV/AIFF/FLAC/MP3/Ogg Vorbis on a worker thread;
  rubato's FFT resampler converts to the engine rate once; a min/max peak pyramid serves
  waveforms. Decoded audio is held **in memory** under a budget (4 GiB 64-bit, 512 MiB 32-bit).
  Source files are only opened for reading; region edits change offsets/fades/gain.
* Output: cpal (Apache-2.0) device stream owned by its own thread; when no device opens, an
  internal clock runs the same engine silently and the UI says "No audio output".
* The UI mirrors bar/grid maths only for drawing, verified against Rust golden vectors.

Consequences: deterministic offline renders and diagnostic exports reuse the playback path.
Incremental disk streaming, latency compensation, recording and instruments are not implemented;
long sessions are bounded by the memory budget. Device behaviour, xruns and latency on real
hardware are unverified until tested on each target.
