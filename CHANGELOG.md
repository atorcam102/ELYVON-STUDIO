# Changelog

All notable changes to Elyvon Studio are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The release pipeline copies the
section of each version into the GitHub release and into the update dialog.

## [Unreleased]

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
