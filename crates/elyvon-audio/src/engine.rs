//! Engine status contracts and the real-time engine.
//!
//! [`create`] returns a control-side [`EngineHandle`] and an [`Engine`] that
//! is moved into the audio callback (or the offline renderer). They talk
//! through three pre-allocated SPSC queues:
//!
//! * transport commands → engine,
//! * new [`PlaybackPlan`]s → engine,
//! * replaced plans ← engine (freed on the control thread, never in the
//!   callback).
//!
//! [`Engine::process`] never allocates, locks, logs or performs IO. Output is
//! a pure function of the playhead position and the plan, so renders are
//! identical for any block size.

use std::sync::Arc;
use std::time::Instant;

use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;
use ts_rs::TS;

use crate::contracts::{MidiEvent, MidiMessage};
use crate::plan::{ClickSounds, MAX_TRACKS, PlanTrack, PlaybackPlan};
use crate::transport::{EngineShared, PlayState, TransportCommand};

/// What the audio engine can currently do, as reported to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum EngineStatus {
    Unavailable {
        reason: String,
    },
    Running {
        config: AudioConfig,
        /// Device name, or a description of the internal clock.
        device: String,
        output: OutputKind,
    },
}

/// Where the engine's output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OutputKind {
    /// A real audio device.
    Device,
    /// No device: blocks are processed on a timer and the output is discarded.
    InternalClock,
}

/// Stream configuration negotiated with an audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub input_channels: u16,
    pub output_channels: u16,
}

/// Per-callback timing information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext {
    pub sample_rate: u32,
    /// Frames in this block (≤ the configured buffer size).
    pub frames: usize,
    /// Song position of the first frame, in samples, if the transport runs.
    pub song_position_samples: Option<u64>,
}

/// Non-interleaved audio buffers for one block: one slice per channel.
#[derive(Debug)]
pub struct AudioBlock<'a, 'b> {
    pub inputs: &'a [&'b [f32]],
    pub outputs: &'a mut [&'b mut [f32]],
}

/// A node in the future processing graph (insert effect, instrument, …).
///
/// * [`prepare`](Self::prepare) runs on a non-real-time thread. Allocate here.
/// * [`process`](Self::process) runs on the real-time audio thread: no
///   allocation, no locks, no IO, no logging.
/// * [`reset`](Self::reset) follows the same rules as `process`.
pub trait AudioProcessor: Send {
    fn prepare(&mut self, config: &AudioConfig);
    fn process(&mut self, ctx: &ProcessContext, block: &mut AudioBlock<'_, '_>);
    fn reset(&mut self);
}

/// Receives scheduled MIDI events (per track, ordered by frame within a block).
/// There is no instrument yet: the device callback discards them; tests and
/// diagnostics capture them.
pub trait MidiSink {
    fn event(&mut self, track: usize, event: MidiEvent);
}

/// Discards MIDI.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMidi;

impl MidiSink for NoMidi {
    fn event(&mut self, _: usize, _: MidiEvent) {}
}

const COMMAND_CAPACITY: usize = 256;
const PLAN_CAPACITY: usize = 4;
const GARBAGE_CAPACITY: usize = 8;
const MAX_EVENTS_PER_TRACK_SPAN: usize = 256;

/// Control-thread side of an engine.
#[derive(Debug)]
pub struct EngineHandle {
    commands: Producer<TransportCommand>,
    plans: Producer<Box<PlaybackPlan>>,
    garbage: Consumer<Box<PlaybackPlan>>,
    pending: Option<Box<PlaybackPlan>>,
    shared: Arc<EngineShared>,
    sample_rate: u32,
}

impl EngineHandle {
    pub fn shared(&self) -> &Arc<EngineShared> {
        &self.shared
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Queues a transport command. `false` if the queue is full.
    pub fn send(&mut self, command: TransportCommand) -> bool {
        self.commands.push(command).is_ok()
    }

    /// Publishes a plan. If the queue is full the newest plan is kept and sent
    /// by the next [`poll`](Self::poll); older pending plans are dropped here.
    pub fn publish(&mut self, plan: PlaybackPlan) {
        debug_assert_eq!(plan.sample_rate, self.sample_rate);
        self.pending = Some(Box::new(plan));
        self.poll();
    }

    /// Frees replaced plans and retries a pending publish.
    pub fn poll(&mut self) {
        while let Ok(old) = self.garbage.pop() {
            drop(old);
        }
        if let Some(plan) = self.pending.take()
            && let Err(rtrb::PushError::Full(plan)) = self.plans.push(plan)
        {
            self.pending = Some(plan);
        }
    }

    pub fn has_pending_plan(&self) -> bool {
        self.pending.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
struct CountIn {
    pos: u64,
    total: u64,
    samples_per_beat: f64,
    beats: u32,
    beats_per_bar: u32,
    target: u64,
}

/// Per-track accumulators for diagnostic renders.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackMeasure {
    pub peak: f32,
    pub sum_squares: f64,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy)]
struct PendingEvent {
    frame: u32,
    off: bool,
    pitch: u8,
    velocity: u8,
}

/// The real-time side. Move it into the audio callback.
pub struct Engine {
    plan: Box<PlaybackPlan>,
    plans: Consumer<Box<PlaybackPlan>>,
    garbage: Producer<Box<PlaybackPlan>>,
    commands: Consumer<TransportCommand>,
    shared: Arc<EngineShared>,
    state: PlayState,
    position: u64,
    play_start: u64,
    count_in: Option<CountIn>,
    max_block: usize,
    /// Stereo interleaved scratch for one track and for the master.
    track_buf: Vec<f32>,
    mix: Vec<f32>,
    /// Sounding-note counts per track and pitch (stuck-note protection).
    active_notes: Vec<[u8; 128]>,
    events: Vec<PendingEvent>,
    measure: Option<Vec<TrackMeasure>>,
    clip_output: bool,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("state", &self.state)
            .field("position", &self.position)
            .field("generation", &self.plan.generation)
            .finish_non_exhaustive()
    }
}

/// Creates a connected engine pair. `max_block` bounds the internal block;
/// larger callbacks are processed in several internal blocks.
pub fn create(sample_rate: u32, max_block: usize, plan: PlaybackPlan) -> (EngineHandle, Engine) {
    assert_eq!(plan.sample_rate, sample_rate, "plan built for another rate");
    let max_block = max_block.clamp(16, 1 << 15);
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (plan_tx, plan_rx) = RingBuffer::new(PLAN_CAPACITY);
    let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let shared = Arc::new(EngineShared::new(sample_rate));
    let handle = EngineHandle {
        commands: cmd_tx,
        plans: plan_tx,
        garbage: garbage_rx,
        pending: None,
        shared: Arc::clone(&shared),
        sample_rate,
    };
    let engine = Engine {
        plan: Box::new(plan),
        plans: plan_rx,
        garbage: garbage_tx,
        commands: cmd_rx,
        shared,
        state: PlayState::Stopped,
        position: 0,
        play_start: 0,
        count_in: None,
        max_block,
        track_buf: vec![0.0; max_block * 2],
        mix: vec![0.0; max_block * 2],
        active_notes: vec![[0u8; 128]; MAX_TRACKS],
        events: Vec::with_capacity(MAX_EVENTS_PER_TRACK_SPAN),
        measure: None,
        clip_output: true,
    };
    (handle, engine)
}

impl Engine {
    pub fn max_block(&self) -> usize {
        self.max_block
    }

    pub fn state(&self) -> PlayState {
        self.state
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    /// Hard-clip the device output to ±1 (on by default; off for renders).
    pub fn set_clip_output(&mut self, clip: bool) {
        self.clip_output = clip;
    }

    /// Enables per-track level measurement (allocates; call before processing).
    pub fn enable_measurement(&mut self) {
        self.measure = Some(vec![TrackMeasure::default(); MAX_TRACKS]);
    }

    pub fn measurements(&self) -> Option<&[TrackMeasure]> {
        self.measure.as_deref()
    }

    /// Renders `out.len() / channels` interleaved frames. Real-time safe.
    pub fn process(&mut self, out: &mut [f32], channels: usize, midi: &mut dyn MidiSink) {
        let started = Instant::now();
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.swap_plan(midi);
        self.drain_commands(midi);
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.max_block);
            self.mix[..n * 2].fill(0.0);
            self.render(n, done as u32, midi);
            let dst = &mut out[done * channels..(done + n) * channels];
            let (mut pl, mut pr) = (0.0f32, 0.0f32);
            for (i, frame) in dst.chunks_exact_mut(channels).enumerate() {
                let mut l = self.mix[i * 2];
                let mut r = self.mix[i * 2 + 1];
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
                if self.clip_output {
                    l = l.clamp(-1.0, 1.0);
                    r = r.clamp(-1.0, 1.0);
                }
                if channels == 1 {
                    frame[0] = 0.5 * (l + r);
                } else {
                    frame[0] = l;
                    frame[1] = r;
                    for s in &mut frame[2..] {
                        *s = 0.0;
                    }
                }
            }
            self.shared.peak_master(0, pl);
            self.shared.peak_master(1, pr);
            done += n;
        }
        let ticks = self
            .plan
            .map
            .tick_at_sample(self.position, self.plan.sample_rate);
        let remaining = self.count_in.map_or(0, |c| c.total - c.pos);
        self.shared.publish(
            self.state,
            self.position,
            ticks,
            remaining,
            self.plan.generation,
        );
        self.shared
            .add_block(frames as u64, started.elapsed().as_nanos() as u64);
    }

    fn swap_plan(&mut self, midi: &mut dyn MidiSink) {
        while self.garbage.slots() > 0 {
            let Ok(new) = self.plans.pop() else { break };
            if new.map_key != self.plan.map_key {
                // Keep the musical position when the tempo map changes.
                let rate = self.plan.sample_rate;
                let remap = |frame: u64, old: &PlaybackPlan, new: &PlaybackPlan| {
                    elyvon_core::tempo::round_frame(
                        new.map
                            .samples_at(old.map.tick_at_sample(frame, rate), rate),
                    )
                };
                self.position = remap(self.position, &self.plan, &new);
                self.play_start = remap(self.play_start, &self.plan, &new);
            }
            self.all_notes_off(0, midi);
            let old = std::mem::replace(&mut self.plan, new);
            // A free slot was checked above; push cannot fail.
            let _ = self.garbage.push(old);
        }
    }

    fn drain_commands(&mut self, midi: &mut dyn MidiSink) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                TransportCommand::Play {
                    from_tick,
                    count_in,
                    pre_roll,
                } => self.start(from_tick, count_in, pre_roll, midi),
                TransportCommand::Stop => {
                    if self.state == PlayState::Stopped {
                        self.position = self.play_start;
                    }
                    self.halt(PlayState::Stopped, midi);
                }
                TransportCommand::Pause => {
                    if self.state != PlayState::Stopped {
                        self.halt(PlayState::Paused, midi);
                    }
                }
                TransportCommand::Locate { tick } => {
                    let frame = self.plan.map.sample_at_tick(tick, self.plan.sample_rate);
                    match self.count_in.as_mut() {
                        Some(c) => c.target = frame,
                        None => self.position = frame,
                    }
                    if self.state == PlayState::Stopped || self.state == PlayState::Paused {
                        self.play_start = frame;
                    }
                    self.all_notes_off(0, midi);
                }
            }
        }
    }

    fn halt(&mut self, state: PlayState, midi: &mut dyn MidiSink) {
        if let Some(c) = self.count_in.take() {
            self.position = c.target;
        }
        self.state = state;
        self.all_notes_off(0, midi);
    }

    fn start(
        &mut self,
        from_tick: Option<u64>,
        count_in: bool,
        pre_roll: bool,
        midi: &mut dyn MidiSink,
    ) {
        let rate = self.plan.sample_rate;
        let map = &self.plan.map;
        let mut tick = match from_tick {
            Some(t) => t,
            None => map.tick_at_sample(self.position, rate).round() as u64,
        };
        if pre_roll && self.plan.pre_roll_bars > 0 {
            let bar = map.bar_at(tick);
            let back = bar.bar.saturating_sub(u64::from(self.plan.pre_roll_bars));
            if back < bar.bar {
                // Same position inside a bar that many bars earlier (clamped
                // to that bar when its metre is shorter).
                let earlier = map.bar_start(back);
                let room = map.bar_start(back + 1) - earlier;
                tick = earlier + (tick - bar.start).min(room - 1);
            }
        }
        let target = map.sample_at_tick(tick, rate);
        let ts = map.time_signature_at(tick);
        let bpm = map.bpm_at(tick);
        self.all_notes_off(0, midi);
        self.play_start = target;
        if count_in && self.plan.count_in_bars > 0 {
            let samples_per_beat = f64::from(rate) * 60.0 / bpm * 4.0 / f64::from(ts.denominator);
            let beats = u32::from(self.plan.count_in_bars) * u32::from(ts.numerator);
            self.count_in = Some(CountIn {
                pos: 0,
                total: elyvon_core::tempo::round_frame(f64::from(beats) * samples_per_beat).max(1),
                samples_per_beat,
                beats,
                beats_per_bar: u32::from(ts.numerator),
                target,
            });
            self.position = target;
            self.state = PlayState::CountIn;
        } else {
            self.count_in = None;
            self.position = target;
            self.state = PlayState::Playing;
        }
    }

    fn all_notes_off(&mut self, frame: u32, midi: &mut dyn MidiSink) {
        for (track, notes) in self.active_notes.iter_mut().enumerate() {
            for (pitch, count) in notes.iter_mut().enumerate() {
                while *count > 0 {
                    *count -= 1;
                    midi.event(
                        track,
                        MidiEvent {
                            frame_offset: frame,
                            message: MidiMessage::NoteOff {
                                channel: 0,
                                pitch: pitch as u8,
                                velocity: 0,
                            },
                        },
                    );
                }
            }
        }
    }

    /// Renders `n` frames into `mix[..n*2]`; `base` is the frame offset of
    /// this internal block inside the callback (for MIDI timestamps).
    fn render(&mut self, n: usize, base: u32, midi: &mut dyn MidiSink) {
        let mut off = 0usize;
        if let Some(mut c) = self.count_in {
            let len = n.min((c.total - c.pos) as usize);
            self.render_count_in(&c, len);
            c.pos += len as u64;
            off = len;
            if c.pos >= c.total {
                self.count_in = None;
                self.position = c.target;
                self.state = PlayState::Playing;
            } else {
                self.count_in = Some(c);
                return;
            }
        }
        if self.state != PlayState::Playing {
            return;
        }
        while off < n {
            let mut len = n - off;
            let cycle = self.plan.cycle;
            if let Some((_, ce)) = cycle
                && self.position < ce
                && self.position + len as u64 > ce
            {
                len = (ce - self.position) as usize;
            }
            self.render_span(self.position, len, off, base, midi);
            self.position += len as u64;
            off += len;
            if let Some((cs, ce)) = cycle
                && self.position == ce
            {
                self.position = cs;
                self.all_notes_off(base + off as u32, midi);
                self.shared.add_cycle();
            }
        }
    }

    fn render_count_in(&mut self, c: &CountIn, len: usize) {
        let clicks = &self.plan.clicks;
        let click_len = clicks.len() as u64;
        let from = c.pos;
        let to = c.pos + len as u64;
        for k in 0..c.beats {
            let at = elyvon_core::tempo::round_frame(f64::from(k) * c.samples_per_beat);
            if at >= to {
                break;
            }
            if at + click_len <= from {
                continue;
            }
            let sound = if k % c.beats_per_bar == 0 {
                &clicks.accent
            } else {
                &clicks.normal
            };
            mix_click(
                &mut self.mix,
                0,
                from,
                to,
                at,
                sound,
                self.plan.metronome_gain,
            );
        }
    }

    fn render_span(
        &mut self,
        pos: u64,
        len: usize,
        off: usize,
        base: u32,
        midi: &mut dyn MidiSink,
    ) {
        let q = pos + len as u64;
        let plan = &*self.plan;
        for (ti, track) in plan.tracks.iter().enumerate() {
            if !track.audible {
                release_track(&mut self.active_notes[ti], ti, base + off as u32, midi);
                continue;
            }
            if !track.segments.is_empty() {
                let buf = &mut self.track_buf[..len * 2];
                buf.fill(0.0);
                if mix_track(track, pos, q, buf) {
                    let (mut peak, mut sum) = (0.0f32, 0.0f64);
                    let dst = &mut self.mix[off * 2..(off + len) * 2];
                    for (d, s) in dst.iter_mut().zip(buf.iter()) {
                        *d += *s;
                        peak = peak.max(s.abs());
                        sum += f64::from(*s) * f64::from(*s);
                    }
                    self.shared.peak_track(ti, peak);
                    if let Some(m) = self.measure.as_mut().and_then(|m| m.get_mut(ti)) {
                        m.peak = m.peak.max(peak);
                        m.sum_squares += sum;
                    }
                }
            }
            if let Some(m) = self.measure.as_mut().and_then(|m| m.get_mut(ti)) {
                m.frames += len as u64;
            }
            if !track.notes.is_empty() {
                schedule_notes(
                    track,
                    pos,
                    q,
                    base + off as u32,
                    &mut self.events,
                    &mut self.active_notes[ti],
                    ti,
                    &self.shared,
                    midi,
                );
            }
        }
        if plan.metronome {
            let clicks = &plan.clicks;
            let rate = plan.sample_rate;
            let lookback = pos.saturating_sub(clicks.len() as u64);
            let from_tick = plan.map.tick_at_sample(lookback, rate).ceil() as u64;
            for beat in plan.map.beats(from_tick, u64::MAX) {
                let at = plan.map.sample_at_tick(beat.tick, rate);
                if at >= q {
                    break;
                }
                let sound = if beat.downbeat && plan.accent {
                    &clicks.accent
                } else {
                    &clicks.normal
                };
                mix_click(&mut self.mix, off, pos, q, at, sound, plan.metronome_gain);
            }
        }
    }
}

/// Mixes the part of a click starting at absolute frame `at` that falls in
/// `[from, to)` into `mix` (stereo) starting at internal offset `off`.
fn mix_click(mix: &mut [f32], off: usize, from: u64, to: u64, at: u64, sound: &[f32], gain: f32) {
    let start = at.max(from);
    let end = (at + sound.len() as u64).min(to);
    for t in start..end {
        let s = sound[(t - at) as usize] * gain;
        let i = (off + (t - from) as usize) * 2;
        mix[i] += s;
        mix[i + 1] += s;
    }
}

/// Mixes a track's audio for `[p, q)` into `buf` (stereo). Returns whether
/// anything was written.
fn mix_track(track: &PlanTrack, p: u64, q: u64, buf: &mut [f32]) -> bool {
    let mut any = false;
    for seg in &track.segments[track.segment_range(p, q)] {
        let a = seg.start.max(p);
        let b = seg.end.min(q);
        if a >= b {
            continue;
        }
        any = true;
        let media = &seg.media;
        let stereo = media.planes.len() > 1;
        let (gl, gr) = if stereo {
            track.stereo_gain
        } else {
            track.mono_gain
        };
        let left = &media.planes[0];
        let right = &media.planes[media.planes.len() - 1];
        for t in a..b {
            let Some(src) = seg.source_frame(t) else {
                break;
            };
            let g = seg.envelope(t);
            let i = ((t - p) as usize) * 2;
            buf[i] += left[src] * g * gl;
            buf[i + 1] += right[src] * g * gr;
        }
    }
    any
}

fn release_track(notes: &mut [u8; 128], track: usize, frame: u32, midi: &mut dyn MidiSink) {
    for (pitch, count) in notes.iter_mut().enumerate() {
        while *count > 0 {
            *count -= 1;
            midi.event(
                track,
                MidiEvent {
                    frame_offset: frame,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        pitch: pitch as u8,
                        velocity: 0,
                    },
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn schedule_notes(
    track: &PlanTrack,
    p: u64,
    q: u64,
    frame0: u32,
    events: &mut Vec<PendingEvent>,
    active: &mut [u8; 128],
    ti: usize,
    shared: &EngineShared,
    midi: &mut dyn MidiSink,
) {
    events.clear();
    let first = track
        .notes
        .partition_point(|n| n.on < p.saturating_sub(track.max_note_len));
    for note in &track.notes[first..] {
        if note.on >= q {
            break;
        }
        let mut push = |e: PendingEvent| {
            if events.len() < events.capacity() {
                events.push(e);
            } else {
                shared.add_midi_overflow();
            }
        };
        if note.on >= p {
            push(PendingEvent {
                frame: (note.on - p) as u32,
                off: false,
                pitch: note.pitch,
                velocity: note.velocity,
            });
        }
        if note.off >= p && note.off < q {
            push(PendingEvent {
                frame: (note.off - p) as u32,
                off: true,
                pitch: note.pitch,
                velocity: 0,
            });
        }
    }
    // Stable order: by frame, note-offs before note-ons (allocation-free sort).
    events.sort_unstable_by_key(|e| (e.frame, !e.off, e.pitch));
    for e in events.iter() {
        let count = &mut active[e.pitch as usize];
        let message = if e.off {
            if *count == 0 {
                continue;
            }
            *count -= 1;
            MidiMessage::NoteOff {
                channel: 0,
                pitch: e.pitch,
                velocity: 0,
            }
        } else {
            *count = count.saturating_add(1);
            MidiMessage::NoteOn {
                channel: 0,
                pitch: e.pitch,
                velocity: e.velocity,
            }
        };
        midi.event(
            ti,
            MidiEvent {
                frame_offset: frame0 + e.frame,
                message,
            },
        );
    }
}

/// Kept for callers that only need a click sound outside a plan.
pub fn click_sounds(rate: u32) -> ClickSounds {
    ClickSounds::synthesise(rate)
}
