# Phase audits

Each phase ends with an audit that lists what was built, the commands run, their results, the
platforms actually built and tested, and the limitations. A phase is **PARTIAL** while any
acceptance criterion is unmet or unverified (for example, listening tests on real hardware).

| Audit | Scope |
| --- | --- |
| [PHASE-02](PHASE-02.md) | Technical baseline: versioned contracts, fixtures, reproducible checks, CI evidence |
| [PHASE-03](PHASE-03.md) | Historical audio-engine probe (CPAL/ALSA and headless clock) |
| [PHASE-04-05](PHASE-04-05.md) | Temporal engine, transport and tempo; clips and audio regions |
| [PHASE-06-07](PHASE-06-07.md) | Audio capture and native MIDI |
| [PHASE-08-09](PHASE-08-09.md) | Piano roll and mixer graph, with 06–07 re-verified on Windows |
| [PHASE-10-11](PHASE-10-11.md) | Mix automation, non-destructive editing, takes and comping |
| [PHASE-12-13](PHASE-12-13.md) | Native effects: EQ, dynamics, utilities, delay, reverbs, modulation, distortion, pitch |

Raw measurements referenced by the audits are kept in [`data/`](data/).
