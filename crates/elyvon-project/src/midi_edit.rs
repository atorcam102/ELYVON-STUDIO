//! Piano-roll editing (Phase 08): note and controller-lane commands.
//!
//! Notes are addressed by their per-region [`MidiNote::id`], so a selection
//! stays valid across edits. Positions are region-relative ticks. Every
//! function leaves notes sorted by `(start, pitch, id)` and grows the region
//! to cover the notes it edits; `edit::apply` then revalidates the project,
//! so a failed edit restores the previous state.

use std::collections::{BTreeMap, HashMap, HashSet};

use elyvon_core::grid::GridSpec;
use elyvon_core::{TempoMap, Ticks};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edit::{EditOutcome, invalid};
use crate::error::EditError;
use crate::io::StoredMidiEvent;
use crate::model::{MidiNote, Project, Region, RegionContent, RegionId, Track, TrackKind};
use crate::validate::limits;

/// A note to create (its id is assigned by the region).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoteSpec {
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
    #[ts(type = "number")]
    pub start: Ticks,
    #[ts(type = "number")]
    pub length: Ticks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoteEdge {
    Start,
    End,
}

/// Notes of one region; `None` selects every note of the region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoteTarget {
    pub region_id: RegionId,
    #[ts(optional)]
    pub ids: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VelocityMode {
    Set,
    Add,
    /// Percent of the current velocity.
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum NoteTransform {
    /// Moves note starts towards the grid (bar-relative, so it follows metre
    /// changes and swing). Reversible: it always starts from the
    /// unquantised position.
    Quantize {
        grid: GridSpec,
        strength: u8,
    },
    /// Restores the positions before quantisation.
    Unquantize,
    /// Deterministic random offsets (same seed, same result).
    Humanize {
        timing: u32,
        velocity: u8,
        length: u32,
        #[ts(type = "number")]
        seed: u64,
    },
    Transpose {
        semitones: i32,
    },
    /// Extends each note to the next later note start, minus `gap` ticks.
    Legato {
        gap: u32,
    },
    Velocity {
        mode: VelocityMode,
        value: i32,
    },
    Mute {
        muted: bool,
    },
    Channel {
        channel: u8,
    },
    FixedLength {
        #[ts(type = "number")]
        length: Ticks,
    },
}

/// A controller lane of the piano roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ControllerLane {
    /// Control change (64 = sustain).
    Controller {
        channel: u8,
        controller: u8,
    },
    /// 14-bit pitch bend, 8192 = centre.
    PitchBend {
        channel: u8,
    },
    ChannelPressure {
        channel: u8,
    },
    PolyPressure {
        channel: u8,
        pitch: u8,
    },
    Program {
        channel: u8,
    },
}

impl ControllerLane {
    pub fn channel(self) -> u8 {
        match self {
            Self::Controller { channel, .. }
            | Self::PitchBend { channel }
            | Self::ChannelPressure { channel }
            | Self::PolyPressure { channel, .. }
            | Self::Program { channel } => channel,
        }
    }

    pub fn max_value(self) -> u16 {
        match self {
            Self::PitchBend { .. } => 16383,
            _ => 127,
        }
    }

    /// Whether a stored event belongs to this lane.
    pub fn matches(self, bytes: &[u8]) -> bool {
        let (Some(&status), ch) = (bytes.first(), self.channel()) else {
            return false;
        };
        if status >= 0xf0 || status & 0x0f != ch {
            return false;
        }
        match self {
            Self::Controller { controller, .. } => {
                status & 0xf0 == 0xb0 && bytes.get(1) == Some(&controller)
            }
            Self::PitchBend { .. } => status & 0xf0 == 0xe0,
            Self::ChannelPressure { .. } => status & 0xf0 == 0xd0,
            Self::PolyPressure { pitch, .. } => {
                status & 0xf0 == 0xa0 && bytes.get(1) == Some(&pitch)
            }
            Self::Program { .. } => status & 0xf0 == 0xc0,
        }
    }

    pub fn encode(self, value: u16) -> Vec<u8> {
        let ch = self.channel();
        let v = (value.min(127)) as u8;
        match self {
            Self::Controller { controller, .. } => vec![0xb0 | ch, controller, v],
            Self::PitchBend { .. } => {
                let value = value.min(16383);
                vec![0xe0 | ch, (value & 127) as u8, (value >> 7) as u8]
            }
            Self::ChannelPressure { .. } => vec![0xd0 | ch, v],
            Self::PolyPressure { pitch, .. } => vec![0xa0 | ch, pitch, v],
            Self::Program { .. } => vec![0xc0 | ch, v],
        }
    }

    /// Lane value of a matching event.
    pub fn decode(self, bytes: &[u8]) -> Option<u16> {
        if !self.matches(bytes) {
            return None;
        }
        match self {
            Self::PitchBend { .. } => {
                Some(u16::from(*bytes.get(1)?) | (u16::from(*bytes.get(2)?) << 7))
            }
            Self::Controller { .. } | Self::PolyPressure { .. } => {
                bytes.get(2).map(|v| u16::from(*v))
            }
            Self::ChannelPressure { .. } | Self::Program { .. } => {
                bytes.get(1).map(|v| u16::from(*v))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanePoint {
    #[ts(type = "number")]
    pub tick: Ticks,
    pub value: u16,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Whether `bytes` is a note-on or note-off message.
pub fn is_note_message(bytes: &[u8]) -> bool {
    bytes
        .first()
        .is_some_and(|s| matches!(s & 0xf0, 0x80 | 0x90) && *s < 0xf0)
}

/// Assigns ids 1, 2, … to notes in order. Used for notes from imports,
/// recordings, migrations and the demo.
pub fn with_ids(mut notes: Vec<MidiNote>) -> Vec<MidiNote> {
    for (i, n) in notes.iter_mut().enumerate() {
        n.id = u32::try_from(i + 1).unwrap_or(u32::MAX);
    }
    notes
}

fn sort_notes(notes: &mut [MidiNote]) {
    notes.sort_by_key(|n| (n.start, n.pitch, n.id));
}

/// Splits raw MIDI 1.0 events into notes (paired per channel and key, first
/// in first out, as SMF readers conventionally do) and the remaining controller/system events. Note-offs at
/// the region end close held notes; trailing all-notes-off/all-sound-off
/// closers are dropped because notes now end explicitly.
pub fn split_note_events(
    events: &[StoredMidiEvent],
    length: Ticks,
) -> (Vec<MidiNote>, Vec<StoredMidiEvent>) {
    let mut active: HashMap<(u8, u8), std::collections::VecDeque<(Ticks, u8)>> = HashMap::new();
    let mut notes = Vec::new();
    let mut rest = Vec::new();
    for e in events {
        let Some(&status) = e.bytes.first() else {
            continue;
        };
        if status < 0xf0 && matches!(status & 0xf0, 0x80 | 0x90) && e.bytes.len() == 3 {
            let ch = status & 0x0f;
            let key = e.bytes[1];
            if status & 0xf0 == 0x90 && e.bytes[2] > 0 {
                active
                    .entry((ch, key))
                    .or_default()
                    .push_back((e.tick, e.bytes[2]));
            } else if let Some((start, velocity)) = active
                .get_mut(&(ch, key))
                .and_then(std::collections::VecDeque::pop_front)
            {
                let mut n =
                    MidiNote::new(key, velocity, start, e.tick.saturating_sub(start).max(1));
                n.channel = ch;
                notes.push(n);
            }
            continue;
        }
        if status < 0xf0
            && status & 0xf0 == 0xb0
            && matches!(e.bytes.get(1), Some(120 | 123))
            && e.tick >= length
        {
            continue;
        }
        rest.push(e.clone());
    }
    for ((ch, key), held) in active {
        for (start, velocity) in held {
            let mut n = MidiNote::new(key, velocity, start, length.saturating_sub(start).max(1));
            n.channel = ch;
            notes.push(n);
        }
    }
    notes.sort_by_key(|n| (n.start, n.pitch, n.channel));
    (with_ids(notes), rest)
}

/// Schema 3 → 4: move note events of recorded/imported regions into notes
/// and give every note an id.
pub fn upgrade_regions(project: &mut Project) {
    for track in &mut project.tracks {
        for region in &mut track.regions {
            let RegionContent::Midi { notes } = &mut region.content else {
                continue;
            };
            if region.midi_events.iter().any(|e| is_note_message(&e.bytes)) {
                let (n, rest) = split_note_events(&region.midi_events, region.length);
                *notes = n;
                region.midi_events = rest;
            } else {
                let old = std::mem::take(notes);
                *notes = with_ids(old);
            }
        }
    }
}

fn midi_region_mut(project: &mut Project, id: RegionId) -> Result<&mut Region, EditError> {
    let (ti, ri) = project
        .find_region(id)
        .ok_or(EditError::UnknownRegion(id))?;
    let region = &mut project.tracks[ti].regions[ri];
    if !matches!(region.content, RegionContent::Midi { .. }) {
        return Err(EditError::Incompatible("this is not a MIDI region".into()));
    }
    Ok(region)
}

fn notes_of(region: &mut Region) -> &mut Vec<MidiNote> {
    match &mut region.content {
        RegionContent::Midi { notes } => notes,
        RegionContent::Audio { .. } => unreachable!("checked by midi_region_mut"),
    }
}

fn next_id(notes: &[MidiNote]) -> Result<u32, EditError> {
    notes
        .iter()
        .map(|n| n.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| EditError::Limit("note ids exhausted; duplicate the region".into()))
}

/// Grows the region to cover every note end (Logic-style auto-extend).
fn cover_notes(region: &mut Region) -> Result<(), EditError> {
    let end = match &region.content {
        RegionContent::Midi { notes } => notes.iter().map(MidiNote::end).max().unwrap_or(0),
        RegionContent::Audio { .. } => 0,
    };
    if region.start.saturating_add(end) > limits::MAX_TICKS {
        return Err(invalid("notes would end out of range"));
    }
    region.length = region.length.max(end);
    Ok(())
}

fn check_spec(n: &NoteSpec) -> Result<(), EditError> {
    if n.pitch > 127 || !(1..=127).contains(&n.velocity) || n.channel > 15 || n.length == 0 {
        return Err(invalid(
            "notes need pitch 0–127, velocity 1–127, channel 1–16 and a positive length",
        ));
    }
    Ok(())
}

fn select<'a>(
    notes: &'a mut [MidiNote],
    ids: &HashSet<u32>,
) -> impl Iterator<Item = &'a mut MidiNote> {
    notes.iter_mut().filter(move |n| ids.contains(&n.id))
}

fn id_set(notes: &[MidiNote], ids: &[u32]) -> Result<HashSet<u32>, EditError> {
    let set: HashSet<u32> = ids.iter().copied().collect();
    let present = notes.iter().filter(|n| set.contains(&n.id)).count();
    if set.is_empty() {
        return Err(invalid("select one or more notes"));
    }
    if present != set.len() {
        return Err(invalid("a selected note no longer exists"));
    }
    Ok(set)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub(crate) fn add_notes(
    project: &mut Project,
    region_id: RegionId,
    specs: &[NoteSpec],
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    if specs.is_empty() {
        return Err(invalid("no notes to add"));
    }
    let region = midi_region_mut(project, region_id)?;
    let notes = notes_of(region);
    if notes.len() + specs.len() > limits::MAX_NOTES_PER_REGION {
        return Err(EditError::Limit("too many notes in this region".into()));
    }
    let mut id = next_id(notes)?;
    for spec in specs {
        check_spec(spec)?;
        let mut n = MidiNote::new(spec.pitch, spec.velocity, spec.start, spec.length);
        n.channel = spec.channel;
        n.id = id;
        outcome.created_notes.push(id);
        id = id
            .checked_add(1)
            .ok_or_else(|| EditError::Limit("note ids exhausted".into()))?;
        notes.push(n);
    }
    sort_notes(notes);
    cover_notes(region)
}

pub(crate) fn remove_notes(
    project: &mut Project,
    region_id: RegionId,
    ids: &[u32],
) -> Result<(), EditError> {
    let notes = notes_of(midi_region_mut(project, region_id)?);
    let set = id_set(notes, ids)?;
    notes.retain(|n| !set.contains(&n.id));
    Ok(())
}

pub(crate) fn update_notes(
    project: &mut Project,
    region_id: RegionId,
    updates: &[MidiNote],
) -> Result<(), EditError> {
    let region = midi_region_mut(project, region_id)?;
    let notes = notes_of(region);
    let mut index: HashMap<u32, usize> = notes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    for u in updates {
        let i = index
            .remove(&u.id)
            .ok_or_else(|| invalid("a selected note no longer exists (or is listed twice)"))?;
        check_spec(&NoteSpec {
            pitch: u.pitch,
            velocity: u.velocity,
            channel: u.channel,
            start: u.start,
            length: u.length,
        })?;
        let mut n = *u;
        // Position edits made by hand clear stored quantisation unless the
        // start is unchanged.
        if n.start != notes[i].start {
            n.quantize_offset = 0;
        }
        notes[i] = n;
    }
    sort_notes(notes);
    cover_notes(region)
}

pub(crate) fn move_notes(
    project: &mut Project,
    region_id: RegionId,
    ids: &[u32],
    delta_ticks: i64,
    delta_pitch: i32,
    copy: bool,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    let region = midi_region_mut(project, region_id)?;
    let notes = notes_of(region);
    let set = id_set(notes, ids)?;
    let (mut min_start, mut lo, mut hi) = (Ticks::MAX, 127u8, 0u8);
    for n in notes.iter().filter(|n| set.contains(&n.id)) {
        min_start = min_start.min(n.start);
        lo = lo.min(n.pitch);
        hi = hi.max(n.pitch);
    }
    // The group keeps its shape: clamp at the region start and the MIDI range.
    let dt = delta_ticks.max(-(min_start.min(i64::MAX as u64) as i64));
    let dp = delta_pitch.clamp(-i32::from(lo), 127 - i32::from(hi));
    if dt == 0 && dp == 0 && !copy {
        return Ok(());
    }
    let shift = |n: &mut MidiNote| -> Result<(), EditError> {
        n.start = n
            .start
            .checked_add_signed(dt)
            .filter(|s| *s <= limits::MAX_TICKS)
            .ok_or_else(|| invalid("notes would move out of range"))?;
        n.pitch = (i32::from(n.pitch) + dp) as u8;
        Ok(())
    };
    if copy {
        if notes.len() + set.len() > limits::MAX_NOTES_PER_REGION {
            return Err(EditError::Limit("too many notes in this region".into()));
        }
        let mut id = next_id(notes)?;
        let mut copies = Vec::with_capacity(set.len());
        for n in notes.iter().filter(|n| set.contains(&n.id)) {
            let mut c = *n;
            shift(&mut c)?;
            c.id = id;
            outcome.created_notes.push(id);
            id = id
                .checked_add(1)
                .ok_or_else(|| EditError::Limit("note ids exhausted".into()))?;
            copies.push(c);
        }
        notes.extend(copies);
    } else {
        for n in select(notes, &set) {
            shift(n)?;
        }
    }
    sort_notes(notes);
    cover_notes(region)
}

pub(crate) fn resize_notes(
    project: &mut Project,
    region_id: RegionId,
    ids: &[u32],
    edge: NoteEdge,
    delta: i64,
) -> Result<(), EditError> {
    let region = midi_region_mut(project, region_id)?;
    let notes = notes_of(region);
    let set = id_set(notes, ids)?;
    for n in select(notes, &set) {
        match edge {
            NoteEdge::End => {
                let len = (n.length as i128 + i128::from(delta)).max(1);
                n.length = u64::try_from(len)
                    .ok()
                    .filter(|l| *l <= limits::MAX_TICKS)
                    .ok_or_else(|| invalid("note would be too long"))?;
            }
            NoteEdge::Start => {
                let end = n.end();
                let start = (n.start as i128 + i128::from(delta)).clamp(0, end as i128 - 1) as u64;
                n.start = start;
                n.length = end - start;
                n.quantize_offset = 0;
            }
        }
    }
    sort_notes(notes);
    cover_notes(region)
}

/// Small deterministic generator for humanise (xorshift64*).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15 | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in `[-range, range]`.
    fn offset(&mut self, range: u64) -> i64 {
        if range == 0 {
            return 0;
        }
        (self.next() % (2 * range + 1)) as i64 - range as i64
    }
}

pub(crate) fn transform_notes(
    project: &mut Project,
    targets: &[NoteTarget],
    transform: &NoteTransform,
) -> Result<(), EditError> {
    if targets.is_empty() {
        return Err(invalid("select one or more MIDI regions"));
    }
    let map = if matches!(transform, NoteTransform::Quantize { .. }) {
        Some(
            project
                .transport
                .tempo_map()
                .map_err(|e| invalid(e.to_string()))?,
        )
    } else {
        None
    };
    let mut seen = HashSet::new();
    for (index, target) in targets.iter().enumerate() {
        if !seen.insert(target.region_id) {
            return Err(invalid("a region is listed twice"));
        }
        let region = midi_region_mut(project, target.region_id)?;
        let region_start = region.start;
        let notes = notes_of(region);
        let set: HashSet<u32> = match &target.ids {
            Some(ids) => id_set(notes, ids)?,
            None => notes.iter().map(|n| n.id).collect(),
        };
        apply_transform(
            notes,
            &set,
            transform,
            region_start,
            map.as_ref(),
            index as u64,
        )?;
        sort_notes(notes);
        cover_notes(region)?;
    }
    Ok(())
}

fn apply_transform(
    notes: &mut [MidiNote],
    set: &HashSet<u32>,
    transform: &NoteTransform,
    region_start: Ticks,
    map: Option<&TempoMap>,
    salt: u64,
) -> Result<(), EditError> {
    match *transform {
        NoteTransform::Quantize { grid, strength } => {
            let map = map.ok_or_else(|| invalid("missing tempo map"))?;
            let strength = i128::from(strength.min(100));
            for n in select(notes, set) {
                let original = n.unquantized_start();
                let abs = region_start.saturating_add(original);
                let snapped = grid.snap(map, abs);
                let target = abs as i128 + (snapped as i128 - abs as i128) * strength / 100;
                let start = (target - region_start as i128).max(0) as u64;
                let offset = i32::try_from(start as i128 - original as i128)
                    .map_err(|_| invalid("quantise offset out of range"))?;
                n.start = start;
                n.quantize_offset = offset;
            }
        }
        NoteTransform::Unquantize => {
            for n in select(notes, set) {
                n.start = n.unquantized_start();
                n.quantize_offset = 0;
            }
        }
        NoteTransform::Humanize {
            timing,
            velocity,
            length,
            seed,
        } => {
            let mut rng = Rng::new(seed.wrapping_add(salt.wrapping_mul(0x1000_0000_01b3)));
            for n in select(notes, set) {
                let dt = rng.offset(u64::from(timing));
                let dv = rng.offset(u64::from(velocity));
                let dl = rng.offset(u64::from(length));
                n.start = n.start.saturating_add_signed(dt);
                n.velocity = (i64::from(n.velocity) + dv).clamp(1, 127) as u8;
                n.length = (n.length as i64 + dl).max(1) as u64;
            }
        }
        NoteTransform::Transpose { semitones } => {
            for n in notes.iter().filter(|n| set.contains(&n.id)) {
                if !(0..=127).contains(&(i32::from(n.pitch) + semitones)) {
                    return Err(invalid("the transposition would leave the MIDI note range"));
                }
            }
            for n in select(notes, set) {
                n.pitch = (i32::from(n.pitch) + semitones) as u8;
            }
        }
        NoteTransform::Legato { gap } => {
            let mut starts: Vec<Ticks> = notes
                .iter()
                .filter(|n| set.contains(&n.id))
                .map(|n| n.start)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            for n in select(notes, set) {
                let i = starts.partition_point(|&s| s <= n.start);
                if let Some(&next) = starts.get(i) {
                    n.length = (next - n.start).saturating_sub(u64::from(gap)).max(1);
                }
            }
        }
        NoteTransform::Velocity { mode, value } => {
            for n in select(notes, set) {
                let v = i64::from(n.velocity);
                let value = i64::from(value);
                let v = match mode {
                    VelocityMode::Set => value,
                    VelocityMode::Add => v + value,
                    VelocityMode::Scale => (v * value + 50) / 100,
                };
                n.velocity = v.clamp(1, 127) as u8;
            }
        }
        NoteTransform::Mute { muted } => {
            for n in select(notes, set) {
                n.muted = muted;
            }
        }
        NoteTransform::Channel { channel } => {
            if channel > 15 {
                return Err(invalid("MIDI channels are 1–16"));
            }
            for n in select(notes, set) {
                n.channel = channel;
            }
        }
        NoteTransform::FixedLength { length } => {
            if length == 0 || length > limits::MAX_TICKS {
                return Err(invalid("note length must be positive"));
            }
            for n in select(notes, set) {
                n.length = length;
            }
        }
    }
    Ok(())
}

/// Moves the lower voices of chords (notes starting together) to new tracks
/// below the source track. The highest voice stays in the region; voice k
/// goes to the k-th new track. Controller events stay with the source.
pub(crate) fn split_chord_voices(
    project: &mut Project,
    region_id: RegionId,
    ids: Option<&[u32]>,
    outcome: &mut EditOutcome,
) -> Result<(), EditError> {
    const MAX_VOICES: usize = 8;
    let (ti, ri) = project
        .find_region(region_id)
        .ok_or(EditError::UnknownRegion(region_id))?;
    let source = project.tracks[ti].regions[ri].clone();
    let RegionContent::Midi { notes } = &source.content else {
        return Err(EditError::Incompatible("this is not a MIDI region".into()));
    };
    let set: HashSet<u32> = match ids {
        Some(ids) => id_set(notes, ids)?,
        None => notes.iter().map(|n| n.id).collect(),
    };
    let mut chords: BTreeMap<Ticks, Vec<MidiNote>> = BTreeMap::new();
    for n in notes.iter().filter(|n| set.contains(&n.id)) {
        chords.entry(n.start).or_default().push(*n);
    }
    let mut voices: Vec<Vec<MidiNote>> = Vec::new();
    for chord in chords.values_mut() {
        chord.sort_by(|a, b| b.pitch.cmp(&a.pitch).then(a.id.cmp(&b.id)));
        for (k, n) in chord.iter().enumerate().skip(1) {
            let k = k.min(MAX_VOICES - 1);
            if voices.len() < k {
                voices.resize(k, Vec::new());
            }
            voices[k - 1].push(*n);
        }
    }
    if voices.is_empty() {
        return Err(invalid("the selection contains no chords"));
    }
    if project.tracks.len() + voices.len() > limits::MAX_TRACKS {
        return Err(EditError::Limit("too many tracks".into()));
    }
    let moved: HashSet<u32> = voices.iter().flatten().map(|n| n.id).collect();
    if let RegionContent::Midi { notes } = &mut project.tracks[ti].regions[ri].content {
        notes.retain(|n| !moved.contains(&n.id));
    }
    let src = &project.tracks[ti];
    let (kind, color, name, io, strip) = (
        src.kind,
        src.color,
        src.name.clone(),
        src.io.clone(),
        src.strip.clone(),
    );
    for (k, voice) in voices.into_iter().enumerate() {
        let mut track = Track::new(kind, voice_name(&name, k + 2), color);
        track.io = crate::io::TrackIo {
            armed: false,
            ..io.clone()
        };
        track.strip = crate::mixer::StripRouting {
            group: None,
            ..strip.clone()
        };
        let mut region = source.clone();
        region.id = RegionId::new();
        region.name = voice_name(&source.name, k + 2);
        region.midi_events = Vec::new();
        region.capture = None;
        let mut voice = voice;
        for n in &mut voice {
            n.quantize_offset = n.quantize_offset.clamp(i32::MIN, i32::MAX);
        }
        sort_notes(&mut voice);
        region.content = RegionContent::Midi { notes: voice };
        outcome.created_tracks.push(track.id);
        outcome.created_regions.push(region.id);
        track.regions.push(region);
        project.tracks.insert(ti + 1 + k, track);
    }
    debug_assert!(matches!(kind, TrackKind::Instrument | TrackKind::Midi));
    Ok(())
}

fn voice_name(base: &str, voice: usize) -> String {
    let suffix = format!(" · voice {voice}");
    let keep = crate::validate::limits::MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    format!("{}{suffix}", base.chars().take(keep).collect::<String>())
}

pub(crate) fn edit_lane(
    project: &mut Project,
    region_id: RegionId,
    lane: ControllerLane,
    from: Ticks,
    to: Ticks,
    points: &[LanePoint],
) -> Result<(), EditError> {
    if lane.channel() > 15
        || matches!(lane, ControllerLane::Controller { controller, .. } if controller > 119)
        || matches!(lane, ControllerLane::PolyPressure { pitch, .. } if pitch > 127)
    {
        return Err(invalid(
            "lanes need a channel 1–16, a controller 0–119 and a pitch 0–127",
        ));
    }
    if from > to {
        return Err(invalid("the lane range is reversed"));
    }
    let region = midi_region_mut(project, region_id)?;
    if points.len() > 16_384 {
        return Err(EditError::Limit("too many lane points".into()));
    }
    for p in points {
        if p.value > lane.max_value() || p.tick > region.length {
            return Err(invalid(format!(
                "lane values must be 0–{} and inside the region",
                lane.max_value()
            )));
        }
    }
    region
        .midi_events
        .retain(|e| !(lane.matches(&e.bytes) && e.tick >= from && e.tick < to));
    if region.midi_events.len() + points.len() > limits::MAX_NOTES_PER_REGION {
        return Err(EditError::Limit(
            "too many MIDI events in this region".into(),
        ));
    }
    region
        .midi_events
        .extend(points.iter().map(|p| StoredMidiEvent {
            tick: p.tick,
            bytes: lane.encode(p.value),
        }));
    // Stable: equal ticks keep their recorded order.
    region.midi_events.sort_by_key(|e| e.tick);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{EditCommand, apply};
    use elyvon_core::grid::GridDivision;

    fn region_project() -> (Project, RegionId) {
        let mut p = Project::new("P");
        let t = apply(
            &mut p,
            &EditCommand::AddTrack {
                kind: TrackKind::Instrument,
                name: None,
                index: None,
            },
        )
        .unwrap()
        .created_tracks[0];
        let r = apply(
            &mut p,
            &EditCommand::CreateMidiRegion {
                track_id: t,
                start: 3840,
                length: 3840,
            },
        )
        .unwrap()
        .created_regions[0];
        (p, r)
    }

    fn notes(p: &Project, r: RegionId) -> Vec<MidiNote> {
        let (ti, ri) = p.find_region(r).unwrap();
        match &p.tracks[ti].regions[ri].content {
            RegionContent::Midi { notes } => notes.clone(),
            _ => unreachable!(),
        }
    }

    fn spec(pitch: u8, start: Ticks, length: Ticks) -> NoteSpec {
        NoteSpec {
            pitch,
            velocity: 100,
            channel: 0,
            start,
            length,
        }
    }

    #[test]
    fn add_move_resize_copy_and_remove_notes() {
        let (mut p, r) = region_project();
        let out = apply(
            &mut p,
            &EditCommand::AddNotes {
                region_id: r,
                notes: vec![spec(60, 0, 480), spec(64, 480, 480)],
            },
        )
        .unwrap();
        assert_eq!(out.created_notes, vec![1, 2]);
        apply(
            &mut p,
            &EditCommand::MoveNotes {
                region_id: r,
                ids: vec![1, 2],
                delta_ticks: -1000,
                delta_pitch: 100,
                copy: false,
            },
        )
        .unwrap();
        let n = notes(&p, r);
        // Clamped as a group: earliest stays at 0, highest at 127.
        assert_eq!((n[0].start, n[0].pitch), (0, 123));
        assert_eq!((n[1].start, n[1].pitch), (480, 127));
        let out = apply(
            &mut p,
            &EditCommand::MoveNotes {
                region_id: r,
                ids: vec![1],
                delta_ticks: 3840,
                delta_pitch: -12,
                copy: true,
            },
        )
        .unwrap();
        assert_eq!(out.created_notes, vec![3]);
        let n = notes(&p, r);
        assert_eq!(n.len(), 3);
        // The copy extends the region past its original length.
        let (ti, ri) = p.find_region(r).unwrap();
        assert_eq!(p.tracks[ti].regions[ri].length, 3840 + 480);
        apply(
            &mut p,
            &EditCommand::ResizeNotes {
                region_id: r,
                ids: vec![2],
                edge: NoteEdge::Start,
                delta: 10_000,
            },
        )
        .unwrap();
        let two = notes(&p, r).into_iter().find(|n| n.id == 2).unwrap();
        assert_eq!((two.start, two.length), (959, 1));
        apply(
            &mut p,
            &EditCommand::RemoveNotes {
                region_id: r,
                ids: vec![1, 3],
            },
        )
        .unwrap();
        assert_eq!(notes(&p, r).len(), 1);
        assert!(
            apply(
                &mut p,
                &EditCommand::RemoveNotes {
                    region_id: r,
                    ids: vec![1]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn quantize_is_reversible_and_follows_metre_changes() {
        let (mut p, r) = region_project();
        // Bar 2 (where the region starts) is in 7/8: eighths are 480 ticks.
        p.transport.meter_changes = vec![elyvon_core::MeterChange {
            bar: 1,
            time_signature: elyvon_core::TimeSignature {
                numerator: 7,
                denominator: 8,
            },
        }];
        apply(
            &mut p,
            &EditCommand::AddNotes {
                region_id: r,
                notes: vec![spec(60, 470, 100), spec(62, 1000, 100)],
            },
        )
        .unwrap();
        let grid = GridSpec {
            division: GridDivision::Eighth,
            triplet: false,
            swing: 0,
        };
        let q = |strength| EditCommand::TransformNotes {
            targets: vec![NoteTarget {
                region_id: r,
                ids: None,
            }],
            transform: NoteTransform::Quantize { grid, strength },
        };
        apply(&mut p, &q(100)).unwrap();
        let n = notes(&p, r);
        assert_eq!(n.iter().map(|n| n.start).collect::<Vec<_>>(), [480, 960]);
        assert_eq!(n[0].unquantized_start(), 470);
        // Re-quantising at 50 % starts from the played position again.
        apply(&mut p, &q(50)).unwrap();
        assert_eq!(
            notes(&p, r).iter().map(|n| n.start).collect::<Vec<_>>(),
            [475, 980]
        );
        apply(
            &mut p,
            &EditCommand::TransformNotes {
                targets: vec![NoteTarget {
                    region_id: r,
                    ids: None,
                }],
                transform: NoteTransform::Unquantize,
            },
        )
        .unwrap();
        let n = notes(&p, r);
        assert_eq!(n.iter().map(|n| n.start).collect::<Vec<_>>(), [470, 1000]);
        assert!(n.iter().all(|n| n.quantize_offset == 0));
    }

    #[test]
    fn triplet_and_swing_quantize() {
        let (mut p, r) = region_project();
        apply(
            &mut p,
            &EditCommand::AddNotes {
                region_id: r,
                notes: vec![spec(60, 300, 10), spec(60, 600, 10)],
            },
        )
        .unwrap();
        let grid = |triplet, swing| GridSpec {
            division: GridDivision::Eighth,
            triplet,
            swing,
        };
        apply(
            &mut p,
            &EditCommand::TransformNotes {
                targets: vec![NoteTarget {
                    region_id: r,
                    ids: Some(vec![1]),
                }],
                transform: NoteTransform::Quantize {
                    grid: grid(true, 0),
                    strength: 100,
                },
            },
        )
        .unwrap();
        assert!(notes(&p, r).iter().any(|n| n.id == 1 && n.start == 320));
        apply(
            &mut p,
            &EditCommand::TransformNotes {
                targets: vec![NoteTarget {
                    region_id: r,
                    ids: Some(vec![2]),
                }],
                transform: NoteTransform::Quantize {
                    grid: grid(false, 60),
                    strength: 100,
                },
            },
        )
        .unwrap();
        // Off-beat eighth with 60 % swing: 480 + 480·60/300 = 576.
        assert!(notes(&p, r).iter().any(|n| n.id == 2 && n.start == 576));
    }

    #[test]
    fn transforms_validate_and_apply() {
        let (mut p, r) = region_project();
        apply(
            &mut p,
            &EditCommand::AddNotes {
                region_id: r,
                notes: vec![spec(60, 0, 100), spec(64, 0, 100), spec(67, 960, 100)],
            },
        )
        .unwrap();
        let t = |transform| EditCommand::TransformNotes {
            targets: vec![NoteTarget {
                region_id: r,
                ids: None,
            }],
            transform,
        };
        assert!(apply(&mut p, &t(NoteTransform::Transpose { semitones: 61 })).is_err());
        apply(&mut p, &t(NoteTransform::Transpose { semitones: 12 })).unwrap();
        apply(&mut p, &t(NoteTransform::Legato { gap: 10 })).unwrap();
        let n = notes(&p, r);
        assert_eq!(n.iter().map(|n| n.pitch).collect::<Vec<_>>(), [72, 76, 79]);
        assert_eq!(
            n.iter().map(|n| n.length).collect::<Vec<_>>(),
            [950, 950, 100]
        );
        apply(
            &mut p,
            &t(NoteTransform::Velocity {
                mode: VelocityMode::Scale,
                value: 200,
            }),
        )
        .unwrap();
        assert!(notes(&p, r).iter().all(|n| n.velocity == 127));
        let before = notes(&p, r);
        let human = t(NoteTransform::Humanize {
            timing: 20,
            velocity: 10,
            length: 5,
            seed: 7,
        });
        apply(&mut p, &human).unwrap();
        let once = notes(&p, r);
        assert_ne!(once, before);
        let mut again = p.clone();
        let (ti, ri) = again.find_region(r).unwrap();
        again.tracks[ti].regions[ri].content = RegionContent::Midi { notes: before };
        apply(&mut again, &human).unwrap();
        assert_eq!(notes(&again, r), once, "humanise is deterministic per seed");
        apply(&mut p, &t(NoteTransform::Mute { muted: true })).unwrap();
        assert!(notes(&p, r).iter().all(|n| n.muted));
        assert!(apply(&mut p, &t(NoteTransform::Channel { channel: 16 })).is_err());
    }

    #[test]
    fn chord_voices_move_to_new_tracks() {
        let (mut p, r) = region_project();
        apply(
            &mut p,
            &EditCommand::AddNotes {
                region_id: r,
                notes: vec![
                    spec(60, 0, 480),
                    spec(64, 0, 480),
                    spec(67, 0, 480),
                    spec(62, 960, 480),
                ],
            },
        )
        .unwrap();
        let out = apply(
            &mut p,
            &EditCommand::SplitChordVoices {
                region_id: r,
                ids: None,
            },
        )
        .unwrap();
        assert_eq!(out.created_tracks.len(), 2);
        let top: Vec<u8> = notes(&p, r).iter().map(|n| n.pitch).collect();
        assert_eq!(top, [67, 62]);
        assert_eq!(notes(&p, out.created_regions[0])[0].pitch, 64);
        assert_eq!(notes(&p, out.created_regions[1])[0].pitch, 60);
        assert_eq!(p.tracks[1].id, out.created_tracks[0]);
    }

    #[test]
    fn controller_lanes_replace_a_range() {
        let (mut p, r) = region_project();
        let lane = ControllerLane::Controller {
            channel: 0,
            controller: 64,
        };
        let pts = |v: &[(u64, u16)]| {
            v.iter()
                .map(|&(tick, value)| LanePoint { tick, value })
                .collect::<Vec<_>>()
        };
        let edit = |from, to, points| EditCommand::EditControllerLane {
            region_id: r,
            lane,
            from,
            to,
            points,
        };
        apply(&mut p, &edit(0, 3840, pts(&[(0, 127), (960, 0)]))).unwrap();
        apply(
            &mut p,
            &EditCommand::EditControllerLane {
                region_id: r,
                lane: ControllerLane::PitchBend { channel: 0 },
                from: 0,
                to: 3840,
                points: pts(&[(480, 16383), (1440, 8192)]),
            },
        )
        .unwrap();
        apply(&mut p, &edit(900, 1000, pts(&[(950, 64)]))).unwrap();
        let (ti, ri) = p.find_region(r).unwrap();
        let events = &p.tracks[ti].regions[ri].midi_events;
        let cc: Vec<_> = events
            .iter()
            .filter_map(|e| lane.decode(&e.bytes).map(|v| (e.tick, v)))
            .collect();
        assert_eq!(cc, [(0, 127), (950, 64)]);
        let bend: Vec<_> = events
            .iter()
            .filter_map(|e| {
                ControllerLane::PitchBend { channel: 0 }
                    .decode(&e.bytes)
                    .map(|v| (e.tick, v))
            })
            .collect();
        assert_eq!(bend, [(480, 16383), (1440, 8192)]);
        assert!(apply(&mut p, &edit(0, 10, pts(&[(0, 128)]))).is_err());
    }

    #[test]
    fn raw_note_events_become_notes() {
        let ev = |tick, bytes: &[u8]| StoredMidiEvent {
            tick,
            bytes: bytes.to_vec(),
        };
        let events = vec![
            ev(0, &[0x91, 60, 90]),
            ev(0, &[0xb1, 64, 127]),
            ev(100, &[0x91, 60, 80]),
            ev(200, &[0x81, 60, 0]),
            ev(300, &[0x91, 60, 0]),
            ev(400, &[0x92, 72, 50]),
            ev(500, &[0xb2, 123, 0]),
        ];
        let (notes, rest) = split_note_events(&events, 500);
        // Same key overlaps pair first-in first-out; held notes end at 500.
        let got: Vec<_> = notes
            .iter()
            .map(|n| (n.channel, n.pitch, n.velocity, n.start, n.length))
            .collect();
        assert_eq!(
            got,
            [
                (1, 60, 90, 0, 200),
                (1, 60, 80, 100, 200),
                (2, 72, 50, 400, 100)
            ]
        );
        assert_eq!(rest, vec![ev(0, &[0xb1, 64, 127])]);
        assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), [1, 2, 3]);
    }
}
