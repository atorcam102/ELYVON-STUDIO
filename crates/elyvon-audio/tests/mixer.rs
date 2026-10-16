//! Phase 09: the mixer is the real signal path. Volume, pan, sends, buses,
//! solo, mute, groups and inserts change rendered samples; routing cycles are
//! rejected before they apply; meters read the real channel.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::engine;
use elyvon_audio::plan::build_plan;
use elyvon_audio::render::{RenderRequest, Rendered, render, render_stems};
use elyvon_audio::transport::{MeterReader, TransportCommand};
use elyvon_project::mixer::{
    AuxId, ChannelFormat, ChannelRef, FilterMode, OutputTarget, PanLaw, Processor, Send,
};
use elyvon_project::mixer_edit::StripPatch;
use elyvon_project::{EditCommand, TrackId};

const RATE: u32 = 48_000;
const FRAMES: usize = 9_600;

fn constant(l: f32, r: f32) -> std::sync::Arc<elyvon_audio::media::MediaBuffer> {
    buffer(vec![vec![l; FRAMES], vec![r; FRAMES]], RATE, RATE)
}

fn track_with(s: &mut Session, l: f32, r: f32) -> TrackId {
    let t = s.audio_track();
    s.place(t, 0, &constant(l, r), "c");
    t
}

fn out(s: &Session) -> Rendered {
    render(
        &s.project,
        &s.lookup(),
        RenderRequest::whole_song(RATE),
        None,
    )
}

/// Left/right of a settled frame in the middle of the render.
fn mid(r: &Rendered) -> (f32, f32) {
    let i = 4000 * 2;
    (r.samples[i], r.samples[i + 1])
}

fn aux(s: &mut Session, name: &str) -> AuxId {
    s.apply(EditCommand::AddAux {
        name: Some(name.into()),
        sources: vec![],
    })
    .created_auxes[0]
}

fn route(s: &mut Session, channel: ChannelRef, to: Option<AuxId>) {
    s.apply(EditCommand::SetStrip {
        channel,
        patch: StripPatch {
            output: Some(to.map_or(OutputTarget::Master, |id| OutputTarget::Aux { id })),
            ..StripPatch::default()
        },
    });
}

fn send(s: &mut Session, channel: ChannelRef, target: AuxId, level_db: f64, pre: bool) {
    s.apply(EditCommand::SetSend {
        channel,
        target,
        send: Some(Send {
            target,
            level_db,
            pre_fader: pre,
            muted: false,
        }),
    });
}

fn tr(id: TrackId) -> ChannelRef {
    ChannelRef::Track { id }
}

#[test]
fn unity_gain_is_bit_exact_through_track_bus_and_master() {
    let mut s = Session::new(120.0);
    let t = track_with(&mut s, 0.5, -0.25);
    let direct = out(&s);
    let bus = aux(&mut s, "Bus");
    route(&mut s, tr(t), Some(bus));
    let via_bus = out(&s);
    for r in [&direct, &via_bus] {
        assert!(
            r.samples[..9000 * 2]
                .chunks(2)
                .all(|f| f[0] == 0.5 && f[1] == -0.25),
            "A → bus → master at 0 dB is the identity"
        );
    }
}

#[test]
fn pan_laws_and_stereo_balance() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.place(t, 0, &buffer(vec![vec![0.5; FRAMES]], RATE, RATE), "mono");
    let h = std::f32::consts::FRAC_1_SQRT_2 * 0.5;
    for (law, centre) in [
        (PanLaw::ConstantPower, h),
        (PanLaw::ConstantPowerCompensated, 0.5),
        (PanLaw::Linear, 0.25),
    ] {
        s.apply(EditCommand::SetPanLaw { law });
        let (l, r) = mid(&out(&s));
        assert!(
            (l - centre).abs() < 1e-6 && (r - centre).abs() < 1e-6,
            "{law:?}"
        );
    }
    s.apply(EditCommand::SetPanLaw {
        law: PanLaw::ConstantPower,
    });
    s.apply(EditCommand::SetTrackPan { id: t, pan: -1.0 });
    let (l, r) = mid(&out(&s));
    assert!((l - 0.5).abs() < 1e-6 && r.abs() < 1e-6);
    // Forcing stereo uses balance: hard left keeps the left side at unity.
    s.apply(EditCommand::SetStrip {
        channel: tr(t),
        patch: StripPatch {
            format: Some(ChannelFormat::Stereo),
            pan: Some(0.5),
            ..StripPatch::default()
        },
    });
    let (l, r) = mid(&out(&s));
    assert!((l - 0.25).abs() < 1e-6 && (r - 0.5).abs() < 1e-6, "{l} {r}");
}

#[test]
fn volume_and_pre_post_fader_sends_change_the_signal() {
    let mut s = Session::new(120.0);
    let t = track_with(&mut s, 0.4, 0.4);
    s.apply(EditCommand::SetTrackVolume {
        id: t,
        volume_db: -6.0,
    });
    let g = 10f32.powf(-6.0 / 20.0);
    assert!((mid(&out(&s)).0 - 0.4 * g).abs() < 1e-6);
    let fx = aux(&mut s, "FX");
    s.apply(EditCommand::SetTrackVolume {
        id: t,
        volume_db: -96.0,
    });
    send(&mut s, tr(t), fx, 0.0, true);
    assert!(
        (mid(&out(&s)).0 - 0.4).abs() < 1e-6,
        "pre-fader send ignores the fader"
    );
    send(&mut s, tr(t), fx, 0.0, false);
    assert_eq!(mid(&out(&s)).0, 0.0, "post-fader send follows the fader");
    s.apply(EditCommand::SetTrackVolume {
        id: t,
        volume_db: 0.0,
    });
    send(&mut s, tr(t), fx, -6.0, false);
    assert!((mid(&out(&s)).0 - (0.4 + 0.4 * g)).abs() < 1e-6);
    // The aux fader scales only what reaches it.
    s.apply(EditCommand::SetStrip {
        channel: ChannelRef::Aux { id: fx },
        patch: StripPatch {
            volume_db: Some(-96.0),
            ..StripPatch::default()
        },
    });
    assert!((mid(&out(&s)).0 - 0.4).abs() < 1e-6);
}

/// A → X → master, C → master with a post-fader send to X, B → master.
fn solo_session() -> (Session, [TrackId; 3], AuxId) {
    let mut s = Session::new(120.0);
    let a = track_with(&mut s, 0.1, 0.1);
    let b = track_with(&mut s, 0.2, 0.2);
    let c = track_with(&mut s, 0.4, 0.4);
    let x = aux(&mut s, "X");
    route(&mut s, tr(a), Some(x));
    send(&mut s, tr(c), x, 0.0, false);
    (s, [a, b, c], x)
}

#[test]
fn solo_keeps_destination_buses_and_source_paths() {
    let (mut s, [a, b, c], x) = solo_session();
    let level = |s: &Session| mid(&out(s)).0;
    assert!((level(&s) - 1.1).abs() < 1e-6);
    s.apply(EditCommand::SetTracksSoloed {
        ids: vec![a],
        soloed: true,
    });
    assert!((level(&s) - 0.1).abs() < 1e-6, "soloing A keeps its bus X");
    s.apply(EditCommand::SetTracksSoloed {
        ids: vec![a],
        soloed: false,
    });
    s.apply(EditCommand::SetStrip {
        channel: ChannelRef::Aux { id: x },
        patch: StripPatch {
            soloed: Some(true),
            ..StripPatch::default()
        },
    });
    assert!(
        (level(&s) - 0.5).abs() < 1e-6,
        "soloing X plays what feeds it (A and C's send) but not C's direct output"
    );
    s.apply(EditCommand::SetStrip {
        channel: tr(b),
        patch: StripPatch {
            solo_safe: Some(true),
            ..StripPatch::default()
        },
    });
    assert!((level(&s) - 0.7).abs() < 1e-6, "solo-safe B stays audible");
    // Mute wins and silences sends too.
    s.apply(EditCommand::SetStrip {
        channel: ChannelRef::Aux { id: x },
        patch: StripPatch {
            soloed: Some(false),
            ..StripPatch::default()
        },
    });
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![c],
        muted: true,
    });
    assert!((level(&s) - 0.3).abs() < 1e-6);
}

#[test]
fn groups_link_volume_mute_and_solo() {
    let mut s = Session::new(120.0);
    let a = track_with(&mut s, 0.1, 0.1);
    let b = track_with(&mut s, 0.2, 0.2);
    s.apply(EditCommand::SetTrackVolume {
        id: b,
        volume_db: 3.0,
    });
    let group = s
        .apply(EditCommand::AddGroup {
            name: None,
            members: vec![tr(a), tr(b)],
        })
        .created_groups[0];
    s.apply(EditCommand::SetTrackVolume {
        id: a,
        volume_db: -6.0,
    });
    // B moved by the same −6 dB from +3 dB.
    assert_eq!(s.project.tracks[1].volume_db, -3.0);
    let ga = 10f32.powf(-6.0 / 20.0);
    let gb = 10f32.powf(-3.0 / 20.0);
    assert!((mid(&out(&s)).0 - (0.1 * ga + 0.2 * gb)).abs() < 1e-6);
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![a],
        muted: true,
    });
    assert_eq!(mid(&out(&s)).0, 0.0, "mute is linked");
    let mut g = s.project.mixer.groups[0].clone();
    g.link_mute = false;
    s.apply(EditCommand::UpdateGroup { group: g });
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![a],
        muted: false,
    });
    assert!(s.project.tracks[1].muted, "unlinked: B keeps its own mute");
    s.apply(EditCommand::RemoveGroup { id: group });
    assert!(s.project.tracks.iter().all(|t| t.strip.group.is_none()));
}

#[test]
fn routing_cycles_are_rejected_before_they_apply() {
    let mut s = Session::new(120.0);
    let x = aux(&mut s, "X");
    let y = aux(&mut s, "Y");
    route(&mut s, ChannelRef::Aux { id: x }, Some(y));
    let before = s.project.clone();
    let cyc = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SetStrip {
            channel: ChannelRef::Aux { id: y },
            patch: StripPatch {
                output: Some(OutputTarget::Aux { id: x }),
                ..StripPatch::default()
            },
        },
    );
    assert!(cyc.unwrap_err().to_string().contains("cycle"));
    let via_send = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SetSend {
            channel: ChannelRef::Aux { id: y },
            target: x,
            send: Some(Send {
                target: x,
                level_db: 0.0,
                pre_fader: false,
                muted: false,
            }),
        },
    );
    assert!(via_send.is_err());
    let own = elyvon_project::edit::apply(
        &mut s.project,
        &EditCommand::SetSend {
            channel: ChannelRef::Aux { id: x },
            target: x,
            send: Some(Send {
                target: x,
                level_db: 0.0,
                pre_fader: false,
                muted: false,
            }),
        },
    );
    assert!(own.is_err());
    assert_eq!(
        s.project, before,
        "rejected edits leave the project unchanged"
    );
    assert!(!elyvon_project::mixer::can_route(
        &s.project,
        ChannelRef::Aux { id: y },
        x
    ));
    assert!(elyvon_project::mixer::can_route(
        &s.project,
        ChannelRef::Aux { id: x },
        y
    ));
}

#[test]
fn routing_survives_save_and_reopen() {
    let (mut s, [a, ..], x) = solo_session();
    s.apply(EditCommand::AddInsert {
        channel: tr(a),
        processor: Processor::Trim { gain_db: -3.0 },
        index: None,
    });
    s.apply(EditCommand::AddFolder {
        name: Some("Drums".into()),
        tracks: vec![a],
    });
    s.apply(EditCommand::AddGroup {
        name: None,
        members: vec![ChannelRef::Aux { id: x }],
    });
    let bytes = elyvon_project::format::encode(&s.project, "test");
    let reopened = elyvon_project::format::decode(&bytes).unwrap();
    assert_eq!(reopened, s.project);
    let before = out(&s);
    s.project = reopened;
    assert_eq!(out(&s).report.checksum, before.report.checksum);
    // Removing the bus re-routes its sources to the master; nothing dangles.
    s.apply(EditCommand::RemoveAuxes { ids: vec![x] });
    assert_eq!(s.project.tracks[0].strip.output, OutputTarget::Master);
    assert!(s.project.tracks[2].strip.sends.is_empty());
}

#[test]
fn meters_read_the_real_channel_and_clip_is_sticky() {
    let (mut s, [_, b, c], _) = solo_session();
    s.apply(EditCommand::SetTracksMuted {
        ids: vec![c],
        muted: true,
    });
    s.apply(EditCommand::SetTrackVolume {
        id: b,
        volume_db: 6.0,
    });
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 256, build_plan(&s.project, &lookup, RATE, 1));
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut buf = vec![0.0f32; 512];
    let mut midi = engine::NoMidi;
    for _ in 0..20 {
        eng.process(&mut buf, 2, &mut midi);
    }
    let mut reader = MeterReader::default();
    let m = handle.shared().read_meters(&mut reader, 3, 1);
    let g = 10f32.powf(6.0 / 20.0);
    assert!((m.tracks[0].peak[0] - 0.1).abs() < 1e-6);
    assert!((m.tracks[1].peak[0] - 0.2 * g).abs() < 1e-5);
    assert_eq!(m.tracks[2].peak, [0.0, 0.0], "muted channel meters silence");
    assert!((m.auxes[0].peak[0] - 0.1).abs() < 1e-6, "bus X carries A");
    assert!(m.tracks[1].rms[0] > 0.0 && m.tracks[1].rms[0] < m.tracks[1].peak[0]);
    assert!(!m.master.clip);
    // +18 dB on B: the channel and the master clip; the device output is
    // still limited to full scale.
    let mut hot = s.project.clone();
    elyvon_project::edit::apply(
        &mut hot,
        &EditCommand::SetStrip {
            channel: tr(b),
            patch: StripPatch {
                volume_db: Some(6.0),
                ..StripPatch::default()
            },
        },
    )
    .unwrap();
    elyvon_project::edit::apply(
        &mut hot,
        &EditCommand::AddInsert {
            channel: tr(b),
            processor: Processor::Trim { gain_db: 12.0 },
            index: None,
        },
    )
    .unwrap();
    handle.publish(build_plan(&hot, &lookup, RATE, 2));
    for _ in 0..20 {
        eng.process(&mut buf, 2, &mut midi);
    }
    assert!(buf.iter().all(|v| v.abs() <= 1.0));
    let m = handle.shared().read_meters(&mut reader, 3, 1);
    assert!(m.tracks[1].clip && m.master.clip && !m.tracks[0].clip);
    let m = handle.shared().read_meters(&mut reader, 3, 1);
    assert!(m.master.clip, "clip indicators hold until reset");
    handle.shared().reset_clips();
    handle.publish(build_plan(&s.project, &lookup, RATE, 3));
    for _ in 0..20 {
        eng.process(&mut buf, 2, &mut midi);
    }
    assert!(!handle.shared().read_meters(&mut reader, 3, 1).master.clip);
}

#[test]
fn graph_swaps_during_playback_ramp_instead_of_stepping() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.place(
        t,
        0,
        &buffer(vec![sine(440.0, 0.5, RATE, 96_000)], RATE, RATE),
        "sine",
    );
    let lookup = s.lookup();
    let (mut handle, mut eng) = engine::create(RATE, 256, build_plan(&s.project, &lookup, RATE, 1));
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut midi = engine::NoMidi;
    let mut all = Vec::new();
    let mut buf = vec![0.0f32; 512];
    let mut edited = s.project.clone();
    for round in 0..12 {
        eng.process(&mut buf, 2, &mut midi);
        all.extend_from_slice(&buf);
        // Alternate fader jumps, pan jumps and an insert during playback.
        let cmd = match round % 3 {
            0 => EditCommand::SetTrackVolume {
                id: t,
                volume_db: if round % 2 == 0 { -24.0 } else { 0.0 },
            },
            1 => EditCommand::SetTrackPan {
                id: t,
                pan: if round % 2 == 0 { -1.0 } else { 1.0 },
            },
            _ => EditCommand::AddInsert {
                channel: tr(t),
                processor: Processor::Filter {
                    mode: FilterMode::LowPass,
                    cutoff_hz: 15_000.0,
                    q: 0.7,
                },
                index: None,
            },
        };
        elyvon_project::edit::apply(&mut edited, &cmd).unwrap();
        handle.publish(build_plan(&edited, &lookup, RATE, 2 + round as u64));
        handle.poll();
    }
    // A full-scale-step would jump by up to 0.5; the sine itself moves at most
    // 2π·440/48000·0.5 ≈ 0.029 per sample.
    let worst = all
        .chunks(2)
        .zip(all.chunks(2).skip(1))
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0f32, f32::max);
    assert!(worst < 0.04, "largest sample step {worst}");
    assert!(all.iter().all(|v| v.is_finite()));
}

#[test]
fn plugin_delay_is_compensated_on_parallel_paths() {
    let mut s = Session::new(120.0);
    let mut impulse = vec![0.0f32; FRAMES];
    impulse[1000] = 0.25;
    let click = buffer(vec![impulse.clone(), impulse], RATE, RATE);
    let a = s.audio_track();
    s.place(a, 0, &click, "a");
    let b = s.audio_track();
    s.place(b, 0, &click, "b");
    s.apply(EditCommand::AddInsert {
        channel: tr(a),
        processor: Processor::Limiter {
            ceiling_db: 0.0,
            lookahead_ms: 5.0,
            release_ms: 50.0,
        },
        index: None,
    });
    let check = |s: &Session| {
        let r = out(s);
        let plan = build_plan(&s.project, &s.lookup(), RATE, 1);
        assert_eq!(plan.stats.latency_frames, 240);
        let left = r.channel(0);
        let hits: Vec<usize> = (0..left.len()).filter(|&i| left[i] != 0.0).collect();
        assert_eq!(
            hits,
            [1240],
            "both paths align after the limiter's look-ahead"
        );
        assert!((left[1240] - 0.5).abs() < 1e-6);
    };
    check(&s);
    // Through a bus: A → X (with the limiter on X), B direct.
    s.apply(EditCommand::RemoveInsert {
        channel: tr(a),
        id: s.project.tracks[0].strip.inserts[0].id,
    });
    let x = aux(&mut s, "X");
    route(&mut s, tr(a), Some(x));
    s.apply(EditCommand::AddInsert {
        channel: ChannelRef::Aux { id: x },
        processor: Processor::Limiter {
            ceiling_db: 0.0,
            lookahead_ms: 5.0,
            release_ms: 50.0,
        },
        index: None,
    });
    check(&s);
}

#[test]
fn compensated_paths_stay_in_phase() {
    let mut s = Session::new(120.0);
    let tone = buffer(vec![sine(1000.0, 0.25, RATE, 48_000)], RATE, RATE);
    let a = s.audio_track();
    s.place(a, 0, &tone, "a");
    let b = s.audio_track();
    s.place(b, 0, &tone, "b");
    let single = rms(&out(&s).channel(0)[10_000..40_000]) / 2.0;
    s.apply(EditCommand::AddInsert {
        channel: tr(a),
        processor: Processor::Limiter {
            ceiling_db: 0.0,
            lookahead_ms: 3.3,
            release_ms: 50.0,
        },
        index: None,
    });
    let both = rms(&out(&s).channel(0)[10_000..40_000]);
    assert!(
        (both / single - 2.0).abs() < 0.01,
        "coherent sum (no comb filtering): {}",
        both / single
    );
}

#[test]
fn inserts_are_audible() {
    let mut s = Session::new(120.0);
    let t = s.audio_track();
    s.place(
        t,
        0,
        &buffer(vec![sine(100.0, 0.5, RATE, 48_000)], RATE, RATE),
        "low",
    );
    let dry = rms(&out(&s).channel(0)[9_600..40_000]);
    s.apply(EditCommand::AddInsert {
        channel: tr(t),
        processor: Processor::Trim {
            gain_db: -6.020_599_913,
        },
        index: None,
    });
    let trimmed = rms(&out(&s).channel(0)[9_600..40_000]);
    assert!((trimmed / dry - 0.5).abs() < 1e-4);
    let id = s
        .apply(EditCommand::AddInsert {
            channel: tr(t),
            processor: Processor::Filter {
                mode: FilterMode::HighPass,
                cutoff_hz: 2000.0,
                q: 0.7,
            },
            index: Some(0),
        })
        .created_inserts[0];
    let filtered = rms(&out(&s).channel(0)[9_600..40_000]);
    assert!(filtered < trimmed * 0.01, "{filtered}");
    let processor = s.project.tracks[0].strip.inserts[0].processor;
    s.apply(EditCommand::SetInsert {
        channel: tr(t),
        id,
        bypass: true,
        processor,
    });
    assert!((rms(&out(&s).channel(0)[9_600..40_000]) - trimmed).abs() < 1e-6);
}

#[test]
fn stems_sum_to_the_master_and_export() {
    let (mut s, [a, ..], _) = solo_session();
    s.apply(EditCommand::SetTrackPan { id: a, pan: 0.3 });
    let channels = elyvon_audio::render::stem_channels(&s.project);
    let mut stems: std::collections::HashMap<usize, Vec<f32>> = Default::default();
    let report = render_stems(
        &s.project,
        &s.lookup(),
        RenderRequest::whole_song(RATE),
        &mut |slot, block| stems.entry(slot).or_default().extend_from_slice(block),
    );
    let master = &stems[&elyvon_audio::plan::MASTER_SLOT];
    assert_eq!(master.len() as u64, report.frames * 2);
    // Master = tracks routed to master (B, C) + bus X (A and C's send).
    let x = elyvon_audio::plan::aux_slot(0);
    for i in 0..master.len() {
        let sum = stems[&1][i] + stems[&2][i] + stems[&x][i];
        assert!((master[i] - sum).abs() < 1e-6, "sample {i}");
    }
    assert_eq!(channels.len(), 5);
    let dir = tempfile::tempdir().unwrap();
    let exported = elyvon_audio::render::export_stems(
        dir.path(),
        &s.project,
        &s.lookup(),
        RenderRequest::whole_song(RATE),
    )
    .unwrap();
    assert_eq!(exported.stems.len(), 5);
    for stem in &exported.stems {
        let decoded =
            elyvon_audio::media::decode_file(std::path::Path::new(&stem.path), u64::MAX).unwrap();
        assert_eq!(decoded.info.frames, report.frames);
    }
    assert!(dir.path().join("stems.json").exists());
}
