//! Phase 11: the edit history stays exact and bounded on large projects.
#![allow(clippy::unwrap_used)]

use elyvon_project::automation::{AutomationParam, AutomationTarget, PointSpec};
use elyvon_project::history::{History, Snapshot, approx_bytes};
use elyvon_project::mixer::ChannelRef;
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, MediaInfo, PlacementTrack, Project,
    ProjectSession, RegionId, TrackKind,
};

/// 64 audio tracks × 120 regions, each track with a 2 000-point lane.
fn large_session() -> (ProjectSession, Vec<RegionId>) {
    let mut s = ProjectSession::new_empty();
    let mut regions = Vec::new();
    for t in 0..64 {
        let track = s
            .apply(&EditCommand::AddTrack {
                kind: TrackKind::Audio,
                name: None,
                index: None,
            })
            .unwrap()
            .created_tracks[0];
        // One file per track, placed 120 times (the pool holds 4096 files).
        let assets: Vec<MediaAsset> = (0..1)
            .map(|i| MediaAsset {
                id: MediaId::new(),
                name: format!("t{t}-{i}.wav"),
                path: format!("Media/t{t}-{i}.wav"),
                info: Some(MediaInfo {
                    sample_rate: 48_000,
                    channels: 2,
                    frames: 48_000,
                    format: "wav".into(),
                    size_bytes: 192_044,
                    fingerprint: format!("{t:08x}{i:08x}"),
                }),
            })
            .collect();
        let placements = (0..120)
            .map(|i| AudioPlacement {
                media_id: assets[0].id,
                track: PlacementTrack::Existing { id: track },
                start: i as u64 * 1920,
            })
            .collect();
        regions.extend(
            s.apply(&EditCommand::ImportAudio { assets, placements })
                .unwrap()
                .created_regions,
        );
        s.apply(&EditCommand::AddAutomationPoints {
            target: AutomationTarget {
                channel: ChannelRef::Track { id: track },
                region: None,
                param: AutomationParam::Volume,
            },
            points: (0..2000)
                .map(|k| PointSpec {
                    tick: k * 113,
                    value: -((k % 40) as f64),
                    curve: None,
                })
                .collect(),
        })
        .unwrap();
    }
    (s, regions)
}

#[test]
fn hundreds_of_edits_undo_and_redo_exactly_on_a_large_project() {
    let (mut s, regions) = large_session();
    assert_eq!(s.project().region_count(), 64 * 120);
    let base = s.project().clone();
    let mut states = vec![base.clone()];
    let started = std::time::Instant::now();
    let mut seed = 7u64;
    for k in 0..150u64 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let id = regions[(seed >> 33) as usize % regions.len()];
        let cmd = match k % 6 {
            0 => EditCommand::MoveRegions {
                ids: vec![id],
                delta_ticks: 7,
                track_offset: 0,
            },
            1 => EditCommand::SlipRegions {
                ids: vec![id],
                delta_ticks: -3,
            },
            2 => EditCommand::SetFadeCurve {
                ids: vec![id],
                curve: elyvon_project::FadeCurve::SCurve,
            },
            3 => EditCommand::SetRegionsMuted {
                ids: vec![id],
                muted: k % 4 == 0,
            },
            4 => EditCommand::SetRegionsReversed {
                ids: vec![id],
                reversed: true,
            },
            _ => EditCommand::NormalizeRegions {
                gains: vec![elyvon_project::region_edit::RegionGain {
                    id,
                    gain_db: -(k as f64) / 10.0,
                }],
            },
        };
        s.apply(&cmd).unwrap();
        // No-op edits (already reversed, already muted) add no undo step.
        if s.project() != states.last().unwrap() {
            states.push(s.project().clone());
        }
    }
    let edits = started.elapsed();
    // Undo everything, checking every intermediate state, then redo.
    let started = std::time::Instant::now();
    let mut undone = 0;
    while s.undo() {
        undone += 1;
        let expected = &states[states.len() - 1 - undone];
        assert!(s.project() == expected, "state after {undone} undos");
    }
    let undo_time = started.elapsed();
    // Steps are kept up to the memory budget (oldest dropped first).
    let per_step = approx_bytes(&states[0]);
    let fit = (History::DEFAULT_BUDGET / per_step).min(History::DEFAULT_LIMIT);
    assert!(
        undone == states.len() - 1 || undone + 1 >= fit,
        "{undone} steps kept; {fit} fit in the budget"
    );
    assert_eq!(s.project(), &states[states.len() - 1 - undone]);
    while s.redo() {}
    assert_eq!(s.project(), states.last().unwrap());
    let bytes = approx_bytes(s.project());
    eprintln!(
        "{} regions, {} automation points, ~{:.1} MiB per snapshot: {} edits {edits:?}, {undone} undos {undo_time:?}",
        s.project().region_count(),
        64 * 2000,
        bytes as f64 / (1 << 20) as f64,
        states.len() - 1
    );
    assert!(edits.as_secs_f64() < 60.0 && undo_time.as_secs_f64() < 30.0);
}

#[test]
fn the_history_drops_old_steps_to_stay_within_its_memory_budget() {
    let (s, _) = large_session();
    let one = approx_bytes(s.project());
    let mut h = History::with_budget(1000, one * 5);
    for i in 0..20 {
        h.record(Snapshot {
            project: s.project().clone(),
            state_id: i,
            label: "Edit",
        });
    }
    assert!(h.undo_len() <= 5, "{} snapshots kept", h.undo_len());
    assert!(h.bytes() <= one * 5);
    // At least one step always remains, however large the project.
    let mut tiny = History::with_budget(1000, 1);
    tiny.record(Snapshot {
        project: Project::new("x"),
        state_id: 0,
        label: "Edit",
    });
    tiny.record(Snapshot {
        project: s.project().clone(),
        state_id: 1,
        label: "Edit",
    });
    assert_eq!(tiny.undo_len(), 1);
}
