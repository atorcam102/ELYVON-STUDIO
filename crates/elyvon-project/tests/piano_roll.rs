//! Phase 08 acceptance at the session level: grouped undo/redo never loses
//! or duplicates notes, and editing 10 000 notes stays within the declared
//! budget (printed with `--nocapture`).
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use elyvon_core::grid::{GridDivision, GridSpec};
use elyvon_project::midi_edit::{NoteEdge, NoteSpec, NoteTarget, NoteTransform, VelocityMode};
use elyvon_project::{EditCommand, MidiNote, ProjectSession, RegionContent, RegionId, TrackKind};

fn setup() -> (ProjectSession, RegionId) {
    let mut s = ProjectSession::new_empty();
    let t = s
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Instrument,
            name: None,
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    let r = s
        .apply(&EditCommand::CreateMidiRegion {
            track_id: t,
            start: 0,
            length: 3840 * 4,
        })
        .unwrap()
        .created_regions[0];
    (s, r)
}

fn notes(s: &ProjectSession, r: RegionId) -> Vec<MidiNote> {
    let p = s.project();
    let (ti, ri) = p.find_region(r).unwrap();
    match &p.tracks[ti].regions[ri].content {
        RegionContent::Midi { notes } => notes.clone(),
        RegionContent::Audio { .. } => unreachable!(),
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn random_edit(rng: &mut Rng, r: RegionId, current: &[MidiNote]) -> EditCommand {
    let pick = |rng: &mut Rng| -> Vec<u32> {
        let k = 1 + rng.below(3) as usize;
        (0..k)
            .map(|_| current[rng.below(current.len() as u64) as usize].id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    if current.is_empty() || rng.below(5) == 0 {
        return EditCommand::AddNotes {
            region_id: r,
            notes: (0..1 + rng.below(4))
                .map(|_| NoteSpec {
                    pitch: 30 + rng.below(60) as u8,
                    velocity: 1 + rng.below(127) as u8,
                    channel: rng.below(16) as u8,
                    start: rng.below(15_000),
                    length: 1 + rng.below(2000),
                })
                .collect(),
        };
    }
    match rng.below(8) {
        0 => EditCommand::RemoveNotes {
            region_id: r,
            ids: pick(rng),
        },
        1 | 2 => EditCommand::MoveNotes {
            region_id: r,
            ids: pick(rng),
            delta_ticks: rng.below(2000) as i64 - 1000,
            delta_pitch: rng.below(9) as i32 - 4,
            copy: rng.below(3) == 0,
        },
        3 => EditCommand::ResizeNotes {
            region_id: r,
            ids: pick(rng),
            edge: if rng.below(2) == 0 {
                NoteEdge::Start
            } else {
                NoteEdge::End
            },
            delta: rng.below(800) as i64 - 400,
        },
        4 => EditCommand::TransformNotes {
            targets: vec![NoteTarget {
                region_id: r,
                ids: Some(pick(rng)),
            }],
            transform: NoteTransform::Quantize {
                grid: GridSpec {
                    division: GridDivision::Sixteenth,
                    triplet: rng.below(2) == 0,
                    swing: 0,
                },
                strength: rng.below(101) as u8,
            },
        },
        5 => EditCommand::TransformNotes {
            targets: vec![NoteTarget {
                region_id: r,
                ids: None,
            }],
            transform: NoteTransform::Velocity {
                mode: VelocityMode::Add,
                value: rng.below(21) as i32 - 10,
            },
        },
        6 => EditCommand::Batch {
            commands: vec![
                EditCommand::MoveNotes {
                    region_id: r,
                    ids: pick(rng),
                    delta_ticks: 240,
                    delta_pitch: 0,
                    copy: true,
                },
                EditCommand::TransformNotes {
                    targets: vec![NoteTarget {
                        region_id: r,
                        ids: None,
                    }],
                    transform: NoteTransform::Unquantize,
                },
            ],
        },
        _ => EditCommand::TransformNotes {
            targets: vec![NoteTarget {
                region_id: r,
                ids: Some(pick(rng)),
            }],
            transform: NoteTransform::Legato { gap: 0 },
        },
    }
}

#[test]
fn repeated_undo_redo_never_loses_or_duplicates_notes() {
    let (mut s, r) = setup();
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut states = vec![s.project().clone()];
    let mut applied = 0;
    while applied < 150 {
        let cmd = random_edit(&mut rng, r, &notes(&s, r));
        // Rejected and no-op edits leave no history.
        if s.apply(&cmd).is_ok() && s.project() != states.last().unwrap() {
            let n = notes(&s, r);
            let mut ids: Vec<u32> = n.iter().map(|n| n.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n.len(), "unique ids after {cmd:?}");
            states.push(s.project().clone());
            applied += 1;
        }
    }
    for round in 0..3 {
        for expected in states.iter().rev().skip(1) {
            assert!(s.undo(), "round {round}");
            assert_eq!(s.project(), expected);
        }
        for expected in states.iter().skip(1) {
            assert!(s.redo());
            assert_eq!(s.project(), expected);
        }
    }
}

#[test]
fn batch_is_one_undo_step() {
    let (mut s, r) = setup();
    s.apply(&EditCommand::AddNotes {
        region_id: r,
        notes: vec![NoteSpec {
            pitch: 60,
            velocity: 90,
            channel: 0,
            start: 0,
            length: 480,
        }],
    })
    .unwrap();
    let before = notes(&s, r);
    let out = s
        .apply(&EditCommand::Batch {
            commands: vec![
                EditCommand::MoveNotes {
                    region_id: r,
                    ids: vec![1],
                    delta_ticks: 480,
                    delta_pitch: 0,
                    copy: true,
                },
                EditCommand::MoveNotes {
                    region_id: r,
                    ids: vec![1],
                    delta_ticks: 960,
                    delta_pitch: 0,
                    copy: true,
                },
            ],
        })
        .unwrap();
    assert_eq!(out.created_notes, vec![2, 3]);
    assert_eq!(s.view().undo_label.as_deref(), Some("Copy Notes"));
    s.undo();
    assert_eq!(notes(&s, r), before);
    // A failing batch applies nothing.
    let bad = s.apply(&EditCommand::Batch {
        commands: vec![
            EditCommand::RemoveNotes {
                region_id: r,
                ids: vec![1],
            },
            EditCommand::RemoveNotes {
                region_id: r,
                ids: vec![1],
            },
        ],
    });
    assert!(bad.is_err());
    assert_eq!(notes(&s, r), before);
}

/// Declared budget for 10 000 notes on this machine class: an edit applied
/// to every note plus the full view serialisation the UI receives stays
/// under 250 ms in an unoptimised debug build (release figures are recorded
/// in docs/audit/PHASE-08-09.md).
#[test]
fn ten_thousand_notes_stay_editable() {
    let (mut s, r) = setup();
    let specs: Vec<NoteSpec> = (0..10_000u64)
        .map(|i| NoteSpec {
            pitch: 36 + (i % 48) as u8,
            velocity: 64 + (i % 60) as u8,
            channel: (i % 4) as u8,
            start: i * 37 % 200_000,
            length: 120 + i % 240,
        })
        .collect();
    let t = Instant::now();
    s.apply(&EditCommand::AddNotes {
        region_id: r,
        notes: specs,
    })
    .unwrap();
    let add = t.elapsed();
    let all = NoteTarget {
        region_id: r,
        ids: None,
    };
    let ids: Vec<u32> = notes(&s, r).iter().map(|n| n.id).collect();
    let edits = [
        EditCommand::TransformNotes {
            targets: vec![all.clone()],
            transform: NoteTransform::Transpose { semitones: 2 },
        },
        EditCommand::MoveNotes {
            region_id: r,
            ids: ids.clone(),
            delta_ticks: 120,
            delta_pitch: -1,
            copy: false,
        },
        EditCommand::TransformNotes {
            targets: vec![all.clone()],
            transform: NoteTransform::Quantize {
                grid: GridSpec {
                    division: GridDivision::Sixteenth,
                    triplet: false,
                    swing: 30,
                },
                strength: 80,
            },
        },
        EditCommand::ResizeNotes {
            region_id: r,
            ids: ids[..500].to_vec(),
            edge: NoteEdge::End,
            delta: 60,
        },
    ];
    let mut worst = Duration::ZERO;
    for cmd in &edits {
        let t = Instant::now();
        s.apply(cmd).unwrap();
        let json = serde_json::to_vec(&s.view()).unwrap();
        let took = t.elapsed();
        worst = worst.max(took);
        println!(
            "10k notes: {} + view ({} KiB) in {:.1} ms",
            cmd.label(),
            json.len() / 1024,
            took.as_secs_f64() * 1000.0
        );
    }
    let t = Instant::now();
    s.undo();
    let undo = t.elapsed();
    println!(
        "10k notes: add {:.1} ms, undo {:.2} ms, worst edit+view {:.1} ms",
        add.as_secs_f64() * 1000.0,
        undo.as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0
    );
    assert_eq!(notes(&s, r).len(), 10_000);
    assert!(worst < Duration::from_millis(250), "{worst:?}");
}
