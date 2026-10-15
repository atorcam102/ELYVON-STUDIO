# Repository audit and technical contracts — Phase 02

**Audit date:** 2026-10-15. **Entry revision:** [`b30cd67`](https://github.com/atorcam102/ELYVON-STUDIO/commit/b30cd673309fe416099db888fa5cb043506d69f8).
**Overall phase: PARTIAL.** Core editing/persistence exists. Audio/device functionality,
production signing, hardware validation and complete packaged third-party notices do not.
This is a technical audit, not a claim that a playable DAW or every platform is complete.

## Evidence and reproducibility

- Phase 1 merge: [PR #1](https://github.com/atorcam102/ELYVON-STUDIO/pull/1).
- Redesigned native UI: [`fbc4566`](https://github.com/atorcam102/ELYVON-STUDIO/commit/fbc456623aced9396cc235f6fd2f5d5697a1cd14).
- Baseline [CI 38757410164](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38757410164): frontend,
  Rust fmt/clippy/workspace tests/bindings, real Linux binary E2E, Windows x64/x86 bundles all succeeded.
- Actual [Windows preview 2 Release](https://github.com/atorcam102/ELYVON-STUDIO/releases/tag/v0.1.0-windows-preview.2),
  published by [run 38760867335](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38760867335),
  has four separate EXE/MSI installers and SHA256SUMS. Public downloads were checked against those hashes.
  They are **unsigned**, not an updater test and not proof of launch on a Windows device.
- Historical [Release run 38760423899](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38760423899)
  failed before building: `v0.1.0-windows-preview.2` differs from application version `0.1.0`.
  Preview tag pushes are now excluded from that rebuild workflow; explicit dispatch still validates versions.
- Current phase: [PR #2](https://github.com/atorcam102/ELYVON-STUDIO/pull/2),
  [CI 38765992553](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553),
  [native raw measurements](audit/native-metrics-38765992553.json). Frontend, dependencies, native workspace/E2E/measurements and Windows x64/x86
  installer checks passed; exact context and observed 216 ms maximum frame interval are in the validation report.
- This phase adds frozen wire/signal fixtures, a native process restart regression, a native measurement
  runner, dependency policy and CI artifacts. See [validation](audit/VALIDATION.md).

Run from the checkout, with Node >=22.12, stable Rust >=1.88 and Tauri native prerequisites:

```bash
npm ci
npm run check
npm run check:rust
node scripts/dependency-inventory.mjs dependency-inventory.json
npm audit --audit-level=low
cargo deny --locked check        # cargo-deny 0.20.2, policy in deny.toml
actionlint -shellcheck= -pyflakes= # actionlint 1.7.12; no shellcheck/pyflakes claim
npm run tauri -- build --debug --no-bundle
xvfb-run -a dbus-run-session -- node --test e2e/app.e2e.mjs
xvfb-run -a dbus-run-session -- node e2e/measure.mjs
```

`npm run check:rust` includes tracked binding drift detection: regenerate and commit bindings
before this check on a branch that changes Rust types. It needs the built frontend for the Tauri shell.
Linux GUI commands need WebKitWebDriver, tauri-driver, Xvfb and D-Bus. Native dialogs are not automated:
E2E uses the same native open/save IPC commands the dialogs call. No mock bridge is used in native E2E.

## Inventory at entry

`git ls-files` recorded **200 tracked files**, clean main. The checkout contains a 5-crate Rust
workspace, a React application and a separate static design reference. `cargo metadata --locked`
reports **486 Rust packages (5 first-party)**; `package-lock.json` contains **218 dependency entries**,
including optional/platform/dev packages. These are lockfile counts, not counts of runtime libraries.

| Component | Locked version / state | Source |
| --- | --- | --- |
| App / project file | 0.1.0 / schema 1 | Cargo.toml, package.json, tauri.conf.json; check:versions |
| Rust | edition 2024, minimum 1.88; CI uses stable | Cargo.toml; no pinned rust-toolchain file |
| Tauri native / build | 2.12.4 / 2.7.3 | Cargo.lock |
| Tauri JS API / CLI | 2.12.4 / 2.12.3 | package-lock.json |
| Updater / dialog / log (Rust) | 2.14.0 / 2.8.3 / 2.10.1 | Cargo.lock |
| React / TypeScript / Vite | 19.3.1 / 6.0.3 / 8.3.4 | package-lock.json |
| Store / icons | Zustand 5.0.15 / Phosphor 2.1.10 | package-lock.json |
| Licence | First-party distribution licence undecided; no LICENSE file | README, workspace manifests |

Full reproducible inventories are uploaded as `audit-evidence/dependency-inventory.json`.
The Rust lock change in this phase only adds the existing serde_json dev dependency to elyvon-audio;
no dependency version is upgraded. New tooling is CI/local tooling, not a production dependency.

## Runners and platform evidence

Compilation, installer production, automated GUI execution and physical-device validation are
separate evidence. **No physical hardware test was performed in this audit.**

| OS / CPU | Rust target | Regular CI / runner | Installer evidence | GUI / device evidence |
| --- | --- | --- | --- | --- |
| Windows x64 Intel/AMD | x86_64-pc-windows-msvc | windows-latest | EXE + MSI built, published preview 2 | No Windows launch/install/device test |
| Windows x86 32-bit | i686-pc-windows-msvc | windows-latest, cross target | Separate EXE + MSI built, published preview 2 | No 32-bit host install/launch test |
| Windows ARM64 | aarch64-pc-windows-msvc | No regular build; legacy release config: windows-11-arm | No verified artifact | Untested |
| macOS Intel | x86_64-apple-darwin | No regular build; legacy: macos-15-intel | No verified artifact | Untested; signing/notarisation unknown |
| macOS Apple Silicon | aarch64-apple-darwin | No regular build; legacy: macos-latest | No verified artifact | Untested; signing/notarisation unknown |
| Ubuntu x64 | x86_64-unknown-linux-gnu | ubuntu-24.04 tests/E2E | Debug app compiled; no current installer distribution | Native WebKit E2E under Xvfb; no audio/device test |
| Ubuntu ARM64 | aarch64-unknown-linux-gnu | No regular build; legacy: ubuntu-22.04-arm | No verified artifact | Untested |

Current user-authorised distribution is Windows x86/x64. Seven platform entries in
`elyvon-update::platform` and the legacy full release matrix describe intended packaging;
they do **not** establish effective support. Do not dispatch that broader release matrix or
publish a stable release as part of this phase. Runner labels/availability outside regular CI
have not been exercised here. No audio APIs, device drivers, plug-ins or Atmos support are verified.

## Contracts and data preservation

[CONTRACTS.md](CONTRACTS.md) maps every public boundary and its version. `app_contracts`
reports IPC/project/audio/jobs versions. Session, track, region and musical timeline types remain
Rust-owned; TypeScript bindings are generated by ts-rs and checked for drift. UI selection/zoom
remain presentation state. Graph/MIDI/job contracts have validation and frozen fixtures, but
there is **no graph executor, scheduler, MIDI port or real-time callback**.

Project schema stays **1**. New control contracts are not serialised into project files.
The schema-1 fixture round-trip/save/restart/recovery tests protect all model fields and media
references. Recovery and atomic saves already existed; this phase does not replace them.

## Updater, permissions and secrets review

- `pubkey` is `UNCONFIGURED`. Unsigned previews correctly report updating unavailable;
  no latest.json or signatures are attached to preview 2. The presence of a SHA-256 file does not
  replace code signing or trusted updater signatures.
- Release preflight checks tag == app version, public key configuration and a test signature
  against the embedded key when signing secrets are supplied. Unit fixtures already cover
  valid/tampered/wrong-key/unversioned signatures. A production-key end-to-end install is untested.
- The secrets-list API returned **403 Resource not accessible by integration**. Existence and
  validity of repository signing/notarisation secrets are **unknown**, not absent. No secret was
  read, changed, printed or generated.
- Windows NSIS and MSI select their own installer entry; macOS updates use `.app.tar.gz`,
  distribution uses DMG; Linux uses independent `.deb` and `.AppImage`. Collector paths are
  target-specific under `target/<triple>/release/bundle`. macOS updater archive is omitted in
  unsigned mode, stale updater signatures are not copied. Platform/name/version rejection is
  tested in elyvon-update; macOS/AppImage installation paths remain untested on device.
- Stable endpoint is `releases/latest/download/latest.json`; prereleases are not offered to
  stable installations (`ALLOW_PRERELEASE=false`). An independent beta endpoint/channel picker
  **does not exist** yet; SemVer prerelease comparison alone is not a complete beta channel.
- CSP, freezePrototype and explicit dialog/log capabilities are present. No filesystem plugin
  is exposed to JavaScript. `core:default` remains Tauri's baseline; this audit does not certify
  every individual subpermission as minimal. Own native commands can access paths requested by
  the trusted local UI. There is no audio-device entitlement because no backend exists.
- CI has contents:read. Only publication jobs receive contents:write. Signing keys appear in
  the release build/preflight steps; no PR job receives them. Production signing and protection
  rules are unchanged. Actions currently use version tags rather than immutable SHAs: hardening
  remains pending. This is not an exhaustive security assessment.

## Bugs, limitations and next work

| ID | Finding / disposition | Remaining acceptance requirement |
| --- | --- | --- |
| B-01 | RustSec RUSTSEC-2024-0370: proc-macro-error 1.0.4 via GTK3/glib-macros is unmaintained. No safe upstream upgrade reported. Specific maintenance-only exception in deny.toml; no vulnerability exception. Duplicate transitive crate versions remain warnings. | Reassess upstream GTK/Tauri path before stable; keep inventory and advisory checks active. |
| B-02 | No first-party distribution licence; package-level licences inventoried and checked. Phosphor attribution recorded. Complete third-party licence texts are not bundled in installers. | Decide licence and ship required notices, including OS/bundled library obligations; no legal clearance claimed. |
| B-03 | Desktop dependencies unavailable in cloud container; apt installation denied. Core Rust and frontend are runnable; native GUI evidence comes from CI. | See native CI evidence; hardware Windows x86/x64 install/open/save/restart remains required. |
| B-04 | Production key/signing installation test and separate beta channel unverified/unimplemented. | Configure through maintainer secret settings and verify signed upgrades on device before stable. |
| B-05 | Legacy release workflow still contains seven builds while regular distribution is Windows only. Preview tag collision is fixed; no broader release was dispatched. | Scope/gate that workflow for a future explicitly authorised release; verify each target independently. |
| B-06 | Timeline benchmark uses the small Rust demo, debug mode and a virtual display; two of 120 frame intervals exceeded 50 ms (maximum 216 ms). | Release-mode large-session benchmark and real hardware budgets remain unset/unmeasured. |

**Next phase:** implement the first native audio backend and sample-clock transport behind these
contracts, with real device and RT-safety tests, in a separately authorised functional phase.
Do not enable playback, meters, plug-ins, jobs or channel promises merely because types now exist.
