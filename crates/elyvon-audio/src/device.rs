//! Audio output: a real device through cpal (feature `device`) or the
//! internal clock (always available).
//!
//! The engine runs inside the output callback. cpal streams may not be `Send`
//! on every platform, so each stream is created, owned and dropped by its own
//! thread; [`RunningOutput`] stops it when dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use ts_rs::TS;

use crate::engine::{AudioConfig, Engine, NoMidi, OutputKind};
use crate::transport::EngineShared;

/// An output device the user can choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceInfo {
    /// Stable identifier (`host:device`).
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// What a device accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiated {
    pub device_id: Option<String>,
    pub device_name: String,
    pub config: AudioConfig,
}

/// A running output; stops (and joins its thread) when dropped.
#[derive(Debug)]
pub struct RunningOutput {
    pub config: AudioConfig,
    pub name: String,
    pub kind: OutputKind,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RunningOutput {
    /// The stream reported a fatal error (device unplugged, …).
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

impl Drop for RunningOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

/// Internal clock: processes blocks on a timer and discards the output. Used
/// when no device is available so the transport, playhead and metronome
/// timing still come from the engine (silently).
pub fn start_internal_clock(mut engine: Engine, sample_rate: u32, block: u32) -> RunningOutput {
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let block = block.clamp(64, 4096);
    let thread = std::thread::Builder::new()
        .name("elyvon-internal-clock".into())
        .spawn(move || {
            let mut out = vec![0.0f32; block as usize * 2];
            let period = Duration::from_secs_f64(f64::from(block) / f64::from(sample_rate));
            let mut next = Instant::now();
            while !flag.load(Ordering::Relaxed) {
                engine.process(&mut out, 2, &mut NoMidi);
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else if now - next > period * 8 {
                    // Fell far behind (suspended machine): resynchronise.
                    next = now;
                }
            }
        })
        .ok();
    if thread.is_none() {
        failed.store(true, Ordering::Relaxed);
    }
    RunningOutput {
        config: AudioConfig {
            sample_rate,
            buffer_frames: block,
            input_channels: 0,
            output_channels: 2,
        },
        name: "Internal clock (no audio output)".into(),
        kind: OutputKind::InternalClock,
        stop,
        failed,
        thread,
    }
}

#[cfg(feature = "device")]
mod cpal_output {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{BufferSize, Device, ErrorKind, SampleFormat, StreamConfig, SupportedBufferSize};

    fn device_id(d: &Device) -> Option<String> {
        d.id().ok().map(|id| id.to_string())
    }

    fn device_name(d: &Device) -> String {
        d.description()
            .map(|desc| desc.name().to_owned())
            .or_else(|_| d.id().map(|id| id.to_string()))
            .unwrap_or_else(|_| "Unknown device".into())
    }

    pub fn output_devices() -> Vec<DeviceInfo> {
        let host = cpal::default_host();
        let default = host.default_output_device().and_then(|d| device_id(&d));
        let Ok(devices) = host.output_devices() else {
            return Vec::new();
        };
        devices
            .filter_map(|d| {
                let id = device_id(&d)?;
                Some(DeviceInfo {
                    is_default: default.as_deref() == Some(id.as_str()),
                    name: device_name(&d),
                    id,
                })
            })
            .collect()
    }

    fn find(id: Option<&str>) -> Result<Device, String> {
        let host = cpal::default_host();
        match id {
            None => host
                .default_output_device()
                .ok_or_else(|| "no default output device".to_owned()),
            Some(id) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| device_id(d).as_deref() == Some(id))
                .ok_or_else(|| format!("output device {id} is not available")),
        }
    }

    const FORMATS: [SampleFormat; 4] = [
        SampleFormat::F32,
        SampleFormat::I16,
        SampleFormat::I32,
        SampleFormat::U16,
    ];

    /// Picks the preferred rate when the device supports it, otherwise its
    /// default rate; stereo when possible; a supported sample format.
    pub fn negotiate(
        id: Option<&str>,
        preferred_rate: u32,
        buffer: Option<u32>,
    ) -> Result<(Negotiated, SampleFormat), String> {
        let device = find(id)?;
        let default = device.default_output_config().map_err(|e| e.to_string())?;
        let ranges: Vec<_> = device
            .supported_output_configs()
            .map_err(|e| e.to_string())?
            .filter(|r| FORMATS.contains(&r.sample_format()))
            .collect();
        let pick = ranges
            .iter()
            .filter(|r| {
                r.min_sample_rate() <= preferred_rate && preferred_rate <= r.max_sample_rate()
            })
            .min_by_key(|r| {
                (
                    r.channels() != 2,
                    FORMATS.iter().position(|f| *f == r.sample_format()),
                )
            })
            .map(|r| (*r).with_sample_rate(preferred_rate));
        let chosen = match pick {
            Some(c) => c,
            None if FORMATS.contains(&default.sample_format()) => default,
            None => return Err("the device offers no supported sample format".into()),
        };
        let buffer_frames = match (buffer, chosen.buffer_size()) {
            (Some(b), SupportedBufferSize::Range { min, max }) => b.clamp(*min, *max),
            (Some(b), SupportedBufferSize::Unknown) => b,
            (None, _) => 0,
        };
        Ok((
            Negotiated {
                device_id: device_id(&device),
                device_name: device_name(&device),
                config: AudioConfig {
                    sample_rate: chosen.sample_rate(),
                    buffer_frames,
                    input_channels: 0,
                    output_channels: chosen.channels(),
                },
            },
            chosen.sample_format(),
        ))
    }

    /// Starts the stream on its own thread. Returns once the stream plays or
    /// failed to start.
    pub fn start(
        negotiated: &Negotiated,
        format: SampleFormat,
        mut engine: Engine,
        shared: Arc<EngineShared>,
    ) -> Result<RunningOutput, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let id = negotiated.device_id.clone();
        let cfg = negotiated.config;
        let stop_flag = Arc::clone(&stop);
        let failed_flag = Arc::clone(&failed);
        let thread = std::thread::Builder::new()
            .name("elyvon-audio-device".into())
            .spawn(move || {
                let device = match find(id.as_deref()) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let channels = usize::from(cfg.output_channels.max(1));
                let config = StreamConfig {
                    channels: cfg.output_channels,
                    sample_rate: cfg.sample_rate,
                    buffer_size: if cfg.buffer_frames > 0 {
                        BufferSize::Fixed(cfg.buffer_frames)
                    } else {
                        BufferSize::Default
                    },
                };
                let shared_err = Arc::clone(&shared);
                let err_flag = Arc::clone(&failed_flag);
                let on_error = move |e: cpal::Error| match e.kind() {
                    ErrorKind::Xrun => shared_err.add_xrun(),
                    ErrorKind::RealtimeDenied => {}
                    _ => err_flag.store(true, Ordering::Relaxed),
                };
                // Scratch for non-f32 devices, sized generously up front.
                let mut scratch = vec![0.0f32; engine.max_block() * channels * 4];
                let stream = match format {
                    SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
                        config,
                        move |data: &mut [f32], _| engine.process(data, channels, &mut NoMidi),
                        on_error,
                        None,
                    ),
                    SampleFormat::I16 => device.build_output_stream::<i16, _, _>(
                        config,
                        move |data: &mut [i16], _| {
                            convert(&mut engine, &mut scratch, data, channels, |s| {
                                (s * 32767.0) as i16
                            })
                        },
                        on_error,
                        None,
                    ),
                    SampleFormat::I32 => device.build_output_stream::<i32, _, _>(
                        config,
                        move |data: &mut [i32], _| {
                            convert(&mut engine, &mut scratch, data, channels, |s| {
                                (f64::from(s) * 2_147_483_647.0) as i32
                            })
                        },
                        on_error,
                        None,
                    ),
                    _ => device.build_output_stream::<u16, _, _>(
                        config,
                        move |data: &mut [u16], _| {
                            convert(&mut engine, &mut scratch, data, channels, |s| {
                                ((s + 1.0) * 32767.5) as u16
                            })
                        },
                        on_error,
                        None,
                    ),
                };
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                while !stop_flag.load(Ordering::Relaxed) {
                    std::thread::park_timeout(Duration::from_millis(200));
                }
                drop(stream);
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(RunningOutput {
                config: cfg,
                name: negotiated.device_name.clone(),
                kind: OutputKind::Device,
                stop,
                failed,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err("the audio device did not start in time".into())
            }
        }
    }

    /// Renders f32 into `scratch` (in chunks) and converts for integer devices.
    fn convert<T: Copy>(
        engine: &mut Engine,
        scratch: &mut [f32],
        data: &mut [T],
        channels: usize,
        f: impl Fn(f32) -> T,
    ) {
        let chunk = (scratch.len() / channels) * channels;
        for out in data.chunks_mut(chunk.max(channels)) {
            let tmp = &mut scratch[..out.len()];
            engine.process(tmp, channels, &mut NoMidi);
            for (o, s) in out.iter_mut().zip(tmp.iter()) {
                *o = f(s.clamp(-1.0, 1.0));
            }
        }
    }
}

#[cfg(feature = "device")]
pub use cpal_output::{negotiate, output_devices, start};

/// Without the `device` feature there are no devices.
#[cfg(not(feature = "device"))]
pub fn output_devices() -> Vec<DeviceInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlaybackPlan;
    use crate::transport::{PlayState, TransportCommand};

    #[test]
    fn internal_clock_advances_the_engine_in_real_time() {
        let (mut handle, engine) = crate::engine::create(48_000, 256, PlaybackPlan::empty(48_000));
        let output = start_internal_clock(engine, 48_000, 256);
        handle.send(TransportCommand::Play {
            from_tick: Some(0),
            count_in: false,
            pre_roll: false,
        });
        std::thread::sleep(Duration::from_millis(300));
        let snap = handle.shared().snapshot();
        assert_eq!(snap.state, PlayState::Playing);
        // Loose bounds: CI machines can be slow, but time must pass at ~1×.
        assert!(snap.seconds > 0.1 && snap.seconds < 1.5, "{}", snap.seconds);
        assert!(!output.has_failed());
        drop(output);
        let stopped_at = handle.shared().snapshot().samples;
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            handle.shared().snapshot().samples,
            stopped_at,
            "thread stopped"
        );
    }
}
