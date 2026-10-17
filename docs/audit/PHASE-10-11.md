# Phases 10–11 — Mix automation, non-destructive editing, takes and comping

Date: 2026-10-17. Branch `fases-10-11` (from `65f9fc9`, merged with `main` at `f7b3d47`),
project schema 5, application version unchanged (0.3.0). **Overall: PARTIAL** — every software
acceptance test below passes on Windows x64, but nothing was listened to on speakers, no
physical control surface or microphone was used, and no other target platform was built.

## Environment (the user's laptop, local only)

Windows 11 Pro 10.0.26200 x86-64; Rust 1.99.1 `x86_64-pc-windows-msvc`; Node 24.21.0;
WebView2/Edge 155.0.4283.60; tauri-driver 2.1.0; msedgedriver 155. No GitHub Actions, cloud
runner or remote builder was used. The work was done in a separate git worktree because another
local session was editing the main checkout at the same time.

## Entry state

Phases 08–09 (0.3.0) were in place: engine mixer graph with typed inserts and sends, ramped
static parameters, snapshot undo, recorded take folders with whole-take selection, region
split/trim/fades/loop, media relink. Absent before this work: any automation, comping,
reverse, slip, ripple, marquee, glue, slices, normalise, bounce, freeze, media repair/collect.

## Phase 10 — automation

| Requirement | Implementation | Evidence |
| --- | --- | --- |
| Track/region automation for fader, pan, sends, mute and DSP parameters with stable ids | `elyvon-project/src/automation.rs`: `AutomationParam` (send by aux id, insert parameter by insert id), channel lanes on tracks/auxes/master, region lanes that replace the track lane inside the region and move with it; lanes of removed sends/inserts are pruned in the same undo step | `tests/automation.rs` (project), `region_lanes_override_the_track_lane_inside_their_region_and_move_with_it` |
| Read, Touch, Latch, Write; capture without locks on the callback | `automation_record.rs` recorder (passes, cycle wraps via the engine's cycle counter, locates, hold-then-ramp for still controls, RDP thinning); `ParamOverride` SPSC queue into the callback; `src-tauri/src/automation.rs` starts/finishes with the transport and applies one `WriteAutomation` edit | project tests for Touch/Latch/Write/cycles; `touch_overrides_reach_the_callback_without_clicks_and_release_to_the_lane`; native E2E drags the mixer fader while playing in Touch mode |
| Linear, step, Bézier; sample-accurate smoothing | 1-D cubic Bézier; envelopes compiled to frames and split at tempo changes (exact in ticks); 2 ms de-click ramps ending on the point's frame, linear in gain for levels; insert coefficients on an absolute 32-frame grid, trim/bypass per frame | `linear_ramps_are_exact_in_ticks_across_tempo_changes`, `steps_ramp_into_the_new_value_and_land_on_the_point`, `thousands_of_points…` (largest sample step 0.0052 = 0.5/96 on a DC signal) |
| Lane editor with points, selection, move, copy, duplicate, undo | `src/features/editors/AutomationEditor.tsx` (channel/parameter/mode/region selectors, add by click, drag (Alt copies, Shift free), rubber band, Delete, Ctrl+C/V/D, curve menu, Simplify); drawing mirrors the core (`src/lib/automation.ts`) | Vitest golden vectors shared with Rust (`automation_golden.json`); native E2E adds a point on the canvas and undoes it |
| Persistence; offline render identical to live | Schema 5; render uses the same engine | `automation_survives_save_and_reopen_identically`; `renders_are_identical_for_any_block_size` (64/512/1031 bit-exact); `live_playback_equals_the_offline_render_including_seeks` (irregular callbacks 9…4096 frames, bit-exact; locate while playing) |
| Thousands of points, abrupt changes, tempo maps, discrete parameters | — | 2 000 steps across 11 tempo changes; 100 000-point lane; mute/bypass switches |

Acceptance: automating a filter and a fader changes the signal measurably — **met**
(`automating_a_filter_cutoff_changes_the_spectrum`: 5 kHz sine −30 dB at a 200 Hz cutoff, within
1 dB at 20 kHz, monotonic sweep; `volume_ramp_fixture_has_the_expected_rms_per_segment`: 28
segments of 100 ms within ±0.05 dB of the lane). Write/Touch never modify unarmed channels,
region lanes or anything while stopped, and the project reopens identically — **met**
(`write_mode_does_not_touch_unarmed_channels_or_region_lanes`,
`read_and_off_modes_and_stopped_transport_never_write`, save/reopen equality, E2E checks region
lanes and the static fader are untouched). No clicks from control transitions within what is
technically expected — **met in measurement** (≤ 0.0052 per sample on full-scale steps; touch
override max step < 0.001); not confirmed by ear.

## Phase 11 — non-destructive editing, takes and comping

| Requirement | Implementation | Evidence (`crates/elyvon-audio/tests/editing.rs` unless noted) |
| --- | --- | --- |
| Split/glue, slip, ripple, marquee, fades, editable crossfades, normalise, reverse, clip gain, bounce in place, freeze | `region_edit.rs`, `comp.rs`, engine `plan.rs`/`analysis.rs`; timeline Pointer/Marquee tools, Ripple mode, Ctrl+Alt-drag slip, context menu, inspector, `track_freeze`/`track_bounce`/`region_normalize` | `glue_joins_contiguous_pieces…`, `slip_moves_the_audio_inside_the_region`, `ripple_delete_and_marquee_ranges…`, `crossfades_sum_correctly_for_every_fade_shape`, `reverse_plays_backwards…`, `normalize_sets_a_non_destructive_gain…` (−1.000 dBFS), `bounce_in_place…` (max error < 1e-5), `freeze_plays_the_rendered_file_identically_and_refuses_edits` |
| Takes and comping from loop recordings; smooth transitions; reversible consolidation | `TakeComp` segments over a take folder; virtual regions with crossfades centred on each boundary; `FlattenComp`/`UnflattenComp` insert/remove exactly those regions; takes editor (`TakesEditor.tsx`) | `crates/elyvon-audio/tests/comping.rs`: three cycle passes through the real capture writer, comp, numerical check per segment, export decoded back, save/reopen, undo/redo, flatten/unflatten identical, take files' bytes unchanged; native E2E comps by dragging across a take and verifies the exported WAV samples |
| Transients, markers, slices; multitrack groups with phase lock | Energy-rise onset detector refined to the sample; `SliceRegion`; markers by batch; `MixGroup.linkEdit` applies split/trim/move/slip/delete/mute to regions with the same span | `transients_are_found_where_hits_start` (±1 ms, slicing inaudible), `phase_locked_group_edits_keep_multitrack_recordings_aligned` |
| Missing media, corrupt files, caches; relink; destructive confirmations | `media/repair.rs`: search by name + size + content fingerprint, verified collect into `Media/`, unused files moved to `.elyvon-backup/<time>/` only with `confirm: true`; rescan of failed media; Project media dialog | `tests/media_repair.rs` (look-alike refused, moved project folder resolves, backup not delete); native E2E repair + collect + refused unconfirmed clean-up |
| Robust, scalable history; backups for approved destructive operations | Memory-budgeted snapshot history (512 MiB / 96 MiB 32-bit); clean-up keeps a backup | `crates/elyvon-project/tests/history.rs` |
| Fade sums, overlapping edits, undo after large projects, media consistency | — | linear/S-curve crossfades of identical material deviate < 1e-5; equal power keeps uncorrelated level within 0.6 dB, linear dips ≈ 3 dB; 100 random edits on 7 680 regions + 128 000 points, every undo state compared |

Acceptance: a comp of several takes plays a continuous composite without losing the takes —
**met** (numerical, export, reopen, undo; E2E). Reverse/normalise/freeze change the audio and
undo/redo and recovery work — **met** for reverse/normalise/freeze and undo/redo; crash recovery
uses the existing snapshot mechanism unchanged (schema 5 data is part of the snapshot; not
re-tested by a forced crash in this phase). Media stay original by default and projects become
portable with collect media — **met** (source fingerprints checked, moved project folder
resolves every file).

## Commands run (this laptop, Windows x64)

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | **402 passed, 0 failed** (1 ignored hardware probe) |
| `npx tsc --noEmit`, `npx eslint . --max-warnings 0`, `npx vitest run` | pass; Vitest 45/45 |
| `npm run tauri -- build --debug --no-bundle` | pass (`x86_64-pc-windows-msvc`, debug) |
| `node --test e2e/app.e2e.mjs` (tauri-driver + msedgedriver, real binary) | **17/17 pass** (2 new) |
| `cargo test --release -p elyvon-audio --test automation -- --nocapture` | figures below |
| `cargo test --release -p elyvon-project --test history -- --nocapture` | figures below |

Measured (release, this laptop): a 100 000-point lane compiles in 31 ms and a 104 s render with
it takes 0.87 s; a 2 000-point lane over 11 tempo changes renders 12 s in 40 ms; the history test
(7 680 regions, 128 000 points, ≈ 8.3 MiB per snapshot) applies 100 edits in 3.1 s (≈ 31 ms per
edit including validation) and undoes 61 steps in 68 ms — the 512 MiB budget keeps 61 steps of
this project and drops the oldest. The callback allocation test covers envelopes, overrides,
plan swaps with lanes and Stop.

Screenshots of the real app: [`docs/screenshots/phase-10-11-windows/`](../screenshots/phase-10-11-windows/)
(`12-automation-lane.png`, `13-takes-comp.png`).

## Integration into `main` with the chrome redesign

The branch was merged locally into `main` (merge `86b9c13`, no push) on top of the desktop chrome
redesign (`72e8514`, Spanish menu bar, control bar, side panels, library, score and smart-tempo
panels), which was committed unchanged first. Conflicts were resolved by keeping both: the bottom
panel has Editor de teclado, Partitura, Tempo inteligente, Automatización and Tomas y audio; the
strip's automation slot is the real mode selector; settings accept every tab.

Redesign controls connected to existing features afterwards: toolbar Automatización (opens the
lane editor), Seguir cursor (the arrangement's follow mode), Herramientas (Puntero, Marquesina,
ripple); inspector Bucle (audio regions), Cuantizar and Transponer (all notes of the selected MIDI
region, one undo step each); the Tempo and Compás lists show every tempo and metre change and
locate on click. Partitura draws a simplified, read-only notation of the selected MIDI region
(pitch with sharps and ledger lines, position inside each bar, metre changes; no rhythmic
values, says so in the panel; `scoreModel.test.ts`). Tempo inteligente shows the tempo map at
the cursor and states that tempo analysis is not available. Controls without a feature behind
them (Live Loops, Flex, library sounds, Smart Controls assignment, several Edición/Funciones
menus) keep the redesign's «todavía no disponible» notice.

Verified on `main` on this laptop (Windows x64): `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test --workspace` **402 passed**; `npm run check` (tsc, ESLint, Vitest **50/50**); `npm run
tauri -- build --debug --no-bundle`; native E2E **17/17** after moving the selectors to the
Spanish chrome (Undo, status and settings now live in «Más controles»; the mixer opens from
Mezclador (X); the inspector also shows a channel strip, so mixer queries are scoped to the
bottom panel; the piano-roll pencil uses the reference default velocity 80; the suite starts
from a clean layout because the layout is persisted). CSS edits made by the parallel UI session
after the build were not part of this verification.

## Platform matrix

| Target | Built | Tested |
| --- | --- | --- |
| Windows x64 (`x86_64-pc-windows-msvc`) | yes, debug, locally | Rust/Vitest/native E2E as above; internal device output; no installer built (not requested) |
| Windows x86, Windows ARM64, macOS x64/ARM64, Ubuntu x64/ARM64 | no | no — no support claimed |

## Bugs, limitations and risks

- Touch/Latch/Write capture is stamped with the engine position when the UI gesture reaches the
  core (IPC latency, a few ms); playback of written data is sample-accurate. No hardware control
  surface support.
- Region lanes are drawn and edited in the lane editor but never written by gestures.
- Snapshot undo stays O(project) per edit (≈ 31 ms per edit on the large test project).
- Glue of non-contiguous or different files is refused with a pointer to Bounce in Place.
- Freeze removes the insert look-ahead latency (the frozen track is aligned to the timeline, so
  the whole mix plays that much earlier than the live, compensated graph). A tempo or file change
  makes a freeze stale: the track plays live and the inspector says so.
- Transient detection is energy-based; dense or legato material needs a different sensitivity.
- In a 1280 × 780 window the mixer strips (with the new automation-mode selector) need vertical
  scrolling to reach the faders.
- On Windows the capture queue (170 ms) reported disk backpressure in tests that fed it faster
  than real time; real-time pacing passed. Worth watching with real devices and antivirus scans.
- Not done: listening tests, physical microphone/loopback recording of a cycle, other platforms.

## Next phase

Phase 12 — essential DSP effects (EQ and dynamics), whose parameters can use the insert
automation added here. Before that: listen to this branch on Windows hardware (automation moves,
comp crossfades, freeze/bounce) and record a real cycle take with a microphone.
