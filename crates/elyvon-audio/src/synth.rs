//! "Elyvon Test Tone": a small built-in polyphonic instrument so instrument
//! tracks, the virtual keyboard and piano-roll previews make sound without
//! plug-in hosting. It is a test instrument (sine with a soft second
//! harmonic and an ADSR envelope), not a production synthesizer.
//!
//! MIDI 1.0 handled: note on/off, sustain (CC64), volume (CC7), expression
//! (CC11), all sound/notes off (CC120/123), reset (CC121) and ±2 semitone
//! pitch bend, per channel. Pressure and program change are accepted and
//! ignored. Fixed voice storage: no allocation after construction.

const VOICES: usize = 32;
const ATTACK_S: f32 = 0.004;
const DECAY_S: f32 = 0.25;
const SUSTAIN: f32 = 0.6;
const RELEASE_S: f32 = 0.18;
/// Peak level of one full-velocity voice (headroom for chords).
const VOICE_GAIN: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    stage: Stage,
    channel: u8,
    pitch: u8,
    /// Key released while the pedal was down.
    held_by_pedal: bool,
    level: f32,
    amp: f32,
    phase: f64,
    age: u64,
}

impl Voice {
    const IDLE: Self = Self {
        stage: Stage::Idle,
        channel: 0,
        pitch: 0,
        held_by_pedal: false,
        level: 0.0,
        amp: 0.0,
        phase: 0.0,
        age: 0,
    };
}

#[derive(Debug, Clone, Copy)]
struct Channel {
    bend: f32,
    volume: f32,
    expression: f32,
    pedal: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            bend: 0.0,
            volume: 100.0 / 127.0,
            expression: 1.0,
            pedal: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestSynth {
    rate: f32,
    voices: [Voice; VOICES],
    channels: [Channel; 16],
    clock: u64,
    attack: f32,
    decay: f32,
    release: f32,
}

impl TestSynth {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            rate,
            voices: [Voice::IDLE; VOICES],
            channels: [Channel::default(); 16],
            clock: 0,
            attack: 1.0 / (ATTACK_S * rate),
            decay: (1.0 - SUSTAIN) / (DECAY_S * rate),
            release: 1.0 / (RELEASE_S * rate),
        }
    }

    /// Continue sounding the voices of the previous graph.
    pub fn adopt(&mut self, old: &TestSynth) {
        if old.rate == self.rate {
            self.voices = old.voices;
            self.channels = old.channels;
            self.clock = old.clock;
        }
    }

    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.stage != Stage::Idle)
            .count()
    }

    /// Handles one MIDI 1.0 channel message. Real-time safe.
    pub fn message(&mut self, bytes: &[u8]) {
        let Some(&status) = bytes.first() else {
            return;
        };
        if status >= 0xf0 {
            return;
        }
        let ch = status & 0x0f;
        let a = bytes.get(1).copied().unwrap_or(0);
        let b = bytes.get(2).copied().unwrap_or(0);
        match status & 0xf0 {
            0x90 if b > 0 => self.note_on(ch, a, b),
            0x80 | 0x90 => self.note_off(ch, a),
            0xb0 => self.control(ch, a, b),
            0xe0 => {
                let v = i32::from(a) | (i32::from(b) << 7);
                self.channels[ch as usize].bend = (v - 8192) as f32 / 8192.0 * 2.0;
            }
            _ => {}
        }
    }

    fn note_on(&mut self, ch: u8, pitch: u8, velocity: u8) {
        self.clock += 1;
        // Retrigger the same key on the same channel instead of stacking.
        let slot = self
            .voices
            .iter()
            .position(|v| v.stage != Stage::Idle && v.channel == ch && v.pitch == pitch)
            .or_else(|| self.voices.iter().position(|v| v.stage == Stage::Idle))
            .unwrap_or_else(|| self.steal());
        let v = &mut self.voices[slot];
        let amp = f32::from(velocity) / 127.0;
        *v = Voice {
            stage: Stage::Attack,
            channel: ch,
            pitch,
            held_by_pedal: false,
            level: if v.stage == Stage::Idle { 0.0 } else { v.level },
            amp: amp * amp * VOICE_GAIN,
            phase: if v.stage == Stage::Idle { 0.0 } else { v.phase },
            age: self.clock,
        };
    }

    /// Quietest releasing voice, otherwise the oldest.
    fn steal(&self) -> usize {
        let releasing = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.stage == Stage::Release)
            .min_by(|a, b| a.1.level.total_cmp(&b.1.level))
            .map(|(i, _)| i);
        releasing.unwrap_or_else(|| {
            self.voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.age)
                .map_or(0, |(i, _)| i)
        })
    }

    fn note_off(&mut self, ch: u8, pitch: u8) {
        let pedal = self.channels[ch as usize].pedal;
        for v in &mut self.voices {
            if v.stage != Stage::Idle
                && v.stage != Stage::Release
                && v.channel == ch
                && v.pitch == pitch
            {
                if pedal {
                    v.held_by_pedal = true;
                } else {
                    v.stage = Stage::Release;
                }
            }
        }
    }

    fn control(&mut self, ch: u8, cc: u8, value: u8) {
        let c = &mut self.channels[ch as usize];
        match cc {
            7 => c.volume = f32::from(value) / 127.0,
            11 => c.expression = f32::from(value) / 127.0,
            64 => {
                c.pedal = value >= 64;
                if !c.pedal {
                    for v in &mut self.voices {
                        if v.channel == ch && v.held_by_pedal {
                            v.held_by_pedal = false;
                            v.stage = Stage::Release;
                        }
                    }
                }
            }
            120 => {
                for v in self.voices.iter_mut().filter(|v| v.channel == ch) {
                    *v = Voice::IDLE;
                }
            }
            121 => *c = Channel::default(),
            123 => {
                c.pedal = false;
                for v in self.voices.iter_mut().filter(|v| v.channel == ch) {
                    if v.stage != Stage::Idle {
                        v.stage = Stage::Release;
                        v.held_by_pedal = false;
                    }
                }
            }
            _ => {}
        }
    }

    /// Releases every voice (stop, locate, panic) without a click.
    pub fn all_notes_off(&mut self) {
        for c in &mut self.channels {
            c.pedal = false;
        }
        for v in &mut self.voices {
            if v.stage != Stage::Idle {
                v.stage = Stage::Release;
                v.held_by_pedal = false;
            }
        }
    }

    /// Adds `frames` of output to interleaved stereo `out`.
    pub fn render(&mut self, out: &mut [f32]) {
        let tau = std::f64::consts::TAU;
        for v in &mut self.voices {
            if v.stage == Stage::Idle {
                continue;
            }
            let ch = self.channels[v.channel as usize];
            let freq = 440.0 * 2f64.powf((f64::from(v.pitch) - 69.0 + f64::from(ch.bend)) / 12.0);
            let step = freq / f64::from(self.rate);
            let gain = v.amp * ch.volume * ch.expression;
            for frame in out.as_chunks_mut::<2>().0.iter_mut() {
                match v.stage {
                    Stage::Attack => {
                        v.level += self.attack;
                        if v.level >= 1.0 {
                            v.level = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        v.level -= self.decay;
                        if v.level <= SUSTAIN {
                            v.level = SUSTAIN;
                            v.stage = Stage::Sustain;
                        }
                    }
                    Stage::Sustain => {}
                    Stage::Release => {
                        v.level -= self.release;
                        if v.level <= 0.0 {
                            *v = Voice::IDLE;
                            break;
                        }
                    }
                    Stage::Idle => break,
                }
                let p = v.phase * tau;
                let s = (p.sin() + 0.25 * (2.0 * p).sin()) as f32 * gain * v.level;
                frame[0] += s;
                frame[1] += s;
                v.phase += step;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut TestSynth, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        s.render(&mut out);
        out
    }

    #[test]
    fn plays_the_right_pitch_and_releases() {
        let mut s = TestSynth::new(48_000);
        s.message(&[0x90, 69, 127]);
        let out = run(&mut s, 48_000);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left[4800..]
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        // 440 Hz over 0.9 s.
        assert!((crossings as i64 - 396).abs() <= 2, "{crossings}");
        s.message(&[0x80, 69, 0]);
        let tail = run(&mut s, 48_000);
        assert_eq!(s.active_voices(), 0);
        assert!(tail[tail.len() - 100..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn sustain_pedal_holds_released_keys() {
        let mut s = TestSynth::new(48_000);
        s.message(&[0xb0, 64, 127]);
        s.message(&[0x90, 60, 100]);
        s.message(&[0x80, 60, 0]);
        run(&mut s, 48_000);
        assert_eq!(s.active_voices(), 1, "held by the pedal");
        s.message(&[0xb0, 64, 0]);
        run(&mut s, 48_000);
        assert_eq!(s.active_voices(), 0);
    }

    #[test]
    fn pitch_bend_and_panic() {
        let mut s = TestSynth::new(48_000);
        s.message(&[0xe0, 127, 127]);
        s.message(&[0x90, 57, 127]); // A3 bent up 2 semitones ≈ 246.9 Hz
        let out = run(&mut s, 48_000);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left[4800..]
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        assert!((crossings as i64 - 222).abs() <= 2, "{crossings}");
        for p in 0..40 {
            s.message(&[0x90, 30 + p, 100]);
        }
        assert_eq!(
            s.active_voices(),
            VOICES,
            "voices are stolen, not allocated"
        );
        s.all_notes_off();
        run(&mut s, 48_000);
        assert_eq!(s.active_voices(), 0);
    }
}
