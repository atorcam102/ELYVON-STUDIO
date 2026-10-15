//! The demonstration session shown from the welcome screen.
//!
//! Kept in its own module so demo content can never leak into new projects:
//! [`crate::model::Project::new`] is always empty. Audio regions here have no
//! source file — they are placeholders and the UI labels them as such.

use elyvon_core::{Key, KeyMode, PitchClass, Tempo, Ticks};

use crate::model::{
    MidiNote, Project, Region, RegionContent, RegionId, TRACK_PALETTE, Track, TrackKind,
};

const BAR: Ticks = 3840; // 4/4
const BEAT: Ticks = 960;

fn region(name: &str, start_bar: Ticks, bars: Ticks, content: RegionContent) -> Region {
    Region {
        id: RegionId::new(),
        name: name.into(),
        start: start_bar * BAR,
        length: bars * BAR,
        muted: false,
        content,
    }
}

fn placeholder_audio(name: &str, start_bar: Ticks, bars: Ticks) -> Region {
    region(name, start_bar, bars, RegionContent::audio(None))
}

fn notes(pattern: &[(u8, Ticks, Ticks)], bars: Ticks, velocity: u8) -> RegionContent {
    let mut out = Vec::new();
    let pattern_len = pattern
        .iter()
        .map(|&(_, s, l)| s + l)
        .max()
        .unwrap_or(BAR)
        .div_ceil(BAR)
        * BAR;
    let mut offset = 0;
    while offset < bars * BAR {
        for &(pitch, start, length) in pattern {
            if offset + start < bars * BAR {
                out.push(MidiNote {
                    pitch,
                    velocity,
                    start: offset + start,
                    length,
                });
            }
        }
        offset += pattern_len;
    }
    RegionContent::Midi { notes: out }
}

fn chords() -> Vec<(u8, Ticks, Ticks)> {
    // Am – F – C – G, one chord per bar.
    let voicings: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [55, 60, 64], [55, 59, 62]];
    voicings
        .iter()
        .enumerate()
        .flat_map(|(bar, chord)| {
            chord
                .iter()
                .map(move |&p| (p, bar as Ticks * BAR, BAR - BEAT / 4))
        })
        .collect()
}

fn bass() -> Vec<(u8, Ticks, Ticks)> {
    let roots = [45u8, 41, 48, 43];
    roots
        .iter()
        .enumerate()
        .flat_map(|(bar, &root)| {
            let base = bar as Ticks * BAR;
            [
                (root, base, BEAT * 3 / 2),
                (root, base + BEAT * 2, BEAT / 2),
                (root + 12, base + BEAT * 3, BEAT / 2),
            ]
        })
        .collect()
}

fn drums() -> Vec<(u8, Ticks, Ticks)> {
    let mut p = Vec::new();
    for beat in 0..4 {
        let t = beat * BEAT;
        p.push((42, t, BEAT / 4)); // closed hat
        p.push((42, t + BEAT / 2, BEAT / 4));
        if beat % 2 == 0 {
            p.push((36, t, BEAT / 4)); // kick
        } else {
            p.push((38, t, BEAT / 4)); // snare
        }
    }
    p
}

fn track(kind: TrackKind, name: &str, color: usize, regions: Vec<Region>) -> Track {
    let mut t = Track::new(kind, name, TRACK_PALETTE[color]);
    t.regions = regions;
    t
}

/// Builds the demo session: a short arrangement in A minor at 112 BPM.
pub fn demo_project() -> Project {
    let mut p = Project::new("Demo Session");
    p.transport.tempo = Tempo::new(112.0).unwrap_or(Tempo::DEFAULT);
    p.transport.key = Key {
        tonic: PitchClass::A,
        mode: KeyMode::Minor,
    };
    p.transport.loop_range.start = 8 * BAR;
    p.transport.loop_range.end = 16 * BAR;

    let mut drums_t = track(
        TrackKind::Instrument,
        "Drums",
        6,
        vec![
            region("Beat A", 0, 8, notes(&drums(), 8, 96)),
            region("Beat B", 8, 16, notes(&drums(), 16, 110)),
        ],
    );
    drums_t.volume_db = -3.0;
    let bass_t = track(
        TrackKind::Instrument,
        "Bass",
        3,
        vec![region("Bassline", 4, 20, notes(&bass(), 20, 100))],
    );
    let mut keys_t = track(
        TrackKind::Instrument,
        "Keys",
        0,
        vec![
            region("Chords", 0, 8, notes(&chords(), 8, 80)),
            region("Chords", 12, 12, notes(&chords(), 12, 88)),
        ],
    );
    keys_t.pan = -0.25;
    let lead_t = track(
        TrackKind::Midi,
        "Hardware Lead",
        9,
        vec![region(
            "Hook",
            16,
            8,
            notes(
                &[
                    (69, 0, BEAT),
                    (72, BEAT, BEAT / 2),
                    (71, BEAT * 3 / 2, BEAT / 2),
                    (67, BEAT * 2, BEAT * 2),
                ],
                8,
                90,
            ),
        )],
    );
    let mut vox_t = track(
        TrackKind::Audio,
        "Lead Vocal",
        1,
        vec![
            placeholder_audio("Verse Vox", 8, 8),
            placeholder_audio("Chorus Vox", 16, 8),
        ],
    );
    vox_t.pan = 0.0;
    let mut gtr_t = track(
        TrackKind::Audio,
        "Guitar",
        2,
        vec![placeholder_audio("Guitar DI", 4, 16)],
    );
    gtr_t.pan = 0.3;
    gtr_t.volume_db = -4.5;

    p.tracks = vec![drums_t, bass_t, keys_t, lead_t, vox_t, gtr_t];
    p.length_bars = 32;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_project_is_valid() {
        let p = demo_project();
        crate::validate::validate(&p).unwrap();
        assert!(p.region_count() > 5);
    }

    #[test]
    fn new_projects_contain_no_demo_data() {
        assert!(Project::new("x").tracks.is_empty());
    }
}
