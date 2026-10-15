# Phase 02 validation evidence

Date: 2026-10-15. Audit entry: b30cd673309fe416099db888fa5cb043506d69f8.
This report separates current changes from historical CI and physical-device claims.

## Current cloud container

- Debian 13 x86_64, Node 24.19.0, npm 11.9.0.
- Installed official Rust stable 1.99.0, rustfmt and clippy under /workspace/tooling;
  rustup-init SHA-256 verified against the official checksum before execution.
- cargo-deny 0.20.2 and actionlint 1.7.12: official release archives, SHA-256 verified.
- Native Tauri GUI prerequisites are missing. `apt-get update` failed with EACCES on
  /var/lib/apt/lists, including outside the filesystem sandbox; sudo is not installed.
  No local desktop launch/RSS/GUI benchmark is claimed. Native runs execute in CI instead.

| Command | Observed result |
| --- | --- |
| cargo test --locked -p elyvon-core -p elyvon-project -p elyvon-audio -p elyvon-update | Passed: 124 tests including generated binding tests, 0 failed; Tauri shell excluded locally |
| cargo clippy --locked -p elyvon-core -p elyvon-project -p elyvon-audio -p elyvon-update --all-targets -- -D warnings | Passed after fixing newly introduced lint findings |
| cargo fmt --all --check | Passed |
| npm run check | Passed: versions/typecheck/lint/31 Vitest tests/production build |
| npm audit --json | 0 reported vulnerabilities, 218 dependency entries at audit time |
| node scripts/dependency-inventory.mjs .local/dependency-inventory.json | 218 npm entries; 486 Rust packages including 5 first-party; source/integrity/licence metadata checked |
| cargo deny --locked check, initial strict policy | Failed: private workspace path wildcards and maintenance advisory RUSTSEC-2024-0370; licences/sources passed |
| cargo deny --locked check, reviewed policy | Passed with duplicate-version warnings and the explicit B-01 maintenance-only exception; no vulnerability exception |
| actionlint -shellcheck= -pyflakes= | Passed; checks YAML/Actions expressions, not shellcheck or pyflakes |

## Native CI evidence

Baseline full native run: [38757410164](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38757410164)
on fbc4566 (before Phase 02). It passed workspace tests/clippy/binding drift, 10 native E2E
cases and separate Windows x64/x86 bundles. It is not proof of the new phase's checks.

The current phase's CI creates:

- `native-evidence`: actual WebKit screenshots and native-metrics.json, retained 30 days.
- `audit-evidence`: complete dependency inventory, npm audit and cargo-deny output, retained 30 days.
- `ci-windows-x64`, `ci-windows-x86`: independent verification installers, retained 3 days.
- `frontend-dist`: production frontend. No release is published automatically by this audit.

The new E2E verifies app_contracts versions and stops/restarts the native executable before
opening a saved session and comparing the entire Project. Native file-picker interactions,
physical installation and audio devices remain outside this automation.

## Measurement protocol and limits

`node e2e/measure.mjs` launches the actual debug binary via tauri-driver three times with
isolated XDG paths. It measures wall time from starting WebDriver to controlbar + successful
project_get, then samples idle /proc RSS after one second. Memory sums only the app PID and
its observed descendants, including WebKit; shared pages can be counted more than once.
It opens Rust's demo and scrolls the real timeline for 120 requestAnimationFrame intervals,
recording raw samples, median and p95. A zero-scroll run fails.

This is a warm-cache diagnostic on Linux x64/Xvfb, not cold startup, executable-only time,
unique RAM, physical FPS, release optimisation, callback CPU, audio latency or a large-session
benchmark. No performance threshold or hardware guarantee is invented. The runner writes
machine/build metadata and the exact commit with every result. Missing or failed measurements
must stay unmeasured; they cannot be filled using Chromium mocks or estimated values.

The first phase CI attempt (run 38765613839) failed in the generic audit-tool installer
(exit 76, before audits ran). It was replaced with scripts/install-audit-tools.sh, which
uses the same official, checksum-verified archives tested locally. This is a tooling failure,
not a passing dependency audit.

## Measured native results — current phase

[CI run 38765992553](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553)
completed successfully with all six jobs green on PR #2 head
f949aba679dc731fa02e69ace39b6dcc2e4c93ff, using GitHub's test merge
de4992ba8618a1a12d09e4f5e0540cf853c47d69. Workspace fmt/clippy/tests/bindings and native E2E + measurement steps passed.
Both Windows x64/x86 installer build, collection and upload steps passed:
[x64 artifact](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553/artifacts/11990431405)
and [x86 artifact](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553/artifacts/11990307033).
These are verification artifacts (EXE/MSI per target), not hardware installation evidence or a new Release.

[Raw measurements](native-metrics-38765992553.json) were retrieved from the native job's
`Native metrics` check annotation (check-run 116529915591), containing the same report as
[native-evidence](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553/artifacts/11989571280).
The durable raw JSON SHA-256 is `1f9ef4a281041de84048a4120ef1d1f5a121db6321caac9d30cc67545d7f66cc`. Runner: Ubuntu 24.04 x64,
AMD EPYC 7763, 4 logical CPUs, ~16 GB RAM; native **debug** build and Xvfb. No hardware claim.

| Sequential warm launch | WebDriver-to-ready wall time | Aggregate idle RSS (app + WebKit) |
| --- | --- | --- |
| 1 | 683.3 ms | 643.1 MiB |
| 2 | 723.8 ms | 628.1 MiB |
| 3 | 618.9 ms | 626.2 MiB |

The demo dataset has **6 tracks / 9 regions / 32 bars**. The viewport was 1280 × 775,
scroll range 3226 px. Of 120 sampled frame intervals: median **17 ms**, p95 **28 ms**,
maximum **216 ms**; **2 intervals exceeded 50 ms** (216 and 214 ms). These stalls are retained
in the raw data, not discarded as outliers. This small debug-session run is a baseline,
not an optimisation pass or a real-time performance acceptance result. The aggregate memory
includes shared pages and is not unique application RAM. Large sessions, release builds,
Windows startup/RSS and physical displays remain unmeasured.

[Dependency evidence](https://github.com/atorcam102/ELYVON-STUDIO/actions/runs/38765992553/artifacts/11990260597)
contains exact package/licence/source inventory and audit output. The dependency/licence/workflow
job passed under the policy recorded in deny.toml (including the explicit B-01 maintenance exception).
Artifact retention is finite; raw measurement JSON is also committed here for reproducibility.

## Changed files and implementation

Implementation: [PR #2](https://github.com/atorcam102/ELYVON-STUDIO/pull/2),
[contract commit 70ca2bb](https://github.com/atorcam102/ELYVON-STUDIO/commit/70ca2bb0d6e7c97660311f3f0dfb1d61376e3216),
[tooling correction f949aba](https://github.com/atorcam102/ELYVON-STUDIO/commit/f949aba679dc731fa02e69ace39b6dcc2e4c93ff).
The [PR file list](https://github.com/atorcam102/ELYVON-STUDIO/pull/2/files) is the complete diff.

- Native contracts: audio/contracts.rs, core/jobs.rs; module exports; generated src/bindings types;
  src-tauri commands/system.rs and lib.rs; typed src/lib/ipc.ts.
- Presentation contract: src/state/uiContract.ts and ui.ts (no visual redesign).
- Fixtures/tests: audio/tests, core/tests, project/tests/persistence.rs, native E2E and WebDriver lifecycle.
- Verification: scripts/check.mjs, dependency-inventory.mjs, install-audit-tools.sh, deny.toml,
  ci.yml, preview-tag exclusion in release.yml, package scripts and one dev dependency lock entry.
- Documentation: README, CONTRIBUTING, THIRD_PARTY_NOTICES, STATUS, FEATURE_MATRIX, CONTRACTS,
  ARCHITECTURE, ROADMAP, RELEASES, numbered ADRs and this evidence report/raw JSON.

Reusable cloud install/start instructions were tested and saved to the environment draft.
That save does not publish or apply the draft, nor supply the missing native GUI libraries.
