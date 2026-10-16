//! Bounded MIDI packets and sample-timed handoff to native port workers.
//! UMP/MIDI 2.0 is intentionally a separate future wire format.
#[cfg(feature = "device")]
pub mod ports;
pub mod smf;
use crate::contracts::MidiMessage;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub len: u16,
    pub data: [u8; 256],
}
impl Packet {
    pub const PANIC: Self = Self {
        len: 0,
        data: [0; 256],
    };
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if !elyvon_project::io::validate_midi_bytes(bytes) {
            return None;
        }
        let mut data = [0; 256];
        data[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            len: bytes.len() as u16,
            data,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.data[..usize::from(self.len).min(256)]
    }
    pub fn channel(&self) -> Option<u8> {
        (self.len > 0 && self.data[0] < 0xf0).then_some(self.data[0] & 15)
    }
    pub fn from_message(message: MidiMessage) -> Option<Self> {
        let (channel, data, n) = match message {
            MidiMessage::NoteOn {
                channel,
                pitch,
                velocity,
            } => (channel, [0x90, pitch, velocity], 3),
            MidiMessage::NoteOff {
                channel,
                pitch,
                velocity,
            } => (channel, [0x80, pitch, velocity], 3),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => (channel, [0xb0, controller, value], 3),
            MidiMessage::ProgramChange { channel, program } => (channel, [0xc0, program, 0], 2),
            MidiMessage::PitchBend { channel, value } => {
                if value > 16383 {
                    return None;
                }
                (channel, [0xe0, (value & 127) as u8, (value >> 7) as u8], 3)
            }
            MidiMessage::ChannelPressure { channel, pressure } => (channel, [0xd0, pressure, 0], 2),
            MidiMessage::PolyPressure {
                channel,
                pitch,
                pressure,
            } => (channel, [0xa0, pitch, pressure], 3),
        };
        if channel > 15 {
            return None;
        }
        let mut data = data;
        data[0] |= channel;
        Self::parse(&data[..n])
    }
    pub fn message(&self) -> Option<MidiMessage> {
        let ch = self.channel()?;
        let a = self.data[1];
        let b = self.data[2];
        Some(match self.data[0] & 0xf0 {
            0x80 => MidiMessage::NoteOff {
                channel: ch,
                pitch: a,
                velocity: b,
            },
            0x90 => MidiMessage::NoteOn {
                channel: ch,
                pitch: a,
                velocity: b,
            },
            0xa0 => MidiMessage::PolyPressure {
                channel: ch,
                pitch: a,
                pressure: b,
            },
            0xb0 => MidiMessage::ControlChange {
                channel: ch,
                controller: a,
                value: b,
            },
            0xc0 => MidiMessage::ProgramChange {
                channel: ch,
                program: a,
            },
            0xd0 => MidiMessage::ChannelPressure {
                channel: ch,
                pressure: a,
            },
            0xe0 => MidiMessage::PitchBend {
                channel: ch,
                value: u16::from(a) | (u16::from(b) << 7),
            },
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Copy)]
pub struct ScheduledPacket {
    pub track: usize,
    pub deadline: std::time::Instant,
    pub packet: Packet,
}
