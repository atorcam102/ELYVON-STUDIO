# ADR 0004: Make dependency debt and measurements reviewable

Status: Accepted for this implementation · 2026-10-15.

Context: the repository had no licence/advisory/workflow syntax gate or reproducible native
measurement runner. Decision: shared frontend/Rust checks, cargo-deny 0.20.2, npm audit, reviewed
npm licence metadata, actionlint 1.7.12 and uploaded raw evidence. Inventory all locked targets.
Retain one explicit maintenance-only RustSec exception RUSTSEC-2024-0370 (GTK3 build macro,
no safe upstream upgrade); do not waive known vulnerabilities. Duplicate versions stay warnings.
Licence metadata is not legal clearance; bundled notices remain pending. Measure the real native
debug binary under Xvfb, preserve raw samples, label overhead and avoid invented budgets.
Consequence: Phase 02 remains partial until missing hardware/signing/licence criteria are met;
CI can pass under its documented policy without those criteria being misrepresented.
