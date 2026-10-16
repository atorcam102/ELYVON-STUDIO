//! Native MIDI owner: device callbacks enqueue packets; this worker performs
//! OS sends, timestamped monitoring, MIDI recording and port recovery.
use crate::{
    error::{ErrorCode, IpcError, IpcResult},
    state::AppState,
};
use elyvon_audio::{
    midi::{
        Packet,
        ports::{MidiObservation, MidiPortList, Ports, Route},
        smf,
    },
    transport::{EngineShared, PlayState, TransportCommand},
};
use elyvon_project::{
    Project, ProjectId, TrackId, TrackKind,
    io::{MidiImportRegion, StoredMidiEvent},
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender},
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use ts_rs::TS;
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
#[derive(Debug, Clone, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MidiStatus {
    pub ports: MidiPortList,
    pub monitor: Vec<MidiObservation>,
    pub errors: Vec<String>,
    pub recording: bool,
    #[ts(type = "number")]
    pub invalid_messages: u64,
    #[ts(type = "number")]
    pub max_scheduler_jitter_us: u64,
}
#[derive(Debug, Clone)]
pub struct RecordedMidi {
    pub project: ProjectId,
    pub regions: Vec<MidiImportRegion>,
    pub error: Option<String>,
}
enum Request {
    Configure(Vec<Route>),
    Refresh,
    Send(TrackId, Packet, SyncSender<Result<(), String>>),
    /// Piano-roll audition: note on now, note off after the duration. Not
    /// recorded.
    Preview(TrackId, u8, u8, u8, Duration),
    Panic(SyncSender<()>),
    Start(
        Project,
        u64,
        Arc<AtomicBool>,
        SyncSender<Result<(), String>>,
    ),
    Stop(bool, SyncSender<Result<RecordedMidi, String>>),
    Shutdown(SyncSender<()>),
}
#[derive(Default)]
pub struct MidiService {
    tx: Mutex<Option<SyncSender<Request>>>,
    status: Arc<Mutex<MidiStatus>>,
    recording: Arc<AtomicBool>,
    pending: Arc<Mutex<Option<RecordedMidi>>>,
}
impl std::fmt::Debug for MidiService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiService").finish_non_exhaustive()
    }
}
impl MidiService {
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Acquire)
    }
    pub fn ensure_idle(&self) -> IpcResult<()> {
        if self.is_recording() {
            Err(IpcError::new(
                ErrorCode::Busy,
                "Stop MIDI recording before editing the project or transport.",
            ))
        } else {
            Ok(())
        }
    }
    pub fn status(&self) -> MidiStatus {
        let mut s = lock(&self.status).clone();
        s.recording = self.is_recording();
        s
    }
    fn send_request(&self, r: Request) -> Result<(), String> {
        lock(&self.tx)
            .as_ref()
            .ok_or("MIDI service is starting")?
            .try_send(r)
            .map_err(|_| "MIDI control queue is busy".into())
    }
    pub fn configure(&self, project: &Project) {
        let routes = project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind != TrackKind::Audio)
            .map(|(index, t)| Route {
                index,
                track: t.id,
                io: t.io.clone(),
            })
            .collect();
        let _ = self.send_request(Request::Configure(routes));
    }
    pub fn refresh(&self) -> IpcResult<()> {
        self.send_request(Request::Refresh).map_err(midi_error)
    }
    pub fn panic(&self) -> IpcResult<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.send_request(Request::Panic(tx)).map_err(midi_error)?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| midi_error("MIDI panic timed out"))
    }
    pub fn send(&self, track: TrackId, bytes: Vec<u8>) -> IpcResult<()> {
        let packet = Packet::parse(&bytes)
            .ok_or_else(|| midi_error("invalid MIDI packet (SysEx limit: 256 bytes)"))?;
        let (tx, rx) = mpsc::sync_channel(1);
        self.send_request(Request::Send(track, packet, tx))
            .map_err(midi_error)?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| midi_error("MIDI send timed out"))?
            .map_err(midi_error)
    }
    /// Auditions a note on the track's instrument or MIDI output; the note
    /// is released by the worker after `duration` (at most 4 s), so a lost
    /// pointer-up can never leave it hanging.
    pub fn preview(
        &self,
        track: TrackId,
        channel: u8,
        pitch: u8,
        velocity: u8,
        duration: Duration,
    ) -> IpcResult<()> {
        if channel > 15 || pitch > 127 || !(1..=127).contains(&velocity) {
            return Err(midi_error("invalid preview note"));
        }
        self.send_request(Request::Preview(
            track,
            channel,
            pitch,
            velocity,
            duration.min(Duration::from_secs(4)),
        ))
        .map_err(midi_error)
    }
    pub fn start_record(&self, state: &AppState, tick: u64) -> IpcResult<()> {
        let project = {
            let session = state.session();
            state.ensure_editable()?;
            if self.recording.swap(true, Ordering::AcqRel) {
                return Err(midi_error("MIDI recording is already running"));
            }
            session.project().clone()
        };
        let (tx, rx) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = self
            .send_request(Request::Start(project, tick, Arc::clone(&cancelled), tx))
            .and_then(|_| {
                rx.recv_timeout(Duration::from_secs(15))
                    .map_err(|_| "MIDI record startup timed out".to_string())?
            });
        if result.is_err() {
            cancelled.store(true, Ordering::Release);
            self.recording.store(false, Ordering::Release);
        }
        result.map_err(midi_error)
    }
    pub fn stop_record(&self, cancel: bool) -> IpcResult<RecordedMidi> {
        if let Some(capture) = lock(&self.pending).clone() {
            return Ok(if cancel {
                RecordedMidi {
                    project: capture.project,
                    regions: vec![],
                    error: None,
                }
            } else {
                capture
            });
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.send_request(Request::Stop(cancel, tx))
            .map_err(midi_error)?;
        rx.recv_timeout(Duration::from_secs(15))
            .map_err(|_| midi_error("MIDI stop timed out"))?
            .map_err(midi_error)
    }
    pub fn finish_record(&self) {
        lock(&self.pending).take();
        self.recording.store(false, Ordering::Release);
    }
    pub fn shutdown(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.send_request(Request::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
    pub fn start_worker(&self, app: AppHandle) {
        let (tx, rx) = mpsc::sync_channel(256);
        *lock(&self.tx) = Some(tx);
        let status = Arc::clone(&self.status);
        let pending = Arc::clone(&self.pending);
        let spawned = std::thread::Builder::new()
            .name("elyvon-midi-ports".into())
            .spawn(move || {
                let mut ports = Ports::new();
                let mut recording: Option<MidiRecording> = None;
                let mut previews: Vec<(Instant, TrackId, Packet)> = Vec::new();
                // Live messages also play the track's built-in instrument.
                let live = |track: TrackId, packet: &Packet| {
                    app.state::<AppState>()
                        .audio
                        .send_live(track, packet.bytes());
                };
                let mut published = Instant::now() - Duration::from_secs(1);
                loop {
                    let context = app.state::<AppState>().audio.poll_midi();
                    let shared = context.as_ref().map(|(_, _, s)| Arc::clone(s));
                    if let Some((packets, overflow, _)) = context {
                        ports.schedule(packets, overflow);
                    }
                    while let Ok(request) = rx.try_recv() {
                        match request {
                            Request::Configure(routes) => ports.configure(routes, false),
                            Request::Refresh => ports.configure(ports.routes.clone(), true),
                            Request::Panic(reply) => {
                                for (_, track, off) in previews.drain(..) {
                                    live(track, &off);
                                }
                                ports.panic();
                                let _ = reply.send(());
                            }
                            Request::Send(track, packet, reply) => {
                                let now = Instant::now();
                                live(track, &packet);
                                let result = ports.send(track, packet, "virtual keyboard", now);
                                if let Some(record) = &mut recording
                                    && let Some(shared) = &shared
                                {
                                    record.push(track, packet, now, shared);
                                }
                                let _ = reply.send(result);
                            }
                            Request::Preview(track, channel, pitch, velocity, duration) => {
                                let now = Instant::now();
                                let (Some(on), Some(off)) = (
                                    Packet::parse(&[0x90 | channel, pitch, velocity]),
                                    Packet::parse(&[0x80 | channel, pitch, 0]),
                                ) else {
                                    continue;
                                };
                                live(track, &on);
                                if let Err(e) = ports.send(track, on, "preview", now) {
                                    ports.error(e);
                                }
                                if previews.len() < 256 {
                                    previews.push((now + duration, track, off));
                                }
                            }
                            Request::Start(project, tick, cancelled, reply) => {
                                let result = (|| -> Result<(), String> {
                                    if cancelled.load(Ordering::Acquire) {
                                        return Err("MIDI startup was cancelled".into());
                                    }
                                    if recording.is_some() {
                                        return Err("MIDI recording is already running".into());
                                    }
                                    if !project
                                        .tracks
                                        .iter()
                                        .any(|t| t.kind != TrackKind::Audio && t.io.armed)
                                    {
                                        return Err(
                                            "Arm at least one MIDI or instrument track.".into()
                                        );
                                    }
                                    let shared =
                                        shared.as_ref().ok_or("audio engine is not running")?;
                                    if tick > elyvon_project::validate::limits::MAX_TICKS {
                                        return Err("MIDI start exceeds timeline bounds".into());
                                    }
                                    let record = MidiRecording::new(
                                        project.clone(),
                                        tick,
                                        shared.sample_rate,
                                    )?;
                                    let t = &project.transport;
                                    app.state::<AppState>()
                                        .audio
                                        .command(TransportCommand::Play {
                                            from_tick: Some(tick),
                                            count_in: t.count_in_bars > 0,
                                            pre_roll: t.pre_roll_bars > 0,
                                        })
                                        .ok_or("transport command failed")?;
                                    if cancelled.load(Ordering::Acquire) {
                                        app.state::<AppState>()
                                            .audio
                                            .command(TransportCommand::Stop);
                                        return Err("MIDI startup was cancelled".into());
                                    }
                                    recording = Some(record);
                                    Ok(())
                                })();
                                if reply.send(result).is_err() {
                                    recording = None;
                                    ports.panic();
                                    app.state::<AppState>()
                                        .audio
                                        .command(TransportCommand::Stop);
                                }
                            }
                            Request::Stop(cancel, reply) => {
                                ports.panic();
                                app.state::<AppState>()
                                    .audio
                                    .command(TransportCommand::Stop);
                                let endpoint = shared
                                    .as_ref()
                                    .and_then(|s| s.capture_position(Duration::ZERO))
                                    .map(|(frame, _, cycle)| (frame, cycle));
                                let result = recording
                                    .take()
                                    .ok_or_else(|| "No MIDI recording is running.".into())
                                    .map(|r| r.finish(cancel, endpoint));
                                if let Ok(data) = &result {
                                    *lock(&pending) = Some(data.clone());
                                }
                                let _ = reply.send(result);
                            }
                            Request::Shutdown(reply) => {
                                ports.panic();
                                let _ = reply.send(());
                                return;
                            }
                        }
                    }
                    for incoming in ports.incoming() {
                        let name = ports
                            .list
                            .inputs
                            .get(incoming.port)
                            .map_or_else(|| "disconnected input".into(), |p| p.name.clone());
                        for route in ports.matching(&incoming) {
                            ports.observe(
                                "input",
                                &name,
                                Some(route.track),
                                incoming.packet,
                                incoming.when,
                            );
                            if route.io.midi_monitor {
                                live(route.track, &incoming.packet);
                                if let Err(e) =
                                    ports.send(route.track, incoming.packet, "input", incoming.when)
                                {
                                    ports.error(e);
                                }
                            }
                            if let (Some(record), Some(shared)) = (&mut recording, &shared) {
                                record.push(route.track, incoming.packet, incoming.when, shared);
                            }
                        }
                    }
                    let now = Instant::now();
                    previews.retain(|&(due, track, off)| {
                        if due > now {
                            return true;
                        }
                        live(track, &off);
                        if let Err(e) = ports.send(track, off, "preview", now) {
                            ports.error(e);
                        }
                        false
                    });
                    ports.tick();
                    if published.elapsed() >= Duration::from_millis(50) {
                        *lock(&status) = MidiStatus {
                            ports: ports.list.clone(),
                            monitor: ports.monitor.iter().cloned().collect(),
                            errors: ports.errors.iter().cloned().collect(),
                            recording: recording.is_some(),
                            invalid_messages: ports.health.invalid.load(Ordering::Relaxed),
                            max_scheduler_jitter_us: ports.max_jitter_us,
                        };
                        published = Instant::now();
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        if let Err(e) = spawned {
            lock(&self.status)
                .errors
                .push(format!("MIDI worker could not start: {e}"));
        }
    }
}
fn midi_error(s: impl Into<String>) -> IpcError {
    IpcError::new(ErrorCode::AudioUnavailable, s)
}
struct MidiRecording {
    project: Project,
    map: elyvon_core::TempoMap,
    start: u64,
    rate: u32,
    started: Instant,
    events: HashMap<(TrackId, u64), Vec<StoredMidiEvent>>,
    error: Option<String>,
}
impl MidiRecording {
    fn new(project: Project, start: u64, rate: u32) -> Result<Self, String> {
        let map = project.transport.tempo_map().map_err(|e| e.to_string())?;
        Ok(Self {
            project,
            map,
            start,
            rate,
            started: Instant::now(),
            events: HashMap::new(),
            error: None,
        })
    }
    fn push(&mut self, track: TrackId, packet: Packet, when: Instant, shared: &EngineShared) {
        let Some(t) = self
            .project
            .tracks
            .iter()
            .find(|t| t.id == track && t.io.armed && t.kind != TrackKind::Audio)
        else {
            return;
        };
        if when < self.started
            || t.io
                .midi_channel
                .is_some_and(|ch| packet.channel() != Some(ch))
        {
            return;
        }
        if shared.sample_rate != self.rate {
            self.error = Some(
                "The engine rate changed; captured MIDI before the interruption was preserved."
                    .into(),
            );
            return;
        }
        let Some((frame, state, cycle)) =
            shared.capture_position(Instant::now().saturating_duration_since(when))
        else {
            return;
        };
        if state != PlayState::Playing {
            return;
        }
        let tick = self.map.tick_at_sample(frame, self.rate).round() as u64;
        if cycle == 0 && tick < self.start {
            return;
        }
        let punch = &self.project.transport.punch;
        if punch.enabled && (tick < punch.start || tick >= punch.end) {
            return;
        }
        let base = if cycle > 0 {
            self.project.transport.loop_range.start
        } else {
            self.start
        };
        let events = self.events.entry((track, cycle)).or_default();
        if events.len() >= 100000 {
            self.error = Some(
                "MIDI recording reached 100000 events; captured events were preserved.".into(),
            );
            return;
        }
        events.push(StoredMidiEvent {
            tick: tick.saturating_sub(base),
            bytes: packet.bytes().to_vec(),
        });
    }
    fn finish(self, cancel: bool, endpoint: Option<(u64, u64)>) -> RecordedMidi {
        let mut regions = Vec::new();
        if !cancel {
            for ((track, cycle), mut events) in self.events {
                events.sort_by_key(|e| e.tick);
                let base = if cycle > 0 {
                    self.project.transport.loop_range.start
                } else {
                    self.start
                };
                let stop_tick = endpoint.map(|(frame, last_cycle)| {
                    if cycle < last_cycle && self.project.transport.loop_range.enabled {
                        self.project.transport.loop_range.end
                    } else {
                        self.map.tick_at_sample(frame, self.rate).round() as u64
                    }
                });
                let end_tick = stop_tick.map(|t| {
                    if self.project.transport.punch.enabled {
                        t.min(self.project.transport.punch.end)
                    } else {
                        t
                    }
                });
                let length = end_tick
                    .map_or(1, |t| t.saturating_sub(base))
                    .max(events.last().map_or(1, |e| e.tick + 1));
                smf::close_sequence(&mut events, length);
                let notes = smf::notes_from_events(&events, length);
                regions.push(MidiImportRegion {
                    name: format!("Recorded MIDI · take {}", cycle + 1),
                    track_id: Some(track),
                    start: if cycle > 0 {
                        self.project.transport.loop_range.start
                    } else {
                        self.start
                    },
                    length,
                    notes,
                    events,
                });
            }
        }
        regions.sort_by_key(|r| (r.track_id, r.start));
        RecordedMidi {
            project: self.project.id,
            regions,
            error: self.error,
        }
    }
}
