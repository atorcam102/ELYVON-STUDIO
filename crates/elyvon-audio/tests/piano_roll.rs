//! Phase 08: a sequence built only with piano-roll commands plays the
//! expected events through the engine (irregular metres, triplets, tempo
//! ramps, overlapping notes, controller lanes), sounds on the built-in
//! instrument, survives edits during playback without hanging notes and
//! exports to an equivalent Standard MIDI File.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::contracts::MidiMessage;
use elyvon_audio::engine;
use elyvon_audio::midi::smf;
use elyvon_audio::plan::build_plan;
use elyvon_audio::render::{RenderRequest, render};
use elyvon_audio::transport::TransportCommand;
use elyvon_core::grid::{GridDivision, GridSpec};
use elyvon_core::{MeterChange, Tempo, TempoChange, TimeSignature};
use elyvon_project::midi_edit::{
    ControllerLane, LanePoint, NoteEdge, NoteSpec, NoteTarget, NoteTransform,
};
use elyvon_project::{EditCommand, RegionContent, RegionId, TrackId, TrackKind};

const RATE: u32 = 48_000;

fn spec(pitch: u8, channel: u8, start: u64, length: u64) -> NoteSpec {
    NoteSpec {
        pitch,
        velocity: 100,
        channel,
        start,
        length,
    }
}

/// 4/4, then 7/8 from bar 2 and 5/4 from bar 3; an accelerating tempo ramp
/// (an eighth-note staircase from 90 to 150 BPM over two bars).
fn session() -> (Session, TrackId, RegionId) {
    let mut s = Session::new(90.0);
    s.project.transport.meter_changes = vec![
        MeterChange {
            bar: 1,
            time_signature: TimeSignature {
                numerator: 7,
                denominator: 8,
            },
        },
        MeterChange {
            bar: 2,
            time_signature: TimeSignature {
                numerator: 5,
                denominator: 4,
            },
        },
    ];
    s.project.transport.tempo_changes = (1..=15)
        .map(|i| TempoChange {
            tick: i * 480,
            bpm: Tempo::new(90.0 + 4.0 * i as f64).unwrap(),
        })
        .collect();
    let k = s
        .apply(EditCommand::AddTrack {
            kind: TrackKind::Instrument,
            name: None,
            index: None,
        })
        .created_tracks[0];
    let r = s
        .apply(EditCommand::CreateMidiRegion {
            track_id: k,
            start: 0,
            length: 3840,
        })
        .created_regions[0];
    (s, k, r)
}

fn notes(s: &Session, r: RegionId) -> Vec<elyvon_project::MidiNote> {
    let (ti, ri) = s.project.find_region(r).unwrap();
    match &s.project.tracks[ti].regions[ri].content {
        RegionContent::Midi { notes } => notes.clone(),
        RegionContent::Audio { .. } => unreachable!(),
    }
}

/// Draws, drags, resizes, duplicates, quantises and deletes like a user would.
fn compose(s: &mut Session, r: RegionId) {
    // Draw: loose timing, an overlapping same-pitch pair and a chord.
    s.apply(EditCommand::AddNotes {
        region_id: r,
        notes: vec![
            spec(60, 0, 10, 900),
            spec(60, 0, 470, 900), // overlaps the first
            spec(64, 1, 1930, 400),
            spec(67, 1, 1925, 400),
            spec(72, 0, 3000, 300),
        ],
    });
    // Drag the chord up a tone; drag-copy the last note a bar later
    // (into the 7/8 bar, which extends the region).
    s.apply(EditCommand::MoveNotes {
        region_id: r,
        ids: vec![3, 4],
        delta_ticks: 0,
        delta_pitch: 2,
        copy: false,
    });
    s.apply(EditCommand::MoveNotes {
        region_id: r,
        ids: vec![5],
        delta_ticks: 3840,
        delta_pitch: 0,
        copy: true,
    });
    s.apply(EditCommand::ResizeNotes {
        region_id: r,
        ids: vec![5],
        edge: NoteEdge::End,
        delta: 180,
    });
    // Quantise everything to eighth triplets, at full strength.
    s.apply(EditCommand::TransformNotes {
        targets: vec![NoteTarget {
            region_id: r,
            ids: None,
        }],
        transform: NoteTransform::Quantize {
            grid: GridSpec {
                division: GridDivision::Eighth,
                triplet: true,
                swing: 0,
            },
            strength: 100,
        },
    });
    // Sustain pedal lane.
    s.apply(EditCommand::EditControllerLane {
        region_id: r,
        lane: ControllerLane::Controller {
            channel: 0,
            controller: 64,
        },
        from: 0,
        to: 3840,
        points: vec![
            LanePoint {
                tick: 0,
                value: 127,
            },
            LanePoint {
                tick: 1920,
                value: 0,
            },
        ],
    });
}

#[test]
fn sequence_built_in_the_piano_roll_plays_exactly() {
    let (mut s, _, r) = session();
    compose(&mut s, r);
    let n = notes(&s, r);
    // Triplet eighths are 320 ticks: 10 → 0, 470 → 320 (480 is nearer
    // 320·1.5 = 480? no: 470 is 150 from 320 and 170 from 640) …
    let starts: Vec<(u8, u64)> = n.iter().map(|n| (n.pitch, n.start)).collect();
    assert_eq!(
        starts,
        [
            (60, 0),
            (60, 320),
            (66, 1920),
            (69, 1920),
            (72, 2880),
            (72, 6720)
        ]
    );
    let map = s.project.transport.tempo_map().unwrap();
    for block in [64, 512, 1031] {
        let mut log = MidiCapture::default();
        let lookup = s.lookup();
        let plan = build_plan(&s.project, &lookup, RATE, 1);
        let (mut handle, mut eng) = engine::create(RATE, block, plan);
        handle.send(TransportCommand::Play {
            from_tick: Some(0),
            count_in: false,
            pre_roll: false,
        });
        let total = map.sample_at_tick(8400, RATE) as usize;
        let mut buf = vec![0.0f32; block * 2];
        let mut done = 0;
        let mut peak = 0.0f32;
        while done < total {
            let k = block.min(total - done);
            log.block_start = done as u64;
            eng.process(&mut buf[..k * 2], 2, &mut log);
            peak = buf[..k * 2].iter().fold(peak, |p, v| p.max(v.abs()));
            done += k;
        }
        assert!(peak > 0.05, "the built-in instrument is audible");
        let on: Vec<(u64, u8, u8)> = log
            .events
            .iter()
            .filter_map(|&(f, _, m)| match m {
                MidiMessage::NoteOn { channel, pitch, .. } => Some((f, channel, pitch)),
                _ => None,
            })
            .collect();
        let expected: Vec<(u64, u8, u8)> = n
            .iter()
            .map(|n| (map.sample_at_tick(n.start, RATE), n.channel, n.pitch))
            .collect();
        assert_eq!(
            on, expected,
            "note-ons at their tempo-mapped frames (block {block})"
        );
        let offs: Vec<(u64, u8)> = log
            .events
            .iter()
            .filter_map(|&(f, _, m)| match m {
                MidiMessage::NoteOff { pitch, .. } => Some((f, pitch)),
                _ => None,
            })
            .collect();
        assert_eq!(offs.len(), n.len(), "every note is released once");
        for note in &n {
            let end = map.sample_at_tick((note.end()).min(4320 + 3840), RATE);
            assert!(
                offs.contains(&(end, note.pitch)),
                "{note:?} released at {end}"
            );
        }
        let pedal: Vec<u64> = log
            .events
            .iter()
            .filter_map(|&(f, _, m)| match m {
                MidiMessage::ControlChange { controller: 64, .. } => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(pedal, [0, map.sample_at_tick(1920, RATE)]);
    }
}

#[test]
fn piano_roll_sequence_exports_an_equivalent_smf() {
    let (mut s, _, r) = session();
    compose(&mut s, r);
    let file = smf::encode(&s.project).unwrap();
    let parsed = smf::decode(&file).unwrap();
    assert_eq!(parsed.tempo_changes.len(), 15);
    assert_eq!(parsed.meter_changes.len(), 2);
    let (back, rest) = elyvon_project::midi_edit::split_note_events(
        &parsed.regions[0].events,
        parsed.regions[0].length,
    );
    let ours = notes(&s, r);
    let strip = |v: &[elyvon_project::MidiNote]| -> Vec<_> {
        v.iter()
            .map(|n| (n.start, n.pitch, n.channel, n.velocity, n.length))
            .collect()
    };
    assert_eq!(strip(&back), strip(&ours));
    let (ti, ri) = s.project.find_region(r).unwrap();
    assert_eq!(rest, s.project.tracks[ti].regions[ri].midi_events);
    // Muted notes are kept in the project but not exported or played.
    s.apply(EditCommand::TransformNotes {
        targets: vec![NoteTarget {
            region_id: r,
            ids: Some(vec![1]),
        }],
        transform: NoteTransform::Mute { muted: true },
    });
    let parsed = smf::decode(&smf::encode(&s.project).unwrap()).unwrap();
    let (back, _) =
        elyvon_project::midi_edit::split_note_events(&parsed.regions[0].events, u64::MAX);
    assert_eq!(back.len(), ours.len() - 1);
    let plan = build_plan(&s.project, &s.lookup(), RATE, 1);
    assert_eq!(plan.stats.notes, ours.len() - 1);
}

#[test]
fn editing_during_playback_keeps_held_notes_and_releases_removed_ones() {
    let (mut s, k, r) = session();
    s.apply(EditCommand::AddNotes {
        region_id: r,
        notes: vec![spec(60, 0, 0, 3000), spec(64, 0, 0, 3000)],
    });
    let lookup = |_: elyvon_project::MediaId| None; // MIDI only
    let (mut handle, mut eng) = engine::create(RATE, 256, build_plan(&s.project, &lookup, RATE, 1));
    handle.send(TransportCommand::Play {
        from_tick: Some(0),
        count_in: false,
        pre_roll: false,
    });
    let mut log = MidiCapture::default();
    let mut buf = vec![0.0f32; 512];
    let mut run = |eng: &mut engine::Engine, log: &mut MidiCapture, blocks: usize| {
        for _ in 0..blocks {
            eng.process(&mut buf, 2, log);
            log.block_start += 256;
        }
    };
    run(&mut eng, &mut log, 10);
    // A mixer edit while the notes sound: nothing is cut.
    s.apply(EditCommand::SetTrackVolume {
        id: k,
        volume_db: -3.0,
    });
    handle.publish(build_plan(&s.project, &lookup, RATE, 2));
    run(&mut eng, &mut log, 4);
    let offs = |log: &MidiCapture| {
        log.events
            .iter()
            .filter(|e| matches!(e.2, MidiMessage::NoteOff { .. }))
            .count()
    };
    assert_eq!(offs(&log), 0, "unrelated edits keep held notes");
    // Deleting one sounding note releases exactly that note.
    s.apply(EditCommand::RemoveNotes {
        region_id: r,
        ids: vec![2],
    });
    handle.publish(build_plan(&s.project, &lookup, RATE, 3));
    run(&mut eng, &mut log, 2);
    let released: Vec<u8> = log
        .events
        .iter()
        .filter_map(|e| match e.2 {
            MidiMessage::NoteOff { pitch, .. } => Some(pitch),
            _ => None,
        })
        .collect();
    assert_eq!(released, [64]);
    // Stop releases the rest; nothing hangs.
    handle.send(TransportCommand::Stop);
    run(&mut eng, &mut log, 2);
    let mut held = std::collections::HashMap::<(u8, u8), i32>::new();
    for e in &log.events {
        match e.2 {
            MidiMessage::NoteOn { channel, pitch, .. } => {
                *held.entry((channel, pitch)).or_default() += 1
            }
            MidiMessage::NoteOff { channel, pitch, .. } => {
                *held.entry((channel, pitch)).or_default() -= 1
            }
            _ => {}
        }
    }
    assert!(held.values().all(|v| *v == 0), "{held:?}");
}

#[test]
fn instrument_render_follows_tempo_and_note_lengths() {
    let (mut s, _, r) = session();
    s.apply(EditCommand::AddNotes {
        region_id: r,
        notes: vec![spec(69, 0, 960, 960)],
    });
    let rendered = render(
        &s.project,
        &s.lookup(),
        RenderRequest::whole_song(RATE),
        None,
    );
    let left = rendered.channel(0);
    let map = s.project.transport.tempo_map().unwrap();
    let on = map.sample_at_tick(960, RATE) as usize;
    let off = map.sample_at_tick(1920, RATE) as usize;
    assert!(
        left[..on].iter().all(|v| *v == 0.0),
        "silent before the note"
    );
    assert!(rms(&left[on + 2000..off - 100]) > 0.03);
    // 180 ms release, then silence.
    let tail = off + RATE as usize / 4;
    if tail < left.len() {
        assert!(left[tail..].iter().all(|v| v.abs() < 1e-6));
    }
}
