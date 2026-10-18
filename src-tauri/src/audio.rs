//! Native audio service: owns the engine handle, the output (device or
//! internal clock), the decoded media store and the media loader thread.
//!
//! Nothing here runs on the audio thread. Lock order: session → engine
//! (`inner`) → media store → loading set; never the reverse. The service rebuilds and publishes a
//! playback plan after every project change, loads media in the background
//! and publishes engine state to the UI as events at ~30 Hz.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use elyvon_audio::device::{self, RunningOutput};
use elyvon_audio::engine::{self, EngineHandle, EngineStatus, OutputKind};
use elyvon_audio::media::{
    MediaBuffer, MediaStatus, MediaStore, default_budget_bytes, resolve_path,
};
use elyvon_audio::plan::{PlaybackPlan, build_plan};
use elyvon_audio::transport::{TransportCommand, TransportSnapshot};
use elyvon_core::TapTempo;
use elyvon_project::{MediaId, Project};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::AudioSettings;
use crate::state::AppState;

pub const POSITION_EVENT: &str = "audio://position";
pub const METERS_EVENT: &str = "audio://meters";
pub const STATUS_EVENT: &str = "audio://status";
pub const MEDIA_EVENT: &str = "media://status";

const INTERNAL_BLOCK: u32 = 512;
const MAX_BLOCK: usize = 4096;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Inner {
    plan: Option<PlaybackPlan>,
    handle: Option<EngineHandle>,
    output: Option<RunningOutput>,
    status: EngineStatus,
    /// Project rate the current output was opened for.
    requested_rate: u32,
    tap: TapTempo,
    meter_reader: elyvon_audio::transport::MeterReader,
    epoch: Instant,
    /// Effect-editor spectrum (reads the engine's analyser queue).
    analyzer: Option<elyvon_audio::spectrum::SpectrumAnalyzer>,
}

struct LoadJob {
    id: MediaId,
    path: PathBuf,
    rate: u32,
}

pub struct AudioService {
    inner: Mutex<Inner>,
    store: Mutex<MediaStore>,
    loading: Mutex<HashSet<MediaId>>,
    loader: Mutex<Option<Sender<LoadJob>>>,
    generation: AtomicU64,
}

impl std::fmt::Debug for AudioService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioService").finish_non_exhaustive()
    }
}

impl Default for AudioService {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioService {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                plan: None,
                handle: None,
                output: None,
                status: EngineStatus::Unavailable {
                    reason: "The audio engine is starting.".into(),
                },
                requested_rate: Project::DEFAULT_SAMPLE_RATE,
                tap: TapTempo::default(),
                meter_reader: elyvon_audio::transport::MeterReader::default(),
                epoch: Instant::now(),
                analyzer: None,
            }),
            store: Mutex::new(MediaStore::new(
                Project::DEFAULT_SAMPLE_RATE,
                default_budget_bytes(),
            )),
            loading: Mutex::new(HashSet::new()),
            loader: Mutex::new(None),
            generation: AtomicU64::new(1),
        }
    }

    pub fn poll_midi(
        &self,
    ) -> Option<(
        Vec<elyvon_audio::midi::ScheduledPacket>,
        bool,
        Arc<elyvon_audio::transport::EngineShared>,
    )> {
        let mut inner = lock(&self.inner);
        let h = inner.handle.as_mut()?;
        let (events, overflow) = h.drain_midi();
        Some((events, overflow, Arc::clone(h.shared())))
    }
    pub fn capture_context(
        &self,
    ) -> Option<(
        u32,
        Arc<elyvon_audio::transport::EngineShared>,
        Option<elyvon_audio::engine::MonitorInput>,
    )> {
        let mut inner = lock(&self.inner);
        let handle = inner.handle.as_mut()?;
        Some((
            handle.sample_rate(),
            Arc::clone(handle.shared()),
            handle.take_monitor_input(),
        ))
    }
    pub fn status(&self) -> EngineStatus {
        lock(&self.inner).status.clone()
    }

    pub fn rate(&self) -> u32 {
        lock(&self.store).rate()
    }

    /// (Re)opens the output for `project_rate` and connects a fresh engine.
    /// Falls back to the internal clock when no device can be opened.
    pub fn start(&self, app: &AppHandle, settings: &AudioSettings, project_rate: u32) {
        app.state::<AppState>().capture.interrupt();
        let mut inner = lock(&self.inner);
        // Stop the old output first so the device is free.
        inner.output = None;
        inner.handle = None;
        inner.plan = None;
        inner.analyzer = None;
        inner.meter_reader = elyvon_audio::transport::MeterReader::default();
        inner.requested_rate = project_rate;
        let mut device_error = None;
        let mut started = None;
        if settings.use_device {
            let buffer = (settings.buffer_frames > 0).then_some(settings.buffer_frames);
            match device::negotiate(settings.output_device.as_deref(), project_rate, buffer) {
                Ok((negotiated, format)) => {
                    let rate = negotiated.config.sample_rate;
                    let (handle, mut eng) =
                        engine::create(rate, MAX_BLOCK, PlaybackPlan::empty(rate));
                    eng.set_realtime(true);
                    let shared = Arc::clone(handle.shared());
                    match device::start(&negotiated, format, eng, shared) {
                        Ok(output) => started = Some((handle, output)),
                        Err(e) => device_error = Some(e),
                    }
                }
                Err(e) => device_error = Some(e),
            }
        }
        let (handle, output) = started.unwrap_or_else(|| {
            let (handle, mut eng) =
                engine::create(project_rate, MAX_BLOCK, PlaybackPlan::empty(project_rate));
            eng.set_realtime(true);
            let output = device::start_internal_clock(eng, project_rate, INTERNAL_BLOCK);
            (handle, output)
        });
        let rate = output.config.sample_rate;
        match (&device_error, output.kind) {
            (Some(e), OutputKind::InternalClock) => {
                log::warn!("audio device unavailable ({e}); using the internal clock")
            }
            _ => log::info!(
                "audio output: {} at {} Hz ({:?})",
                output.name,
                rate,
                output.kind
            ),
        }
        inner.status = EngineStatus::Running {
            config: output.config,
            device: match &device_error {
                Some(e) if settings.use_device => {
                    format!("{} — device unavailable: {e}", output.name)
                }
                _ => output.name.clone(),
            },
            output: output.kind,
        };
        let mut handle = handle;
        inner.analyzer = handle
            .take_analyzer()
            .map(elyvon_audio::spectrum::SpectrumAnalyzer::new);
        inner.handle = Some(handle);
        inner.output = Some(output);
        drop(inner);
        let rate_changed = {
            let mut store = lock(&self.store);
            let changed = store.rate() != rate;
            store.set_rate(rate);
            changed
        };
        if rate_changed {
            lock(&self.loading).clear();
        }
        let _ = app.emit(STATUS_EVENT, self.status());
        self.sync(app);
    }

    /// Rebuilds the playback plan from the open project and publishes it;
    /// queues decoding of media that is not loaded yet.
    pub fn sync(&self, app: &AppHandle) {
        let state = app.state::<AppState>();
        let (project, dir) = {
            let s = state.session();
            (
                s.project().clone(),
                s.path().and_then(Path::parent).map(Path::to_path_buf),
            )
        };
        state.midi.configure(&project);
        let restart = {
            let inner = lock(&self.inner);
            inner.handle.is_some() && inner.requested_rate != project.sample_rate
        };
        if restart {
            let settings = state.settings().audio.clone();
            // `start` calls back into `sync` once the new engine runs.
            self.start(app, &settings, project.sample_rate);
            return;
        }
        self.queue_loads(app, &project, dir.as_deref());
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let mut inner = lock(&self.inner);
        let Some(handle) = inner.handle.as_mut() else {
            return;
        };
        let plan = {
            let store = lock(&self.store);
            build_plan(&project, &|id| store.ready(id), store.rate(), generation)
        };
        if plan.sample_rate == handle.sample_rate() {
            handle.publish(plan.clone());
            inner.plan = Some(plan);
        }
    }

    fn queue_loads(&self, app: &AppHandle, project: &Project, dir: Option<&Path>) {
        let missing: Vec<(MediaId, PathBuf)> = {
            let store = lock(&self.store);
            let mut loading = lock(&self.loading);
            project
                .media
                .iter()
                .filter(|a| !store.contains(a.id) && loading.insert(a.id))
                .map(|a| (a.id, resolve_path(&a.path, dir)))
                .collect()
        };
        if missing.is_empty() {
            return;
        }
        let rate = self.rate();
        let sender = self.loader_sender(app);
        for (id, path) in missing {
            if sender.send(LoadJob { id, path, rate }).is_err() {
                lock(&self.loading).remove(&id);
            }
        }
    }

    fn loader_sender(&self, app: &AppHandle) -> Sender<LoadJob> {
        let mut slot = lock(&self.loader);
        if let Some(tx) = slot.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel::<LoadJob>();
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("elyvon-media-loader".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let audio = &app.state::<AppState>().audio;
                    let (cached, budget, current_rate) = {
                        let store = lock(&audio.store);
                        (
                            store.cached(&job.path),
                            store.remaining_bytes(),
                            store.rate(),
                        )
                    };
                    if current_rate != job.rate {
                        lock(&audio.loading).remove(&job.id);
                        continue;
                    }
                    let started = Instant::now();
                    let result = match cached {
                        Some(buf) => Ok(buf),
                        None => MediaStore::load_file(&job.path, job.rate, budget),
                    };
                    match &result {
                        Ok(buf) => log::info!(
                            "loaded {} ({} Hz, {} ch, {:.1} s) in {:?}",
                            job.path.display(),
                            buf.info.sample_rate,
                            buf.info.channels,
                            buf.info.duration_seconds(),
                            started.elapsed()
                        ),
                        Err(e) => log::warn!("could not load {}: {e}", job.path.display()),
                    }
                    {
                        let mut store = lock(&audio.store);
                        if store.rate() == job.rate {
                            store.insert(job.id, &job.path, result);
                        }
                    }
                    lock(&audio.loading).remove(&job.id);
                    audio.sync(&app);
                    let _ = app.emit(MEDIA_EVENT, audio.media_statuses(&app));
                }
            });
        if let Err(e) = spawned {
            log::error!("could not start the media loader: {e}");
        }
        *slot = Some(tx.clone());
        tx
    }

    /// Inserts already decoded media (from an import) so it plays immediately.
    pub fn insert_loaded(&self, id: MediaId, path: &Path, buffer: Arc<MediaBuffer>) {
        let mut store = lock(&self.store);
        if buffer.rate == store.rate() {
            store.insert(id, path, Ok(buffer));
        }
    }

    pub fn buffer(&self, id: MediaId) -> Option<Arc<MediaBuffer>> {
        lock(&self.store).ready(id)
    }

    /// Forgets a failed entry so the next sync retries it (relink, retry).
    pub fn forget(&self, id: MediaId) {
        lock(&self.store).retain_ids(|m| *m != id);
    }

    pub fn remaining_budget(&self) -> usize {
        lock(&self.store).remaining_bytes()
    }

    pub fn media_statuses(&self, app: &AppHandle) -> Vec<MediaStatus> {
        let project = app.state::<AppState>().session().project().media.clone();
        lock(&self.store).statuses(&project)
    }

    /// Sends a transport command and returns the state after the request.
    pub fn command(&self, cmd: TransportCommand) -> Option<TransportSnapshot> {
        let warm = {
            let inner = lock(&self.inner);
            let tick = match cmd {
                TransportCommand::Play { from_tick, .. } => from_tick.or_else(|| {
                    inner
                        .handle
                        .as_ref()
                        .map(|h| h.shared().snapshot().ticks.round() as u64)
                }),
                TransportCommand::Locate { tick } => Some(tick),
                _ => None,
            };
            tick.and_then(|tick| inner.plan.as_ref().map(|plan| (tick, plan.clone())))
        };
        // Never hold the engine/control mutex across disk IO: the MIDI worker
        // needs to drain timestamped events while pages are being prepared.
        if let Some((tick, plan)) = warm
            && let Err(e) = plan.prefetch(plan.map.sample_at_tick(tick, plan.sample_rate), 8192)
        {
            log::warn!("could not prepare transport: {e}");
            return None;
        }
        let mut inner = lock(&self.inner);
        let handle = inner.handle.as_mut()?;
        if !handle.send(cmd) {
            log::warn!("transport command queue full; dropped {cmd:?}");
            return None;
        }
        Some(handle.shared().snapshot())
    }

    /// Plays a live MIDI channel message on a track's built-in instrument
    /// (keyboard, input monitor, piano-roll preview). `false` when the track
    /// has no running engine or the queue is full.
    pub fn send_live(&self, track: elyvon_project::TrackId, bytes: &[u8]) -> bool {
        let Some(message) = elyvon_audio::engine::LiveMidi::new(track.0.as_u128(), bytes) else {
            return false;
        };
        lock(&self.inner)
            .handle
            .as_mut()
            .is_some_and(|h| h.send_live(message))
    }

    /// Queues a live automation control value for the callback.
    pub fn send_override(&self, o: elyvon_audio::automation::ParamOverride) -> bool {
        lock(&self.inner)
            .handle
            .as_mut()
            .is_some_and(|h| h.send_override(o))
    }

    /// The prepared buffers of every pool entry (offline renders).
    pub fn buffers(
        &self,
        project: &Project,
    ) -> std::collections::HashMap<MediaId, Arc<MediaBuffer>> {
        let store = lock(&self.store);
        project
            .media
            .iter()
            .filter_map(|a| store.ready(a.id).map(|b| (a.id, b)))
            .collect()
    }

    pub fn reset_clips(&self) {
        if let Some(h) = lock(&self.inner).handle.as_ref() {
            h.shared().reset_clips();
        }
    }

    /// Feeds the spectrum analyser from meter slot `slot` (`None`: off).
    pub fn select_analyzer(&self, slot: Option<usize>) {
        let mut inner = lock(&self.inner);
        if let Some(h) = inner.handle.as_ref() {
            h.shared().set_analyzer_slot(slot);
        }
        if let Some(a) = inner.analyzer.as_mut() {
            a.reset();
        }
    }

    /// The latest spectrum of the selected strip.
    pub fn spectrum(&self) -> Option<elyvon_audio::spectrum::Spectrum> {
        let mut inner = lock(&self.inner);
        let rate = inner.handle.as_ref()?.sample_rate();
        inner.analyzer.as_mut().map(|a| a.read(rate))
    }

    /// Insert latency compensated on the current graph's longest path.
    pub fn latency_frames(&self) -> usize {
        lock(&self.inner)
            .plan
            .as_ref()
            .map_or(0, |p| p.stats.latency_frames)
    }

    pub fn snapshot(&self) -> Option<TransportSnapshot> {
        lock(&self.inner)
            .handle
            .as_ref()
            .map(|h| h.shared().snapshot())
    }

    /// Registers a tap (monotonic native clock). Returns the tempo estimate.
    pub fn tap(&self) -> Option<f64> {
        let mut inner = lock(&self.inner);
        let now = inner.epoch.elapsed().as_secs_f64();
        inner.tap.tap(now)
    }

    /// Called ~30 times per second by the publisher thread.
    fn tick(
        &self,
        app: &AppHandle,
        last: &mut Option<TransportSnapshot>,
        last_meters_active: &mut bool,
    ) {
        // Never hold the engine lock while taking the session lock.
        let (tracks, auxes) = {
            let state = app.state::<AppState>();
            let session = state.session();
            let p = session.project();
            (p.tracks.len(), p.mixer.auxes.len())
        };
        let (snapshot, meters, failed, stats) = {
            let mut guard = lock(&self.inner);
            let inner = &mut *guard;
            let failed = inner.output.as_ref().is_some_and(RunningOutput::has_failed);
            let Some(handle) = inner.handle.as_mut() else {
                return;
            };
            handle.poll();
            let shared = handle.shared();
            let snapshot = shared.snapshot();
            let meters = shared.read_meters(&mut inner.meter_reader, tracks, auxes);
            (snapshot, meters, failed, shared.stats())
        };
        if failed {
            log::error!("the audio device stopped; switching to the internal clock");
            let settings = AudioSettings {
                use_device: false,
                ..app.state::<AppState>().settings().audio.clone()
            };
            let rate = lock(&self.inner).requested_rate;
            self.start(app, &settings, rate);
            return;
        }
        let playing = snapshot.state != elyvon_audio::transport::PlayState::Stopped
            && snapshot.state != elyvon_audio::transport::PlayState::Paused;
        if last.as_ref() != Some(&snapshot) {
            let _ = app.emit(POSITION_EVENT, snapshot);
            *last = Some(snapshot);
        }
        // Keep publishing while anything (live instruments, tails, decaying
        // RMS) still shows, then go quiet: no idle event traffic.
        let visible = |m: &elyvon_audio::transport::ChannelMeter| {
            m.peak.iter().chain(&m.rms).any(|v| *v > 1e-5)
        };
        let active = playing
            || visible(&meters.master)
            || meters.tracks.iter().chain(&meters.auxes).any(visible);
        if active || *last_meters_active {
            let _ = app.emit(METERS_EVENT, (meters, stats));
        }
        *last_meters_active = active;
    }
}

/// Starts the ~30 Hz publisher of engine position and meters.
pub fn spawn_publisher(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("elyvon-transport-events".into())
        .spawn(move || {
            let mut last = None;
            let mut meters_active = false;
            loop {
                std::thread::sleep(Duration::from_millis(33));
                let state = app.state::<AppState>();
                state.audio.tick(&app, &mut last, &mut meters_active);
                if let Some(snap) = state.audio.snapshot() {
                    state.automation.on_snapshot(&app, &snap);
                }
            }
        });
    if let Err(e) = spawned {
        log::error!("could not start the transport publisher: {e}");
    }
}
