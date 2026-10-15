//! Driver-free headless clock.
//!
//! Runs the real [`EngineCore`](elyvon_audio::render::EngineCore) on its own
//! thread, paced by the system clock at the selected buffer size, and throws
//! the rendered audio away. It lets CI (and machines without an audio
//! device) exercise transport, clock and meters end to end. It is never
//! chosen automatically and is always labelled "no audio output".

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use elyvon_audio::format::{BufferLayout, ChannelLayout};
use elyvon_audio::status::{AudioErrorCode, StreamInfo};
use elyvon_audio::telemetry::{CallbackMonitor, Telemetry};

use crate::backend::{Backend, BackendError, CoreFactory, InputRequest, OpenStream, OutputRequest};
use crate::types::{ConfigRange, DefaultConfig, DeviceInfo, DeviceList, Direction, HostInfo};

pub const HEADLESS_HOST: &str = "headless";
pub const HEADLESS_DEVICE: &str = "headless:clock";
pub const HEADLESS_NAME: &str = "Headless clock (no audio output)";
const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
const BUFFERS: [u32; 6] = [64, 128, 256, 512, 1024, 2048];
const CHANNELS: u16 = 2;

#[derive(Debug, Default, Clone, Copy)]
pub struct HeadlessBackend;

pub fn host_info() -> HostInfo {
    HostInfo {
        id: HEADLESS_HOST.into(),
        name: "Headless".into(),
        is_default: false,
        headless: true,
    }
}

pub fn device_info() -> DeviceInfo {
    DeviceInfo {
        host: HEADLESS_HOST.into(),
        id: HEADLESS_DEVICE.into(),
        name: HEADLESS_NAME.into(),
        manufacturer: Some("Elyvon".into()),
        driver: None,
        direction: Direction::Output,
        is_default: true,
        default_config: Some(DefaultConfig {
            channels: CHANNELS,
            sample_rate: 48_000,
            sample_format: "f32".into(),
            bits_per_sample: 32,
        }),
        ranges: vec![ConfigRange {
            channels: CHANNELS,
            min_sample_rate: RATES[0],
            max_sample_rate: RATES[RATES.len() - 1],
            sample_format: "f32".into(),
            bits_per_sample: 32,
            supported_by_engine: true,
            min_buffer_frames: Some(BUFFERS[0]),
            max_buffer_frames: Some(BUFFERS[BUFFERS.len() - 1]),
        }],
        sample_rates: RATES.to_vec(),
        buffer_sizes: BUFFERS.to_vec(),
        max_channels: CHANNELS,
        error: None,
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Backend for HeadlessBackend {
    fn list(&self) -> DeviceList {
        DeviceList {
            hosts: vec![host_info()],
            outputs: vec![device_info()],
            inputs: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn output_exists(&self, _host: Option<&str>, device: &str) -> bool {
        device == HEADLESS_DEVICE
    }

    fn open_output(
        &self,
        request: &OutputRequest,
        make_core: CoreFactory<'_>,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        let mut notices = Vec::new();
        let sample_rate = match request.sample_rate {
            Some(r) if RATES.contains(&r) => r,
            Some(r) => {
                notices.push(crate::cpal_backend::adjusted(format!(
                    "The headless clock does not support {r} Hz; using 48000 Hz."
                )));
                48_000
            }
            None => 48_000,
        };
        let buffer = match request.buffer_frames {
            Some(b) if BUFFERS.contains(&b) => b,
            Some(b) => {
                notices.push(crate::cpal_backend::adjusted(format!(
                    "The headless clock does not support {b}-frame buffers; using 256."
                )));
                256
            }
            None => 256,
        };
        let mut core = make_core(sample_rate, CHANNELS);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("elyvon-headless-audio".into())
                .spawn(move || {
                    let period =
                        Duration::from_secs_f64(f64::from(buffer) / f64::from(sample_rate));
                    // Allocated before the loop: the loop itself never allocates.
                    let mut out = vec![0.0f32; buffer as usize * usize::from(CHANNELS)];
                    let mut monitor = CallbackMonitor::new(sample_rate);
                    let origin = Instant::now();
                    let mut next = origin;
                    while !stop.load(Ordering::Acquire) {
                        let started = Instant::now();
                        core.process_interleaved(&mut out, usize::from(CHANNELS));
                        let elapsed = started.elapsed().as_nanos() as u64;
                        let at = started.duration_since(origin).as_nanos() as u64;
                        monitor.observe(&telemetry, u64::from(buffer), Some(at), elapsed, None);
                        next += period;
                        let now = Instant::now();
                        if next > now {
                            std::thread::sleep(next - now);
                        } else {
                            next = now; // fell behind: resynchronise, gap is counted
                        }
                    }
                })
                .map_err(|e| {
                    BackendError::new(
                        AudioErrorCode::Backend,
                        format!("Could not start the headless clock thread: {e}"),
                    )
                })?
        };
        Ok(OpenStream {
            info: StreamInfo {
                host: "Headless".into(),
                device_id: HEADLESS_DEVICE.into(),
                device_name: HEADLESS_NAME.into(),
                sample_rate,
                requested_buffer_frames: Some(buffer),
                reported_buffer_frames: Some(buffer),
                channels: CHANNELS,
                layout: ChannelLayout::Stereo,
                buffer_layout: BufferLayout::Interleaved,
                sample_format: "f32".into(),
                bits_per_sample: 32,
                nominal_latency_ms: Some(buffer as f32 * 1000.0 / sample_rate as f32),
                headless: true,
            },
            notices,
            handle: Box::new(Running {
                stop,
                thread: Some(thread),
            }),
        })
    }

    fn open_input(
        &self,
        _request: &InputRequest,
        _telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        Err(BackendError::new(
            AudioErrorCode::DeviceNotAvailable,
            "The headless clock has no inputs.",
        ))
    }
}
