# Technical contracts

`app_contracts` (read-only Tauri IPC) returns `{ ipc: 2, project: 2, audio: 1, jobs: 1 }`; the
typed client is `ipc.app.contracts()`. IPC 2 (Phases 04–05) adds the engine status shape, transport,
media and render commands and the `audio://*`/`media://status` events. Project schema 2 adds the
tempo/metre map, markers, metronome, count-in, pre-roll, punch, sample rate, media pool and audio
region parameters (migration from 1 is automatic). The MIDI/graph contract stays at version 1.
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
| Errors | src-tauri/error.rs: IpcError/ErrorCode | Stable codes (adds `audioUnavailable`, `media`), human-readable message | Existing real native commands |
| Tempo/metre map | core/tempo.rs: TempoMap, TempoChange, MeterChange; core/grid.rs: GridSpec | Tempo steps at ticks; metre changes at zero-based bars; frames = round-half-up of exact seconds × rate | Saved in schema 2; golden vectors `core/tests/fixtures/grid_golden.json` shared with `src/lib/tempoMap.ts` |
| Transport | audio/transport.rs: TransportCommand, TransportSnapshot, PlayState, Meters, EngineStats | Commands applied at the next block; snapshot = engine frame, fractional ticks through the engine's map, seconds | `transport_command`, `transport_state`, `transport_tap`; events `audio://position` (~30 Hz on change), `audio://meters`, `audio://status` |
| Engine status/devices | audio/engine.rs: EngineStatus{running: config, device, output}, OutputKind; device.rs: DeviceInfo | `output` = `device` or `internalClock` (silent) | `audio_status`, `audio_devices`, `audio_restart`; preferences `Settings.audio` |
| Media | audio/media: MediaStatus/MediaState; project MediaAsset/MediaInfo | Offsets/fades in source frames at the file's own rate; fingerprint FNV-1a 64 | `media_import`, `media_relink`, `media_status`, `media_formats`; `media_region_peaks` binary: u32 LE columns + i16 LE [minL,maxL,minR,maxR] per column |
| Render | audio/render.rs: RenderReport | Engine rate, whole song or tick range | `project_render` → 32-bit float WAV + `<file>.json` |

## Validation and RT boundary

Deserialisation checks structure; callers must invoke `validate` for semantic checks. No new
IPC command accepts a graph, MIDI event or job today. Graph validation is bounded (4096 nodes,
65536 edges, 256 channels/node), rejects duplicate IDs/edges, missing nodes, invalid channels
and cycles. Parallel channel connections are legal; fan-in is descriptive and has no implicit
summing implementation. Empty plans are valid descriptions, not running engines. Version 1
has no feedback/delay nodes or latency compensation.

MIDI NoteOn velocity zero is valid MIDI 1.0 and means note-off to a consumer. The separate
persisted MidiNote model requires nonzero note velocity. The Rust scheduler (`engine.rs`) converts
ticks to frames through the plan's tempo map and emits ordered `MidiEvent`s per block to a
`MidiSink`; no instrument or port consumes them yet.

Graphs, plans and job messages allocate on the control thread. They must **never** be built,
cloned, validated or serialised in `process`/`reset`. Plans reach the callback through a
pre-allocated SPSC queue and replaced plans are freed on the control thread
(`tests/rt_alloc.rs` asserts zero allocations/frees inside `Engine::process`).

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

## Schema 4 and mixer/piano-roll IPC (0.3.0)

| Area | Type / command | Rules | Evidence |
| --- | --- | --- | --- |
| Notes | `MidiNote { id, pitch, velocity, channel, muted, start, length, quantizeOffset }` | `id` unique per region; notes are the playback/export source; `midiEvents` excludes note on/off | `v3-midi.elyvon` migration fixture, `midi_edit.rs` tests |
| Piano-roll edits | `addNotes`, `removeNotes`, `updateNotes`, `moveNotes`, `resizeNotes`, `transformNotes`, `splitChordVoices`, `editControllerLane`, `batch` | Note ids, region-relative ticks; `batch` (≤ 256, not nested) is one undo step; `EditOutcome.createdNotes` lists new ids in command order | `crates/elyvon-project/tests/piano_roll.rs` |
| Mixer edits | `setStrip`, `setSend`, `add/remove/move/setInsert`, `addAux`, `removeAuxes`, groups, folders, `setPanLaw` | `ChannelRef = track \| aux \| master`; routing must stay acyclic (validated before apply) | `crates/elyvon-audio/tests/mixer.rs` |
| Meters event | `audio://meters` → `[Meters { master, tracks[], auxes[] }, EngineStats]` | `ChannelMeter { peak[2], rms[2], clip }` linear, post fader; tracks in project order, auxes in mixer order | `mixer.rs` meters test, native E2E |
| Native commands | `midi_preview`, `mixer_status`, `mixer_reset_clips`, `project_export_stems` | Preview notes are released by the native worker (≤ 4 s) and never recorded | src-tauri/src/commands/mixer.rs |

`app_contracts` reports `project: 5` (schema 5, Phases 10–11).

## Schema 5 automation and editing IPC

| Area | Type / command | Rules | Evidence |
| --- | --- | --- | --- |
| Automation edits | `setAutomationMode`, `add/remove/update/moveAutomationPoints`, `removeAutomationLane`, `simplifyAutomation`, `writeAutomation` | `AutomationTarget { channel, region?, param }`; `moveAutomationPoints.deltaValue` is a fraction of the parameter range; `EditOutcome.createdPoints` | `crates/elyvon-project/tests/automation.rs` |
| Live capture | `automation_touch(channel, param, value, phase)` → `bool` | Only while playing and in Touch/Latch/Write; `false` means "apply as an ordinary edit"; the written data arrives as one `writeAutomation` edit and a `project://view` event when playback stops | native E2E |
| Editing edits | `slipRegions`, `rippleDelete`, `splitRange` (`EditOutcome.selectedRegions`), `cutRange`, `glueRegions`, `createCrossfade`, `setRegionsReversed`, `setFadeCurve`, `sliceRegion`, `normalizeRegions`, `compTake`, `setCompCrossfade`, `flatten/unflattenComp`, `freezeTrack`, `unfreezeTrack`, `bounceInPlace`, `relinkMediaFiles`, `removeUnusedMedia` | Each is one undo step; frozen material refuses edits | `crates/elyvon-audio/tests/{editing,comping}.rs` |
| Native commands | `region_normalize`, `region_transients`, `track_freeze`, `track_bounce`, `media_repair`, `media_collect`, `media_unused`, `media_backup_unused(confirm)`, `media_rescan`; `mixer_status.staleFreezes` | Renders refuse to run while playing or with missing media; clean-up needs `confirm: true` and moves files to `.elyvon-backup/` | `tests/media_repair.rs`, native E2E |
