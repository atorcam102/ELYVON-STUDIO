# ADR 0003: Separate intended targets from verified distribution

Status: Accepted for this implementation · 2026-10-15.

Context: seven target descriptors and a legacy release matrix exist; only Windows x86/x64
are currently authorised for distribution. Decision: preserve separate installer/platform keys,
keep regular distribution limited to those two, and classify all seven individually. Linux x64
GUI CI is test infrastructure, not a claim of Linux installer support. Skip preview tag pushes
in the SemVer rebuild flow because preview tags refer to already built 0.1.0 binaries.
Consequence: no new platform is enabled or stable release published; physical-device and signing
validation remain explicit gaps. See ../STATUS.md and ../RELEASES.md.
