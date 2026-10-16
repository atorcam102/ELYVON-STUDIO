//! A real CPAL input stream. Sample-rate negotiation is explicit: a device
//! must support the engine rate; mismatches are errors, never speed changes.
#![cfg(feature = "device")]
use crate::device::{DeviceInfo, RunningOutput};
use crate::engine::{AudioConfig, OutputKind};
use crate::recording::CaptureTap;
use crate::transport::EngineShared;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, SampleFormat, StreamConfig, SupportedBufferSize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

fn id(d: &Device) -> Option<String> {
    d.id().ok().map(|v| v.to_string())
}
fn find(wanted: Option<&str>) -> Result<Device, String> {
    let host = cpal::default_host();
    match wanted {
        None => host.default_input_device().ok_or_else(|| {
            "no default input device; connect an input and select it in Audio settings".into()
        }),
        Some(w) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| id(d).as_deref() == Some(w))
            .ok_or_else(|| format!("selected input {w} is unavailable")),
    }
}
pub fn input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| id(&d));
    host.input_devices()
        .map(|ds| {
            ds.filter_map(|d| {
                let id = id(&d)?;
                Some(DeviceInfo {
                    name: d
                        .description()
                        .map(|d| d.name().to_owned())
                        .unwrap_or_else(|_| id.clone()),
                    is_default: default.as_deref() == Some(&id),
                    id,
                })
            })
            .collect()
        })
        .unwrap_or_default()
}
#[derive(Debug, Clone)]
pub struct InputConfig {
    pub device: Option<String>,
    pub rate: u32,
    pub minimum_channels: u16,
    pub buffer: u32,
}
pub fn negotiate(request: &InputConfig) -> Result<(AudioConfig, SampleFormat), String> {
    let device = find(request.device.as_deref())?;
    let format=device.supported_input_configs().map_err(|e|e.to_string())?.filter(|c|c.min_sample_rate()<=request.rate&&c.max_sample_rate()>=request.rate&&c.channels()>=request.minimum_channels&&c.channels()<=32&&matches!(c.sample_format(),SampleFormat::F32|SampleFormat::I16|SampleFormat::I32|SampleFormat::U16)).min_by_key(|c|(c.channels(),c.sample_format()!=SampleFormat::F32)).ok_or_else(||"input does not support the engine rate/channels; choose a matching device or project sample rate (live input SRC is unavailable)".to_string())?;
    let buffer = match format.buffer_size() {
        SupportedBufferSize::Range { min, max } => request.buffer.clamp(*min, *max),
        SupportedBufferSize::Unknown => request.buffer,
    };
    Ok((
        AudioConfig {
            sample_rate: request.rate,
            buffer_frames: buffer,
            input_channels: format.channels(),
            output_channels: 0,
        },
        format.sample_format(),
    ))
}

pub fn start(
    request: InputConfig,
    config: AudioConfig,
    format: SampleFormat,
    mut tap: CaptureTap,
    shared: Arc<EngineShared>,
) -> Result<RunningOutput, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let (s, f) = (Arc::clone(&stop), Arc::clone(&failed));
    let health = Arc::clone(&tap.health);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("elyvon-input-device".into())
        .spawn(move || {
            let result = (|| -> Result<(), String> {
                let device = find(request.device.as_deref())?;
                let cfg = StreamConfig {
                    channels: config.input_channels,
                    sample_rate: config.sample_rate,
                    buffer_size: if config.buffer_frames == 0 {
                        BufferSize::Default
                    } else {
                        BufferSize::Fixed(config.buffer_frames)
                    },
                };
                let error_health = Arc::clone(&health);
                let error_flag = Arc::clone(&f);
                let on_error = move |e: cpal::Error| {
                    if e.kind() == cpal::ErrorKind::RealtimeDenied {
                        return;
                    }
                    error_health.disconnected.store(true, Ordering::Release);
                    error_flag.store(true, Ordering::Release);
                };
                let mut scratch = vec![0.0f32; 4096 * usize::from(config.input_channels)];
                let stream = match format {
                    SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                        cfg,
                        move |data, info| capture(&mut tap, data, info, &shared),
                        on_error,
                        None,
                    ),
                    SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                        cfg,
                        move |data, info| {
                            convert(&mut tap, &mut scratch, data, info, &shared, |s| {
                                f32::from(s) / 32768.0
                            })
                        },
                        on_error,
                        None,
                    ),
                    SampleFormat::I32 => device.build_input_stream::<i32, _, _>(
                        cfg,
                        move |data, info| {
                            convert(&mut tap, &mut scratch, data, info, &shared, |s| {
                                s as f32 / 2147483648.0
                            })
                        },
                        on_error,
                        None,
                    ),
                    SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
                        cfg,
                        move |data, info| {
                            convert(&mut tap, &mut scratch, data, info, &shared, |s| {
                                f32::from(s) / 32768.0 - 1.0
                            })
                        },
                        on_error,
                        None,
                    ),
                    _ => return Err("unsupported input sample format".into()),
                }
                .map_err(|e| e.to_string())?;
                stream.play().map_err(|e| e.to_string())?;
                let _ = tx.send(Ok(()));
                while !s.load(Ordering::Acquire) && !health.disconnected.load(Ordering::Acquire) {
                    std::thread::park_timeout(Duration::from_millis(20));
                }
                drop(stream);
                Ok(())
            })();
            if let Err(e) = result {
                f.store(true, Ordering::Release);
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(RunningOutput {
            config,
            name: "Audio input".into(),
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
            stop.store(true, Ordering::Release);
            Err("input device startup timed out".into())
        }
    }
}
fn capture(
    tap: &mut CaptureTap,
    data: &[f32],
    info: &cpal::InputCallbackInfo,
    shared: &EngineShared,
) {
    let t = info.timestamp();
    let age = t.callback.duration_since(t.capture);
    tap.health.input_latency_frames.store(
        (age.as_secs_f64() * f64::from(shared.sample_rate)).round() as u64,
        Ordering::Relaxed,
    );
    if let Some((position, state, cycle)) = shared.capture_position(age) {
        tap.process(data, position, state, cycle);
    } else {
        tap.health.overflow.store(true, Ordering::Release);
    }
}
fn convert<T: Copy>(
    tap: &mut CaptureTap,
    scratch: &mut [f32],
    data: &[T],
    info: &cpal::InputCallbackInfo,
    shared: &EngineShared,
    convert: impl Fn(T) -> f32,
) {
    let t = info.timestamp();
    let age = t.callback.duration_since(t.capture);
    tap.health.input_latency_frames.store(
        (age.as_secs_f64() * f64::from(shared.sample_rate)).round() as u64,
        Ordering::Relaxed,
    );
    let Some((position, state, cycle)) = shared.capture_position(age) else {
        tap.health.overflow.store(true, Ordering::Release);
        return;
    };
    let channels = usize::from(tap.input_channels());
    let chunk = (scratch.len() / channels) * channels;
    for (block, source) in data.chunks(chunk).enumerate() {
        for (dst, &src) in scratch.iter_mut().zip(source) {
            *dst = convert(src);
        }
        tap.process(
            &scratch[..source.len()],
            position + (block * chunk / channels) as u64,
            state,
            cycle,
        );
    }
}
