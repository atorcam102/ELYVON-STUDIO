<p align="center">
  <img src="assets/brand/elyvon-studio-icon-1024.png" width="80" alt="Elyvon Studio icon">
</p>

# Elyvon Studio

[![CI](https://github.com/atorcam102/ELYVON-STUDIO/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/atorcam102/ELYVON-STUDIO/actions/workflows/ci.yml)

A desktop digital audio workstation in development, built with **Rust, Tauri 2, React and TypeScript**. It currently supports arranging tracks and regions, inspecting MIDI notes and managing `.elyvon` projects through a graphite studio interface.

**Development branch: Phases 04–07, partial hardware validation.** The native engine plays imported audio, streams large files, records armed audio inputs to recoverable WAV/BWF takes, and records/routes MIDI 1.0. These branch capabilities are not the published Windows preview. Physical audio/MIDI devices and other operating systems remain unverified here; see [the current validation report](docs/audit/PHASE-06-07.md).

![Elyvon Studio main interface: a six-track demo session, arrangement and mixer](docs/screenshots/01-arrangement-mixer.png)

*Current interface on `main`: LCD transport, colour-coded tracks, MIDI regions and mixer. Demo waveforms are illustrations.*

[Download for Windows](#download-for-windows) · [Current features](#current-features) · [More screenshots](#interface-screenshots) · [Builds from main](#builds-from-main) · [Development](#development)

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

- Physical-device validation across the intended operating systems; multi-device input clock drift and live input sample-rate conversion.
- Advanced audio editing, final mixdown formats and bounce; import, region editing and diagnostic WAV export are implemented.
- Piano-roll note editing, playback note chase and internal instruments; MIDI 2.0/UMP.
- Built-in instruments, sampler, effects, buses and plug-in hosting.
- Loop library with audio previews.

Playback uses the native engine clock; the internal-clock fallback is labelled as silent. Audio recording requires a usable input device, armed tracks and matching sample rates. MIDI without an external output is explicitly monitor-only. Effect slots remain unavailable without hosted processors.

The repository includes update infrastructure with version, platform and signature validation. The current unsigned preview **does not enable automatic updates**. See [releases and signing](docs/RELEASES.md) for configuration details.

## Interface screenshots

### Library and inspector

The library sits on the left, with the selected track's properties and project controls beside the arrangement.

![File library and inspector open beside the selected Keys track and arrangement](docs/screenshots/02-library-inspector.png)

### Piano-roll preview

Notes from the Keys track's **Chords** region appear below the arrangement. The **READ-ONLY PREVIEW** label identifies the editor's current scope.

![Piano roll showing notes from the Chords region beneath the demo arrangement](docs/screenshots/03-piano-roll.png)

Screenshots show the main frontend from `main`, rendered in Chromium at **1280 × 800** using Tauri's official test bridge and demo-session data. They are not captures of the Windows installer. [Screenshot details](docs/screenshots/README.md).

## Native recording and MIDI development workflow

1. Run `npm ci` and `npm run tauri dev` with the native prerequisites below.
2. For audio, add audio tracks, arm them in the inspector, choose their zero-based input channels
   through the displayed one-based channel controls, and enable software monitoring as needed.
   Open **Input** to select the input device and recording offset. **Record** starts the project
   count-in/pre-roll/punch/cycle settings. **Stop** finalizes takes into one undoable project edit.
3. Cycle passes form a take group. Select the active take in the region inspector. Save the
   project to retain the media references. Interrupted `.partial.wav` takes are listed under
   **Input → Recoverable takes** after restoring their project.
4. Open **MIDI**, select a MIDI/instrument track and its ports/channel filters, then arm and record
   it. The virtual keyboard feeds the native monitor or selected external port. **Panic** releases
   held keys and sustain. MIDI and audio recording currently run as separate sessions.
5. **MIDI → Import/Export MIDI** handles SMF 0/1. File tempo/metre maps are applied only when
   importing into an empty project at the beginning. **Input → Measure aligned loopback files**
   measures file correlation without automatically applying a possibly inappropriate IO offset.

All engine, recording and MIDI scheduling work runs in Rust. Callback allocation tests, fault
injection, SMF round trips and native GUI tests are part of the regular checks. A passing headless
check does not establish microphone, interface or MIDI-keyboard behavior on physical hardware.

## Download for Windows

Current distribution focuses on **Windows x64 and x86**.

**For most users with 64-bit Windows, choose the x64 EXE installer.** Download and install only one package for your architecture.

Each architecture has two installation formats: **EXE** (`-setup.exe`, NSIS) is the standard setup wizard; **MSI** uses Windows Installer and is useful for managed or automated deployments. Both formats contain the same application and features.

| Architecture | EXE installer | MSI installer |
| --- | --- | --- |
| **Windows x64 · 64-bit** | [Download EXE](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.3/Elyvon-Studio-v0.1.0-windows-x64-setup.exe) | [Download MSI](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.3/Elyvon-Studio-v0.1.0-windows-x64.msi) |
| **Windows x86 · 32-bit** | [Download EXE](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.3/Elyvon-Studio-v0.1.0-windows-x86-setup.exe) | [Download MSI](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.3/Elyvon-Studio-v0.1.0-windows-x86.msi) |

[View the release and notes](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.1.0-windows-preview.3) · [SHA-256 checksums](https://github.com/atorcam102/ELYVON-STUDIO/releases/download/v0.1.0-windows-preview.3/SHA256SUMS.txt)

**Windows preview 3 includes the redesigned native interface and the Phase 02 technical baseline**, built from commit `974b09c` ([successful source CI](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38768320136)). Its installers have neither updater signatures nor Authenticode signatures.

The [current CI workflow](.github/workflows/ci.yml) produces EXE and MSI installers for Windows x64/x86. Configuration for other platforms remains in the repository; their distribution is outside the current scope.

### Builds from main

For builds newer than the published preview, open the latest successful [CI run on `main`](https://github.com/atorcam102/ELYVON-STUDIO/actions/workflows/ci.yml?query=branch%3Amain) and download the appropriate **Artifacts** archive:

| Architecture | CI artifact | Contents |
| --- | --- | --- |
| Windows x64 · 64-bit | `ci-windows-x64` | Separate EXE and MSI installers for `x86_64-pc-windows-msvc` |
| Windows x86 · 32-bit | `ci-windows-x86` | Separate EXE and MSI installers for `i686-pc-windows-msvc` |

These verification builds are unsigned. CI artifacts expire after **3 days** and downloading them requires a GitHub login. Published releases remain available after their source CI artifacts expire.

Each CI run also checks the frontend, Rust workspace, generated TypeScript bindings, dependency licences/advisories and workflow syntax. The Linux test runner builds and exercises the real desktop binary, including saving a project, restarting the process and reopening it. `native-evidence` contains GUI screenshots and startup, idle-memory and timeline measurements; `audit-evidence` contains dependency reports. Both evidence archives are retained for 30 days.

Windows installer builds do not establish installation or audio-device support on physical hardware. See the [support matrix](docs/STATUS.md#runners-and-platform-evidence).

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
- **Stable Rust 1.90 or newer**.
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
npm run check                     # versions, typecheck, lint, tests, build
npm run check:rust                # fmt, clippy, workspace tests, binding drift
npm run tauri -- build --no-bundle # native application without installers
```

Installer generation and publication are documented in [RELEASES](docs/RELEASES.md).

### Phase 02 technical baseline

- **Versioned contracts:** native `app_contracts` reports IPC, project, audio and job contract versions. Rust generates the shared TypeScript types; UI presentation state remains separate from the project model.
- **Compatibility fixtures:** schema-1 `.elyvon` projects retain their tracks, regions, notes and media references across save, reopen and recovery. Frozen MIDI, graph and job fixtures check wire compatibility and reject invalid data.
- **Future-engine boundaries:** MIDI events use callback-relative frame offsets; graph plans validate routing, channels and cycles; job snapshots describe progress and errors. The MIDI backend is now implemented on this development branch; a hosted graph executor and job scheduler remain future work.
- **Reproducible checks:** shared local/CI commands, `cargo-deny`, npm audit, licence/source inventories and `actionlint`. The GTK3 dependency's maintenance-only exception is documented as **B-01** in [STATUS](docs/STATUS.md).

The [validated baseline CI](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553) passed all six jobs, including both Windows installer builds and native GUI tests. The [validation report](docs/audit/VALIDATION.md) records commands, raw measurements, screenshots and their limitations. Measurements use a debug Linux build under Xvfb; they are not Windows or physical-hardware performance guarantees.

Phase 02 remains **partial** while hardware validation, a production-key signed upgrade, complete packaged licence notices and an independent beta update channel are outstanding. The [feature matrix](docs/FEATURE_MATRIX.md) distinguishes implemented, partial, simulated and absent capabilities.

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

- [Repository status, support matrix and known gaps](docs/STATUS.md)
- [Feature implementation matrix](docs/FEATURE_MATRIX.md)
- [Versioned technical contracts](docs/CONTRACTS.md)
- [Architecture decisions](docs/ADR/README.md)
- [Audit validation and measurements](docs/audit/VALIDATION.md)
- [Third-party notices and outstanding licence work](THIRD_PARTY_NOTICES.md)
- [Architecture](docs/ARCHITECTURE.md)
- [`.elyvon` project format](docs/PROJECT_FORMAT.md)
- [Releases, signing and updates](docs/RELEASES.md)
- [Roadmap](docs/ROADMAP.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

## Privacy and licence

No telemetry. Update checks use the network and can be disabled in **Settings → Updates**.

© 2026 Elyvon. All rights reserved. A distribution licence has not yet been established for this project.
