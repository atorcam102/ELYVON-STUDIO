//! SMF type 0/1 import and type 1 export. Musical ticks use PPQ 960.
//! SMPTE division, type 2 and fragmented SysEx are rejected explicitly.
use super::Packet;
use elyvon_core::{MeterChange, PPQ, Tempo, TempoChange, TempoMap, TimeSignature};
use elyvon_project::{
    MidiNote, Project, RegionContent,
    io::{MidiImportRegion, StoredMidiEvent},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiFile {
    pub regions: Vec<MidiImportRegion>,
    pub tempo: Tempo,
    pub tempo_changes: Vec<TempoChange>,
    pub time_signature: TimeSignature,
    pub meter_changes: Vec<MeterChange>,
}
const MAX_BYTES: usize = 16 << 20;
const MAX_EVENTS: usize = 100000;
fn clean_name(s: &[u8], fallback: &str) -> String {
    let s: String = String::from_utf8_lossy(s)
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect();
    if s.trim().is_empty() {
        fallback.into()
    } else {
        s
    }
}
pub fn decode(bytes: &[u8]) -> Result<MidiFile, String> {
    if bytes.len() > MAX_BYTES {
        return Err("MIDI file exceeds 16 MiB".into());
    }
    // midly is deliberately tolerant; enforce complete chunk envelopes here
    // so a truncated download cannot be imported as a shorter successful song.
    if bytes.len() < 14 || &bytes[..4] != b"MThd" {
        return Err("invalid SMF header".into());
    }
    let header =
        u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| "invalid SMF header")?) as usize;
    if header < 6 || header > bytes.len() - 8 {
        return Err("truncated SMF header".into());
    }
    let tracks = u16::from_be_bytes([bytes[10], bytes[11]]) as usize;
    let mut at = 8 + header;
    for _ in 0..tracks {
        if at > bytes.len() || bytes.len() - at < 8 || &bytes[at..at + 4] != b"MTrk" {
            return Err("missing SMF track chunk".into());
        }
        let len = u32::from_be_bytes(
            bytes[at + 4..at + 8]
                .try_into()
                .map_err(|_| "invalid SMF track length")?,
        ) as usize;
        at = at
            .checked_add(8)
            .and_then(|a| a.checked_add(len))
            .filter(|a| *a <= bytes.len())
            .ok_or("truncated SMF track")?;
    }
    if at != bytes.len() {
        return Err("unexpected bytes after SMF tracks".into());
    }
    let smf = midly::Smf::parse(bytes).map_err(|e| e.to_string())?;
    if smf.header.format == midly::Format::Sequential {
        return Err("SMF type 2 is not supported".into());
    }
    let ppq = match smf.header.timing {
        midly::Timing::Metrical(p) if p.as_int() > 0 => u64::from(p.as_int()),
        _ => return Err("SMPTE/timecode MIDI division is not supported".into()),
    };
    if smf.tracks.len() > 512 {
        return Err("MIDI has too many tracks".into());
    }
    let scale = |t: u64| -> Result<u64, String> {
        let tick = (u128::from(t) * u128::from(PPQ) + u128::from(ppq / 2)) / u128::from(ppq);
        if tick > u128::from(elyvon_project::validate::limits::MAX_TICKS) {
            Err("MIDI exceeds the timeline length limit".into())
        } else {
            Ok(tick as u64)
        }
    };
    let mut tempos = BTreeMap::new();
    let mut meters = BTreeMap::new();
    let mut regions = Vec::new();
    let mut count = 0;
    for (index, track) in smf.tracks.iter().enumerate() {
        let mut absolute = 0u64;
        let mut events = Vec::new();
        let mut name = format!("MIDI {}", index + 1);
        for event in track {
            count += 1;
            if count > MAX_EVENTS {
                return Err("MIDI exceeds 100000 events".into());
            }
            absolute = absolute
                .checked_add(u64::from(event.delta.as_int()))
                .ok_or("MIDI time overflow")?;
            let tick = scale(absolute)?;
            use midly::{MetaMessage as M, MidiMessage as Msg, TrackEventKind as K};
            let bytes = match event.kind {
                K::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        Msg::NoteOn { key, vel } => vec![0x90 | ch, key.as_int(), vel.as_int()],
                        Msg::NoteOff { key, vel } => vec![0x80 | ch, key.as_int(), vel.as_int()],
                        Msg::Aftertouch { key, vel } => vec![0xa0 | ch, key.as_int(), vel.as_int()],
                        Msg::Controller { controller, value } => {
                            vec![0xb0 | ch, controller.as_int(), value.as_int()]
                        }
                        Msg::ProgramChange { program } => vec![0xc0 | ch, program.as_int()],
                        Msg::ChannelAftertouch { vel } => vec![0xd0 | ch, vel.as_int()],
                        Msg::PitchBend { bend } => {
                            let v = bend.0.as_int();
                            vec![0xe0 | ch, (v & 127) as u8, (v >> 7) as u8]
                        }
                    }
                }
                K::SysEx(data) => {
                    let mut b = vec![0xf0];
                    b.extend_from_slice(data);
                    if Packet::parse(&b).is_none() {
                        return Err(
                            "SysEx must be a complete MIDI 1.0 packet of at most 256 bytes".into(),
                        );
                    }
                    b
                }
                K::Escape(data) => {
                    if Packet::parse(data).is_none() {
                        return Err("unsupported SMF escape / fragmented SysEx packet".into());
                    }
                    data.to_vec()
                }
                K::Meta(M::Tempo(us)) => {
                    let n = us.as_int();
                    if n == 0 {
                        return Err("zero MIDI tempo".into());
                    }
                    let bpm = Tempo::new(60_000_000.0 / f64::from(n))
                        .ok_or("MIDI tempo is outside 20–999 BPM")?;
                    tempos.insert(tick, bpm);
                    continue;
                }
                K::Meta(M::TimeSignature(n, d, _, _)) => {
                    let denominator = 1u8
                        .checked_shl(u32::from(d))
                        .ok_or("unsupported MIDI metre")?;
                    let ts = TimeSignature {
                        numerator: n,
                        denominator,
                    };
                    if !ts.is_valid() {
                        return Err("unsupported MIDI metre".into());
                    }
                    meters.insert(tick, ts);
                    continue;
                }
                K::Meta(M::TrackName(text)) => {
                    name = clean_name(text, &name);
                    continue;
                }
                _ => continue,
            };
            events.push(StoredMidiEvent { tick, bytes });
        }
        if !events.is_empty() {
            let length =
                scale(absolute)?.max(events.last().map_or(1, |e| e.tick.saturating_add(1)));
            close_sequence(&mut events, length);
            let notes = notes_from_events(&events, length);
            regions.push(MidiImportRegion {
                name,
                track_id: None,
                start: 0,
                length,
                notes,
                events,
            });
        }
    }
    if regions.is_empty() {
        return Err("the MIDI file contains no playable events".into());
    }
    if tempos.len() > 4096 || meters.len() > 4096 {
        return Err("too many MIDI tempo/metre changes".into());
    }
    let tempo = tempos.remove(&0).unwrap_or(Tempo::DEFAULT);
    let time_signature = meters.remove(&0).unwrap_or_default();
    let tempo_changes: Vec<_> = tempos
        .into_iter()
        .map(|(tick, bpm)| TempoChange { tick, bpm })
        .collect();
    let mut meter_changes = Vec::new();
    for (tick, ts) in meters {
        let map = TempoMap::new(tempo, &tempo_changes, time_signature, &meter_changes)
            .map_err(|e| e.to_string())?;
        let bar = map.bar_at(tick);
        if tick != bar.start {
            return Err("a MIDI metre change is inside a bar; Elyvon currently requires bar-line metre changes".into());
        }
        meter_changes.push(MeterChange {
            bar: u32::try_from(bar.bar).map_err(|_| "MIDI bar overflow")?,
            time_signature: ts,
        });
    }
    Ok(MidiFile {
        regions,
        tempo,
        tempo_changes,
        time_signature,
        meter_changes,
    })
}
/// Note projection of raw events (pairs note on/off per channel and key).
/// `ImportMidi` performs the same split, so imported notes are editable.
pub fn notes_from_events(events: &[StoredMidiEvent], end: u64) -> Vec<MidiNote> {
    elyvon_project::midi_edit::split_note_events(events, end).0
}
fn vlq(mut value: u64, out: &mut Vec<u8>) -> Result<(), String> {
    if value > 0x0fff_ffff {
        return Err("SMF delta exceeds 28-bit limit".into());
    }
    let mut bytes = [0u8; 4];
    let mut at = 3;
    bytes[at] = (value & 127) as u8;
    value >>= 7;
    while value > 0 {
        at -= 1;
        bytes[at] = ((value & 127) as u8) | 128;
        value >>= 7;
    }
    out.extend_from_slice(&bytes[at..]);
    Ok(())
}
fn track_chunk(events: &mut [(u64, Vec<u8>)], out: &mut Vec<u8>) -> Result<(), String> {
    events.sort_by_key(|e| e.0);
    let mut data = Vec::new();
    let mut previous = 0;
    for (tick, bytes) in events.iter() {
        vlq(tick.saturating_sub(previous), &mut data)?;
        data.extend_from_slice(bytes);
        previous = *tick;
    }
    data.extend_from_slice(&[0, 0xff, 0x2f, 0]);
    let len = u32::try_from(data.len()).map_err(|_| "MIDI track too large")?;
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend(data);
    Ok(())
}
pub fn encode(project: &Project) -> Result<Vec<u8>, String> {
    elyvon_project::validate::validate(project).map_err(|e| e.to_string())?;
    let tracks: Vec<_> = project
        .tracks
        .iter()
        .filter(|t| t.kind != elyvon_project::TrackKind::Audio)
        .collect();
    if tracks.is_empty() {
        return Err("the project has no MIDI tracks".into());
    }
    let mut out = b"MThd\0\0\0\x06\0\x01".to_vec();
    out.extend_from_slice(&((tracks.len() + 1) as u16).to_be_bytes());
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    let mut conductor = Vec::new();
    let t = &project.transport;
    for (tick, tempo) in
        std::iter::once((0, t.tempo)).chain(t.tempo_changes.iter().map(|c| (c.tick, c.bpm)))
    {
        let us = (60_000_000.0 / tempo.bpm()).round() as u32;
        conductor.push((
            tick,
            vec![0xff, 0x51, 3, (us >> 16) as u8, (us >> 8) as u8, us as u8],
        ));
    }
    let map = t.tempo_map().map_err(|e| e.to_string())?;
    for (tick, ts) in std::iter::once((0, t.time_signature)).chain(
        t.meter_changes
            .iter()
            .map(|c| (map.bar_start(u64::from(c.bar)), c.time_signature)),
    ) {
        conductor.push((
            tick,
            vec![
                0xff,
                0x58,
                4,
                ts.numerator,
                ts.denominator.trailing_zeros() as u8,
                24,
                8,
            ],
        ));
    }
    track_chunk(&mut conductor, &mut out)?;
    for track in tracks {
        let mut name = vec![0xff, 3];
        vlq(track.name.len() as u64, &mut name)?;
        name.extend_from_slice(track.name.as_bytes());
        let mut events = vec![(0, name)];
        // (tick, order, bytes): at one tick controllers come first, then
        // note-offs, then note-ons, so repeated notes are never cut short.
        let mut timed: Vec<(u64, u8, Vec<u8>)> = Vec::new();
        for region in track.regions.iter().filter(|r| !r.muted) {
            if let RegionContent::Midi { notes } = &region.content {
                for n in notes.iter().filter(|n| !n.muted && n.start < region.length) {
                    let ch = n.channel & 0x0f;
                    let end = n.end().min(region.length);
                    timed.push((
                        region.start + n.start,
                        2,
                        vec![0x90 | ch, n.pitch, n.velocity],
                    ));
                    timed.push((region.start + end, 1, vec![0x80 | ch, n.pitch, 0]));
                }
            }
            for e in region
                .midi_events
                .iter()
                .filter(|e| e.tick <= region.length)
            {
                let Some(packet) = Packet::parse(&e.bytes) else {
                    return Err("invalid stored MIDI packet".into());
                };
                let bytes = if packet.data[0] == 0xf0 {
                    let mut b = vec![0xf0];
                    vlq(packet.len as u64 - 1, &mut b)?;
                    b.extend_from_slice(&packet.bytes()[1..]);
                    b
                } else if packet.data[0] >= 0xf0 {
                    let mut b = vec![0xf7];
                    vlq(u64::from(packet.len), &mut b)?;
                    b.extend_from_slice(packet.bytes());
                    b
                } else {
                    packet.bytes().to_vec()
                };
                timed.push((region.start + e.tick, 0, bytes));
            }
        }
        timed.sort_by_key(|e| (e.0, e.1));
        events.extend(timed.into_iter().map(|(tick, _, bytes)| (tick, bytes)));
        track_chunk(&mut events, &mut out)?;
    }
    if out.len() > MAX_BYTES {
        return Err("MIDI export exceeds 16 MiB".into());
    }
    Ok(out)
}

/// End held notes and release sustain at a region boundary. Existing complete
/// sequences remain byte-for-byte unchanged (including channel and order).
pub fn close_sequence(events: &mut Vec<StoredMidiEvent>, end: u64) {
    let mut active = [[0u16; 128]; 16];
    let mut sustain = [false; 16];
    for e in events.iter() {
        let Some(p) = Packet::parse(&e.bytes) else {
            continue;
        };
        let Some(ch) = p.channel() else {
            continue;
        };
        let ch = ch as usize;
        let key = p.data[1] as usize;
        match p.data[0] & 0xf0 {
            0x90 if p.data[2] > 0 => active[ch][key] = active[ch][key].saturating_add(1),
            0x80 | 0x90 => active[ch][key] = active[ch][key].saturating_sub(1),
            0xb0 if p.data[1] == 64 => sustain[ch] = p.data[2] >= 64,
            0xb0 if matches!(p.data[1], 120 | 123) => active[ch].fill(0),
            _ => {}
        }
    }
    for ch in 0u8..16 {
        let used = active[ch as usize].iter().any(|n| *n > 0);
        for key in 0u8..128 {
            if active[ch as usize][key as usize] > 0 {
                events.push(StoredMidiEvent {
                    tick: end,
                    bytes: vec![0x80 | ch, key, 0],
                });
            }
        }
        if sustain[ch as usize] {
            events.push(StoredMidiEvent {
                tick: end,
                bytes: vec![0xb0 | ch, 64, 0],
            });
        }
        if used {
            events.push(StoredMidiEvent {
                tick: end,
                bytes: vec![0xb0 | ch, 123, 0],
            });
        }
    }
}
