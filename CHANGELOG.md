# Changelog

All notable changes to Elyvon Studio are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release pipeline copies the
section of each version into the GitHub release and into the update dialog.

## [Unreleased]

### Added

- Native audio engine: lock-free real-time playback with an audio device (cpal) or, without one, a silent internal clock that still drives the transport.
- Transport: play, stop, pause, locate, cycle, count-in, pre-roll, metronome (accent, level), tap tempo, markers and a stored punch range.
- Tempo map with tempo changes and time-signature changes (odd and compound metres); ruler, grid and snapping follow it; snap grid with triplets and swing.
- Audio import (WAV, AIFF, FLAC, MP3, Ogg Vorbis; any sample rate) by dialog, drag and drop and the file browser; media pool with missing/changed-file detection and relink.
- Real waveforms; non-destructive trim, split, duplicate, cut/copy/paste, region mute, gain, fades, crossfades between overlapping regions and looping.
- Offline diagnostic render to 32-bit float WAV with a JSON level report.
- Audio preferences: output device, buffer size, internal clock.

### Changed

- Project files are now schema 2; schema-1 projects are migrated automatically on open (older app versions refuse schema-2 files instead of misreading them).
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
