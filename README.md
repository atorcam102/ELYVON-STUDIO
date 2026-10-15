<p align="center">
  <img src="assets/brand/elyvon-studio-icon-1024.png" width="80" alt="Elyvon Studio icon">
</p>

# Elyvon Studio

[![CI](https://github.com/atorcam102/ELYVON-STUDIO/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/atorcam102/ELYVON-STUDIO/actions/workflows/ci.yml)

A desktop digital audio workstation in development, built with **Rust, Tauri 2, React and TypeScript**. It currently supports arranging tracks and regions, inspecting MIDI notes and managing `.elyvon` projects through a graphite studio interface.

**Status: v0.1.0 · Phase 1.** Project management and editing controls are implemented. The audio engine is still pending: this version does not play or record sound yet.

![Elyvon Studio main interface: a six-track demo session, arrangement and mixer](docs/screenshots/01-arrangement-mixer.png)

*Current interface on `main`: LCD transport, colour-coded tracks, MIDI regions and mixer. Demo waveforms are illustrations.*

[Download for Windows](#download-for-windows) · [Current features](#current-features) · [More screenshots](#interface-screenshots) · [Development](#development)

## Current features

| Area | What you can do today |
| --- | --- |
| **Projects** | Create, open and save `.elyvon` projects; use Save As and preserve names, tracks, regions and musical settings. |
| **History and recovery** | Undo and redo edits, receive unsaved-change prompts and restore recovery snapshots after an unexpected exit. Project files are saved atomically. |
| **Tracks** | Create audio, instrument and MIDI tracks; rename, duplicate, delete, reorder, recolour and select multiple tracks. |
| **Arrangement** | Navigate by bar, move the cursor, adjust zoom and use the grid and snapping. Move regions in time or between compatible tracks, and create empty MIDI regions. |
| **Musical settings** | Change tempo, time signature and key; define and edit the project's cycle range. |
| **Mixer** | Adjust volume, pan, mute and solo per track. These settings are saved in the project; they do not process audio yet. |
| **Piano roll** | Inspect a MIDI region's notes and their timing. This is a read-only preview. |
| **File library** | Browse folders and open `.elyvon` projects from the left sidebar. |
| **Workspace** | Show, hide and resize the library, inspector and editors; switch between mixer and piano roll. |
| **Desktop controls** | Use native menus, keyboard shortcuts, preferences and diagnostic logs. |
| **Demo session** | Explore a six-track arrangement: Drums, Bass, Keys, Hardware Lead, Lead Vocal and Guitar. New projects start empty. |

### Still to come

- Audio playback, recording and device selection.
- Audio file import and editing; export and bounce.
- MIDI input/output and piano-roll note editing.
- Built-in instruments, sampler, effects, buses and plug-in hosting.
- Loop library with audio previews.

Playback and recording controls remain disabled when the engine is unavailable. Effect slots are visual placeholders without processing. Meters animate only when you explicitly enable **Simulate meters**.

The repository includes update infrastructure with version, platform and signature validation. The current unsigned preview **does not enable automatic updates**. See [releases and signing](docs/RELEASES.md) for configuration details.

## Interface screenshots

### Library and inspector

The library sits on the left, with the selected track's properties and project controls beside the arrangement.

![File library and inspector open beside the selected Keys track and arrangement](docs/screenshots/02-library-inspector.png)

### Piano-roll preview

Notes from the Keys track's **Chords** region appear below the arrangement. The **READ-ONLY PREVIEW** label identifies the editor's current scope.

![Piano roll showing notes from the Chords region beneath the demo arrangement](docs/screenshots/03-piano-roll.png)

Screenshots show the main frontend from `main`, rendered in Chromium at **1280 × 800** using Tauri's official test bridge and demo-session data. They are not captures of the Windows installer. [Screenshot details](docs/screenshots/README.md).

## Download for Windows

Current distribution focuses on **Windows x64 and x86**.

| Architecture | EXE installer | MSI installer |
| --- | --- | --- |
| **Windows x64 · 64-bit** | [Download EXE](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.2/Elyvon-Studio-v0.1.0-windows-x64-setup.exe) | [Download MSI](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.2/Elyvon-Studio-v0.1.0-windows-x64.msi) |
| **Windows x86 · 32-bit** | [Download EXE](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.2/Elyvon-Studio-v0.1.0-windows-x86-setup.exe) | [Download MSI](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.2/Elyvon-Studio-v0.1.0-windows-x86.msi) |

[View the release and notes](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.1.0-windows-preview.2) · [SHA-256 checksums](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.2/SHA256SUMS.txt)

**Windows preview 2 includes the redesigned native interface shown above**, built from commit `fbc4566`. Its installers have neither updater signatures nor Authenticode signatures.

The [current CI workflow](.github/workflows/ci.yml) produces EXE and MSI installers for Windows x64/x86. Configuration for other platforms remains in the repository; their distribution is outside the current scope.

## Getting started

1. Open Elyvon Studio and create a project, or choose **Or explore the demo session** in the empty arrangement. You can also open the demo from the native **File** menu.
2. Add tracks using **Functions** or the **+** button. Use the inspector to view their properties.
3. Select and arrange regions. Double-click an empty area on an instrument or MIDI track to create a MIDI region.
4. Adjust volume, pan, mute and solo in the mixer. Select a MIDI region and open **Piano Roll** to inspect its notes.
5. Save your project as an `.elyvon` file.

### Main Windows shortcuts

| Action | Shortcut |
| --- | --- |
| New / open project | `Ctrl+N` / `Ctrl+O` |
| Save / save as | `Ctrl+S` / `Ctrl+Shift+S` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` |
| Duplicate tracks | `Ctrl+D` |
| Delete selection | `Delete` |
| Inspector / library / editors | `I` / `B` / `E` |
| Mixer | `X` |
| Toggle the cycle range | `L` |
| Move the cursor to the start | `Home` |

The full list is available in **Settings → Shortcuts**.

## Development

### Requirements

- **Node.js 22.12 or newer** and npm.
- **Stable Rust 1.88 or newer**.
- On Windows: Microsoft C++ build tools and WebView2, following [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/).
- Native dependencies for other development environments are described in [CONTRIBUTING](CONTRIBUTING.md).

### Run the application

```bash
npm ci
npm run tauri dev
```

This starts Vite and the desktop application with its native bridge.

### Checks and builds

```bash
npm run typecheck
npm run lint
npm test
npm run build                      # production frontend
cargo test --workspace             # Rust tests and binding generation
npm run tauri -- build --no-bundle # native application without installers
```

Installer generation and publication are documented in [RELEASES](docs/RELEASES.md).

### Visual reference

The `/reference.html` page, available through `npm run dev`, preserves Live Loops, step-sequencer and chord mockups for design comparisons. **These are reference views with static data**, not production features of the main application.

## Repository structure

```text
crates/elyvon-core/      Musical time, colours and atomic file writes
crates/elyvon-project/   Project model, .elyvon format, history and recovery
crates/elyvon-audio/     Contracts for the future audio engine
crates/elyvon-update/    Update policy, manifests and release CLI
src-tauri/              Desktop application, IPC, menus and preferences
src/features/           Transport, tracks, arrangement, mixer, inspector and library
src/components/         Shared interface controls
src/styles/             Theme, layout and current design styles
src/reference/          Visual reference mockups
e2e/                    WebDriver tests against the real application binary
docs/screenshots/       Current frontend screenshots
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [`.elyvon` project format](docs/PROJECT_FORMAT.md)
- [Releases, signing and updates](docs/RELEASES.md)
- [Roadmap](docs/ROADMAP.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## Privacy and licence

No telemetry. Update checks use the network and can be disabled in **Settings → Updates**.

© 2026 Elyvon. All rights reserved. A distribution licence has not yet been established for this project.
