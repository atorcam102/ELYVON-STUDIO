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
use crate::automation::{ParamOverride, StripKey};
use crate::dsp::Smoothed;
use elyvon_project::automation::InsertParam;
use crate::plan::{
    ClickSounds, Dest, MASTER_SLOT, MAX_AUXES, MAX_CHANNELS, MAX_TRACKS, PlanTrack, PlaybackPlan,
    Strip,
};
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
    fn packet(&mut self, track: usize, frame: u32, packet: crate::midi::Packet) {
        if let Some(message) = packet.message() {
            self.event(
                track,
                MidiEvent {
                    frame_offset: frame,
                    message,
                },
            );
        }
    }
    fn reset(&mut self, _frame: u32) {}
    fn event(&mut self, track: usize, event: MidiEvent);
}

/// Discards MIDI.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMidi;

impl MidiSink for NoMidi {
    fn event(&mut self, _: usize, _: MidiEvent) {}
}

/// Input-side monitor writer. Returned to the control handle when the input
/// stream is dropped, so repeated Record/Stop does not lose the monitor bus.
#[derive(Debug)]
pub struct MonitorInput {
    producer: Option<Producer<[f32; 2]>>,
    returns: std::sync::mpsc::Sender<Producer<[f32; 2]>>,
}
impl MonitorInput {
    pub fn push(&mut self, frame: [f32; 2]) -> bool {
        self.producer
            .as_mut()
            .is_some_and(|p| p.push(frame).is_ok())
    }
}
impl Drop for MonitorInput {
    fn drop(&mut self) {
        if let Some(p) = self.producer.take() {
            let _ = self.returns.send(p);
        }
    }
}

const COMMAND_CAPACITY: usize = 256;
const PLAN_CAPACITY: usize = 4;
const GARBAGE_CAPACITY: usize = 8;
const MAX_EVENTS_PER_TRACK_SPAN: usize = 256;
const LIVE_CAPACITY: usize = 1024;
/// Internal processing block. Larger callbacks are split, so bus buffers
/// stay small whatever the device buffer size.
const CHUNK: usize = crate::plan::MAX_CHUNK;
const OVERRIDE_CAPACITY: usize = 1024;

/// A live MIDI channel message for one track's internal instrument
/// (virtual keyboard, MIDI input monitor, piano-roll preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveMidi {
    /// `TrackId` bits.
    pub track: u128,
    pub len: u8,
    pub data: [u8; 3],
}

impl LiveMidi {
    /// `None` for SysEx/system messages (the test instrument ignores them).
    pub fn new(track: u128, bytes: &[u8]) -> Option<Self> {
        let n = bytes.len();
        if !(1..=3).contains(&n) || bytes[0] >= 0xf0 || bytes[0] < 0x80 {
            return None;
        }
        let mut data = [0; 3];
        data[..n].copy_from_slice(bytes);
        Some(Self {
            track,
            len: n as u8,
            data,
        })
    }
}

/// Control-thread side of an engine.
#[derive(Debug)]
pub struct EngineHandle {
    commands: Producer<TransportCommand>,
    monitor_input: Option<Producer<[f32; 2]>>,
    monitor_returns: std::sync::mpsc::Receiver<Producer<[f32; 2]>>,
    monitor_sender: std::sync::mpsc::Sender<Producer<[f32; 2]>>,
    plans: Producer<Box<PlaybackPlan>>,
    garbage: Consumer<Box<PlaybackPlan>>,
    pending: Option<Box<PlaybackPlan>>,
    shared: Arc<EngineShared>,
    sample_rate: u32,
    midi_output: Consumer<crate::midi::ScheduledPacket>,
    midi_overflow: Arc<std::sync::atomic::AtomicBool>,
    live: Producer<LiveMidi>,
    overrides: Producer<ParamOverride>,
}

impl EngineHandle {
    /// Queues a live automation control value (Touch/Latch/Write). `false`
    /// if the queue is full.
    pub fn send_override(&mut self, o: ParamOverride) -> bool {
        self.overrides.push(o).is_ok()
    }

    pub fn drain_midi(&mut self) -> (Vec<crate::midi::ScheduledPacket>, bool) {
        let mut packets = Vec::new();
        while let Ok(p) = self.midi_output.pop() {
            packets.push(p);
        }
        let overflow = self
            .midi_overflow
            .swap(false, std::sync::atomic::Ordering::AcqRel);
        (packets, overflow)
    }

    /// Queues a live message for a track's internal instrument. `false` if
    /// the queue is full.
    pub fn send_live(&mut self, message: LiveMidi) -> bool {
        self.live.push(message).is_ok()
    }

    pub fn take_monitor_input(&mut self) -> Option<MonitorInput> {
        if let Ok(p) = self.monitor_returns.try_recv() {
            self.monitor_input = Some(p);
        }
        self.monitor_input.take().map(|producer| MonitorInput {
            producer: Some(producer),
            returns: self.monitor_sender.clone(),
        })
    }

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
    channel: u8,
    pitch: u8,
    velocity: u8,
}

/// A channel message for the internal instrument within one span.
#[derive(Debug, Clone, Copy)]
struct SynthEvent {
    frame: u32,
    /// 0 = controller/bend/pressure, 1 = note-off, 2 = note-on.
    class: u8,
    seq: u16,
    len: u8,
    data: [u8; 3],
}

type NoteCounts = [[u8; 128]; 16];

/// Per-channel post-fader audio of the last processed block (offline stems).
#[derive(Debug)]
struct Stems {
    frames: usize,
    data: Vec<f32>,
}

/// The real-time side. Move it into the audio callback.
pub struct Engine {
    plan: Box<PlaybackPlan>,
    plans: Consumer<Box<PlaybackPlan>>,
    garbage: Producer<Box<PlaybackPlan>>,
    commands: Consumer<TransportCommand>,
    monitor: Consumer<[f32; 2]>,
    live: Consumer<LiveMidi>,
    overrides: Consumer<ParamOverride>,
    shared: Arc<EngineShared>,
    state: PlayState,
    position: u64,
    play_start: u64,
    count_in: Option<CountIn>,
    max_block: usize,
    chunk: usize,
    /// Interleaved stereo scratch buffers of `chunk` frames.
    track_buf: Vec<f32>,
    scratch: Vec<f32>,
    edge: Vec<f32>,
    /// One `chunk` per auxiliary channel.
    bus: Vec<f32>,
    master_buf: Vec<f32>,
    mix: Vec<f32>,
    /// Sounding-note counts per track, channel and pitch (stuck-note protection).
    active_notes: Vec<NoteCounts>,
    active_scratch: Vec<NoteCounts>,
    active_any: Vec<u32>,
    any_scratch: Vec<u32>,
    events: Vec<PendingEvent>,
    synth_events: Vec<SynthEvent>,
    measure: Option<Vec<TrackMeasure>>,
    stems: Option<Stems>,
    block_offset: usize,
    clip_output: bool,
    release_pending: bool,
    callback_frames: u32,
    callback_clock: u64,
    midi_output: Option<Producer<crate::midi::ScheduledPacket>>,
    midi_overflow: Arc<std::sync::atomic::AtomicBool>,
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

/// Creates a connected engine pair. `max_block` bounds a callback for
/// offline stems; larger device callbacks are processed in internal blocks.
pub fn create(sample_rate: u32, max_block: usize, plan: PlaybackPlan) -> (EngineHandle, Engine) {
    assert_eq!(plan.sample_rate, sample_rate, "plan built for another rate");
    let max_block = max_block.clamp(16, 1 << 15);
    let chunk = max_block.min(CHUNK);
    let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
    let (monitor_tx, monitor_rx) = RingBuffer::new(2048);
    let (monitor_sender, monitor_returns) = std::sync::mpsc::channel();
    let (plan_tx, plan_rx) = RingBuffer::new(PLAN_CAPACITY);
    let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_CAPACITY);
    let (live_tx, live_rx) = RingBuffer::new(LIVE_CAPACITY);
    let (override_tx, override_rx) = RingBuffer::new(OVERRIDE_CAPACITY);
    let shared = Arc::new(EngineShared::new(sample_rate));
    let (midi_tx, midi_rx) = RingBuffer::new(4096);
    let midi_overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = EngineHandle {
        commands: cmd_tx,
        monitor_input: Some(monitor_tx),
        monitor_sender,
        monitor_returns,
        plans: plan_tx,
        garbage: garbage_rx,
        pending: None,
        shared: Arc::clone(&shared),
        sample_rate,
        midi_output: midi_rx,
        midi_overflow: Arc::clone(&midi_overflow),
        live: live_tx,
        overrides: override_tx,
    };
    let engine = Engine {
        plan: Box::new(plan),
        plans: plan_rx,
        garbage: garbage_tx,
        commands: cmd_rx,
        monitor: monitor_rx,
        live: live_rx,
        overrides: override_rx,
        shared,
        state: PlayState::Stopped,
        position: 0,
        play_start: 0,
        count_in: None,
        max_block,
        chunk,
        track_buf: vec![0.0; chunk * 2],
        scratch: vec![0.0; chunk * 2],
        edge: vec![0.0; chunk * 2],
        bus: vec![0.0; MAX_AUXES * chunk * 2],
        master_buf: vec![0.0; chunk * 2],
        mix: vec![0.0; chunk * 2],
        active_notes: vec![[[0u8; 128]; 16]; MAX_TRACKS],
        active_scratch: vec![[[0u8; 128]; 16]; MAX_TRACKS],
        active_any: vec![0; MAX_TRACKS],
        any_scratch: vec![0; MAX_TRACKS],
        events: Vec::with_capacity(MAX_EVENTS_PER_TRACK_SPAN),
        synth_events: Vec::with_capacity(MAX_EVENTS_PER_TRACK_SPAN * 2),
        measure: None,
        stems: None,
        block_offset: 0,
        clip_output: true,
        release_pending: false,
        callback_frames: 0,
        callback_clock: 0,
        midi_output: Some(midi_tx),
        midi_overflow,
    };
    (handle, engine)
}

/// Accumulated block meter for one strip.
#[derive(Debug, Clone, Copy, Default)]
struct BlockMeter {
    peak: [f32; 2],
    sum: [f64; 2],
    clipped: bool,
}

impl BlockMeter {
    #[inline]
    fn scan(buf: &[f32]) -> Self {
        let mut m = Self::default();
        for f in buf.as_chunks::<2>().0.iter() {
            for (ch, &v) in f.iter().enumerate() {
                let a = v.abs();
                if a > 1.0 || a.is_nan() {
                    // Over full scale, or NaN/inf.
                    m.clipped = true;
                }
                if a.is_finite() {
                    m.peak[ch] = m.peak[ch].max(a);
                    m.sum[ch] += f64::from(v) * f64::from(v);
                }
            }
        }
        m
    }
}

/// Adds `src` × ramped `gain` into `dst`, through `delay` when it has length.
/// With `levels`, each frame is also scaled by its automated level.
#[inline]
fn route(
    src: &[f32],
    dst: &mut [f32],
    gain: &mut Smoothed,
    delay: &mut crate::dsp::StereoDelay,
    edge: &mut [f32],
) {
    route_with(src, dst, gain, None, delay, edge);
}

#[inline]
fn route_with(
    src: &[f32],
    dst: &mut [f32],
    gain: &mut Smoothed,
    levels: Option<&[f32]>,
    delay: &mut crate::dsp::StereoDelay,
    edge: &mut [f32],
) {
    let n = src.len() / 2;
    if gain.is_silent() {
        // Keep the delay line's timeline consistent with silence.
        if delay.frames() > 0 {
            edge[..src.len()].fill(0.0);
            delay.process(&mut edge[..src.len()]);
        }
        return;
    }
    let input: &[f32] = if delay.frames() > 0 {
        edge[..src.len()].copy_from_slice(src);
        delay.process(&mut edge[..src.len()]);
        &edge[..src.len()]
    } else {
        src
    };
    let frames = dst
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(input.as_chunks::<2>().0.iter());
    match levels {
        Some(levels) => {
            for (i, (d, s)) in frames.enumerate() {
                let (gl, gr) = gain.at(i, n);
                let k = levels.get(i).copied().unwrap_or(0.0);
                d[0] += s[0] * gl * k;
                d[1] += s[1] * gr * k;
            }
            gain.settle();
        }
        None if gain.is_steady() => {
            let (gl, gr) = gain.target;
            for (d, s) in frames {
                d[0] += s[0] * gl;
                d[1] += s[1] * gr;
            }
        }
        None => {
            for (i, (d, s)) in frames.enumerate() {
                let (gl, gr) = gain.at(i, n);
                d[0] += s[0] * gl;
                d[1] += s[1] * gr;
            }
            gain.settle();
        }
    }
}

/// Automation sub-block for insert parameters, on an absolute frame grid so
/// renders do not depend on the callback size.
const INSERT_STEP: u64 = 32;

/// Runs the inserts of a strip, applying automated parameters.
fn run_inserts(strip: &mut Strip, buf: &mut [f32], pos: u64, advancing: bool) {
    let len = buf.len() / 2;
    let Some(auto) = strip.auto.as_mut().filter(|a| a.has_insert_slots()) else {
        for insert in &mut strip.inserts {
            insert.process(buf);
        }
        return;
    };
    let mut k = 0usize;
    while k < len {
        let n = if advancing {
            let abs = pos + k as u64;
            ((INSERT_STEP - abs % INSERT_STEP) as usize).min(len - k)
        } else {
            len - k
        };
        // Coefficients (cutoff, Q, ceiling, release) step on the absolute
        // grid; trim gain and bypass are applied per frame.
        let grid = (pos + k as u64) / INSERT_STEP * INSERT_STEP;
        for slot in &auto.slots {
            if let crate::automation::Target::Insert(i, param) = slot.target
                && !matches!(param, InsertParam::TrimGain | InsertParam::Bypass)
                && let Some(insert) = strip.inserts.get_mut(i)
            {
                let index = k.saturating_sub((pos + k as u64 - grid) as usize);
                insert.automate(param, slot.value_at(grid, index));
            }
        }
        for (ii, insert) in strip.inserts.iter_mut().enumerate() {
            let levels = |p: InsertParam| {
                auto.slots
                    .iter()
                    .find(|s| s.target == crate::automation::Target::Insert(ii, p) && s.is_dynamic())
                    .map(|s| &s.buf[k..k + n])
            };
            insert.process_levels(
                &mut buf[k * 2..(k + n) * 2],
                levels(InsertParam::TrimGain),
                levels(InsertParam::Bypass),
            );
        }
        k += n;
    }
}

/// Applies the fader (volume × pan × mute) to `buf`. Per sample when
/// automated, otherwise the smoothed static gain.
fn apply_fader(strip: &mut Strip, buf: &mut [f32]) {
    let len = buf.len() / 2;
    if let Some(auto) = strip.auto.as_ref().filter(|a| a.fader_dynamic()) {
        let vol = auto.volume.map(|i| &auto.slots[i]);
        let pan = auto.pan.map(|i| &auto.slots[i]);
        let mute = auto.mute.map(|i| &auto.slots[i]);
        let mut last = (0.0f32, 0.0f32);
        for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let g = vol.map_or(auto.static_gain, |s| {
                let db = s.buf[i];
                if db <= -96.0 { 0.0 } else { crate::gain::db_to_gain(f64::from(db)) as f32 }
            });
            let (pl, pr) = match pan {
                Some(s) => {
                    let (l, r) = crate::plan::pan_gains(auto.mono, f64::from(s.buf[i]), auto.law);
                    (l as f32, r as f32)
                }
                None => auto.static_pan,
            };
            let m = mute.map_or(1.0, |s| 1.0 - s.buf[i].clamp(0.0, 1.0));
            let (gl, gr) = (g * pl * m, g * pr * m);
            f[0] *= gl;
            f[1] *= gr;
            last = (gl, gr);
        }
        // A later static span ramps from the last automated gain.
        strip.fader.current = last;
        return;
    }
    if strip.fader.is_steady() {
        let (gl, gr) = strip.fader.target;
        if (gl, gr) != (1.0, 1.0) {
            for f in buf.as_chunks_mut::<2>().0.iter_mut() {
                f[0] *= gl;
                f[1] *= gr;
            }
        }
    } else {
        for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let (gl, gr) = strip.fader.at(i, len);
            f[0] *= gl;
            f[1] *= gr;
        }
        strip.fader.settle();
    }
}

/// Routes the strip's pre- or post-fader sends.
fn route_sends(strip: &mut Strip, buf: &[f32], bus: &mut [f32], edge: &mut [f32], chunk: usize, pre: bool) {
    let len = buf.len() / 2;
    for (si, send) in strip.sends.iter_mut().enumerate() {
        if send.pre_fader != pre {
            continue;
        }
        let start = send.aux * chunk * 2;
        let dst = &mut bus[start..start + len * 2];
        let levels = if send.automated {
            strip
                .auto
                .as_ref()
                .and_then(|a| a.sends.get(si).copied().flatten().map(|i| &a.slots[i].buf[..len]))
        } else {
            None
        };
        if send.automated && levels.is_none() {
            continue;
        }
        route_with(buf, dst, &mut send.gain, levels, &mut send.delay, edge);
    }
}

/// Processes one strip on `buf` (its input, `len` frames starting at
/// timeline frame `pos`) and routes the result into the aux buses or the
/// master input. Real-time safe.
#[allow(clippy::too_many_arguments)]
fn process_strip(
    strip: &mut Strip,
    buf: &mut [f32],
    edge: &mut [f32],
    bus: &mut [f32],
    master: &mut [f32],
    chunk: usize,
    shared: &EngineShared,
    stems: Option<&mut Stems>,
    stem_offset: usize,
    pos: u64,
    advancing: bool,
) -> BlockMeter {
    let len = buf.len() / 2;
    if let Some(auto) = strip.auto.as_mut() {
        auto.render(pos, len, advancing, advancing);
    }
    if strip.mono {
        for f in buf.as_chunks_mut::<2>().0.iter_mut() {
            let m = 0.5 * (f[0] + f[1]);
            f[0] = m;
            f[1] = m;
        }
    }
    run_inserts(strip, buf, pos, advancing);
    route_sends(strip, buf, bus, edge, chunk, true);
    apply_fader(strip, buf);
    route_sends(strip, buf, bus, edge, chunk, false);
    let meter = BlockMeter::scan(buf);
    shared.meter(
        strip.meter_slot,
        meter.peak,
        meter.sum,
        len as u64,
        meter.clipped,
    );
    if let Some(stems) = stems {
        let gate = strip.out_gate.target.0;
        let base = (strip.meter_slot * stems.frames + stem_offset) * 2;
        if let Some(dst) = stems.data.get_mut(base..base + len * 2) {
            for (d, s) in dst.iter_mut().zip(buf.iter()) {
                *d = s * gate;
            }
        }
    }
    match strip.output {
        Dest::Master => route(
            buf,
            &mut master[..len * 2],
            &mut strip.out_gate,
            &mut strip.out_delay,
            edge,
        ),
        Dest::Aux(a) => {
            let start = a * chunk * 2;
            route(
                buf,
                &mut bus[start..start + len * 2],
                &mut strip.out_gate,
                &mut strip.out_delay,
                edge,
            )
        }
    }
    meter
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

    /// Offline/control thread only. Wait for the disk pages needed by the
    /// next span; never call this from `process` or a device callback.
    pub fn prepare_media(
        &self,
        position: u64,
        frames: usize,
    ) -> Result<(), crate::media::MediaError> {
        self.plan.prefetch(position, frames)
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

    /// Captures every strip's post-fader output of each `process` call
    /// (allocates; offline renders only). Read it with [`Self::stem`].
    pub fn enable_stems(&mut self) {
        self.stems = Some(Stems {
            frames: self.max_block,
            data: vec![0.0; MAX_CHANNELS * self.max_block * 2],
        });
    }

    /// Interleaved stereo stem of meter slot `slot` for the last
    /// `frames` processed frames.
    pub fn stem(&self, slot: usize, frames: usize) -> Option<&[f32]> {
        let s = self.stems.as_ref()?;
        let base = slot * s.frames * 2;
        s.data.get(base..base + frames.min(s.frames) * 2)
    }

    /// Insert latency of the current graph's longest path, in frames.
    pub fn latency_frames(&self) -> usize {
        self.plan.stats.latency_frames
    }

    /// Renders `out.len() / channels` interleaved frames. Real-time safe.
    pub fn process(&mut self, out: &mut [f32], channels: usize, midi: &mut dyn MidiSink) {
        let Some(mut output) = self.midi_output.take() else {
            out.fill(0.0);
            return;
        };
        let overflow = Arc::clone(&self.midi_overflow);
        let mut forward = MidiForward {
            output: &mut output,
            outer: midi,
            started: Instant::now()
                + std::time::Duration::from_secs_f64(
                    self.shared.output_latency() as f64 / f64::from(self.plan.sample_rate),
                ),
            rate: self.plan.sample_rate,
            overflow: &overflow,
        };
        self.process_audio(out, channels, &mut forward);
        self.midi_output = Some(output);
    }

    fn process_audio(&mut self, out: &mut [f32], channels: usize, midi: &mut dyn MidiSink) {
        let started = Instant::now();
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.callback_frames = frames.min(u32::MAX as usize) as u32;
        self.callback_clock = self.shared.clock_now()
            + self.shared.output_latency() * 1_000_000_000 / u64::from(self.plan.sample_rate);
        if self.release_pending {
            self.release_pending = false;
            self.all_notes_off(0, midi);
        }
        self.swap_plan(midi);
        self.drain_commands(midi);
        self.drain_live();
        self.drain_overrides();
        if let Some(s) = self.stems.as_mut() {
            s.data.fill(0.0);
        }
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.chunk);
            self.block_offset = done;
            self.mix[..n * 2].fill(0.0);
            self.render(n, done as u32, midi);
            let dst = &mut out[done * channels..(done + n) * channels];
            let (mut pl, mut pr) = (0.0f32, 0.0f32);
            let (mut sl, mut sr) = (0.0f64, 0.0f64);
            let mut clipped = false;
            for (i, frame) in dst.chunks_exact_mut(channels).enumerate() {
                let input = self.monitor.pop().unwrap_or([0.0; 2]);
                let mut l = self.mix[i * 2] + input[0];
                let mut r = self.mix[i * 2 + 1] + input[1];
                if !l.is_finite() || !r.is_finite() {
                    // Never pass NaN/inf to a device.
                    l = 0.0;
                    r = 0.0;
                    clipped = true;
                }
                pl = pl.max(l.abs());
                pr = pr.max(r.abs());
                sl += f64::from(l) * f64::from(l);
                sr += f64::from(r) * f64::from(r);
                clipped |= l.abs() > 1.0 || r.abs() > 1.0;
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
            self.shared
                .meter(MASTER_SLOT, [pl, pr], [sl, sr], n as u64, clipped);
            if let Some(stems) = self.stems.as_mut() {
                let base = (MASTER_SLOT * stems.frames + done) * 2;
                if let Some(d) = stems.data.get_mut(base..base + n * 2) {
                    d.copy_from_slice(&self.mix[..n * 2]);
                }
            }
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
            frames as u64,
        );
        self.shared
            .add_block(frames as u64, started.elapsed().as_nanos() as u64);
    }

    fn drain_overrides(&mut self) {
        let release = (elyvon_project::automation_record::TOUCH_RELEASE_SECONDS
            * f64::from(self.plan.sample_rate)) as u64;
        while let Ok(o) = self.overrides.pop() {
            let plan = &mut *self.plan;
            let strip = match o.strip {
                StripKey::Track(k) => match plan.find_track(k) {
                    Some(i) => Some(&mut plan.tracks[i].strip),
                    None => None,
                },
                StripKey::Aux(k) => plan.auxes.iter_mut().find(|a| a.strip.key == k).map(|a| &mut a.strip),
                StripKey::Master => Some(&mut plan.master),
            };
            if let Some(auto) = strip.and_then(|s| s.auto.as_mut())
                && auto.mode.writes()
            {
                let mode = auto.mode;
                if let Some(slot) = auto.slot_mut(&o.param) {
                    slot.apply(o.value, o.phase, mode, release);
                }
            }
        }
    }

    /// Ends every live control gesture (transport stopped).
    fn clear_overrides(&mut self) {
        let plan = &mut *self.plan;
        let strips = plan
            .tracks
            .iter_mut()
            .map(|t| &mut t.strip)
            .chain(plan.auxes.iter_mut().map(|a| &mut a.strip))
            .chain(std::iter::once(&mut plan.master));
        for strip in strips {
            if let Some(a) = strip.auto.as_mut() {
                a.clear_overrides();
            }
        }
    }

    fn drain_live(&mut self) {
        while let Ok(m) = self.live.pop() {
            if let Some(i) = self.plan.find_track(m.track)
                && let Some(synth) = self.plan.tracks[i].synth.as_mut()
            {
                synth.message(&m.data[..usize::from(m.len)]);
            }
        }
    }

    fn swap_plan(&mut self, midi: &mut dyn MidiSink) {
        while self.garbage.slots() > 0 {
            let Ok(mut new) = self.plans.pop() else { break };
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
            new.adopt(&mut self.plan);
            self.carry_notes(&mut new, midi);
            let old = std::mem::replace(&mut self.plan, new);
            // A free slot was checked above; push cannot fail.
            let _ = self.garbage.push(old);
        }
    }

    /// Keeps notes that still sound in the new graph and releases the rest,
    /// so editing during playback neither cuts held notes nor leaves any
    /// hanging. Allocation-free.
    fn carry_notes(&mut self, new: &mut PlaybackPlan, midi: &mut dyn MidiSink) {
        let at = self.position;
        let playing = self.state == PlayState::Playing;
        for (i, any) in self.active_any.iter_mut().enumerate() {
            if *any == 0 {
                continue;
            }
            let target = self
                .plan
                .tracks
                .get(i)
                .and_then(|t| new.find_track(t.key))
                .filter(|&j| new.tracks[j].audible);
            for ch in 0..16u8 {
                for pitch in 0..128u8 {
                    let count = &mut self.active_notes[i][ch as usize][pitch as usize];
                    if *count == 0 {
                        continue;
                    }
                    let keep =
                        playing && target.is_some_and(|j| new.tracks[j].covers(at, ch, pitch));
                    let releases = if keep { *count - 1 } else { *count };
                    for _ in 0..releases {
                        midi.event(
                            i,
                            MidiEvent {
                                frame_offset: 0,
                                message: MidiMessage::NoteOff {
                                    channel: ch,
                                    pitch,
                                    velocity: 0,
                                },
                            },
                        );
                        if let Some(synth) = target
                            .or_else(|| self.plan.tracks.get(i).and_then(|t| new.find_track(t.key)))
                            .and_then(|j| new.tracks[j].synth.as_mut())
                        {
                            synth.message(&[0x80 | ch, pitch, 0]);
                        }
                    }
                    if let (true, Some(j)) = (keep, target) {
                        let slot = &mut self.active_scratch[j][ch as usize][pitch as usize];
                        if *slot == 0 {
                            self.any_scratch[j] += 1;
                        }
                        *slot = 1;
                    }
                    *count = 0;
                }
            }
            *any = 0;
        }
        std::mem::swap(&mut self.active_notes, &mut self.active_scratch);
        std::mem::swap(&mut self.active_any, &mut self.any_scratch);
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
        self.clear_overrides();
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
        midi.reset(frame);
        for (track, (notes, any)) in self
            .active_notes
            .iter_mut()
            .zip(self.active_any.iter_mut())
            .enumerate()
        {
            if *any == 0 {
                continue;
            }
            release_track(notes, any, track, frame, midi);
        }
        for track in &mut self.plan.tracks {
            if let Some(s) = track.synth.as_mut() {
                s.all_notes_off();
            }
        }
    }

    /// Renders `n` frames into `mix[..n*2]`; `base` is the frame offset of
    /// this internal block inside the callback (for MIDI timestamps).
    fn render(&mut self, n: usize, base: u32, midi: &mut dyn MidiSink) {
        let mut off = 0usize;
        if let Some(mut c) = self.count_in {
            let len = n.min((c.total - c.pos) as usize);
            self.shared
                .record_clock(self.callback_clock, base, len, c.target, PlayState::CountIn);
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
            self.shared.record_clock(
                self.callback_clock,
                base + off as u32,
                n - off,
                self.position,
                self.state,
            );
            // Live instruments (virtual keyboard, previews) and their effects
            // keep sounding while the transport is stopped.
            self.render_graph(self.position, n - off, off, base, midi, false);
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
            self.shared.record_clock(
                self.callback_clock,
                base + off as u32,
                len,
                self.position,
                PlayState::Playing,
            );
            self.render_graph(self.position, len, off, base, midi, true);
            self.position += len as u64;
            off += len;
            if let Some((cs, ce)) = cycle
                && self.position == ce
            {
                self.position = cs;
                let boundary = base + off as u32;
                if boundary == self.callback_frames {
                    self.release_pending = true;
                } else {
                    self.all_notes_off(boundary, midi);
                }
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

    /// Processes the mixer graph for `[pos, pos + len)`. With `timeline`
    /// false only live sources (instruments) play and no events are read.
    #[allow(clippy::too_many_arguments)]
    fn render_graph(
        &mut self,
        pos: u64,
        len: usize,
        off: usize,
        base: u32,
        midi: &mut dyn MidiSink,
        timeline: bool,
    ) {
        if len == 0 {
            return;
        }
        let q = pos + len as u64;
        let chunk = self.chunk;
        let stem_offset = self.block_offset + off;
        let plan = &mut *self.plan;
        for a in 0..plan.auxes.len() {
            self.bus[a * chunk * 2..a * chunk * 2 + len * 2].fill(0.0);
        }
        self.master_buf[..len * 2].fill(0.0);
        let frame0 = base + off as u32;
        for (ti, track) in plan.tracks.iter_mut().enumerate() {
            self.synth_events.clear();
            if !track.audible {
                release_track(
                    &mut self.active_notes[ti],
                    &mut self.active_any[ti],
                    ti,
                    frame0,
                    midi,
                );
                if let Some(s) = track.synth.as_mut()
                    && s.active_voices() > 0
                {
                    s.all_notes_off();
                }
            } else if timeline {
                let first = track.events.partition_point(|e| e.on < pos);
                for (seq, event) in track.events[first..]
                    .iter()
                    .take_while(|e| e.on < q)
                    .enumerate()
                {
                    let frame = (event.on - pos) as u32;
                    midi.packet(ti, frame0 + frame, event.packet);
                    let bytes = event.packet.bytes();
                    if track.synth.is_some()
                        && (1..=3).contains(&bytes.len())
                        && bytes[0] < 0xf0
                        && self.synth_events.len() < self.synth_events.capacity()
                    {
                        let mut data = [0; 3];
                        data[..bytes.len()].copy_from_slice(bytes);
                        self.synth_events.push(SynthEvent {
                            frame,
                            class: 0,
                            seq: seq.min(u16::MAX as usize) as u16,
                            len: bytes.len() as u8,
                            data,
                        });
                    }
                }
                if !track.notes.is_empty() {
                    schedule_notes(
                        track,
                        pos,
                        q,
                        frame0,
                        &mut self.events,
                        &mut self.active_notes[ti],
                        &mut self.active_any[ti],
                        ti,
                        &self.shared,
                        midi,
                        &mut self.synth_events,
                    );
                }
            }
            if let Some(m) = self.measure.as_mut().and_then(|m| m.get_mut(ti)) {
                m.frames += len as u64;
            }
            let has_synth_sound = track.synth.as_ref().is_some_and(|s| s.active_voices() > 0)
                || !self.synth_events.is_empty();
            if !has_synth_sound && (!timeline || track.strip.is_idle()) {
                continue;
            }
            let buf = &mut self.track_buf[..len * 2];
            buf.fill(0.0);
            if timeline && !track.segments.is_empty() {
                mix_track(track, pos, q, buf, &self.shared);
            }
            if let Some(synth) = track.synth.as_mut() {
                self.synth_events
                    .sort_unstable_by_key(|e| (e.frame, e.class, e.seq));
                let mut cursor = 0usize;
                for e in &self.synth_events {
                    let f = (e.frame as usize).min(len);
                    if f > cursor {
                        synth.render(&mut buf[cursor * 2..f * 2]);
                        cursor = f;
                    }
                    synth.message(&e.data[..usize::from(e.len)]);
                }
                synth.render(&mut buf[cursor * 2..len * 2]);
            }
            let meter = process_strip(
                &mut track.strip,
                buf,
                &mut self.edge,
                &mut self.bus,
                &mut self.master_buf,
                chunk,
                &self.shared,
                self.stems.as_mut(),
                stem_offset,
                pos,
                timeline,
            );
            if let Some(m) = self.measure.as_mut().and_then(|m| m.get_mut(ti)) {
                m.peak = m.peak.max(meter.peak[0].max(meter.peak[1]));
                m.sum_squares += meter.sum[0] + meter.sum[1];
            }
        }
        for &a in &plan.aux_order {
            let strip = &mut plan.auxes[a].strip;
            if strip.is_idle() {
                continue;
            }
            let scratch = &mut self.scratch[..len * 2];
            scratch.copy_from_slice(&self.bus[a * chunk * 2..a * chunk * 2 + len * 2]);
            process_strip(
                strip,
                scratch,
                &mut self.edge,
                &mut self.bus,
                &mut self.master_buf,
                chunk,
                &self.shared,
                self.stems.as_mut(),
                stem_offset,
                pos,
                timeline,
            );
        }
        let master = &mut plan.master;
        let buf = &mut self.master_buf[..len * 2];
        if let Some(a) = master.auto.as_mut() {
            a.render(pos, len, timeline, timeline);
        }
        run_inserts(master, buf, pos, timeline);
        let dst = &mut self.mix[off * 2..(off + len) * 2];
        if master.auto.as_ref().is_some_and(|a| a.fader_dynamic()) {
            apply_fader(master, buf);
            let mut unity = Smoothed::new((1.0, 1.0));
            route(buf, dst, &mut unity, &mut master.out_delay, &mut self.edge);
        } else {
            route(
                buf,
                dst,
                &mut master.fader,
                &mut master.out_delay,
                &mut self.edge,
            );
        }
        if timeline && plan.metronome {
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

/// Mixes a track's audio for `[p, q)` into `buf` (stereo) at unity: the
/// strip applies format, inserts, fader and pan. Returns whether anything
/// was written.
fn mix_track(track: &PlanTrack, p: u64, q: u64, buf: &mut [f32], shared: &EngineShared) -> bool {
    let mut any = false;
    for seg in &track.segments[track.segment_range(p, q)] {
        let a = seg.start.max(p);
        let b = seg.end.min(q);
        if a >= b {
            continue;
        }
        any = true;
        let media = &seg.media;
        let before = media.stream.as_ref().map_or(0, |s| s.underruns());
        for t in a..b {
            let Some(src) = seg.source_frame(t) else {
                break;
            };
            let g = seg.envelope(t);
            let [left, right] = if seg.reversed {
                media.sample_reverse(src)
            } else {
                media.sample(src)
            };
            let i = ((t - p) as usize) * 2;
            buf[i] += left * g;
            buf[i + 1] += right * g;
        }
        let after = media.stream.as_ref().map_or(0, |s| s.underruns());
        shared.add_streaming_underruns(after.saturating_sub(before));
    }
    any
}

fn release_track(
    notes: &mut NoteCounts,
    any: &mut u32,
    track: usize,
    frame: u32,
    midi: &mut dyn MidiSink,
) {
    if *any == 0 {
        return;
    }
    for (channel, pitches) in notes.iter_mut().enumerate() {
        for (pitch, count) in pitches.iter_mut().enumerate() {
            while *count > 0 {
                *count -= 1;
                midi.event(
                    track,
                    MidiEvent {
                        frame_offset: frame,
                        message: MidiMessage::NoteOff {
                            channel: channel as u8,
                            pitch: pitch as u8,
                            velocity: 0,
                        },
                    },
                );
            }
        }
    }
    *any = 0;
}

#[allow(clippy::too_many_arguments)]
fn schedule_notes(
    track: &PlanTrack,
    p: u64,
    q: u64,
    frame0: u32,
    events: &mut Vec<PendingEvent>,
    active: &mut NoteCounts,
    any: &mut u32,
    ti: usize,
    shared: &EngineShared,
    midi: &mut dyn MidiSink,
    synth_events: &mut Vec<SynthEvent>,
) {
    events.clear();
    let mut overflow = false;
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
                overflow = true;
            }
        };
        if note.on >= p {
            push(PendingEvent {
                frame: (note.on - p) as u32,
                off: false,
                channel: note.channel,
                pitch: note.pitch,
                velocity: note.velocity,
            });
        }
        if note.off >= p && note.off < q {
            push(PendingEvent {
                frame: (note.off - p) as u32,
                off: true,
                channel: note.channel,
                pitch: note.pitch,
                velocity: 0,
            });
        }
    }
    if overflow {
        release_track(active, any, ti, frame0, midi);
        midi.reset(frame0);
        events.clear();
        return;
    }
    // Stable order: by frame, note-offs before note-ons (allocation-free sort).
    events.sort_unstable_by_key(|e| (e.frame, !e.off, e.channel, e.pitch));
    let instrument = track.synth.is_some();
    for (seq, e) in events.iter().enumerate() {
        let count = &mut active[e.channel as usize][e.pitch as usize];
        let message = if e.off {
            if *count == 0 {
                continue;
            }
            *count -= 1;
            *any = any.saturating_sub(u32::from(*count == 0));
            MidiMessage::NoteOff {
                channel: e.channel,
                pitch: e.pitch,
                velocity: 0,
            }
        } else {
            if *count == 0 {
                *any += 1;
            }
            *count = count.saturating_add(1);
            MidiMessage::NoteOn {
                channel: e.channel,
                pitch: e.pitch,
                velocity: e.velocity,
            }
        };
        if instrument && synth_events.len() < synth_events.capacity() {
            let (status, v, class) = if e.off {
                (0x80, 0, 1)
            } else {
                (0x90, e.velocity, 2)
            };
            synth_events.push(SynthEvent {
                frame: e.frame,
                class,
                seq: seq.min(u16::MAX as usize) as u16,
                len: 3,
                data: [status | e.channel, e.pitch, v],
            });
        }
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

struct MidiForward<'a> {
    output: &'a mut Producer<crate::midi::ScheduledPacket>,
    outer: &'a mut dyn MidiSink,
    started: Instant,
    rate: u32,
    overflow: &'a std::sync::atomic::AtomicBool,
}
impl MidiSink for MidiForward<'_> {
    fn event(&mut self, track: usize, event: MidiEvent) {
        self.outer.event(track, event);
        if let Some(packet) = crate::midi::Packet::from_message(event.message) {
            self.queue(track, event.frame_offset, packet);
        }
    }
    fn packet(&mut self, track: usize, frame: u32, packet: crate::midi::Packet) {
        self.outer.packet(track, frame, packet);
        self.queue(track, frame, packet);
    }
    fn reset(&mut self, frame: u32) {
        self.outer.reset(frame);
        self.queue(usize::MAX, frame, crate::midi::Packet::PANIC);
    }
}
impl MidiForward<'_> {
    fn queue(&mut self, track: usize, frame: u32, packet: crate::midi::Packet) {
        let event = crate::midi::ScheduledPacket {
            track,
            deadline: self.started
                + std::time::Duration::from_secs_f64(f64::from(frame) / f64::from(self.rate)),
            packet,
        };
        if self.output.push(event).is_err() {
            self.overflow
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}
