//! Project data model.
//!
//! This is the single source of truth for a song. It is purely descriptive: no
//! pixel sizes, selection or zoom — those belong to the presentation layer.

use elyvon_core::tempo::TempoMapError;
use elyvon_core::{Color, Key, MeterChange, Tempo, TempoChange, TempoMap, Ticks, TimeSignature};
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
id_type!(
    /// Identity of an imported audio file in the project's media pool.
    MediaId
);
id_type!(MarkerId);

/// A complete Elyvon Studio song.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Sample rate (Hz) the engine and offline renders run at when possible.
    pub sample_rate: u32,
    pub transport: Transport,
    /// Visible song length in bars. Grows automatically to cover content.
    pub length_bars: u32,
    pub tracks: Vec<Track>,
    /// Imported audio files referenced by regions. Files are never modified.
    pub media: Vec<MediaAsset>,
    /// Named positions on the timeline, sorted by tick.
    pub markers: Vec<Marker>,
    /// Auxiliary channels, groups, folders and the master strip (schema 4).
    #[serde(default)]
    pub mixer: crate::mixer::Mixer,
}

/// Global musical settings of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Transport {
    /// Tempo at the start of the song.
    pub tempo: Tempo,
    /// Time signature of bar 1.
    pub time_signature: TimeSignature,
    pub key: Key,
    pub loop_range: LoopRange,
    /// Tempo steps after the start, strictly increasing by tick.
    pub tempo_changes: Vec<TempoChange>,
    /// Metre changes on bar lines, strictly increasing by bar.
    pub meter_changes: Vec<MeterChange>,
    pub metronome: MetronomeSettings,
    /// Bars of metronome count-in before playback starts (0 = off).
    pub count_in_bars: u8,
    /// Bars played before the start position (0 = off).
    pub pre_roll_bars: u8,
    /// Punch-in/out range (stored and shown; recording is not implemented yet).
    pub punch: LoopRange,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            tempo: Tempo::DEFAULT,
            time_signature: TimeSignature::default(),
            key: Key::default(),
            loop_range: LoopRange::default(),
            tempo_changes: Vec::new(),
            meter_changes: Vec::new(),
            metronome: MetronomeSettings::default(),
            count_in_bars: 0,
            pre_roll_bars: 0,
            punch: LoopRange::default(),
        }
    }
}

impl Transport {
    pub const MAX_COUNT_IN_BARS: u8 = 4;
    pub const MAX_PRE_ROLL_BARS: u8 = 4;

    /// The validated tempo/metre map of these settings.
    pub fn tempo_map(&self) -> Result<TempoMap, TempoMapError> {
        TempoMap::new(
            self.tempo,
            &self.tempo_changes,
            self.time_signature,
            &self.meter_changes,
        )
    }
}

/// Metronome options. Count-in always clicks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MetronomeSettings {
    /// Click during playback.
    pub enabled: bool,
    /// Click level in dB (−96 … +6).
    pub volume_db: f64,
    /// Higher-pitched click on the first beat of each bar.
    pub accent: bool,
}

impl Default for MetronomeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            volume_db: -6.0,
            accent: true,
        }
    }
}

/// A named position (locator) on the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Marker {
    pub id: MarkerId,
    pub name: String,
    #[ts(type = "number")]
    pub tick: Ticks,
}

/// An audio file in the media pool. The file itself is immutable: edits only
/// change how regions reference it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaAsset {
    pub id: MediaId,
    /// Display name (the file name at import time).
    pub name: String,
    /// Absolute path, or relative to the project file's directory.
    pub path: String,
    /// Properties measured when the file was imported; `None` for files that
    /// were never decoded (e.g. migrated from schema 1).
    pub info: Option<MediaInfo>,
}

/// Measured properties of an audio file, used to detect changed files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaInfo {
    pub sample_rate: u32,
    pub channels: u16,
    /// Length in frames at `sample_rate`.
    #[ts(type = "number")]
    pub frames: u64,
    /// Container/codec, e.g. `wav`, `flac`, `mp3`.
    pub format: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
    /// 64-bit FNV-1a of the file bytes, hex.
    pub fingerprint: String,
}

impl MediaInfo {
    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate.max(1))
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
    #[serde(default)]
    pub io: crate::io::TrackIo,
    /// Output, sends, inserts, group and channel format (schema 4).
    #[serde(default)]
    pub strip: crate::mixer::StripRouting,
    /// Mixer folder the track is shown in.
    #[serde(default)]
    pub folder: Option<crate::mixer::FolderId>,
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
    /// Mode and lanes of this channel's automation (schema 5).
    #[serde(default)]
    pub automation: crate::automation::ChannelAutomation,
    /// Set while the track plays a pre-rendered file instead of its regions
    /// and inserts (schema 5).
    #[serde(default)]
    pub freeze: Option<crate::comp::TrackFreeze>,
    /// Comps of recorded take folders (schema 5).
    #[serde(default)]
    pub comps: Vec<crate::comp::TakeComp>,
}

impl Track {
    pub const MIN_VOLUME_DB: f64 = -96.0;
    pub const MAX_VOLUME_DB: f64 = 6.0;

    pub fn new(kind: TrackKind, name: impl Into<String>, color: Color) -> Self {
        Self {
            automation: crate::automation::ChannelAutomation::default(),
            freeze: None,
            comps: Vec::new(),
            io: crate::io::TrackIo::default(),
            strip: crate::mixer::StripRouting::default(),
            folder: None,
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
    #[serde(default)]
    pub capture: Option<crate::io::CaptureMetadata>,
    #[serde(default)]
    pub midi_events: Vec<crate::io::StoredMidiEvent>,
    pub id: RegionId,
    pub name: String,
    /// Position on the timeline.
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub length: Ticks,
    /// A muted region produces no sound or MIDI.
    pub muted: bool,
    pub content: RegionContent,
    /// Region automation (ticks relative to the region start). Inside the
    /// region it replaces the track lane of the same parameter (schema 5).
    #[serde(default)]
    pub automation: Vec<crate::automation::AutomationLane>,
}

impl Region {
    pub fn end(&self) -> Ticks {
        self.start.saturating_add(self.length)
    }
}

/// Shape of a region's fades, and of the crossfades it makes with the
/// region it covers. Every shape's fade-out is the time-reversed fade-in, so
/// the two halves of a crossfade complement each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FadeCurve {
    /// `sin(x·π/2)`: constant power, for uncorrelated material (default).
    #[default]
    EqualPower,
    /// `x`: constant gain, for identical or phase-coherent material.
    Linear,
    /// `(1 − cos(x·π))/2`: constant gain with soft edges.
    SCurve,
}

impl FadeCurve {
    /// Gain of the fade at `x` (0 = silent edge, 1 = full level).
    #[inline]
    pub fn gain(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Self::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
            Self::Linear => x,
            Self::SCurve => 0.5 - 0.5 * (x * std::f32::consts::PI).cos(),
        }
    }
}

/// What a region plays. Audio regions reference media; MIDI regions own notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum RegionContent {
    /// Plays part of a media-pool file. Offsets and fades are in frames of the
    /// source file's own sample rate, so they do not depend on tempo.
    Audio {
        /// `None` for placeholders (e.g. the demo session): silent.
        media: Option<MediaId>,
        /// Source frame heard at the region start (non-destructive trim).
        #[ts(type = "number")]
        offset: u64,
        /// Region gain in dB (−96 … +24).
        gain_db: f64,
        #[ts(type = "number")]
        fade_in: u64,
        #[ts(type = "number")]
        fade_out: u64,
        /// Repeat the source from `offset` to its end until the region ends.
        looped: bool,
        /// Plays its source span backwards: the region covers source frames
        /// `[offset, offset + heard length)` and starts at the last of them.
        /// Cannot be combined with `looped` (schema 5).
        #[serde(default)]
        reversed: bool,
        /// Shape of the fades and of crossfades this region creates.
        #[serde(default)]
        fade_curve: FadeCurve,
    },
    Midi {
        notes: Vec<MidiNote>,
    },
}

impl RegionContent {
    pub const MIN_GAIN_DB: f64 = -96.0;
    pub const MAX_GAIN_DB: f64 = 24.0;

    /// Audio content playing `media` from its start, unity gain, no fades.
    pub fn audio(media: Option<MediaId>) -> Self {
        Self::Audio {
            media,
            offset: 0,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
            looped: false,
            reversed: false,
            fade_curve: FadeCurve::EqualPower,
        }
    }
}

/// A MIDI note with timing relative to the start of its region.
///
/// Notes are the authoritative note data of a region (schema 4): the piano
/// roll edits them and playback and SMF export read them. Controller,
/// pitch-bend, pressure, program and SysEx events live in
/// [`Region::midi_events`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiNote {
    /// Unique within its region; stable across edits.
    #[serde(default)]
    pub id: u32,
    pub pitch: u8,
    pub velocity: u8,
    /// MIDI channel 0–15.
    #[serde(default)]
    pub channel: u8,
    /// A muted note is kept but not played or exported.
    #[serde(default)]
    pub muted: bool,
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub length: Ticks,
    /// `start` minus the position before quantisation (0 when unquantised),
    /// so quantise can be undone or re-applied from the played position.
    #[serde(default)]
    pub quantize_offset: i32,
}

impl MidiNote {
    /// A note on channel 1 with id 0 (ids are assigned when added to a region).
    pub const fn new(pitch: u8, velocity: u8, start: Ticks, length: Ticks) -> Self {
        Self {
            id: 0,
            pitch,
            velocity,
            channel: 0,
            muted: false,
            start,
            length,
            quantize_offset: 0,
        }
    }

    pub fn end(&self) -> Ticks {
        self.start.saturating_add(self.length)
    }

    /// Position before quantisation.
    pub fn unquantized_start(&self) -> Ticks {
        self.start
            .saturating_add_signed(-i64::from(self.quantize_offset))
    }
}

/// Colours assigned round-robin to new tracks (saturated region colours that
/// read well on the dark arrangement background).
pub const TRACK_PALETTE: [Color; 10] = [
    Color::rgb(0x36, 0x5D, 0xCD), // blue
    Color::rgb(0x19, 0xA7, 0xC0), // cyan
    Color::rgb(0x22, 0xBC, 0xBC), // teal
    Color::rgb(0x64, 0x45, 0xD5), // violet
    Color::rgb(0x2F, 0x6C, 0xB4), // steel blue
    Color::rgb(0x3F, 0xA7, 0x5A), // green
    Color::rgb(0xC9, 0x9A, 0x22), // amber
    Color::rgb(0xD9, 0x6F, 0x2B), // orange
    Color::rgb(0xC9, 0x45, 0x5A), // red
    Color::rgb(0x8E, 0x4F, 0xD1), // purple
];

impl Project {
    pub const DEFAULT_LENGTH_BARS: u32 = 64;
    pub const DEFAULT_NAME: &'static str = "Untitled";

    /// A new, empty project with default settings. Contains no demo data.
    pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
    pub const SAMPLE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ProjectId::new(),
            name: name.into(),
            sample_rate: Self::DEFAULT_SAMPLE_RATE,
            transport: Transport::default(),
            length_bars: Self::DEFAULT_LENGTH_BARS,
            tracks: Vec::new(),
            media: Vec::new(),
            markers: Vec::new(),
            mixer: crate::mixer::Mixer::default(),
        }
    }

    pub fn media_asset(&self, id: MediaId) -> Option<&MediaAsset> {
        self.media.iter().find(|m| m.id == id)
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
        let audio = RegionContent::audio(None);
        let midi = RegionContent::Midi { notes: vec![] };
        assert!(TrackKind::Audio.accepts(&audio));
        assert!(!TrackKind::Audio.accepts(&midi));
        assert!(TrackKind::Instrument.accepts(&midi));
        assert!(TrackKind::Midi.accepts(&midi));
        assert!(!TrackKind::Midi.accepts(&audio));
    }
}
