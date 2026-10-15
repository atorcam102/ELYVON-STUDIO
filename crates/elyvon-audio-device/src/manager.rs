//! The audio device manager: one control thread that owns the streams.
//!
//! * Opens the preferred output (falling back to the system default with an
//!   actionable notice when it is missing) and an optional metering input.
//! * Watches for stream faults set by error callbacks (hot-unplug, driver
//!   reset), tears the stream down, reports `deviceLost` and reconnects when
//!   a device is available again. Transport is stopped and the test tone is
//!   off after every reconnection, so no sound starts unexpectedly.
//! * Logs xrun/dropout statistics **here**, never in the audio callback.
//!
//! Nothing in this module touches project data: losing a device can never
//! modify, block or corrupt the open session.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use elyvon_audio::command::{
    COMMAND_QUEUE_CAPACITY, Command, CommandSender, TestTone, command_queue,
};
use elyvon_audio::render::{CoreConfig, EngineCore};
use elyvon_audio::status::{AudioErrorCode, EngineNotice, EngineState, EngineStatus, NoticeCode};
use elyvon_audio::telemetry::{StreamStats, Telemetry, TelemetrySnapshot};

use crate::backend::{Backend, InputRequest, OpenStream, OutputRequest};
use crate::types::{AudioPreferences, DeviceList};

/// Timing of the control loop. Tests shorten these.
#[derive(Debug, Clone, Copy)]
pub struct ManagerOptions {
    /// How often faults are checked.
    pub poll_interval: Duration,
    /// Delay between reconnection attempts.
    pub retry_interval: Duration,
    /// How often stream statistics are logged when they changed.
    pub stats_interval: Duration,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(200),
            retry_interval: Duration::from_secs(2),
            stats_interval: Duration::from_secs(10),
        }
    }
}

/// Called (on the manager thread) whenever the status changes.
pub type StatusListener = Box<dyn Fn(&EngineStatus) + Send + Sync>;

/// Errors for transport/tone commands and manager requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// No stream is running; the message explains why.
    NotRunning(String),
    /// The audio thread is not draining commands (should never happen).
    QueueFull,
    /// The manager did not answer in time or has shut down.
    Unresponsive,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning(m) => f.write_str(m),
            Self::QueueFull => f.write_str("The audio engine is not responding to commands."),
            Self::Unresponsive => f.write_str("The audio device manager did not respond in time."),
        }
    }
}

impl std::error::Error for ControlError {}

struct Shared {
    status: Mutex<EngineStatus>,
    telemetry: Arc<Telemetry>,
    sender: Mutex<Option<CommandSender>>,
    tone: Mutex<TestTone>,
    listener: Option<StatusListener>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Control-thread data stays consistent: every writer replaces whole values.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Request {
    Apply(AudioPreferences, mpsc::Sender<EngineStatus>),
    List(mpsc::Sender<DeviceList>),
    Shutdown,
}

/// How long dropping the manager waits for streams to close. A driver that
/// hangs inside the OS must never prevent the application from quitting.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Handle owned by the application. Dropping it closes all streams.
pub struct AudioManager {
    shared: Arc<Shared>,
    requests: mpsc::Sender<Request>,
    thread: Option<JoinHandle<()>>,
    finished: Mutex<Option<mpsc::Receiver<()>>>,
}

// The desktop shell shares the manager between command threads.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AudioManager>();
};

impl std::fmt::Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager")
            .field("status", &*lock(&self.shared.status))
            .finish_non_exhaustive()
    }
}

impl AudioManager {
    /// Starts the manager thread, which applies `prefs` right away. Returns
    /// immediately: opening a device never blocks application start-up.
    pub fn start(
        backend: Box<dyn Backend>,
        prefs: AudioPreferences,
        options: ManagerOptions,
        listener: Option<StatusListener>,
    ) -> Self {
        let initial = if prefs.enabled {
            EngineStatus::new(
                EngineState::Starting,
                Some("Opening the audio device…".into()),
            )
        } else {
            disabled_status()
        };
        let shared = Arc::new(Shared {
            status: Mutex::new(initial),
            telemetry: Arc::new(Telemetry::new()),
            sender: Mutex::new(None),
            tone: Mutex::new(TestTone::default()),
            listener,
        });
        let (requests, receiver) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("elyvon-audio-manager".into())
            .spawn(move || {
                let mut worker = Worker::new(backend, worker_shared, options, prefs);
                worker.apply_current();
                worker.run(&receiver);
                drop(worker); // closes every stream
                let _ = finished_tx.send(());
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(e) => {
                let mut status = EngineStatus::new(
                    EngineState::Error,
                    Some(format!("Could not start the audio device manager: {e}")),
                );
                status.error_code = Some(AudioErrorCode::Backend);
                *lock(&shared.status) = status;
                None
            }
        };
        Self {
            shared,
            requests,
            thread,
            finished: Mutex::new(Some(finished)),
        }
    }

    pub fn status(&self) -> EngineStatus {
        lock(&self.shared.status).clone()
    }

    /// Meters and counters. Clears the held peaks (one reader: the UI).
    pub fn telemetry(&self) -> TelemetrySnapshot {
        self.shared.telemetry.snapshot()
    }

    pub fn telemetry_block(&self) -> Arc<Telemetry> {
        Arc::clone(&self.shared.telemetry)
    }

    /// Re-opens streams with new preferences and returns the resulting status.
    pub fn apply(
        &self,
        prefs: AudioPreferences,
        timeout: Duration,
    ) -> Result<EngineStatus, ControlError> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .send(Request::Apply(prefs, tx))
            .map_err(|_| ControlError::Unresponsive)?;
        rx.recv_timeout(timeout)
            .map_err(|_| ControlError::Unresponsive)
    }

    /// Scans devices on the manager thread (never concurrently with opening).
    pub fn devices(&self, timeout: Duration) -> Result<DeviceList, ControlError> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .send(Request::List(tx))
            .map_err(|_| ControlError::Unresponsive)?;
        rx.recv_timeout(timeout)
            .map_err(|_| ControlError::Unresponsive)
    }

    fn send(&self, command: Command) -> Result<(), ControlError> {
        let mut sender = lock(&self.shared.sender);
        match sender.as_mut() {
            Some(s) => s.send(command).map_err(|_| ControlError::QueueFull),
            None => Err(ControlError::NotRunning(
                self.status()
                    .message
                    .unwrap_or_else(|| "The audio engine is not running.".into()),
            )),
        }
    }

    pub fn play(&self) -> Result<(), ControlError> {
        self.send(Command::Play)
    }

    pub fn stop(&self) -> Result<(), ControlError> {
        self.send(Command::Stop)
    }

    pub fn locate(&self, frame: u64) -> Result<(), ControlError> {
        self.send(Command::Locate { frame })
    }

    pub fn set_test_tone(&self, tone: TestTone) -> Result<TestTone, ControlError> {
        let rate = self.shared.telemetry.sample_rate().max(1);
        let tone = tone.sanitized(rate);
        self.send(Command::SetTestTone(tone))?;
        *lock(&self.shared.tone) = tone;
        Ok(tone)
    }

    /// The last test-tone settings sent (off after every reconnection).
    pub fn test_tone(&self) -> TestTone {
        *lock(&self.shared.tone)
    }
}

impl Drop for AudioManager {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Shutdown);
        let done = lock(&self.finished)
            .take()
            .is_some_and(|f| f.recv_timeout(SHUTDOWN_TIMEOUT).is_ok());
        match self.thread.take() {
            Some(thread) if done => {
                let _ = thread.join();
            }
            Some(_) => log::warn!("audio device manager did not stop in time; detaching it"),
            None => {}
        }
    }
}

fn disabled_status() -> EngineStatus {
    EngineStatus::new(
        EngineState::Disabled,
        Some("The audio engine is turned off. Turn it on in Settings › Audio.".into()),
    )
}

/// Converts a transport position between sample rates (exact for whole seconds).
pub fn convert_frames(frames: u64, from_rate: u32, to_rate: u32) -> u64 {
    if from_rate == 0 || from_rate == to_rate {
        return frames;
    }
    (u128::from(frames) * u128::from(to_rate) / u128::from(from_rate)) as u64
}

struct Worker {
    backend: Box<dyn Backend>,
    shared: Arc<Shared>,
    options: ManagerOptions,
    prefs: AudioPreferences,
    output: Option<OpenStream>,
    input: Option<OpenStream>,
    next_retry: Option<Instant>,
    preferred_missing: bool,
    next_stats: Instant,
    logged: StreamStats,
}

impl Worker {
    fn new(
        backend: Box<dyn Backend>,
        shared: Arc<Shared>,
        options: ManagerOptions,
        prefs: AudioPreferences,
    ) -> Self {
        let logged = shared.telemetry.stats();
        Self {
            backend,
            shared,
            options,
            prefs,
            output: None,
            input: None,
            next_retry: None,
            preferred_missing: false,
            next_stats: Instant::now() + options.stats_interval,
            logged,
        }
    }

    fn run(&mut self, receiver: &mpsc::Receiver<Request>) {
        loop {
            match receiver.recv_timeout(self.options.poll_interval) {
                Ok(Request::Apply(prefs, reply)) => {
                    self.prefs = prefs;
                    self.apply_current();
                    let _ = reply.send(lock(&self.shared.status).clone());
                }
                Ok(Request::List(reply)) => {
                    let _ = reply.send(self.backend.list());
                }
                Ok(Request::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
        self.close();
        log::info!("audio device manager stopped");
    }

    /// Publishes a status unless it is identical (ignoring the revision), so
    /// repeated retries do not flood the UI with events.
    fn set_status(&self, mut status: EngineStatus) {
        let mut current = lock(&self.shared.status);
        status.revision = current.revision;
        if *current == status {
            return;
        }
        status.revision = current.revision.wrapping_add(1);
        *current = status.clone();
        drop(current);
        if let Some(listener) = &self.shared.listener {
            listener(&status);
        }
    }

    fn close(&mut self) {
        *lock(&self.shared.sender) = None;
        // Dropping a stream joins its callback thread: after this no callback
        // can touch the telemetry of the old stream.
        self.input = None;
        self.output = None;
        *lock(&self.shared.tone) = TestTone::default();
    }

    fn apply_current(&mut self) {
        self.close();
        self.preferred_missing = false;
        self.next_retry = None;
        if !self.prefs.enabled {
            self.set_status(disabled_status());
            return;
        }
        self.set_status(EngineStatus::new(
            EngineState::Starting,
            Some("Opening the audio device…".into()),
        ));
        self.open();
    }

    fn open(&mut self) {
        let telemetry = Arc::clone(&self.shared.telemetry);
        let profile = self.prefs.profile_for(self.prefs.output_device.as_deref());
        let request = OutputRequest {
            host: self.prefs.host.clone(),
            device: self.prefs.output_device.clone(),
            device_name: self.prefs.output_device_name.clone(),
            sample_rate: profile.and_then(|p| p.sample_rate),
            buffer_frames: profile.and_then(|p| p.buffer_frames),
        };
        let carried = (telemetry.transport_frames(), telemetry.sample_rate());
        let clock = telemetry.clock_frames();
        telemetry.reset_stream_stats();
        self.logged = telemetry.stats();
        let mut new_sender = None;
        let result = {
            let core_telemetry = Arc::clone(&telemetry);
            let mut factory = |rate: u32, channels: u16| {
                let (tx, rx) = command_queue(COMMAND_QUEUE_CAPACITY);
                new_sender = Some(tx);
                core_telemetry.set_stream_format(rate, channels, 0);
                EngineCore::new(
                    CoreConfig {
                        sample_rate: rate,
                        output_channels: channels,
                        start_clock: clock,
                        start_transport: convert_frames(carried.0, carried.1, rate),
                    },
                    rx,
                    Arc::clone(&core_telemetry),
                )
            };
            self.backend
                .open_output(&request, &mut factory, Arc::clone(&telemetry))
        };
        let output = match result {
            Ok(output) => output,
            Err(e) => {
                // Retries repeat the same failure: log it once per change.
                if lock(&self.shared.status).message.as_deref() != Some(e.message.as_str()) {
                    log::warn!("audio output could not be opened: {e}");
                }
                let state = if e.code == AudioErrorCode::DeviceNotAvailable {
                    EngineState::NoDevice
                } else {
                    EngineState::Error
                };
                // Busy or missing devices may come back; bad settings will not.
                let retry = matches!(
                    e.code,
                    AudioErrorCode::DeviceNotAvailable | AudioErrorCode::DeviceBusy
                );
                self.next_retry = retry.then(|| Instant::now() + self.options.retry_interval);
                let mut status = EngineStatus::new(state, Some(e.message));
                status.error_code = Some(e.code);
                self.set_status(status);
                return;
            }
        };
        *lock(&self.shared.sender) = new_sender;
        let mut notices = output.notices.clone();
        self.preferred_missing = notices
            .iter()
            .any(|n| n.code == NoticeCode::PreferredDeviceMissing);
        if self.preferred_missing {
            self.next_retry = Some(Instant::now() + self.options.retry_interval);
        }
        if let Some(device) = self.prefs.input_device.clone() {
            let request = InputRequest {
                host: self.prefs.host.clone(),
                device,
                sample_rate: Some(output.info.sample_rate),
            };
            match self.backend.open_input(&request, Arc::clone(&telemetry)) {
                Ok(input) => self.input = Some(input),
                Err(e) => {
                    log::warn!("audio input could not be opened: {e}");
                    notices.push(EngineNotice {
                        code: NoticeCode::InputUnavailable,
                        message: e.message,
                    });
                }
            }
        }
        log::info!(
            "audio output running: {} / {} at {} Hz, {} ch, {} ({:?} frames)",
            output.info.host,
            output.info.device_name,
            output.info.sample_rate,
            output.info.channels,
            output.info.sample_format,
            output.info.reported_buffer_frames
        );
        let mut status = EngineStatus::new(EngineState::Running, None);
        status.notices = notices;
        status.output = Some(output.info.clone());
        status.input = self.input.as_ref().map(|i| i.info.clone());
        self.output = Some(output);
        self.set_status(status);
    }

    fn tick(&mut self) {
        let telemetry = Arc::clone(&self.shared.telemetry);
        if let Some(output) = &self.output {
            if let Some(fault) = telemetry.take_fault() {
                let name = output.info.device_name.clone();
                log::warn!("audio output “{name}” failed ({fault:?}); closing the stream");
                self.close();
                self.next_retry = Some(Instant::now() + self.options.retry_interval);
                let mut status = EngineStatus::new(
                    EngineState::DeviceLost,
                    Some(format!(
                        "“{name}” was disconnected or stopped responding. Playback stopped; \
                         the project is not affected. Elyvon reconnects automatically when an \
                         output is available, or choose another device in Settings › Audio."
                    )),
                );
                status.error_code = Some(AudioErrorCode::DeviceNotAvailable);
                self.set_status(status);
                return;
            }
            if telemetry.take_input_lost() && self.input.is_some() {
                log::warn!("audio input disconnected; output keeps running");
                self.input = None;
                telemetry.set_input_channels(0);
                let mut status = lock(&self.shared.status).clone();
                status.input = None;
                status
                    .notices
                    .retain(|n| n.code != NoticeCode::InputUnavailable);
                status.notices.push(EngineNotice {
                    code: NoticeCode::InputUnavailable,
                    message: "The input device was disconnected. Choose another input in \
                              Settings › Audio."
                        .into(),
                });
                self.set_status(status);
            }
            self.log_stats();
            // Running on the default because the preferred device was missing:
            // switch back once it returns, but never in the middle of playback.
            if self.preferred_missing && self.next_retry.is_some_and(|t| Instant::now() >= t) {
                self.next_retry = Some(Instant::now() + self.options.retry_interval);
                let preferred = self.prefs.output_device.clone().unwrap_or_default();
                if !telemetry.is_playing()
                    && !telemetry.tone_active()
                    && self
                        .backend
                        .output_exists(self.prefs.host.as_deref(), &preferred)
                {
                    log::info!("preferred audio output is back; switching to it");
                    self.apply_current();
                }
            }
            return;
        }
        if self.prefs.enabled && self.next_retry.is_some_and(|t| Instant::now() >= t) {
            self.next_retry = None;
            self.open();
        }
    }

    /// Statistics are logged from this control thread, never the callback.
    fn log_stats(&mut self) {
        if Instant::now() < self.next_stats {
            return;
        }
        self.next_stats = Instant::now() + self.options.stats_interval;
        let stats = self.shared.telemetry.stats();
        let new = |now: u64, before: u64| now.saturating_sub(before);
        let xruns = new(stats.backend_xruns, self.logged.backend_xruns);
        let gaps = new(stats.timing_gaps, self.logged.timing_gaps);
        let late = new(stats.late_callbacks, self.logged.late_callbacks);
        let errors = new(stats.stream_errors, self.logged.stream_errors);
        let clipped = new(stats.clipped_samples, self.logged.clipped_samples);
        if xruns + gaps + late + errors + clipped > 0 {
            log::warn!(
                "audio stream: +{xruns} backend xruns, +{gaps} timing gaps, +{late} late \
                 callbacks, +{errors} errors, +{clipped} clipped samples \
                 (max load {:.0}%, {} callbacks)",
                stats.max_callback_load * 100.0,
                stats.callbacks
            );
        }
        if stats.realtime_denied && !self.logged.realtime_denied {
            log::warn!("audio thread runs without real-time priority");
            let mut status = lock(&self.shared.status).clone();
            if status
                .notices
                .iter()
                .all(|n| n.code != NoticeCode::RealtimeDenied)
            {
                status.notices.push(EngineNotice {
                    code: NoticeCode::RealtimeDenied,
                    message: "The system refused real-time priority for the audio thread. \
                              Use a larger buffer if you hear dropouts."
                        .into(),
                });
                self.set_status(status);
            }
        }
        self.logged = stats;
    }
}
