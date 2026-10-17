# Phases 08–09 — Piano roll and mixer graph (and 06–07 re-verification on Windows)

Date: 2026-10-16. Version 0.3.0, project schema 4. Entry revision `941ea44` plus the uncommitted
0.2.1 MP3/ID3 hotfix that was already in the working tree. **Overall: PARTIAL** — every
software acceptance test below passes on Windows x64, but on-device listening, a physical MIDI
keyboard and every other target platform are still unverified.

## Environment (the user's laptop, local only)

Windows 11 Pro 10.0.26200 x86-64; Rust 1.99.1 `x86_64-pc-windows-msvc` (rustup, installed this
session); Visual Studio 2022 Build Tools 17.14 (MSVC 14.44.35207, Windows SDK 10.0.26100);
Node 24.21.0 / npm 11.19.0; WebView2 155.0.4283.45; tauri-driver 2.1.0; msedgedriver
155.0.4283.45 (Authenticode-signed by Microsoft). No GitHub Actions or remote builder was used.

## Phases 06–07 re-verified on Windows

The 06–07 audit was produced on Linux. A clean worktree of the entry revision (+ hotfix) was
built and tested here first:

| Check | Result on Windows x64 |
| --- | --- |
| `cargo test --workspace --locked` | 271 pass, **1 fail**: `grid_golden` compared a CRLF checkout byte-for-byte (git `core.autocrlf=true`). Fixed by normalising line endings in the test → 272 pass |
| `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings` | pass |
| `npm run check` | pass |
| `npm run tauri -- build --debug --no-bundle` | pass |
| Cargo.lock | the pending 0.2.1 bump had not updated first-party versions in the lock file (`--locked` failed); regenerated (version lines only) |

Defects found in the 06–07 implementation and fixed in this change:

1. Recorded/imported MIDI played from raw note events while `content.notes` was only a
   projection, so notes could not be edited (ADR 0007). Notes are now the source.
2. Note on/off pairing was last-in-first-out, so overlapping same-pitch notes changed length on an
   SMF export/import round trip (found by `piano_roll_sequence_exports_an_equivalent_smf`). Now FIFO.
3. Every plan swap (any edit while playing) sent a MIDI panic and cut held notes. Now only notes
   that no longer exist are released.
4. Instrument tracks had no sound source: the virtual keyboard and MIDI input monitor were
   inaudible unless an external port was routed. The built-in test instrument now plays them.
5. SMF export ignored note channel/mute and did not crop notes to the region end; at equal ticks
   a note-on could precede the previous note-off.
6. ESLint scanned the git-ignored `.local/` tool cache in this checkout.

Hardware probe (`tests/hardware_probe.rs`, ignored by default, enumeration and output
negotiation only — no recording): WASAPI output "Altavoces (Realtek Audio)" negotiated 48 kHz F32
stereo; input "Intel Smart Sound Technology for Digital Microphones" listed; WinMM MIDI output
"Microsoft GS Wavetable Synth"; no MIDI input connected; virtual ports unsupported on Windows (as
documented). The native E2E suite played audio and ran the MIDI recording case on this machine.
Not done: microphone/guitar/loopback recording and listening, physical MIDI keyboard, cable
removal — screen control of the app was declined, and there is no MIDI keyboard attached.

## Phase 08 — piano roll

| Requirement | Implementation | Evidence |
| --- | --- | --- |
| Keyboard, snap, draw/drag/resize/duplicate/delete, range & multi-select, tools | `src/features/editors/PianoRoll.tsx` (HiDPI canvas, culled drawing), `pianoRollModel.ts`; native `addNotes`/`moveNotes`/`resizeNotes`/`removeNotes`; Alt-drag copy, rubber band, Shift/Ctrl toggling, pointer/pencil/eraser | `pianoRollModel.test.ts`; native E2E draws two notes with the pencil on the real canvas, checks 1/16 snap and pitch order, undo/redo and SMF export |
| Velocity, length, channel, CC, sustain, bend, aftertouch, program lanes | `updateNotes`, `editControllerLane` (range replace), lane canvas | `controller_lanes_replace_a_range`, CC64 timing in `sequence_built_in_the_piano_roll_plays_exactly` |
| Note expression | **Absent** — no MIDI 2.0/UMP per-note data exists in the engine | — |
| Reversible quantise, swing, humanise, transpose, legato, chord split, mute, scale highlight, multi-region | `transformNotes` (bar-relative grid, strength, swing, `quantizeOffset`), deterministic humanise, `splitChordVoices`, `batch` | `quantize_is_reversible_and_follows_metre_changes` (7/8), `triplet_and_swing_quantize`, `transforms_validate_and_apply`, `chord_voices_move_to_new_tracks` |
| Zoom/scroll/wheel/trackpad, keys, grouped undo | Native scroll; Ctrl(+Shift)+wheel zoom at the pointer; arrows/Shift, Q, M, Ctrl+A/D, Delete, 1–3; one command per gesture | `repeated_undo_redo_never_loses_or_duplicates_notes` (150 random edits incl. batches, undone/redone 3×, project equality at every step) |
| Playback/preview with the engine, no tempo drift | Notes → plan frames through the tempo map; previews via `midi_preview` (auto note-off) | `sequence_built_in_the_piano_roll_plays_exactly`: 15-step tempo ramp 90→150 BPM, 4/4→7/8→5/4, overlapping notes, block sizes 64/512/1031, exact note-on frames, every note released once; `editing_during_playback_keeps_held_notes_and_releases_removed_ones` |
| Persist edits, realistic heights/lengths at high DPI | Schema 4 notes; canvas sized by `devicePixelRatio` | `schema_v3_midi_fixture_migrates_to_editable_notes`, screenshots below |

Acceptance: sequence built only in the piano roll plays the expected events and exports an
equivalent SMF — **met** (engine and E2E; not listened to on speakers by me). Undo/redo never loses
or duplicates — **met**. 10 000 notes usable by a declared benchmark — **partially met**: the core
is measured, the in-app frame time with 10 000 notes is not.

Declared benchmark (`ten_thousand_notes_stay_editable`, this laptop): applying transpose/move/
quantise/resize to all 10 000 notes **plus** serialising the 1.07 MiB view sent to the UI takes
53–58 ms in a debug build (budget < 250 ms, asserted) and **5.9–6.4 ms in release**; adding
10 000 notes 4.1 ms; undo < 0.01 ms. Culling + rubber-band over 10 000 notes in the UI model:
< 5 ms (asserted in Vitest). Canvas draw time and IPC/JSON.parse in WebView2 with 10 000 notes
were not measured.

## Phase 09 — mixer and routing graph

| Requirement | Implementation | Evidence (`crates/elyvon-audio/tests/mixer.rs` unless noted) |
| --- | --- | --- |
| Strips, gain/pan, mute, solo, solo-safe, mono/stereo, aux buses, master, pre/post sends | `plan.rs` strip graph, `engine.rs` processing, `mixer_edit.rs` | `unity_gain_is_bit_exact_through_track_bus_and_master`, `volume_and_pre_post_fader_sends_change_the_signal`, `solo_keeps_destination_buses_and_source_paths` |
| Cycle detection, invalid routes, feedback policy | `mixer::check_routing` on every validation; policy: feedback is always rejected | `routing_cycles_are_rejected_before_they_apply`; E2E rejects a self-send |
| Peak/RMS/peak hold, headroom, clip, log fader, pan laws | `MeterSlot`/`MeterReader`, UI hold 1.5 s, clip reset, three pan laws | `meters_read_the_real_channel_and_clip_is_sticky`, `pan_laws_and_stereo_balance`; E2E reads −9.0 dBFS on track, bus and master for a 0.5 mono sine (−3 dB law) |
| Typed inserts, safe reordering with graph swaps | Trim/Utility/Filter/Limiter (`dsp.rs`); state adopted across swaps; new/re-enabled inserts fade in | `inserts_are_audible`, `graph_swaps_during_playback_ramp_instead_of_stepping` (largest step < 0.04 with fader/pan jumps and an insert added mid-play; was 0.157 before the fade), `rt_alloc::mixer_graph_instruments_and_swaps_never_allocate` |
| Aux tracks, groups, colours, folders, persistent routing, path latency compensation | Groups link volume/pan/mute/solo; folders collapse in the mixer; PDC per edge | `groups_link_volume_mute_and_solo`, `routing_survives_save_and_reopen`, `plugin_delay_is_compensated_on_parallel_paths`, `compensated_paths_stay_in_phase` |
| Master and stem export; summing, latency and phase tests | `render_stems`/`export_stems`, `project_export_stems` | `stems_sum_to_the_master_and_export` (sample-exact sum, files decode back) |

Acceptance: volume/pan/solo/sends/buses change the signal — **met** in renders, meters and E2E
(not confirmed by ear). Routing preserved on reopen and cycles detected before applying — **met**.
Meters follow the real channel — **met**; the simulated-meter path was removed.

## Commands run (all on this laptop, Windows x64)

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked` | 339 pass, 0 fail (+1 ignored hardware probe, run separately) |
| `cargo test -p elyvon-project --release --test piano_roll ten_thousand -- --nocapture` | figures above |
| `npm run check` (versions, typecheck, ESLint, Vitest 42 tests, production build) | pass |
| `npm run tauri -- build --debug --no-bundle -- --locked` | pass (`x86_64-pc-windows-msvc`, debug) |
| `node --test e2e/app.e2e.mjs` (tauri-driver + msedgedriver, real binary) | **15/15 pass** (2 new: pencil drawing, bus routing + meters) |
| Same suite against the release binary packaged in the installers (`target/x86_64-pc-windows-msvc/release`) | 15/15 pass after fixing the piano-roll note area on short panels (it was 16 px tall with a 140 px bottom panel) |
| `cargo test -p elyvon-audio --features device --test hardware_probe -- --ignored` | devices listed above |
| `cargo deny`, `npm audit`, `actionlint` | **not run** (cargo-deny/actionlint not installed here) |

Screenshots of the real Windows app: [`docs/screenshots/phase-08-09-windows/`](../screenshots/phase-08-09-windows/)
(`04-piano-roll.png`, `10-piano-roll-drawn.png`, `11-mixer-bus-meters.png`, …).

## Platform matrix

| Target | Built | Tested |
| --- | --- | --- |
| Windows x64 (`x86_64-pc-windows-msvc`) | yes, debug, locally | Rust/Vitest/E2E as above; device enumeration; no installer built (not requested) |
| Windows x86, Windows ARM64, macOS x64/ARM64, Ubuntu x64/ARM64 | no | no — no support claimed for this phase |

## Bugs, limitations and risks

- Intermittent `could not export type: Io(... code 1224 ...)` from ts-rs binding tests on Windows
  when another process (indexer/antivirus/editor) maps a freshly written `src/bindings/*.ts`;
  reruns pass. Not reproducible serially or in parallel afterwards.
- Absolute plug-in latency is reported (`mixer_status`) but the timeline is not shifted to hide
  it; only relative path alignment is compensated. Recording offset does not include it.
- The limiter's attack is a hard gain step at look-ahead entry; filters are 12 dB/oct only.
- Mixed mono/stereo files on one `auto` track now use stereo balance instead of the mono pan law.
- MIDI tracks keep an (inactive) strip; their fader/pan do not send MIDI volume/pan.
- Folders group strips in the mixer only; the arrangement does not collapse them.
- No automation, VCA, surround, third-party plug-ins, MIDI 2.0, playback note chase or step input.
- The test instrument is deliberately simple (sine + 2nd harmonic, ADSR).
- In-app performance with 10 000 notes (draw + IPC) is unmeasured; the bottom panel needs more
  height than before to show sends and inserts without scrolling.

## Next phase

Phase 10 — mix and parameter automation (fader, pan, send and insert parameters on the graph
introduced here), after on-device listening tests of this release on Windows.
