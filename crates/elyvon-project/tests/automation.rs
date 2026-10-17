//! Phase 10 project-level behaviour: lanes, modes, writing during playback,
//! persistence, undo and validation.
#![allow(clippy::unwrap_used)]

use elyvon_project::automation::{
    AutomationMode, AutomationParam, AutomationTarget, Curve, InsertParam, PointSpec,
};
use elyvon_project::automation_record::{AutomationRecorder, TouchPhase};
use elyvon_project::mixer::{ChannelRef, Processor};
use elyvon_project::{EditCommand, Project, ProjectSession, RegionContent, TrackId, TrackKind, format};

fn session_with_track() -> (ProjectSession, TrackId) {
    let mut s = ProjectSession::new_empty();
    let t = s
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    (s, t)
}

fn vol(t: TrackId) -> AutomationTarget {
    AutomationTarget {
        channel: ChannelRef::Track { id: t },
        region: None,
        param: AutomationParam::Volume,
    }
}

fn add(s: &mut ProjectSession, target: AutomationTarget, pts: &[(u64, f64)]) -> Vec<u32> {
    s.apply(&EditCommand::AddAutomationPoints {
        target,
        points: pts
            .iter()
            .map(|&(tick, value)| PointSpec {
                tick,
                value,
                curve: None,
            })
            .collect(),
    })
    .unwrap()
    .created_points
}

fn mode(s: &mut ProjectSession, t: TrackId, mode: AutomationMode) {
    s.apply(&EditCommand::SetAutomationMode {
        channel: ChannelRef::Track { id: t },
        mode,
    })
    .unwrap();
}

fn lane_value(p: &Project, t: AutomationTarget, tick: u64) -> f64 {
    elyvon_project::automation::lane(p, &t)
        .unwrap()
        .value_at(tick as f64)
        .unwrap()
}

#[test]
fn points_are_edited_by_id_and_undo_restores_each_step() {
    let (mut s, t) = session_with_track();
    let ids = add(&mut s, vol(t), &[(0, -12.0), (960, 0.0), (1920, -6.0)]);
    assert_eq!(ids, [1, 2, 3]);
    let before = s.project().clone();
    s.apply(&EditCommand::MoveAutomationPoints {
        target: vol(t),
        ids: vec![2],
        delta_ticks: 480,
        delta_value: -0.5,
        copy: false,
    })
    .unwrap();
    let lane = elyvon_project::automation::lane(s.project(), &vol(t)).unwrap();
    assert_eq!(lane.points[1].tick, 1440);
    assert!((lane.points[1].value - (-51.0)).abs() < 1e-9, "half the range down");
    let copies = s
        .apply(&EditCommand::MoveAutomationPoints {
            target: vol(t),
            ids: vec![1, 3],
            delta_ticks: 3840,
            delta_value: 0.0,
            copy: true,
        })
        .unwrap()
        .created_points;
    assert_eq!(copies.len(), 2);
    s.apply(&EditCommand::UpdateAutomationPoints {
        target: vol(t),
        points: vec![elyvon_project::automation::AutomationPoint {
            id: 1,
            tick: 0,
            value: 3.0,
            curve: Curve::Bezier { c1: 0.0, c2: 0.0 },
        }],
    })
    .unwrap();
    s.apply(&EditCommand::RemoveAutomationPoints {
        target: vol(t),
        ids: vec![3],
    })
    .unwrap();
    for _ in 0..4 {
        assert!(s.undo());
    }
    assert_eq!(s.project(), &before);
    for _ in 0..4 {
        assert!(s.redo());
    }
    let lane = elyvon_project::automation::lane(s.project(), &vol(t)).unwrap();
    assert_eq!(lane.points.len(), 4);
    // Values are clamped to the parameter range; switches round to 0/1.
    let mute = AutomationTarget {
        param: AutomationParam::Mute,
        ..vol(t)
    };
    add(&mut s, mute, &[(0, 0.7)]);
    let m = elyvon_project::automation::lane(s.project(), &mute).unwrap();
    assert_eq!((m.points[0].value, m.points[0].curve), (1.0, Curve::Step));
}

#[test]
fn lanes_of_removed_inserts_and_sends_are_pruned_in_the_same_undo_step() {
    let (mut s, t) = session_with_track();
    let insert = s
        .apply(&EditCommand::AddInsert {
            channel: ChannelRef::Track { id: t },
            processor: Processor::Trim { gain_db: 0.0 },
            index: None,
        })
        .unwrap()
        .created_inserts[0];
    let trim = AutomationTarget {
        param: AutomationParam::Insert {
            id: insert,
            param: InsertParam::TrimGain,
        },
        ..vol(t)
    };
    add(&mut s, trim, &[(0, -6.0)]);
    // A parameter that does not exist on the processor is refused.
    assert!(
        s.apply(&EditCommand::AddAutomationPoints {
            target: AutomationTarget {
                param: AutomationParam::Insert {
                    id: insert,
                    param: InsertParam::FilterCutoff,
                },
                ..vol(t)
            },
            points: vec![PointSpec {
                tick: 0,
                value: 100.0,
                curve: None,
            }],
        })
        .is_err()
    );
    s.apply(&EditCommand::RemoveInsert {
        channel: ChannelRef::Track { id: t },
        id: insert,
    })
    .unwrap();
    assert!(s.project().tracks[0].automation.lanes.is_empty());
    assert!(s.undo());
    assert_eq!(s.project().tracks[0].automation.lanes.len(), 1);
}

#[test]
fn touch_writes_while_held_and_returns_to_the_lane_after_release() {
    let (mut s, t) = session_with_track();
    add(&mut s, vol(t), &[(0, -10.0)]);
    mode(&mut s, t, AutomationMode::Touch);
    let mut rec = AutomationRecorder::start(s.project(), 0, 0);
    for tick in (0..960).step_by(120) {
        rec.observe(tick, 0);
    }
    assert!(rec.touch(vol(t), AutomationMode::Touch, TouchPhase::Begin, -3.0, 960, 0));
    for (i, tick) in (1000..2880).step_by(60).enumerate() {
        rec.observe(tick, 0);
        rec.touch(vol(t), AutomationMode::Touch, TouchPhase::Move, -3.0 + i as f64 * 0.1, tick, 0);
    }
    rec.touch(vol(t), AutomationMode::Touch, TouchPhase::End, 0.0, 2880, 0);
    for tick in (2880..7680).step_by(120) {
        rec.observe(tick, 0);
    }
    let cmd = rec.finish(7680, 0, s.project()).unwrap();
    assert_eq!(cmd.label(), "Write Automation");
    s.apply(&cmd).unwrap();
    let p = s.project();
    assert!((lane_value(p, vol(t), 480) - (-10.0)).abs() < 1e-9, "before the touch: lane");
    assert!((lane_value(p, vol(t), 960) - (-3.0)).abs() < 1e-9, "touch starts at its own value");
    assert!(lane_value(p, vol(t), 2000) > -3.0 && lane_value(p, vol(t), 2000) < 0.0);
    assert!((lane_value(p, vol(t), 2880) - 0.0).abs() < 1e-9, "released value");
    // 200 ms at 120 BPM = 384 ticks to return to the old value.
    assert!((lane_value(p, vol(t), 2880 + 400) - (-10.0)).abs() < 1e-9, "back on the lane");
    assert!((lane_value(p, vol(t), 6000) - (-10.0)).abs() < 1e-9);
    // Linear runs are thinned.
    let n = elyvon_project::automation::lane(p, &vol(t)).unwrap().points.len();
    assert!(n < 12, "{n} points");
    assert!(s.undo());
    assert_eq!(elyvon_project::automation::lane(s.project(), &vol(t)).unwrap().points.len(), 1);
}

#[test]
fn read_and_off_modes_and_stopped_transport_never_write() {
    let (mut s, t) = session_with_track();
    for m in [AutomationMode::Off, AutomationMode::Read] {
        mode(&mut s, t, m);
        let mut rec = AutomationRecorder::start(s.project(), 0, 0);
        assert!(!rec.touch(vol(t), m, TouchPhase::Begin, -3.0, 100, 0));
        assert!(rec.is_empty());
        assert!(rec.finish(3840, 0, s.project()).is_none());
    }
    // Region lanes are never written by gestures.
    let region = AutomationTarget {
        region: Some(elyvon_project::RegionId::new()),
        ..vol(t)
    };
    let mut rec = AutomationRecorder::start(s.project(), 0, 0);
    assert!(!rec.touch(region, AutomationMode::Latch, TouchPhase::Begin, -3.0, 100, 0));
}

#[test]
fn write_mode_does_not_touch_unarmed_channels_or_region_lanes() {
    let mut s = ProjectSession::new_empty();
    let mut tracks = Vec::new();
    for _ in 0..2 {
        tracks.push(
            s.apply(&EditCommand::AddTrack {
                kind: TrackKind::Audio,
                name: None,
                index: None,
            })
            .unwrap()
            .created_tracks[0],
        );
    }
    let (a, b) = (tracks[0], tracks[1]);
    // A region with its own lane on the written track.
    let media = elyvon_project::MediaAsset {
        id: elyvon_project::MediaId::new(),
        name: "x.wav".into(),
        path: "x.wav".into(),
        info: Some(elyvon_project::MediaInfo {
            sample_rate: 48_000,
            channels: 1,
            frames: 480_000,
            format: "wav".into(),
            size_bytes: 1,
            fingerprint: "1".into(),
        }),
    };
    let region = s
        .apply(&EditCommand::ImportAudio {
            assets: vec![media.clone()],
            placements: vec![elyvon_project::AudioPlacement {
                media_id: media.id,
                track: elyvon_project::PlacementTrack::Existing { id: a },
                start: 0,
            }],
        })
        .unwrap()
        .created_regions[0];
    let region_lane = AutomationTarget {
        region: Some(region),
        ..vol(a)
    };
    add(&mut s, region_lane, &[(0, -20.0), (960, -5.0)]);
    add(&mut s, vol(a), &[(0, -12.0), (7680, -12.0)]);
    add(&mut s, vol(b), &[(0, -4.0), (3840, -8.0)]);
    s.apply(&EditCommand::SetTrackVolume { id: a, volume_db: -1.5 }).unwrap();
    mode(&mut s, a, AutomationMode::Write);
    let before_b = s.project().tracks[1].clone();
    let before_region = s.project().tracks[0].regions[0].automation.clone();
    let mut rec = AutomationRecorder::start(s.project(), 960, 0);
    for tick in (960..4800).step_by(100) {
        rec.observe(tick, 0);
    }
    // Track b is in Read mode: its gesture is ignored.
    assert!(!rec.touch(vol(b), AutomationMode::Read, TouchPhase::Begin, 0.0, 2000, 0));
    let cmd = rec.finish(4800, 0, s.project()).unwrap();
    s.apply(&cmd).unwrap();
    let p = s.project();
    assert_eq!(p.tracks[1], before_b, "unarmed channel untouched");
    assert_eq!(p.tracks[0].regions[0].automation, before_region, "region lanes untouched");
    assert!((lane_value(p, vol(a), 500) - (-12.0)).abs() < 1e-9);
    assert!((lane_value(p, vol(a), 2000) - (-1.5)).abs() < 1e-9, "written with the fader value");
    assert!((lane_value(p, vol(a), 6000) - (-12.0)).abs() < 1e-9);
}

#[test]
fn latch_holds_until_stop_and_cycle_passes_overwrite_in_order() {
    let (mut s, t) = session_with_track();
    add(&mut s, vol(t), &[(0, -20.0)]);
    mode(&mut s, t, AutomationMode::Latch);
    let mut rec = AutomationRecorder::start(s.project(), 0, 0);
    // Pass 1: grab at 1000 with −6, release at 1500 (latched).
    for tick in (0..1000).step_by(100) {
        rec.observe(tick, 0);
    }
    rec.touch(vol(t), AutomationMode::Latch, TouchPhase::Begin, -6.0, 1000, 0);
    rec.touch(vol(t), AutomationMode::Latch, TouchPhase::End, -6.0, 1500, 0);
    for tick in (1500..3840).step_by(100) {
        rec.observe(tick, 0);
    }
    // Cycle wraps to 0 (epoch 1); the latched value continues and is then
    // moved to −2 in pass 2 from 2000 on.
    for tick in (0..2000).step_by(100) {
        rec.observe(tick, 1);
    }
    rec.touch(vol(t), AutomationMode::Latch, TouchPhase::Move, -2.0, 2000, 1);
    let cmd = rec.finish(3000, 1, s.project()).unwrap();
    s.apply(&cmd).unwrap();
    let p = s.project();
    assert!((lane_value(p, vol(t), 1200) - (-6.0)).abs() < 1e-9, "latched in pass 2 too");
    assert!((lane_value(p, vol(t), 2500) - (-2.0)).abs() < 1e-9, "pass 2 overwrote pass 1");
    assert!((lane_value(p, vol(t), 3500) - (-6.0)).abs() < 1e-9, "pass 1 beyond pass 2's end");
    assert!((lane_value(p, vol(t), 5000) - (-20.0)).abs() < 1e-9, "after the written range");
}

#[test]
fn automation_survives_save_and_reopen_identically() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, t) = session_with_track();
    add(&mut s, vol(t), &[(0, -12.0), (960, 0.0)]);
    s.apply(&EditCommand::UpdateAutomationPoints {
        target: vol(t),
        points: vec![elyvon_project::automation::AutomationPoint {
            id: 2,
            tick: 960,
            value: 0.0,
            curve: Curve::Bezier { c1: 0.25, c2: 0.75 },
        }],
    })
    .unwrap();
    mode(&mut s, t, AutomationMode::Latch);
    let pan = AutomationTarget {
        param: AutomationParam::Pan,
        ..vol(t)
    };
    add(&mut s, pan, &[(0, -1.0), (3840, 1.0)]);
    let path = dir.path().join("auto.elyvon");
    s.save(Some(&path), "test").unwrap();
    let reopened = ProjectSession::open(&path).unwrap();
    assert_eq!(reopened.project(), s.project());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"schemaVersion\": 5"));
    assert!(text.contains("\"mode\": \"latch\""));
}

#[test]
fn schema_v4_files_migrate_without_automation() {
    let text = include_str!("fixtures/v4-mixer.elyvon");
    let p = format::decode(text.as_bytes()).unwrap();
    assert_eq!(p.tracks.len(), 2);
    for t in &p.tracks {
        assert_eq!(t.automation.mode, AutomationMode::Read);
        assert!(t.automation.lanes.is_empty() && t.comps.is_empty() && t.freeze.is_none());
        for r in &t.regions {
            assert!(r.automation.is_empty());
            if let RegionContent::Audio { reversed, .. } = r.content {
                assert!(!reversed);
            }
        }
    }
    assert_eq!(p.mixer.auxes.len(), 1);
    assert!(!p.mixer.groups[0].link_edit);
    // Re-encoded as schema 5 and read back identically.
    let again = format::decode(&format::encode(&p, "test")).unwrap();
    assert_eq!(again, p);
}

#[test]
fn invalid_lanes_are_rejected_on_load() {
    let (mut s, t) = session_with_track();
    add(&mut s, vol(t), &[(0, -12.0)]);
    let mut bytes = String::from_utf8(format::encode(s.project(), "test")).unwrap();
    bytes = bytes.replace("\"value\": -12.0", "\"value\": 12.0");
    assert!(format::decode(bytes.as_bytes()).is_err(), "out-of-range value");
}
