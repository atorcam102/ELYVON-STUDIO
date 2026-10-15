# Elyvon Studio screenshots

Screenshots of the main frontend at commit [`fbc4566`](https://github.com/atorcam102/ELYVON-STUDIO/commit/fbc456623aced9396cc235f6fd2f5d5697a1cd14), with the new design applied to `src/App.tsx` and its components. They are not captures of the `/reference.html` mockup.

| File | View |
| --- | --- |
| `01-arrangement-mixer.png` | Demo-session arrangement and mixer. |
| `02-library-inspector.png` | Library and inspector with Keys selected. |
| `03-piano-roll.png` | Read-only preview of the notes in Chords. |
| `04-engine-headless-playing.png` | Phase 03: real desktop binary playing the demo on the headless clock with the test tone; output strip reads −18.0 dBFS. |
| `05-settings-audio.png` | Phase 03: Settings › Audio showing the open stream and device selection. |

Images 04–05 are WebDriver screenshots of the real native binary (WebKitWebDriver, Xvfb
1280 × 800) taken by `e2e/app.e2e.mjs`, not mock captures. The headless clock produces no sound;
they do not demonstrate audible output on hardware.

## Capture method

- Chromium through Playwright, 1280 × 800 CSS pixels, device scale factor 1.
- Main Vite entry point: `/`, using the Tauri application's components.
- IPC replaced only during capture by the official `@tauri-apps/api/mocks` bridge.
- Sample project matching the six tracks, regions, tempo and MIDI patterns defined in `crates/elyvon-project/src/demo.rs`; stable identifiers are used for capture.
- Example folder data in the library. It does not represent a real installation's disk contents.
- Audio engine marked unavailable, with meter simulation disabled.
- Audio illustrations come from `DemoWaveform` and retain their demo label. No audio files back those regions.

These are actual captures of the rendered frontend, not generated images or composites. They do not demonstrate execution of the Windows installer or working audio playback, recording or processing.

When updating the images, keep the filenames and capture the same main entry point. Also review the features and limitations described in the root README.
