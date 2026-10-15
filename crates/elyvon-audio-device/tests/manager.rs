//! Device manager behaviour without hardware: a scripted fake backend
//! simulates hot-unplug, missing/busy devices and reconnection. The last
//! test drives the real headless clock through the production backend.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use elyvon_audio::command::{TestTone, ToneChannels};
use elyvon_audio::format::{BufferLayout, ChannelLayout};
use elyvon_audio::render::EngineCore;
use elyvon_audio::status::{AudioErrorCode, EngineNotice, EngineState, NoticeCode, StreamInfo};
use elyvon_audio::telemetry::{StreamFault, Telemetry};
use elyvon_audio_device::SystemBackend;
use elyvon_audio_device::backend::{
    Backend, BackendError, CoreFactory, InputRequest, OpenStream, OutputRequest,
};
use elyvon_audio_device::headless::HEADLESS_DEVICE;
use elyvon_audio_device::manager::{AudioManager, ControlError, ManagerOptions, convert_frames};
use elyvon_audio_device::types::{AudioPreferences, AudioSetup, DeviceList};

#[derive(Default)]
struct FakeState {
    /// (id, name) of connected outputs; the first one is the default.
    outputs: Vec<(String, String)>,
    inputs: Vec<String>,
    busy: Vec<String>,
    opened: Vec<OutputRequest>,
    /// Core of the most recent output stream, to simulate callbacks.
    core: Option<EngineCore>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);

impl Fake {
    fn with_outputs(outputs: &[(&str, &str)]) -> Self {
        let fake = Self::default();
        fake.state().outputs = outputs
            .iter()
            .map(|(i, n)| ((*i).to_owned(), (*n).to_owned()))
            .collect();
        fake
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.0.lock().expect("fake state")
    }

    /// Runs `blocks` callbacks of 480 frames on the current core.
    fn callbacks(&self, blocks: usize) {
        let mut state = self.state();
        let core = state.core.as_mut().expect("an open stream");
        let mut buf = vec![0.0f32; 480 * 2];
        for _ in 0..blocks {
            core.process_interleaved(&mut buf, 2);
        }
    }
}

fn info(id: &str, name: &str, rate: u32) -> StreamInfo {
    StreamInfo {
        host: "Fake".into(),
        device_id: id.into(),
        device_name: name.into(),
        sample_rate: rate,
        requested_buffer_frames: None,
        reported_buffer_frames: Some(480),
        channels: 2,
        layout: ChannelLayout::Stereo,
        buffer_layout: BufferLayout::Interleaved,
        sample_format: "f32".into(),
        bits_per_sample: 32,
        nominal_latency_ms: Some(10.0),
        headless: false,
    }
}

impl Backend for Fake {
    fn list(&self) -> DeviceList {
        DeviceList {
            hosts: Vec::new(),
            outputs: Vec::new(),
            inputs: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn output_exists(&self, _host: Option<&str>, device: &str) -> bool {
        self.state().outputs.iter().any(|(id, _)| id == device)
    }

    fn open_output(
        &self,
        request: &OutputRequest,
        make_core: CoreFactory<'_>,
        _telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        let mut state = self.state();
        state.opened.push(request.clone());
        let mut notices = Vec::new();
        let wanted = request
            .device
            .as_ref()
            .and_then(|d| state.outputs.iter().find(|(id, _)| id == d).cloned());
        let (id, name) = match (wanted, &request.device) {
            (Some(d), _) => d,
            (None, preferred) => {
                let default = state.outputs.first().cloned().ok_or_else(|| {
                    BackendError::new(AudioErrorCode::DeviceNotAvailable, "No output device")
                })?;
                if preferred.is_some() {
                    notices.push(EngineNotice {
                        code: NoticeCode::PreferredDeviceMissing,
                        message: "preferred missing".into(),
                    });
                }
                default
            }
        };
        if state.busy.contains(&id) {
            return Err(BackendError::new(AudioErrorCode::DeviceBusy, "busy"));
        }
        let rate = request.sample_rate.unwrap_or(48_000);
        state.core = Some(make_core(rate, 2));
        Ok(OpenStream {
            info: info(&id, &name, rate),
            notices,
            handle: Box::new(()),
        })
    }

    fn open_input(
        &self,
        request: &InputRequest,
        _telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        if self.state().inputs.contains(&request.device) {
            Ok(OpenStream {
                info: info(&request.device, "Mic", 48_000),
                notices: Vec::new(),
                handle: Box::new(()),
            })
        } else {
            Err(BackendError::new(
                AudioErrorCode::DeviceNotAvailable,
                "no mic",
            ))
        }
    }
}

fn fast() -> ManagerOptions {
    ManagerOptions {
        poll_interval: Duration::from_millis(5),
        retry_interval: Duration::from_millis(20),
        stats_interval: Duration::from_millis(20),
    }
}

fn wait_for(
    manager: &AudioManager,
    what: &str,
    f: impl Fn(&elyvon_audio::status::EngineStatus) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f(&manager.status()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "timed out waiting for {what}; status: {:?}",
        manager.status()
    );
}

fn prefs(device: Option<&str>) -> AudioPreferences {
    AudioPreferences {
        output_device: device.map(str::to_owned),
        output_device_name: device.map(|d| format!("{d} name")),
        ..AudioPreferences::default()
    }
}

#[test]
fn opens_the_default_output_and_notifies_listeners() {
    let fake = Fake::with_outputs(&[("a", "Speakers")]);
    let events = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&events);
    let manager = AudioManager::start(
        Box::new(fake.clone()),
        prefs(None),
        fast(),
        Some(Box::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })),
    );
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    let status = manager.status();
    assert_eq!(
        status.output.as_ref().map(|o| o.device_name.as_str()),
        Some("Speakers")
    );
    assert!(status.message.is_none() && status.notices.is_empty());
    // The initial "starting" status is set before the thread runs; "running" is an event.
    assert!(events.load(Ordering::SeqCst) >= 1);
    manager.play().unwrap();
    fake.callbacks(10);
    assert_eq!(manager.telemetry().transport_frames, 4800);
}

#[test]
fn disabled_engine_refuses_transport_with_an_explanation() {
    let fake = Fake::with_outputs(&[("a", "Speakers")]);
    let manager = AudioManager::start(
        Box::new(fake.clone()),
        AudioPreferences {
            enabled: false,
            ..AudioPreferences::default()
        },
        fast(),
        None,
    );
    assert_eq!(manager.status().state, EngineState::Disabled);
    match manager.play() {
        Err(ControlError::NotRunning(message)) => assert!(message.contains("Settings › Audio")),
        other => panic!("unexpected {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        fake.state().opened.is_empty(),
        "a disabled engine opens nothing"
    );
}

#[test]
fn hot_unplug_stops_safely_reports_and_reconnects() {
    let fake = Fake::with_outputs(&[("usb", "USB Interface"), ("int", "Built-in")]);
    let manager = AudioManager::start(Box::new(fake.clone()), prefs(Some("usb")), fast(), None);
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    manager.play().unwrap();
    manager
        .set_test_tone(TestTone {
            enabled: true,
            frequency_hz: 440.0,
            level_db: -18.0,
            channels: ToneChannels::All,
        })
        .unwrap();
    fake.callbacks(100); // one second at 48 kHz
    assert_eq!(manager.telemetry().transport_frames, 48_000);

    // The device disappears: its error callback raises a fault.
    fake.state().outputs.retain(|(id, _)| id != "usb");
    manager.telemetry_block().set_fault(StreamFault::DeviceLost);
    wait_for(&manager, "fallback", |s| {
        s.state == EngineState::Running
            && s.notices
                .iter()
                .any(|n| n.code == NoticeCode::PreferredDeviceMissing)
    });
    // The reconnection used the built-in device, kept the position, stopped
    // the transport and switched the tone off.
    assert_eq!(manager.status().output.unwrap().device_id, "int");
    let t = manager.telemetry();
    assert_eq!(t.transport_frames, 48_000);
    assert!(!t.playing && !t.tone_active);
    assert!(!manager.test_tone().enabled);

    // The preferred interface returns: the manager switches back while idle.
    fake.state()
        .outputs
        .insert(0, ("usb".into(), "USB Interface".into()));
    wait_for(&manager, "preferred device", |s| {
        s.output.as_ref().is_some_and(|o| o.device_id == "usb") && s.notices.is_empty()
    });
}

#[test]
fn unplugging_the_only_device_reports_device_lost_until_one_appears() {
    let fake = Fake::with_outputs(&[("a", "Speakers")]);
    let manager = AudioManager::start(Box::new(fake.clone()), prefs(None), fast(), None);
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    fake.state().outputs.clear();
    manager.telemetry_block().set_fault(StreamFault::DeviceLost);
    wait_for(&manager, "device lost or no device", |s| {
        matches!(s.state, EngineState::DeviceLost | EngineState::NoDevice)
    });
    let status = manager.status();
    assert!(
        status.message.unwrap().contains("Settings › Audio")
            || status.state == EngineState::NoDevice
    );
    assert!(matches!(manager.play(), Err(ControlError::NotRunning(_))));
    // Retries do not flood listeners: identical statuses keep their revision.
    let revision = manager.status().revision;
    std::thread::sleep(Duration::from_millis(100));
    assert!(manager.status().revision <= revision + 1);
    fake.state().outputs.push(("b".into(), "Headphones".into()));
    wait_for(&manager, "reconnected", |s| s.state == EngineState::Running);
    assert_eq!(manager.status().output.unwrap().device_name, "Headphones");
}

#[test]
fn missing_hardware_at_start_gives_no_device_then_recovers() {
    let fake = Fake::with_outputs(&[]);
    let manager = AudioManager::start(Box::new(fake.clone()), prefs(None), fast(), None);
    wait_for(&manager, "no device", |s| s.state == EngineState::NoDevice);
    assert_eq!(
        manager.status().error_code,
        Some(AudioErrorCode::DeviceNotAvailable)
    );
    fake.state().outputs.push(("a".into(), "Speakers".into()));
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
}

#[test]
fn busy_device_is_an_actionable_error_and_is_retried() {
    let fake = Fake::with_outputs(&[("a", "Interface")]);
    fake.state().busy.push("a".into());
    let manager = AudioManager::start(Box::new(fake.clone()), prefs(Some("a")), fast(), None);
    wait_for(&manager, "busy error", |s| {
        s.error_code == Some(AudioErrorCode::DeviceBusy)
    });
    assert_eq!(manager.status().state, EngineState::Error);
    fake.state().busy.clear();
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
}

#[test]
fn per_device_profile_is_applied_and_position_survives_rate_changes() {
    let fake = Fake::with_outputs(&[("a", "Interface")]);
    let manager = AudioManager::start(Box::new(fake.clone()), prefs(Some("a")), fast(), None);
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    manager.locate(48_000 * 3).unwrap();
    fake.callbacks(1);
    let mut p = prefs(Some("a"));
    p.apply_setup(AudioSetup {
        enabled: true,
        host: None,
        output_device: Some("a".into()),
        output_device_name: Some("Interface".into()),
        input_device: None,
        sample_rate: Some(96_000),
        buffer_frames: Some(256),
    });
    let status = manager.apply(p, Duration::from_secs(5)).unwrap();
    assert_eq!(status.output.unwrap().sample_rate, 96_000);
    let last = fake.state().opened.last().cloned().unwrap();
    assert_eq!(
        (last.sample_rate, last.buffer_frames),
        (Some(96_000), Some(256))
    );
    // Three seconds at 48 kHz are three seconds at 96 kHz.
    assert_eq!(manager.telemetry().transport_frames, 96_000 * 3);
    assert_eq!(convert_frames(44_100, 44_100, 48_000), 48_000);
    // The monotonic clock never goes backwards across streams.
    let clock = manager.telemetry().clock_frames;
    assert!(clock >= 480);
}

#[test]
fn input_problems_never_stop_the_output() {
    let fake = Fake::with_outputs(&[("a", "Speakers")]);
    fake.state().inputs.push("mic".into());
    let mut p = prefs(None);
    p.input_device = Some("absent".into());
    let manager = AudioManager::start(Box::new(fake.clone()), p.clone(), fast(), None);
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    let status = manager.status();
    assert!(
        status
            .notices
            .iter()
            .any(|n| n.code == NoticeCode::InputUnavailable)
    );
    assert!(status.input.is_none());

    p.input_device = Some("mic".into());
    let status = manager.apply(p, Duration::from_secs(5)).unwrap();
    assert!(status.input.is_some() && status.notices.is_empty());
    manager.telemetry_block().set_input_lost();
    wait_for(&manager, "input lost notice", |s| {
        s.state == EngineState::Running
            && s.input.is_none()
            && s.notices
                .iter()
                .any(|n| n.code == NoticeCode::InputUnavailable)
    });
}

#[test]
fn headless_clock_runs_the_real_engine_without_hardware() {
    let p = AudioPreferences {
        host: Some("headless".into()),
        output_device: Some(HEADLESS_DEVICE.into()),
        ..AudioPreferences::default()
    };
    let manager = AudioManager::start(Box::new(SystemBackend::default()), p, fast(), None);
    wait_for(&manager, "running", |s| s.state == EngineState::Running);
    let output = manager.status().output.unwrap();
    assert!(output.headless);
    assert_eq!(output.sample_rate, 48_000);
    manager.play().unwrap();
    manager
        .set_test_tone(TestTone {
            enabled: true,
            ..TestTone::default()
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let t = manager.telemetry();
    assert!(t.playing && t.tone_active);
    assert!(t.transport_frames > 0 && t.clock_frames >= t.transport_frames);
    assert!(t.stats.callbacks > 10);
    assert!(
        t.output_peak_db.iter().all(|db| (db + 18.0).abs() < 0.5),
        "{:?}",
        t.output_peak_db
    );
    manager.stop().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let stopped = manager.telemetry().transport_frames;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        manager.telemetry().transport_frames,
        stopped,
        "stop holds the position"
    );
    drop(manager); // joins the clock thread
}

/// A driver that hangs inside the OS for 30 s while opening.
struct HungDriver;

impl Backend for HungDriver {
    fn list(&self) -> DeviceList {
        Fake::default().list()
    }
    fn output_exists(&self, _: Option<&str>, _: &str) -> bool {
        false
    }
    fn open_output(
        &self,
        _: &OutputRequest,
        _: CoreFactory<'_>,
        _: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        std::thread::sleep(Duration::from_secs(30));
        Err(BackendError::new(AudioErrorCode::Backend, "hung"))
    }
    fn open_input(&self, _: &InputRequest, _: Arc<Telemetry>) -> Result<OpenStream, BackendError> {
        Err(BackendError::new(AudioErrorCode::Backend, "hung"))
    }
}

#[test]
fn a_hung_driver_never_blocks_quitting() {
    let manager = AudioManager::start(Box::new(HungDriver), prefs(None), fast(), None);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(manager.status().state, EngineState::Starting);
    let started = Instant::now();
    drop(manager);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "drop waited {:?}",
        started.elapsed()
    );
}
