//! Persisted input/routing choices. Availability is checked at runtime; an
//! absent named port never silently connects to a different physical port.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct TrackIo {
    pub armed: bool,
    /// Zero-based first input channel; one or two contiguous channels.
    pub input_channel: u16,
    pub input_channels: u16,
    pub monitor: bool,
    pub midi_input: Option<String>,
    pub midi_output: Option<String>,
    pub midi_channel: Option<u8>,
    pub output_channel: Option<u8>,
    pub midi_monitor: bool,
}
impl Default for TrackIo {
    fn default() -> Self {
        Self {
            armed: false,
            input_channel: 0,
            input_channels: 1,
            monitor: false,
            midi_input: None,
            midi_output: None,
            midi_channel: None,
            output_channel: None,
            midi_monitor: true,
        }
    }
}
impl TrackIo {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=2).contains(&self.input_channels)
            || self.input_channel.saturating_add(self.input_channels) > 32
        {
            return Err("choose one or two input channels within 1–32");
        }
        if self.output_channel.is_some_and(|ch| ch > 15)
            || self.midi_channel.is_some_and(|ch| ch > 15)
        {
            return Err("MIDI channels must be 1–16");
        }
        if [&self.midi_input, &self.midi_output]
            .into_iter()
            .flatten()
            .any(|s| s.is_empty() || s.len() > 1024 || s.chars().any(char::is_control))
        {
            return Err("invalid MIDI port identifier");
        }
        Ok(())
    }
}

/// Raw MIDI source events are separate from the piano-roll note projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoredMidiEvent {
    #[ts(type = "number")]
    pub tick: u64,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureMetadata {
    pub folder: crate::RegionId,
    pub take: u32,
    #[ts(type = "number")]
    pub recorded_at: u64,
}

pub fn validate_midi_bytes(bytes: &[u8]) -> bool {
    let Some(&status) = bytes.first() else {
        return false;
    };
    if status == 0xf0 {
        return (2..=256).contains(&bytes.len())
            && bytes.last() == Some(&0xf7)
            && bytes[1..bytes.len() - 1].iter().all(|b| *b < 128);
    }
    let n = match status {
        0x80..=0xbf | 0xe0..=0xef => 3,
        0xc0..=0xdf => 2,
        0xf1 | 0xf3 => 2,
        0xf2 => 3,
        0xf6 | 0xf8 | 0xfa..=0xfc | 0xfe | 0xff => 1,
        _ => return false,
    };
    bytes.len() == n && bytes[1..].iter().all(|b| *b < 128)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiImportRegion {
    pub name: String,
    pub track_id: Option<crate::TrackId>,
    #[ts(type = "number")]
    pub start: u64,
    #[ts(type = "number")]
    pub length: u64,
    pub notes: Vec<crate::MidiNote>,
    pub events: Vec<StoredMidiEvent>,
}

/// Non-destructive MIDI crop. Chase held keys and channel state at the left
/// edge, release them at the right edge, and leave SysEx outside the range out.
pub fn crop_midi(events: &[StoredMidiEvent], start: u64, end: u64) -> Vec<StoredMidiEvent> {
    use std::collections::BTreeMap;
    let mut state = BTreeMap::<(u8, u8), Vec<u8>>::new();
    let mut held = BTreeMap::<(u8, u8), u8>::new();
    let mut sustained = BTreeMap::<(u8, u8), u8>::new();
    let mut pedal = [false; 16];
    let update = |bytes: &[u8],
                  held: &mut BTreeMap<(u8, u8), u8>,
                  sustained: &mut BTreeMap<(u8, u8), u8>,
                  pedal: &mut [bool; 16]| {
        if bytes.len() < 2 || bytes[0] >= 0xf0 {
            return;
        }
        let ch = bytes[0] & 15;
        let key = bytes[1];
        match bytes[0] & 0xf0 {
            0x90 if bytes[2] > 0 => {
                held.insert((ch, key), bytes[2]);
                sustained.remove(&(ch, key));
            }
            0x80 | 0x90 => {
                if let Some(v) = held.remove(&(ch, key))
                    && pedal[ch as usize]
                {
                    sustained.insert((ch, key), v);
                }
            }
            0xb0 if key == 64 => {
                pedal[ch as usize] = bytes[2] >= 64;
                if !pedal[ch as usize] {
                    sustained.retain(|(c, _), _| *c != ch);
                }
            }
            0xb0 if key == 120 || key == 123 => {
                held.retain(|(c, _), _| *c != ch);
                sustained.retain(|(c, _), _| *c != ch);
            }
            _ => {}
        }
    };
    for e in events.iter().take_while(|e| e.tick < start) {
        update(&e.bytes, &mut held, &mut sustained, &mut pedal);
        if let Some(&status) = e.bytes.first()
            && (0xa0..=0xef).contains(&status)
            && !(status & 0xf0 == 0xb0 && matches!(e.bytes[1], 120 | 123))
        {
            let key = if status & 0xf0 == 0xb0 || status & 0xf0 == 0xa0 {
                e.bytes[1]
            } else {
                0
            };
            state.insert((status, key), e.bytes.clone());
        }
    }
    let mut out: Vec<_> = state
        .into_values()
        .map(|bytes| StoredMidiEvent { tick: 0, bytes })
        .collect();
    for (&(ch, key), &v) in &held {
        out.push(StoredMidiEvent {
            tick: 0,
            bytes: vec![0x90 | ch, key, v],
        });
    }
    for (&(ch, key), &v) in &sustained {
        out.push(StoredMidiEvent {
            tick: 0,
            bytes: vec![0x90 | ch, key, v],
        });
        out.push(StoredMidiEvent {
            tick: 0,
            bytes: vec![0x80 | ch, key, 0],
        });
    }
    for e in events.iter().filter(|e| e.tick >= start && e.tick <= end) {
        let release = e.bytes.first().is_some_and(|b| {
            b & 0xf0 == 0x80
                || (b & 0xf0 == 0x90 && e.bytes.get(2) == Some(&0))
                || (b & 0xf0 == 0xb0 && e.bytes.get(1) == Some(&64) && e.bytes.get(2) == Some(&0))
        });
        if e.tick == end && !release {
            continue;
        }
        update(&e.bytes, &mut held, &mut sustained, &mut pedal);
        out.push(StoredMidiEvent {
            tick: e.tick - start,
            bytes: e.bytes.clone(),
        });
    }
    let length = end.saturating_sub(start);
    for &(ch, key) in held.keys() {
        out.push(StoredMidiEvent {
            tick: length,
            bytes: vec![0x80 | ch, key, 0],
        });
    }
    for ch in 0u8..16 {
        if pedal[ch as usize] {
            out.push(StoredMidiEvent {
                tick: length,
                bytes: vec![0xb0 | ch, 64, 0],
            });
        }
    }
    out
}
