# Phases 12–13 — Native effects: EQ and dynamics; creative effects and utilities

Date: 2026-10-18. Branch `fases-12-13` (from `main` at `5da3f44`, release 0.4.1), project
schema 6, application version 0.5.0. **Overall: PARTIAL** — every software acceptance test
below passes on Windows x64 (Rust, Vitest and the native end-to-end suite against the real
binary) and the user confirmed on 2026-10-18 that the audio works when listening on his Windows
laptop. macOS ARM64 and Ubuntu x64 are built and tested in CI on request (see *Platform
matrix*); they and the remaining targets have not been tried on a device.

## Environment (the user's laptop, local only)

Windows 11 Pro 10.0.26200 x86-64; Rust 1.99.1 `x86_64-pc-windows-msvc`; Node 24.21.0;
WebView2/Edge with tauri-driver 2.1.0 and msedgedriver. No GitHub Actions, cloud runner or
remote builder was used. The work was verified in a separate git worktree because another local
agent (Codex) was editing the user interface of the main checkout at the same time; the
coordination notes are in `.local/agent-coordination.md`.

## Entry state

0.4.1 had four native inserts (trim, utility, 12 dB/oct high/low-pass, look-ahead limiter), a
mixer graph with latency compensation and sample-accurate automation of a fixed list of insert
parameters. Absent: equaliser, compressor, gate, every time-based, modulation and distortion
effect, presets, A/B, sidechains, gain-reduction meters, a spectrum analyser and tempo-aware DSP.

## What was built

| Area | Implementation |
| --- | --- |
| Effect model | `elyvon-project/src/effects.rs`: 16 `EffectKind`s, an append-only `ParamDef` table per kind (key, Spanish label, unit, range, default, linear/log/choice/toggle, automatable), `EffectSlot` (values stored **by key** on disk, A/B `alternate`, `sidechain`), versioned preset files, original factory presets, sidechain validation (no self-key, no track cycle, ≤ 16 key tracks). `Processor::Effect`, `InsertParam::Effect { kind, index }`, filter modes band-pass/notch, schema 6 |
| DSP (`elyvon-audio/src/fx/`) | `filters.rs` double-precision RBJ biquads and exact magnitude; `eq.rs` 8-band EQ (cuts 12–48 dB/oct Butterworth cascades with resonance, shelves, bells, band-pass, notch), 5 ms band crossfades; `dynamics.rs` feed-forward compressor (soft knee, peak/RMS, auto make-up, parallel mix, sidechain HPF, external key, stereo link) and gate/expander (hold, hysteresis, HPF/LPF key filters, range); `utility.rs` stereo tool (gain, balance, M/S width, Linkwitz–Riley bass mono, DC blocker, polarity and swap crossfaded) and a deterministic test generator; `delay.rs` stereo delay (free or tempo-synced per side, ping-pong, low/high cut in the loop, tape wow/flutter and saturation, Hermite reads, loop gain ≤ 0.99 with a soft limiter); `reverb.rs` original 8-line FDN (Householder matrix, RT60-derived line gains, damping, diffusion, modulation, freeze); `convolution.rs` uniformly partitioned overlap-save convolution (256-frame partitions, realfft) with five impulse responses synthesised from fixed seeds; `modulation.rs` chorus (1–4 voices), flanger, phaser (2–12 stages), tremolo/auto-pan, rotary speaker (crossover, horn/drum rotors with inertia, Doppler and AM); `distortion.rs` four curves with 1×/2×/4× linear-phase oversampling and a bitcrusher; `pitch.rs` two-tap crossfading pitch shifter |
| Engine integration | `dsp.rs` wraps effects as inserts (click-free bypass, automation, latency, `ringing`, GR); `engine.rs` passes the tempo map, sidechain key buffers, economy flag and analyser tap; `plan.rs` assigns key slots, orders tracks so key sources run first and keeps muted key sources alive; `transport.rs` lock-free GR meters, analyser slot, DSP load and economy flag; `spectrum.rs` FFT analyser on the control side |
| Determinism | Effects run 16-frame control chunks aligned to the absolute timeline frame; a chunk cut by the end of a callback resumes in the next one (`Bank::enter`), so output is **bit-identical** for any block size, also while parameters glide. Synchronised LFOs take their phase from the musical position; free LFOs re-anchor after a locate |
| IPC | `effects_catalog`, `effect_presets`, `effect_preset_save`, `effect_preset_delete`, `effect_eq_response`, `effect_details`, `analyzer_select`, `analyzer_spectrum`; `Meters.reduction`, `EngineStats.dspLoad/economy` ([CONTRACTS](../CONTRACTS.md)) |
| UI | `src/features/effects/EffectEditor.tsx`: generic controls from the catalogue (slider + typed value, choices, switches), GR meter, sidechain source, factory/user presets (save, load, delete), A/B, latency and tail. `EqWindow.tsx`: the equaliser window — power (bypass), preset menu with previous/next and "save as", Comparar (A/B), Copiar/Pegar, Deshacer/Rehacer (this equaliser's own history), gain scale; a row of band-shape buttons (toggle a band); one graph with every band's own response as a coloured area, the total curve, handles on each band's curve (drag: frequency and gain, or Q for cuts; wheel: Q), the selected band's column and the measured spectrum behind (0 … −60 dBFS scale on the left); per-band values below (frequency, gain or slope, Q, type) edited by vertical drag, arrows or typing; output gain on the right; Analizador toggle and Procesado estéreo/medio/lateral. "Acoplar Q" and "HQ" are shown disabled ("no disponible"). The mixer strip's EQ slot shows the real curve in miniature and opens (or adds) the equaliser. The layout is functionally inspired by common DAW equalisers but drawn with Elyvon's own palette, shapes and names. Mixer insert menu lists the native effects by category; effect parameters appear in the automation lanes (`src/lib/automation.ts`) |
| Demo | The generated demo session now carries effects from the factory presets: drum EQ, keys EQ + chorus + dotted-eighth delay, bass compressor keyed by the drums, vocal EQ + compressor + plate reverb |

## Phase 12 — equaliser and dynamics

| Requirement | Evidence (`crates/elyvon-audio/src/fx/*` unit tests unless noted) |
| --- | --- |
| Parametric EQ: shelves, bells, HPF/LPF; real response; smoothed changes; numerical stability | `designs_match_their_analytic_response` (measured vs analytic within 0.05 dB), `measured_sweep_matches_the_drawn_response` (8 frequencies, measured gain = drawn curve within 0.1 dB), `slopes_give_12_to_48_db_per_octave` (−3.01 dB at the cutoff), `band_switching_is_click_free`, `extreme_low_band_is_stable_and_accurate` (20 Hz, Q 18, +24 dB at 192 kHz stable; Q 4 within 0.3 dB), `low_frequency_sections_stay_accurate_and_stable_at_high_rates` |
| Compressor: threshold, ratio, attack/release, knee, make-up, sidechain | `compressor_matches_its_static_curve` (3 curves × peak/RMS × 5 levels within 0.05 dB), `attack_and_release_follow_their_time_constants` (1 − 1/e at τ within 3 %), `parallel_mix_blends_dry_and_compressed`, `external_key_ducks_the_signal_and_reports_reduction`; `tests/effects.rs::external_sidechain_ducks_the_keyed_track` (cross-track key, key track later in the project and muted: −20 dB while the key plays, recovers within 0.5 dB) |
| Gate/expander/limiter with GR meters | `gate_closes_to_its_range_and_opens_above_threshold` (−60 dB range within 1 dB), `expander_reduces_below_threshold_by_its_ratio`; limiter GR added to the existing look-ahead limiter; native E2E reads the compressor's GR meter in the editor while playing |
| Utility gain, polarity, width, mono maker, DC blocker, test generator, band-pass | `width_zero_is_mono_and_two_hundred_doubles_the_side`, `mono_maker_removes_the_bass_side_only` (50 Hz side < 0.12, 2 kHz side > 0.97), `dc_blocker_and_polarity`, `generator_levels_are_calibrated_and_reproducible` (within 0.5 dB, identical at any block size), `dsp.rs::band_pass_and_notch_filters` |
| Versioned presets, click-free bypass, A/B, undo/redo, automation, latency report | `presets_are_versioned` (newer version refused, wrong kind refused), `slots_round_trip_by_key_and_tolerate_schema_growth`, `ab_toggles_between_two_settings`; `tests/effects.rs::presets_reopen_and_reproduce_the_same_audio` (bit-identical render), `bypassed_effects_pass_the_signal_exactly_and_switch_without_clicks`, `effect_parameters_are_automatable` (EQ band gain lane 0 → +12 dB measured along the ramp; parameter of another kind rejected), `convolution_latency_is_compensated_against_other_tracks`; native E2E edits a threshold (one undoable `setInsert`), undoes an insert and reopens the saved file |
| IIR/FIR precision, saturation, denormal fallback | f64 biquad state, per-chunk denormal flushing in every recursive path; `every_effect_stays_finite_and_bounded_at_extremes` (min/default/max settings × blocks 32/64/128/333), `automation_stress_keeps_every_effect_finite` (every automatable parameter flipped between its extremes every block at 32/64/128 frames) |
| UI with real controls, spectrum measured off RT, connected meters, golden comparisons | Editor described above; `spectrum.rs` test (1 kHz sine peaks in its band at −6 dBFS within 1.5 dB); native E2E checks the analyser's peak at the track's 440 Hz tone and the GR meter; `tests/effects.rs::every_effect_matches_its_golden_measurements` (RMS, peak, zero-crossings per channel of every effect against `tests/fixtures/effects_golden.json`) |

Acceptance: each effect changes the output measurably and matches DSP reference tests —
**met** (`every_effect_changes_the_signal_measurably`, references above; neutral EQ and utility
are transparent). No NaN/infinity or instability when automating extreme parameters — **met**
(extreme and stress tests at 32/64/128 frames). Presets reopen and reproduce the same result —
**met** (bit-identical render after save/load of a preset file; project round trip).

## Phase 13 — creative effects and utilities

| Requirement | Evidence |
| --- | --- |
| Delay mono/stereo, synchronisable, slap/tape, protected feedback, internal filters | `synchronised_times_follow_the_tempo` (60/120/150 BPM), `synchronised_times_follow_tempo_map_changes` (120 → 60 BPM: quarter echo becomes 1 s), `full_feedback_stays_bounded` (100 % feedback, digital and tape: peak < 4, decays), `ping_pong_alternates_channels`; factory presets *Slapback*, *Cinta gastada*; `tests/effects.rs::tempo_synchronised_delay_at_several_sample_rates` |
| Algorithmic reverb; convolution with redistributable IRs, tail and latency documented | `tail_decays_at_the_requested_rt60_at_any_rate` (T30 within 20 % at 44.1/48/96 kHz for 0.8/2/4 s), `freeze_holds_and_never_explodes`, `predelay_delays_the_onset`; `partitioned_convolution_matches_direct_convolution` (max error < 2e-4), `reported_latency_aligns_dry_and_wet` (256 frames), `impulse_responses_are_deterministic_and_decay`, `economy_mode_shortens_the_tail`; IR origin and latency in [ADR 0009](../ADR/0009-native-effects-with-resumable-control-chunks.md) and `convolution.rs` |
| Chorus, flanger, phaser, tremolo, rotary, distortion, bitcrusher, pitch, stereo imaging | `synchronised_tremolo_lands_on_the_beat`, `synchronised_lfo_follows_tempo_map_changes`, `lfo_restarts_identically_from_any_position`, `phaser_notches_move_and_feedback_is_stable`, `rotary_accelerates_with_inertia`, `reported_latency_is_the_measured_delay` (0/15/23 frames at 1×/2×/4×), `oversampling_reduces_aliasing` (4× alias < 30 % of 1×), `bitcrusher_quantises_to_its_resolution`, `foldback_is_bounded_for_any_drive`, `shifts_by_the_requested_interval` (±12, +7 semitones, +50 cents), stereo imaging in the utility tests |
| Pedalboard, wet/dry, soft bypass, sidechain, per-effect oversampling | The insert chain (8 slots, reorderable) is the pedalboard: each effect has its own mix (balanced law for delay/reverb/chorus, linear for flanger/phaser/distortion), soft bypass (one-block crossfade, or a continuous blend when automated), sidechain on compressor/gate and fixed oversampling per distortion. Suggested order in the editor notes and presets: EQ → dynamics → distortion → modulation → delay → reverb |
| Original presets and project-generated demos; mixer and automation integration | 50 factory presets written for Elyvon (`factory_presets_use_known_keys_and_valid_values`); demo session with effects (`demo_project_is_valid`); `every_effect_in_the_graph_renders_identically_at_any_block_size` (all 16 effects on two tracks, an aux and the master) |
| CPU budgets, graceful behaviour, sample-rate variation | `every_effect_runs_far_faster_than_real_time` (figures below), economy mode (convolution reduced to its first second while callbacks overrun; never offline), `sample_rates_scale_times_and_frequencies` (44.1–192 kHz) |

Acceptance: effects process live audio and offline bounce coherently, without explosive noise
or unlimited feedback loops — **met** (bit-identical bounce at 64/333/512 frames with all 16
effects; bounded feedback tests; tails ring out after stop: `reverb_and_delay_tails_ring_out_after_stop`).
Controls reflect the real state and presets load/save reproducibly — **met** (the editor reads
the project the core returns after each edit; preset round trip). Delay and modulation times
follow BPM and tempo maps — **met** (delay and tremolo tests with a tempo change).

## Real-time safety

`tests/rt_alloc.rs::native_effects_sidechain_and_analyser_never_allocate`: all 16 effects on two
tracks, an aux and the master, a cross-track sidechain, an automated EQ band, the analyser tap,
six graph swaps changing parameters and bypass while playing, then stop with ringing tails —
**0 allocations and 0 frees** inside `Engine::process`. Effects allocate every buffer in `new`
(control thread); state adoption swaps buffers; impulse responses are built and cached on the
control thread.

## Commands run (this laptop, Windows x64, in the `fases-12-13` worktree)

| Command | Result |
| --- | --- |
| `npm ci` | pass |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked --no-fail-fast` | **486 passed, 0 failed** (402 at 0.4.1) |
| `npm run typecheck`, `npm run lint` | pass |
| `npm test` (Vitest) | **56/56 pass** (6 new in `src/lib/effects.test.ts`) |
| `npm run tauri -- build --debug --no-bundle` | pass (`x86_64-pc-windows-msvc`, debug) |
| `node --test e2e/app.e2e.mjs` (tauri-driver + msedgedriver, real binary) | **18/18 pass, twice in a row** (1 new test; 2 existing tests updated for schema 6) |
| `cargo test --release -p elyvon-audio --test effects every_effect_runs -- --nocapture` | figures below |

The golden fixture `crates/elyvon-audio/tests/fixtures/effects_golden.json` was generated once
with `ELYVON_UPDATE_GOLDEN=1` on this laptop and is checked on every run.

## Measured CPU cost (release build, 48 kHz, 128-frame blocks, one core)

| Effect | Fraction of real time | ns per stereo frame |
| --- | ---: | ---: |
| Parametric EQ (default bands) | 0.0018 | 37 |
| Compressor | 0.0040 | 83 |
| Gate / expander | 0.0068 | 142 |
| Stereo utility | 0.0004 | 9 |
| Test generator (pink) | 0.0009 | 18 |
| Delay | 0.0066 | 137 |
| Algorithmic reverb | 0.0091 | 190 |
| Convolution reverb, **6 s IR** | 0.0890 | 1 853 |
| Chorus (2 voices) | 0.0076 | 157 |
| Flanger | 0.0030 | 63 |
| Phaser (4 stages) | 0.0012 | 25 |
| Tremolo | 0.0013 | 26 |
| Rotary | 0.0077 | 160 |
| Distortion (2× oversampling) | 0.0101 | 210 |
| Bitcrusher | 0.0003 | 6 |
| Pitch shifter | 0.0036 | 74 |

One core of this laptop could run roughly 100 instances of most effects, or 11 six-second
convolution reverbs, in real time; the economy path bounds the convolution's cost to its
first second while callbacks overrun. These are single measurements, not a benchmark suite.

## Evidence

Screenshots captured from the real application by the native end-to-end suite:
[14 — compressor editor with live gain reduction](../screenshots/phase-12-13-windows/14-effect-compressor.png)
(−40 dB threshold typed in the editor; the meter shows the engine's reduction while playing),
[15 — equaliser window](../screenshots/phase-12-13-windows/15-effect-eq.png) (high-pass at 40 Hz,
low shelf −6 dB, bell +12 dB at 440 Hz, bell −7.5 dB at 2.5 kHz, high shelf +6 dB: every
coloured area and the white total curve come from the core's biquad sections; the light area
behind is the track's spectrum measured off the audio thread, peaking at the 440 Hz tone),
[16 — mixer strip](../screenshots/phase-12-13-windows/16-strip-eq-thumbnail.png) (the EQ slot
draws the strip's real curve; the insert areas of all strips grow with the fullest one and the faders give up that height, so strips keep fitting and stay aligned).

## Platform matrix

| Target | Built | Tested |
| --- | --- | --- |
| Windows x64 (`x86_64-pc-windows-msvc`) | Yes, debug, on this laptop | Rust, Vitest, native E2E (WebView2) |
| Windows x86, Windows ARM64, macOS x64/ARM64, Ubuntu x64/ARM64 | No | No — no support is claimed |

The DSP is portable Rust without SIMD intrinsics or platform APIs; floating-point results may
differ in the last bits between targets (the golden test uses tolerances for that reason).

## Limitations and pending work

- Listening: the user confirmed that the audio works on his Windows laptop (2026-10-18). There
  is no recorded per-effect listening protocol; character judgements (reverb density, pitch
  shifter artefacts, distortion) remain subjective and unrecorded.
- The pitch shifter is a simple two-tap time-domain design: it modulates the output at
  |ratio − 1| / window (sidebands), moves formants and may double transients.
- No user impulse-response files; the five IRs are synthesised. No third-party plug-ins.
- The pedalboard is the insert chain (8 slots); there is no nested effect container.
- Economy mode only reduces the convolution tail; other effects have fixed cost.
- Key signals are not latency-aligned with their destination when the source path has more
  plug-in latency than the destination.
- Platforms other than Windows x64 were not built.
- The native end-to-end suite runs the app with the same identifier as an installed copy, so it
  shares its preferences; it now forces a known layout at start and restores the original
  preferences at the end.
- The main checkout also contains another agent's uncommitted UI work (a reference "Channel
  EQ" window drawing fixed curves, a project chooser shown at start-up). It is not part of this
  branch; with it applied the native E2E suite cannot start (a modal dialog covers the control
  bar).

## Next phase

Phase 14 — professional sampler and sample libraries. Its processors can reuse this phase's
infrastructure (parameter tables, smoothing, presets, automation, resumable control chunks).
