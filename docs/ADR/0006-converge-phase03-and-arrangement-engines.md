# 0006 — Converge the Phase 03 and arrangement engines

Date: 2026-10-16. Status: accepted for the main integration.

`main` and `claude/gallant-fermat-h1epb3` independently implemented incompatible
audio cores after Phase 02. The former drove a diagnostic tone/silence core and
an independent device manager; the latter integrated musical transport, media,
rendering and subsequently audio/MIDI recording. Combining their IPC, generated
types or callbacks would leave two owners of the same audio device.

Use the arrangement engine and its CPAL services as the single production
implementation. Remove the superseded Phase 03 core/device-manager files and
their incompatible UI contracts from the active tree. Their source, tests and
measurements remain recoverable in commits `a141b5e` and `9f60871`; the Phase 03
validation report is retained as historical evidence.

Keep main's Rust 1.90 minimum, job-contract compatibility fix, inverse tempo
conversion, macOS microphone description/entitlement and WebDriver process
cleanup. Regenerate bindings and run the workspace, frontend and native E2E
checks against this single architecture.

The older diagnostic test-tone panel, detailed host/sample-format selector and
automatic device reconnection manager are not active features of this merged
application. Output selection/restart, input routing/monitoring and explicit
device errors use the arrangement service. Future device-manager work must
drive that engine rather than restoring a parallel core. Physical devices and
platform acceptance remain pending as described in the Phase 06–07 report.
