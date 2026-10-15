//! Project data model.
//!
//! This is the single source of truth for a song. It is purely descriptive: no
//! pixel sizes, selection or zoom — those belong to the presentation layer.

use elyvon_core::{Color, Key, Tempo, Ticks, TimeSignature};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(export, type = "string")]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// Stable identity of a project across saves and renames.
    ProjectId
);
id_type!(TrackId);
id_type!(RegionId);

/// A complete Elyvon Studio song.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub transport: Transport,
    /// Visible song length in bars. Grows automatically to cover content.
    pub length_bars: u32,
    pub tracks: Vec<Track>,
}

/// Global musical settings of the project.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Transport {
    pub tempo: Tempo,
    pub time_signature: TimeSignature,
    pub key: Key,
    pub loop_range: LoopRange,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            tempo: Tempo::DEFAULT,
            time_signature: TimeSignature::default(),
            key: Key::default(),
            loop_range: LoopRange::default(),
        }
    }
}

/// Cycle/loop range in ticks (`start < end`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoopRange {
    pub enabled: bool,
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub end: Ticks,
}

impl Default for LoopRange {
    fn default() -> Self {
        // Bars 1–5 in 4/4.
        Self {
            enabled: false,
            start: 0,
            end: 4 * 3840,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TrackKind {
    /// Recorded or imported audio.
    Audio,
    /// MIDI driving an internal or plug-in instrument.
    Instrument,
    /// MIDI sent to an external device.
    Midi,
}

impl TrackKind {
    pub fn default_name(self) -> &'static str {
        match self {
            TrackKind::Audio => "Audio",
            TrackKind::Instrument => "Instrument",
            TrackKind::Midi => "MIDI",
        }
    }

    /// Whether regions with this content may live on a track of this kind.
    pub fn accepts(self, content: &RegionContent) -> bool {
        matches!(
            (self, content),
            (TrackKind::Audio, RegionContent::Audio { .. })
                | (
                    TrackKind::Instrument | TrackKind::Midi,
                    RegionContent::Midi { .. }
                )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub color: Color,
    pub muted: bool,
    pub soloed: bool,
    /// Channel fader gain in dB. [`Track::MIN_VOLUME_DB`] means silence (−∞).
    pub volume_db: f64,
    /// Stereo position, −1 (left) … +1 (right).
    pub pan: f64,
    pub regions: Vec<Region>,
}

impl Track {
    pub const MIN_VOLUME_DB: f64 = -96.0;
    pub const MAX_VOLUME_DB: f64 = 6.0;

    pub fn new(kind: TrackKind, name: impl Into<String>, color: Color) -> Self {
        Self {
            id: TrackId::new(),
            name: name.into(),
            kind,
            color,
            muted: false,
            soloed: false,
            volume_db: 0.0,
            pan: 0.0,
            regions: Vec::new(),
        }
    }

    /// Whether this track would be heard given the project's solo state.
    ///
    /// A muted track is always silent. When any track is soloed, only soloed
    /// tracks are heard.
    pub fn is_audible(&self, any_solo: bool) -> bool {
        !self.muted && (!any_solo || self.soloed)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Region {
    pub id: RegionId,
    pub name: String,
    /// Position on the timeline.
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub length: Ticks,
    pub content: RegionContent,
}

impl Region {
    pub fn end(&self) -> Ticks {
        self.start.saturating_add(self.length)
    }
}

/// What a region plays. Audio regions reference media; MIDI regions own notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum RegionContent {
    Audio {
        /// `None` until media import exists (Phase 2); such regions are
        /// placeholders and produce no sound.
        source: Option<AudioSource>,
    },
    Midi {
        notes: Vec<MidiNote>,
    },
}

/// Reference to an audio file used by an audio region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioSource {
    /// Path relative to the project file when possible, absolute otherwise.
    pub path: String,
}

/// A MIDI note with timing relative to the start of its region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiNote {
    pub pitch: u8,
    pub velocity: u8,
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub length: Ticks,
}

/// Colours assigned round-robin to new tracks.
pub const TRACK_PALETTE: [Color; 10] = [
    Color::rgb(0x5B, 0x8D, 0xEF), // azure
    Color::rgb(0x3F, 0xB9, 0x9C), // teal
    Color::rgb(0xE0, 0xA1, 0x3B), // amber
    Color::rgb(0xD9, 0x5D, 0x7A), // rose
    Color::rgb(0x9B, 0x7B, 0xE8), // violet
    Color::rgb(0x6C, 0xC2, 0x5A), // green
    Color::rgb(0xE5, 0x73, 0x4A), // coral
    Color::rgb(0x4B, 0xB4, 0xD6), // cyan
    Color::rgb(0xC7, 0x8B, 0xD8), // orchid
    Color::rgb(0xB8, 0xA7, 0x6A), // sand
];

impl Project {
    pub const DEFAULT_LENGTH_BARS: u32 = 64;
    pub const DEFAULT_NAME: &'static str = "Untitled";

    /// A new, empty project with default settings. Contains no demo data.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ProjectId::new(),
            name: name.into(),
            transport: Transport::default(),
            length_bars: Self::DEFAULT_LENGTH_BARS,
            tracks: Vec::new(),
        }
    }

    pub fn any_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.soloed)
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// Locates a region as `(track index, region index)`.
    pub fn find_region(&self, id: RegionId) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(ti, t)| t.regions.iter().position(|r| r.id == id).map(|ri| (ti, ri)))
    }

    pub fn region_count(&self) -> usize {
        self.tracks.iter().map(|t| t.regions.len()).sum()
    }

    /// The end of the last region, in ticks.
    pub fn content_end(&self) -> Ticks {
        self.tracks
            .iter()
            .flat_map(|t| t.regions.iter().map(Region::end))
            .max()
            .unwrap_or(0)
    }

    /// Palette colour for the next new track.
    pub fn next_track_color(&self) -> Color {
        TRACK_PALETTE[self.tracks.len() % TRACK_PALETTE.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_and_mute_determine_audibility() {
        let mut a = Track::new(TrackKind::Audio, "a", TRACK_PALETTE[0]);
        assert!(a.is_audible(false));
        a.muted = true;
        assert!(!a.is_audible(false));
        a.muted = false;
        assert!(!a.is_audible(true), "not soloed while another track is");
        a.soloed = true;
        assert!(a.is_audible(true));
        a.muted = true;
        assert!(!a.is_audible(true), "mute wins over solo");
    }

    #[test]
    fn track_kind_accepts_matching_content() {
        let audio = RegionContent::Audio { source: None };
        let midi = RegionContent::Midi { notes: vec![] };
        assert!(TrackKind::Audio.accepts(&audio));
        assert!(!TrackKind::Audio.accepts(&midi));
        assert!(TrackKind::Instrument.accepts(&midi));
        assert!(TrackKind::Midi.accepts(&midi));
        assert!(!TrackKind::Midi.accepts(&audio));
    }
}
