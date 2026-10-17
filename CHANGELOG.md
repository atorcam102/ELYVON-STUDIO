# Changelog

All notable changes to Elyvon Studio are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release pipeline copies the
section of each version into the GitHub release and into the update dialog.

## [Unreleased]

### Added

- Mix automation: lanes for volume, pan, mute, send levels and insert parameters on tracks, aux channels, the master and regions; Off/Read/Touch/Latch/Write modes; linear, step and Bézier curves; an Automation editor (draw, move, copy, duplicate, curve per point, simplify); mixer faders, pan, mute and sends record automation while playing. Playback is sample-accurate and identical to export.
- Takes and comping: drag across takes in the new Takes & Audio editor; crossfades centred on each boundary; flatten and unflatten without touching the takes.
- Non-destructive editing: reverse, fade shapes (equal power, linear, S-curve), editable crossfades, slip, ripple delete, marquee split/delete, glue, slices and markers at transients, normalise, phase-locked group edits, bounce in place and track freeze.
- Project media dialog: find moved files by content, rescan, collect media into the project folder, remove unused pool entries and move unused project files to a backup folder after confirmation.
- Redesigned desktop chrome (Spanish menu bar, control bar, side panels, library, score and smart-tempo panels) merged with the automation and comping work: the Automatización and Tomas y audio tabs sit beside Editor de teclado, Partitura and Tempo inteligente; the mixer strip's automation slot is the real mode selector; the toolbar's Automatización, Seguir cursor and Herramientas (Puntero, Marquesina, ripple) buttons work; the inspector's Bucle, Cuantizar and Transponer act on the selected region; the Tempo and Compás lists show every tempo and metre change; Partitura draws a simplified, read-only notation of the selected MIDI region (pitch and position; no rhythmic values). Controls without a feature behind them still say they are not available.

### Changed

- Project schema 5 (schema 1–4 files migrate on open without being rewritten).
- The undo history also has a memory budget, so very large projects keep fewer steps instead of exhausting memory.

## [0.3.0] - 2026-10-16

### Added

- Piano roll editing: draw, select (click, Shift/Ctrl, rubber band), drag in time and pitch, Alt-drag copy, resize either edge, erase, duplicate, nudge/transpose from the keyboard, mute notes; metre-aware snap with triplets and swing; reversible quantise with strength, unquantise, humanise, transpose, legato, velocity/channel/length edits and chord-voice split; velocity and controller lanes (modulation, volume, expression, sustain, pitch bend, channel pressure, program); scale highlighting; note audition; multi-region editing as one undo step.
- Built-in "Elyvon Test Tone" instrument on instrument tracks, played by the timeline, the virtual keyboard, MIDI input monitoring and piano-roll previews.
- Mixer graph in the engine: auxiliary buses, pre/post-fader sends, output routing, master strip, native inserts (trim, utility, high/low-pass filter, look-ahead limiter), plug-in delay compensation on parallel paths, mono/stereo/auto channel formats, three pan laws, solo-safe, linked groups and mixer folders.
- Real peak, RMS, peak-hold and sticky clip meters for every track, aux and the master.
- Stem export: master plus post-fader stems per track and aux as 32-bit float WAV with a JSON report.

### Changed

- Project schema 4. Recorded and imported note on/off events become editable notes (first-in-first-out pairing); schema 1–3 files migrate on open without being rewritten. Note-off velocity is not stored.
- Editing during playback no longer sends a MIDI panic: held notes that still exist keep sounding, removed ones are released; fader, pan, send and insert changes ramp instead of clicking.

### Fixed

- Windows: the grid golden-vector test compared CRLF-checked-out fixtures byte for byte.
- Piano roll keeps a usable note area on short editor panels (the controller lane shrinks).
- `collect-artifacts.mjs` failed for checkouts in paths with spaces.

## [0.2.1] - 2026-10-16

### Fixed

- MP3 import and playback with large ID3 metadata or embedded artwork: enable the ID3 readers so the importer reaches the audio instead of exhausting the format probe limit.
- Imported and relinked audio now publishes its ready status immediately, allowing waveforms and the inspector to update without reloading the interface.

## [0.2.0] - 2026-10-16

### Added

- Bounded disk streaming for large audio: packet decoding and FFT conversion into a PCM cache, worker page reads, real-time cache-miss counters and streamed diagnostic WAV export.
- Audio input selection and per-track mono/stereo routing, record arming and monitoring; multitrack capture, count-in/pre-roll/punch/cycle, take selection and undo.
- Checkpointed float WAV/BWF takes with recovery journals; interrupted, cancelled and finalized states, explicit disk backpressure, clipping and device-disconnection reporting.
- Input/output latency estimates, manual recording offset, aligned-file loopback measurement, and tested graph latency planning/delay-line primitives (no hosted plug-in executor yet).
- Native MIDI 1.0 ports, channel filters/routing, virtual ports on supporting Linux/macOS backends, a monitored virtual keyboard, MIDI recording and SMF 0/1 import/type-1 export.
- MIDI panic on stop/locate/cycle/edits, bounded SysEx and queues, preserved source timestamps and measured scheduler lateness. Physical-device timing is unverified.

- Native audio engine: lock-free real-time playback with an audio device (cpal) or, without one, a silent internal clock that still drives the transport.
- Transport: play, stop, pause, locate, cycle, count-in, pre-roll, metronome (accent, level), tap tempo, markers and a stored punch range.
- Tempo map with tempo changes and time-signature changes (odd and compound metres); ruler, grid and snapping follow it; snap grid with triplets and swing.
- Audio import (WAV, AIFF, FLAC, MP3, Ogg Vorbis; any sample rate) by dialog, drag and drop and the file browser; media pool with missing/changed-file detection and relink.
- Real waveforms; non-destructive trim, split, duplicate, cut/copy/paste, region mute, gain, fades, crossfades between overlapping regions and looping.
- Offline diagnostic render to 32-bit float WAV with a JSON level report.
- Audio preferences: output device, buffer size, internal clock.

### Changed

- Consolidated main's Phase 03 and the arrangement/recording engine into one production audio path; retained main's Rust 1.90 minimum and macOS microphone entitlements. The earlier separate diagnostic-tone panel and automatic reconnect manager are superseded (ADR 0006).
- Project files are now schema 3, preserving input/MIDI routing, raw MIDI events and take metadata. Schema-1 and schema-2 projects migrate in memory; original fixtures and files remain unchanged.
- Engine meters replace the simulated ones while the engine runs.

## [0.1.0] - 2026-10-15

First foundation release of Elyvon Studio. There is no audio engine yet:
playback, recording and metering are shown as unavailable.

### Added

- Desktop application for Windows (x64, x86, ARM64), macOS (Intel, Apple Silicon) and Ubuntu (x64, ARM64).
- Arrangement view with bar ruler, cycle range, edit cursor, zoom, snapping grid, track headers and regions that can be selected, moved across time and compatible tracks, created (MIDI) and deleted.
- Audio, instrument and MIDI tracks: create, rename, duplicate, delete, reorder, colour, mute and solo, with multi-selection.
- Inspector, mixer channel strips (volume, pan, mute, solo), read-only piano-roll preview and a file browser.
- Projects in the versioned `.elyvon` format: new, open, save, save as, atomic saves, unsaved-change protection, undo/redo and crash recovery.
- Native menus with platform shortcuts, preferences, logs.
- Signed automatic updates through GitHub Releases, always confirmed by the user.
