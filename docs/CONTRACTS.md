# Technical contracts — version 1

These boundaries describe current model behaviour and explicitly future interfaces. They do not
provide audio/MIDI/job capabilities. `app_contracts` (read-only Tauri IPC) returns
`{ ipc: 1, project: 1, audio: 1, jobs: 1 }`; the typed client is `ipc.app.contracts()`.
Breaking a wire contract requires a version bump, fixtures and migration/negotiation decisions.
Application SemVer, project schema and control-contract versions are independent.

| Boundary | Owner / type | Time / identity | Persistence and evidence |
| --- | --- | --- | --- |
| Session | project/session.rs: SessionView, ProjectOrigin, ApplyResult | revision increments within a session, not a durable cross-restart revision | Full model view after edits; dirty/undo labels are session state |
| Song/tracks/regions | project/model.rs: Project, Track, Region, RegionContent | UUID identities; Ticks u64 bounded to 2^40, exactly representable by JS number | Existing .elyvon schema 1, v1-basic.elyvon fixture |
| Timeline/musical settings | core/music.rs: Ticks, Tempo, TimeSignature; model Transport/LoopRange | PPQ 960; region start absolute ticks, MIDI-note start relative to region | Validation limits; no pixel positions, playhead clock or sample-rate state saved |
| MIDI events | audio/contracts.rs: MidiEvent, MidiMessage | u32 frameOffset within ONE callback (0 <= offset < blockFrames); MIDI channels 0–15, data 0–127 | Frozen midi-v1.json. Separate from persisted MidiNote; no conversion/scheduling or device API |
| Graph | AudioGraphPlan, GraphNode, GraphConnection | u32 node ids, u16 zero-based channels; explicit contractVersion | Not part of project schema; graph-v1.json; no executor |
| Background work | core/jobs.rs: JobSnapshot, JobState | Process-scoped opaque id; completed/total u32 work units, None = unknown | Not persisted; jobs-v1.json. DownloadEvent remains the implemented updater progress contract |
| UI presentation | src/state/uiContract.ts: UiPresentationV1, Selection | UI_CONTRACT_VERSION=1; cursorTicks is an editing cursor, not playback | Only Layout persisted via Settings.schemaVersion=1; selection/zoom/dialogs ephemeral |
| Errors | src-tauri/error.rs: IpcError/ErrorCode | Stable codes, human-readable message | Existing real native commands; contract validators return static Rust errors, not pretend worker events |

## Validation and RT boundary

Deserialisation checks structure; callers must invoke `validate` for semantic checks. No new
IPC command accepts a graph, MIDI event or job today. Graph validation is bounded (4096 nodes,
65536 edges, 256 channels/node), rejects duplicate IDs/edges, missing nodes, invalid channels
and cycles. Parallel channel connections are legal; fan-in is descriptive and has no implicit
summing implementation. Empty plans are valid descriptions, not running engines. Version 1
has no feedback/delay nodes or latency compensation.

MIDI NoteOn velocity zero is valid MIDI 1.0 and means note-off to a future consumer. The separate
persisted MidiNote model requires nonzero note velocity. Event ordering and tick/sample conversion
belong to a future Rust scheduler, not React. AudioBlock's borrowed slices and AudioProcessor's
prepare/process/reset contract already exist; no callback is installed by this phase.

Graphs and job messages allocate on the control thread. They must **never** be built, cloned,
validated or serialised in `process`/`reset`. Future RT code must use preallocated buffers and
queues, atomics, and off-thread reclamation; AtomicF32 alone does not implement a safe graph swap.

Jobs do not claim success from `completed == total`; only Completed signals completion.
Cancelled is a worker acknowledgement, not proof that a cancellation request was received.
`Unavailable` must be shown as such. No scheduler, IPC job submission API or worker exists.

## Compatibility and fixtures

`cargo test --workspace --locked` exports bindings through ts-rs using `.cargo/config.toml`;
never hand-edit `src/bindings`. `npm run check:rust` also rejects generated-binding drift.
`Project`, `EditCommand`, `SessionView`, `Settings`, `IpcError` retain their existing shapes.
The additive `app_contracts` command does not alter existing IPC results.

- `crates/elyvon-project/tests/fixtures/v1-basic.elyvon`: all three track kinds, notes and a media reference.
- `crates/elyvon-project/tests/persistence.rs`: full equality on load/save/reopen, recovery,
  failed-open preservation and restart-equivalent drop/recreate. Native E2E separately restarts the process.
- `crates/elyvon-audio/tests/fixtures/{graph,midi}-v1.json`: frozen JSON keys/discriminants/units.
- `crates/elyvon-core/tests/fixtures/jobs-v1.json`: every job state including unknown totals.
- `crates/elyvon-audio/tests/fixtures/signals-v1.json`: original eight-sample silence/impulse/DC arrays,
  used only for pure gain/pan energy checks. No commercial samples, decoder or playable instrument.

Schema 1 is intentionally unchanged: graph/jobs/device state must not be appended to project
files without a separate schema design and migration fixtures. Existing asset paths are preserved;
this phase does not import, resolve, rewrite or delete media.
