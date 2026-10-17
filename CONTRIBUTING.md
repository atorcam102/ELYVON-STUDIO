# Contributing to Elyvon Studio

## Environment

| Tool | Version |
| --- | --- |
| Rust | stable ≥ 1.90 (`rustup`), target `x86_64-pc-windows-msvc` |
| Node.js | ≥ 22.12 with npm |
| Windows | Visual Studio Build Tools (C++ workload), WebView2 (bundled with Windows 10/11) |
| Linux (tests only) | `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev libssl-dev libasound2-dev patchelf` |

```bash
npm ci
npm run tauri dev
```

## Checks

Run these locally before every commit; they are the same commands the audits record.

```bash
npm run check              # versions, typecheck, lint, Vitest, production build
npm run check:rust         # fmt, clippy -D warnings, workspace tests, binding drift
npm audit --audit-level=low
cargo deny --locked check  # 0.20.2; exception B-01 is documented in docs/STATUS.md
```

### End-to-end tests against the real binary

The suite in `e2e/` drives the built application through `tauri-driver` and WebDriver.

**Windows** (needs [msedgedriver](https://developer.microsoft.com/microsoft-edge/tools/webdriver/)
matching the installed WebView2 version on `PATH`):

```bash
cargo install tauri-driver --locked
npm run tauri -- build --debug --no-bundle
npm run test:e2e
```

Set `E2E_SCREENSHOTS=<folder>` to keep the screenshots the tests take.

**Linux** (headless):

```bash
sudo apt install webkit2gtk-driver xvfb dbus-x11
cargo install tauri-driver --locked
npx tauri build --debug --no-bundle
xvfb-run -a dbus-run-session -- npm run test:e2e
xvfb-run -a dbus-run-session -- node e2e/measure.mjs
```

## Architecture rules

- **Logic lives in Rust.** The UI never computes or validates the model: it sends an
  `EditCommand` and draws the `SessionView`. No DSP in JavaScript.
- **A new project edit** is an `EditCommand` variant in `crates/elyvon-project/src/edit.rs`, an
  undo label and tests. Global validation runs after every edit; new rules go in `validate.rs`.
- **Changing the project format** means incrementing `CURRENT_SCHEMA_VERSION`, adding the
  migration in `format::migrate`, a fixture of the previous schema in `tests/fixtures/` and
  updating `docs/PROJECT_FORMAT.md`. Existing fixtures are never modified.
- **IPC commands** are always `async`; I/O runs in `spawn_blocking`; the session mutex is never
  held across I/O.
- **Real-time audio code** never allocates, locks, does I/O or logs in the callback. Parameters
  travel through atomics or preallocated lock-free queues; `tests/rt_alloc.rs` enforces it.
- **Honesty:** a feature that does not exist is shown disabled with its reason, or says
  «no disponible». Demo data lives only in `elyvon_project::demo`. Platform support is claimed
  only after building and testing on that platform.
- **Design:** use the tokens in `src/styles/tokens.css`; icons go through
  `src/components/Icon.tsx` (Phosphor Icons, MIT, credited in `THIRD_PARTY_NOTICES.md`).
- **Dependencies:** check licences, sources and notices of every new dependency against
  `deny.toml` and `scripts/dependency-inventory.mjs`.
- Actual contracts and support are recorded in `docs/CONTRACTS.md` and `docs/STATUS.md`.

## Repository layout

See the [README](README.md#repository-layout).

## Commits and branches

- Short branches from `main`; messages in the imperative ("Add…", "Fix…") with context in the body.
- Add user-visible changes to `CHANGELOG.md` under `[Unreleased]`.
- Builds and tests run locally on Windows x64. GitHub Actions is not used unless explicitly
  requested; commits carry `[skip ci]` so a push does not start workflows.

## Releases

See [docs/RELEASES.md](docs/RELEASES.md).
