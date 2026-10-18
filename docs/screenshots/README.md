# Elyvon Studio screenshots

Every image here is a capture of the rendered application, never a generated image or composite.
Each set says how it was captured and what it does **not** demonstrate.

| Folder | Version / phase | Captured with |
| --- | --- | --- |
| [`phase-12-13-windows/`](phase-12-13-windows/) | Phases 12–13 — effect editor (compressor with live GR, EQ with the engine's curve and the measured spectrum) | same, Windows 11 x64, branch `fases-12-13` |
| [`0.4.1-windows/`](0.4.1-windows/) | 0.4.1 — refined chrome (icons, editor toolbars) | `e2e/app.e2e.mjs` against the real debug binary on Windows 11 x64 (tauri-driver + msedgedriver) |
| [`0.4.0-windows/`](0.4.0-windows/) | 0.4.0 — redesigned chrome, automation, comping | `e2e/app.e2e.mjs` against the real debug binary on Windows 11 x64 (tauri-driver + msedgedriver) |
| [`phase-10-11-windows/`](phase-10-11-windows/) | Phases 10–11 before the chrome redesign | same, Windows 11 x64 |
| [`phase-08-09-windows/`](phase-08-09-windows/) | 0.3.0 — piano roll and mixer graph | same, Windows 11 x64 |
| [`phase-04-05-linux/`](phase-04-05-linux/) | Phases 04–05 — imported audio, engine playback | `e2e/app.e2e.mjs`, WebKitWebDriver, Xvfb 1280 × 800, Ubuntu 24.04 x64 |
| [`0.1-frontend-mock/`](0.1-frontend-mock/) | 0.1 — first interface | Chromium through Playwright with Tauri's IPC mock bridge |

## Native captures (Windows and Linux)

Taken by the end-to-end suite when `E2E_SCREENSHOTS=<folder>` is set. File names are the test
steps (`01-empty`, `02-demo`, … `13-takes-comp`). The audio engine runs on the device available on
the machine (or on its silent internal clock when there is none, which the status pill says);
nothing was listened to while capturing. They do not demonstrate installing the release.

In `phase-04-05-linux/`: `04-imported-audio-waveform.png` shows a 2-second 440 Hz WAV imported
through `media_import`, drawn from native peak columns; `05-engine-playback.png` shows the LCD and
playhead driven by engine events on the internal clock (no sound card in the container).

## 0.1 frontend mock

Captured at commit [`fbc4566`](https://github.com/atorcam102/ELYVON-STUDIO/commit/fbc456623aced9396cc235f6fd2f5d5697a1cd14)
in Chromium at 1280 × 800, device scale factor 1, with IPC replaced only during capture by the
official `@tauri-apps/api/mocks` bridge and the six-track session of
`crates/elyvon-project/src/demo.rs`. Library folders are example data; the engine was marked
unavailable and region waveforms are `DemoWaveform` illustrations. They do not demonstrate audio
playback, recording or processing.

| File | View |
| --- | --- |
| `01-arrangement-mixer.png` | Demo-session arrangement and mixer |
| `02-library-inspector.png` | Library and inspector with Keys selected |
| `03-piano-roll.png` | Read-only preview of the notes in Chords |
