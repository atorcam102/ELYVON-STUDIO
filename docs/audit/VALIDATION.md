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

Raw current-phase results and CI links are added after the run completes. Until then,
current native measurements and Windows builds are **pending**, not passed.
