//! CPAL backend: WASAPI (Windows), CoreAudio (macOS), ALSA (Linux; reaches
//! PipeWire and PulseAudio through their ALSA plug-ins).
//!
//! JACK, native PipeWire/PulseAudio and ASIO hosts exist in CPAL behind
//! optional features that need extra system libraries or SDK licences; they
//! are not compiled in (see `docs/AUDIO_ENGINE.md`).
//!
//! The output callback moves an [`EngineCore`] into the stream and does only
//! `process_interleaved` + wait-free telemetry. The error callback only
//! touches atomics; the manager thread reacts to faults.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, Host, HostId, OutputCallbackInfo, SampleFormat, SizedSample,
    StreamConfig, SupportedBufferSize, SupportedStreamConfigRange,
};
use elyvon_audio::format::{BufferLayout, ChannelLayout, InputSample, OutputSample};
use elyvon_audio::render::EngineCore;
use elyvon_audio::status::{AudioErrorCode, EngineNotice, NoticeCode, StreamInfo};
use elyvon_audio::telemetry::{CallbackMonitor, MAX_METER_CHANNELS, StreamFault, Telemetry};

use crate::backend::{Backend, BackendError, CoreFactory, InputRequest, OpenStream, OutputRequest};
use crate::types::{
    ConfigRange, DefaultConfig, DeviceInfo, DeviceList, Direction, HostInfo, STANDARD_BUFFER_SIZES,
    STANDARD_SAMPLE_RATES,
};

/// Ranges listed per device (all are still used for negotiation).
const MAX_LISTED_RANGES: usize = 48;

/// How long a backend may take to start a stream before we give up.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// Formats the engine can stream, in order of preference.
const FORMAT_PREFERENCE: [SampleFormat; 10] = [
    SampleFormat::F32,
    SampleFormat::I32,
    SampleFormat::I16,
    SampleFormat::F64,
    SampleFormat::U16,
    SampleFormat::I8,
    SampleFormat::U8,
    SampleFormat::U32,
    SampleFormat::I64,
    SampleFormat::U64,
];

pub(crate) fn adjusted(message: String) -> EngineNotice {
    EngineNotice {
        code: NoticeCode::ConfigAdjusted,
        message,
    }
}

fn engine_supports(format: SampleFormat) -> bool {
    FORMAT_PREFERENCE.contains(&format)
}

fn format_name(format: SampleFormat) -> String {
    format!("{format}")
}

fn map_error(err: &cpal::Error, device: &str) -> BackendError {
    let (code, message) = match err.kind() {
        ErrorKind::DeviceBusy => (
            AudioErrorCode::DeviceBusy,
            format!(
                "“{device}” is in use by another application or locked in exclusive mode. \
                 Close the other application, or choose another device in Settings › Audio."
            ),
        ),
        ErrorKind::DeviceNotAvailable => (
            AudioErrorCode::DeviceNotAvailable,
            format!(
                "“{device}” is not available. It may have been disconnected or disabled. \
                 Connect or enable an output — Elyvon retries automatically — or choose \
                 another device in Settings › Audio."
            ),
        ),
        ErrorKind::PermissionDenied => (
            AudioErrorCode::PermissionDenied,
            format!(
                "The system denied access to “{device}”. Allow Elyvon Studio to use audio \
                 devices (microphone access for inputs) in the system privacy settings, then retry."
            ),
        ),
        ErrorKind::UnsupportedConfig | ErrorKind::InvalidInput => (
            AudioErrorCode::UnsupportedConfig,
            format!(
                "“{device}” rejected the requested sample rate or buffer size. \
                 Choose another setting in Settings › Audio."
            ),
        ),
        ErrorKind::HostUnavailable => (
            AudioErrorCode::HostUnavailable,
            "The selected audio system is not available on this computer. \
             Choose another audio system in Settings › Audio."
                .to_owned(),
        ),
        _ => (
            AudioErrorCode::Backend,
            format!("The audio system reported an error for “{device}”."),
        ),
    };
    BackendError::new(code, format!("{message} ({err})"))
}

/// Error-callback handler. Only atomics: it may run on the audio thread.
fn on_stream_error(telemetry: &Telemetry, err: &cpal::Error) {
    match err.kind() {
        ErrorKind::Xrun => telemetry.count_backend_xrun(),
        ErrorKind::DeviceNotAvailable => telemetry.set_fault(StreamFault::DeviceLost),
        ErrorKind::StreamInvalidated => telemetry.set_fault(StreamFault::Invalidated),
        ErrorKind::RealtimeDenied => telemetry.flag_realtime_denied(),
        // DeviceChanged: the OS rerouted the stream itself and it keeps running.
        _ => telemetry.count_stream_error(),
    }
}

/// Input error handler: a lost input never stops the output stream.
fn on_input_error(telemetry: &Telemetry, err: &cpal::Error) {
    match err.kind() {
        ErrorKind::Xrun => telemetry.count_backend_xrun(),
        ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated => telemetry.set_input_lost(),
        _ => telemetry.count_stream_error(),
    }
}

/// System backend: every CPAL host on this platform.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalBackend;

impl CpalBackend {
    fn host(&self, id: Option<&str>) -> Result<Host, BackendError> {
        match id {
            None => Ok(cpal::default_host()),
            Some(id) => {
                let host_id = HostId::from_str(id).map_err(|_| {
                    BackendError::new(
                        AudioErrorCode::HostUnavailable,
                        format!("The audio system “{id}” does not exist in this build."),
                    )
                })?;
                cpal::host_from_id(host_id).map_err(|e| map_error(&e, id))
            }
        }
    }

    fn find_output(&self, host: &Host, id: &str) -> Option<Device> {
        let device_id = cpal::DeviceId::from_str(id).ok()?;
        host.device_by_id(&device_id)
            .filter(|d| d.supports_output())
    }
}

fn device_name(device: &Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| device.to_string())
}

fn range_info(r: &SupportedStreamConfigRange) -> ConfigRange {
    let (min_buffer_frames, max_buffer_frames) = match r.buffer_size() {
        SupportedBufferSize::Range { min, max } => (Some(*min), Some(*max)),
        SupportedBufferSize::Unknown => (None, None),
    };
    ConfigRange {
        channels: r.channels(),
        min_sample_rate: r.min_sample_rate(),
        max_sample_rate: r.max_sample_rate(),
        sample_format: format_name(r.sample_format()),
        bits_per_sample: r.sample_format().bits_per_sample(),
        supported_by_engine: engine_supports(r.sample_format()),
        min_buffer_frames,
        max_buffer_frames,
    }
}

fn describe(
    host_id: &str,
    device: &Device,
    direction: Direction,
    default_id: Option<&str>,
) -> Option<DeviceInfo> {
    let id = device.id().ok()?.to_string();
    let description = device.description().ok();
    let mut info = DeviceInfo {
        host: host_id.to_owned(),
        is_default: default_id == Some(id.as_str()),
        id,
        name: description
            .as_ref()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|| device.to_string()),
        manufacturer: description
            .as_ref()
            .and_then(|d| d.manufacturer().map(str::to_owned)),
        driver: description
            .as_ref()
            .and_then(|d| d.driver().map(str::to_owned)),
        direction,
        default_config: None,
        ranges: Vec::new(),
        sample_rates: Vec::new(),
        buffer_sizes: Vec::new(),
        max_channels: 0,
        error: None,
    };
    let (ranges, default) = match direction {
        Direction::Output => (
            device
                .supported_output_configs()
                .map(|r| r.collect::<Vec<_>>()),
            device.default_output_config(),
        ),
        Direction::Input => (
            device
                .supported_input_configs()
                .map(|r| r.collect::<Vec<_>>()),
            device.default_input_config(),
        ),
    };
    match ranges {
        Ok(ranges) => {
            // Virtual devices can report hundreds of ranges; the UI needs a summary.
            info.ranges = ranges
                .iter()
                .take(MAX_LISTED_RANGES)
                .map(range_info)
                .collect();
            let usable: Vec<_> = ranges
                .iter()
                .filter(|r| engine_supports(r.sample_format()))
                .collect();
            info.max_channels = usable.iter().map(|r| r.channels()).max().unwrap_or(0);
            info.sample_rates = STANDARD_SAMPLE_RATES
                .into_iter()
                .filter(|rate| usable.iter().any(|r| r.contains_rate(*rate)))
                .collect();
            let (min, max) =
                usable
                    .iter()
                    .fold((u32::MAX, 0), |(lo, hi), r| match r.buffer_size() {
                        SupportedBufferSize::Range { min, max } => (lo.min(*min), hi.max(*max)),
                        SupportedBufferSize::Unknown => (lo, hi),
                    });
            info.buffer_sizes = STANDARD_BUFFER_SIZES
                .into_iter()
                .filter(|b| *b >= min && *b <= max)
                .collect();
        }
        Err(e) => info.error = Some(e.to_string()),
    }
    if let Ok(d) = default {
        info.default_config = Some(DefaultConfig {
            channels: d.channels(),
            sample_rate: d.sample_rate(),
            sample_format: format_name(d.sample_format()),
            bits_per_sample: d.sample_format().bits_per_sample(),
        });
        if let Some(rate) = Some(d.sample_rate()).filter(|r| !info.sample_rates.contains(r)) {
            info.sample_rates.push(rate);
            info.sample_rates.sort_unstable();
        }
    }
    Some(info)
}

/// Chooses channels, rate, format and buffer for a device.
struct Negotiated {
    config: StreamConfig,
    format: SampleFormat,
    notices: Vec<EngineNotice>,
}

fn negotiate(
    name: &str,
    ranges: &[SupportedStreamConfigRange],
    default_channels: u16,
    default_rate: u32,
    want_rate: Option<u32>,
    want_buffer: Option<u32>,
) -> Result<Negotiated, BackendError> {
    let mut notices = Vec::new();
    let usable: Vec<&SupportedStreamConfigRange> = ranges
        .iter()
        .filter(|r| engine_supports(r.sample_format()))
        .collect();
    if usable.is_empty() {
        return Err(BackendError::new(
            AudioErrorCode::UnsupportedFormat,
            format!(
                "“{name}” only offers sample formats the engine cannot stream (for example \
                 24-bit packed or DSD). Choose another device."
            ),
        ));
    }
    let pick = |rate: u32| {
        // Prefer the device's default channel count, then the format order.
        FORMAT_PREFERENCE.iter().find_map(|format| {
            usable
                .iter()
                .filter(|r| r.sample_format() == *format && r.contains_rate(rate))
                .min_by_key(|r| (r.channels() != default_channels, r.channels()))
                .copied()
        })
    };
    let rate = want_rate.unwrap_or(default_rate);
    let (range, rate) = match pick(rate) {
        Some(r) => (r, rate),
        None => {
            notices.push(adjusted(format!(
                "“{name}” does not support {rate} Hz; using its default of {default_rate} Hz."
            )));
            let r = pick(default_rate)
                .or_else(|| usable.first().copied())
                .ok_or_else(|| {
                    BackendError::new(AudioErrorCode::UnsupportedConfig, "No usable configuration")
                })?;
            let rate = if r.contains_rate(default_rate) {
                default_rate
            } else {
                r.max_sample_rate()
            };
            (r, rate)
        }
    };
    let buffer_size = match (want_buffer, range.buffer_size()) {
        (None, _) => BufferSize::Default,
        (Some(b), SupportedBufferSize::Range { min, max }) if b < *min || b > *max => {
            let clamped = b.clamp(*min, *max);
            notices.push(adjusted(format!(
                "“{name}” supports buffers of {min}–{max} frames; using {clamped} instead of {b}."
            )));
            BufferSize::Fixed(clamped)
        }
        (Some(b), _) => BufferSize::Fixed(b),
    };
    Ok(Negotiated {
        config: StreamConfig {
            channels: range.channels(),
            sample_rate: rate,
            buffer_size,
        },
        format: range.sample_format(),
        notices,
    })
}

fn build_output<T: SizedSample + OutputSample>(
    device: &Device,
    config: StreamConfig,
    mut core: EngineCore,
    telemetry: Arc<Telemetry>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = usize::from(config.channels);
    let mut monitor = CallbackMonitor::new(config.sample_rate);
    let error_telemetry = Arc::clone(&telemetry);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &OutputCallbackInfo| {
            // REAL-TIME: no allocation, locks, I/O or logging below.
            let started = Instant::now();
            core.process_interleaved(data, channels);
            let elapsed = started.elapsed().as_nanos() as u64;
            let ts = info.timestamp();
            let latency = ts
                .playback
                .checked_duration_since(ts.callback)
                .map(|d| d.as_nanos() as u64);
            monitor.observe(
                &telemetry,
                (data.len() / channels.max(1)) as u64,
                Some(ts.callback.as_nanos() as u64),
                elapsed,
                latency,
            );
        },
        move |err| on_stream_error(&error_telemetry, &err),
        Some(OPEN_TIMEOUT),
    )
}

fn build_input<T: SizedSample + InputSample>(
    device: &Device,
    config: StreamConfig,
    telemetry: Arc<Telemetry>,
) -> Result<cpal::Stream, cpal::Error> {
    let channels = usize::from(config.channels).max(1);
    let error_telemetry = Arc::clone(&telemetry);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _| {
            // REAL-TIME: peaks on the stack, one atomic max per channel.
            let mut peaks = [0.0f32; MAX_METER_CHANNELS];
            for frame in data.chunks_exact(channels) {
                for (peak, sample) in peaks.iter_mut().zip(frame) {
                    *peak = peak.max(sample.to_unit().abs());
                }
            }
            for (channel, peak) in peaks.iter().take(channels).enumerate() {
                telemetry.hold_input_peak(channel, *peak);
            }
            telemetry.count_input_callback();
        },
        move |err| on_input_error(&error_telemetry, &err),
        Some(OPEN_TIMEOUT),
    )
}

macro_rules! dispatch_format {
    ($format:expr, $build:ident, $($arg:expr),*) => {
        match $format {
            SampleFormat::F32 => $build::<f32>($($arg),*),
            SampleFormat::F64 => $build::<f64>($($arg),*),
            SampleFormat::I8 => $build::<i8>($($arg),*),
            SampleFormat::I16 => $build::<i16>($($arg),*),
            SampleFormat::I32 => $build::<i32>($($arg),*),
            SampleFormat::I64 => $build::<i64>($($arg),*),
            SampleFormat::U8 => $build::<u8>($($arg),*),
            SampleFormat::U16 => $build::<u16>($($arg),*),
            SampleFormat::U32 => $build::<u32>($($arg),*),
            SampleFormat::U64 => $build::<u64>($($arg),*),
            _ => Err(cpal::Error::new(ErrorKind::UnsupportedConfig)),
        }
    };
}

fn stream_info(
    host: &Host,
    device_id: String,
    name: String,
    negotiated: &Negotiated,
    stream: &cpal::Stream,
) -> StreamInfo {
    let requested = match negotiated.config.buffer_size {
        BufferSize::Fixed(n) => Some(n),
        BufferSize::Default => None,
    };
    let reported = stream.buffer_size().ok().filter(|n| *n > 0);
    let rate = negotiated.config.sample_rate;
    StreamInfo {
        host: host.id().name().to_owned(),
        device_id,
        device_name: name,
        sample_rate: rate,
        requested_buffer_frames: requested,
        reported_buffer_frames: reported,
        channels: negotiated.config.channels,
        layout: ChannelLayout::from_channels(negotiated.config.channels),
        buffer_layout: BufferLayout::Interleaved,
        sample_format: format_name(negotiated.format),
        bits_per_sample: negotiated.format.bits_per_sample(),
        nominal_latency_ms: reported
            .or(requested)
            .map(|n| n as f32 * 1000.0 / rate as f32),
        headless: false,
    }
}

/// A CPAL stream wrapped so it can be stored as `Box<dyn Send>`.
struct CpalStream(#[allow(dead_code)] cpal::Stream);

impl Backend for CpalBackend {
    fn list(&self) -> DeviceList {
        let mut list = DeviceList {
            hosts: Vec::new(),
            outputs: Vec::new(),
            inputs: Vec::new(),
            errors: Vec::new(),
        };
        let default_host = cpal::default_host().id();
        for host_id in cpal::available_hosts() {
            let id = host_id.to_string();
            list.hosts.push(HostInfo {
                id: id.clone(),
                name: host_id.name().to_owned(),
                is_default: host_id == default_host,
                headless: false,
            });
            let host = match cpal::host_from_id(host_id) {
                Ok(h) => h,
                Err(e) => {
                    list.errors.push(format!("{}: {e}", host_id.name()));
                    continue;
                }
            };
            let default_out = host
                .default_output_device()
                .and_then(|d| d.id().ok())
                .map(|i| i.to_string());
            let default_in = host
                .default_input_device()
                .and_then(|d| d.id().ok())
                .map(|i| i.to_string());
            match host.devices() {
                Ok(devices) => {
                    for device in devices {
                        if device.supports_output()
                            && let Some(info) =
                                describe(&id, &device, Direction::Output, default_out.as_deref())
                        {
                            list.outputs.push(info);
                        }
                        if device.supports_input()
                            && let Some(info) =
                                describe(&id, &device, Direction::Input, default_in.as_deref())
                        {
                            list.inputs.push(info);
                        }
                    }
                }
                Err(e) => list.errors.push(format!("{}: {e}", host_id.name())),
            }
        }
        list
    }

    fn output_exists(&self, host: Option<&str>, device: &str) -> bool {
        self.host(host)
            .ok()
            .and_then(|h| self.find_output(&h, device))
            .is_some()
    }

    fn open_output(
        &self,
        request: &OutputRequest,
        make_core: CoreFactory<'_>,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        let host = self.host(request.host.as_deref())?;
        let mut notices = Vec::new();
        let device = match &request.device {
            Some(id) => match self.find_output(&host, id) {
                Some(d) => d,
                None => {
                    let fallback = host.default_output_device().ok_or_else(no_output)?;
                    let wanted = request.device_name.as_deref().unwrap_or(id);
                    notices.push(EngineNotice {
                        code: NoticeCode::PreferredDeviceMissing,
                        message: format!(
                            "“{wanted}” is not connected. Using “{}” instead. Reconnect it and \
                             choose “Use preferred device”, or pick another output in Settings › Audio.",
                            device_name(&fallback)
                        ),
                    });
                    fallback
                }
            },
            None => host.default_output_device().ok_or_else(no_output)?,
        };
        let name = device_name(&device);
        let device_id = device
            .id()
            .map(|i| i.to_string())
            .unwrap_or_else(|_| name.clone());
        let default = device
            .default_output_config()
            .map_err(|e| map_error(&e, &name))?;
        let ranges: Vec<_> = device
            .supported_output_configs()
            .map_err(|e| map_error(&e, &name))?
            .collect();
        let mut negotiated = negotiate(
            &name,
            &ranges,
            default.channels(),
            default.sample_rate(),
            request.sample_rate,
            request.buffer_frames,
        )?;
        notices.append(&mut negotiated.notices);
        let format = negotiated.format;
        let open = |config: StreamConfig, make_core: &mut dyn FnMut(u32, u16) -> EngineCore| {
            let core = make_core(config.sample_rate, config.channels);
            let telemetry = Arc::clone(&telemetry);
            dispatch_format!(format, build_output, &device, config, core, telemetry)
        };
        let stream = match open(negotiated.config, make_core) {
            Ok(s) => s,
            // A fixed buffer the backend could not report in advance: retry
            // with the device default instead of failing.
            Err(e)
                if matches!(negotiated.config.buffer_size, BufferSize::Fixed(_))
                    && matches!(
                        e.kind(),
                        ErrorKind::UnsupportedConfig | ErrorKind::InvalidInput
                    ) =>
            {
                notices.push(adjusted(format!(
                    "“{name}” rejected the requested buffer size; using its default buffer."
                )));
                negotiated.config.buffer_size = BufferSize::Default;
                open(negotiated.config, make_core).map_err(|e| map_error(&e, &name))?
            }
            Err(e) => return Err(map_error(&e, &name)),
        };
        stream.play().map_err(|e| map_error(&e, &name))?;
        let info = stream_info(&host, device_id, name, &negotiated, &stream);
        Ok(OpenStream {
            info,
            notices,
            handle: Box::new(CpalStream(stream)),
        })
    }

    fn open_input(
        &self,
        request: &InputRequest,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        let host = self.host(request.host.as_deref())?;
        let device = cpal::DeviceId::from_str(&request.device)
            .ok()
            .and_then(|id| host.device_by_id(&id))
            .filter(|d| d.supports_input())
            .ok_or_else(|| {
                BackendError::new(
                    AudioErrorCode::DeviceNotAvailable,
                    "The selected input is not connected. Choose another input in Settings › Audio.",
                )
            })?;
        let name = device_name(&device);
        let default = device
            .default_input_config()
            .map_err(|e| map_error(&e, &name))?;
        let ranges: Vec<_> = device
            .supported_input_configs()
            .map_err(|e| map_error(&e, &name))?
            .collect();
        let mut negotiated = negotiate(
            &name,
            &ranges,
            default.channels(),
            default.sample_rate(),
            request.sample_rate,
            None,
        )?;
        // A different input rate is fine for metering; say nothing about it.
        negotiated.notices.clear();
        let stream = dispatch_format!(
            negotiated.format,
            build_input,
            &device,
            negotiated.config,
            Arc::clone(&telemetry)
        )
        .map_err(|e| map_error(&e, &name))?;
        stream.play().map_err(|e| map_error(&e, &name))?;
        telemetry.set_input_channels(negotiated.config.channels);
        let id = device
            .id()
            .map(|i| i.to_string())
            .unwrap_or_else(|_| name.clone());
        let info = stream_info(&host, id, name, &negotiated, &stream);
        Ok(OpenStream {
            info,
            notices: Vec::new(),
            handle: Box::new(CpalStream(stream)),
        })
    }
}

fn no_output() -> BackendError {
    BackendError::new(
        AudioErrorCode::DeviceNotAvailable,
        "No audio output device was found. Connect or enable an output device; \
         Elyvon retries automatically.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            min,
            max,
            SupportedBufferSize::Range { min: 64, max: 2048 },
            format,
        )
    }

    #[test]
    fn negotiation_prefers_float_default_channels_and_requested_rate() {
        let ranges = [
            range(2, 44_100, 48_000, SampleFormat::I16),
            range(2, 44_100, 96_000, SampleFormat::F32),
            range(6, 44_100, 96_000, SampleFormat::F32),
        ];
        let n = negotiate("Dev", &ranges, 2, 48_000, Some(96_000), Some(128)).unwrap();
        assert_eq!(n.format, SampleFormat::F32);
        assert_eq!(n.config.channels, 2);
        assert_eq!(n.config.sample_rate, 96_000);
        assert_eq!(n.config.buffer_size, BufferSize::Fixed(128));
        assert!(n.notices.is_empty());
    }

    #[test]
    fn unsupported_rate_and_buffer_are_adjusted_with_notices() {
        let ranges = [range(2, 44_100, 48_000, SampleFormat::I16)];
        let n = negotiate("Dev", &ranges, 2, 48_000, Some(192_000), Some(16)).unwrap();
        assert_eq!(n.config.sample_rate, 48_000);
        assert_eq!(n.config.buffer_size, BufferSize::Fixed(64));
        assert_eq!(n.format, SampleFormat::I16);
        assert_eq!(n.notices.len(), 2);
        assert!(
            n.notices
                .iter()
                .all(|n| n.code == NoticeCode::ConfigAdjusted)
        );
    }

    #[test]
    fn devices_with_only_unstreamable_formats_are_rejected_clearly() {
        let ranges = [range(2, 44_100, 48_000, SampleFormat::I24)];
        let e = negotiate("Dev", &ranges, 2, 48_000, None, None)
            .err()
            .unwrap();
        assert_eq!(e.code, AudioErrorCode::UnsupportedFormat);
    }

    #[test]
    fn error_kinds_map_to_actionable_codes() {
        let busy = map_error(&cpal::Error::new(ErrorKind::DeviceBusy), "USB");
        assert_eq!(busy.code, AudioErrorCode::DeviceBusy);
        assert!(busy.message.contains("another application"));
        let denied = map_error(&cpal::Error::new(ErrorKind::PermissionDenied), "Mic");
        assert_eq!(denied.code, AudioErrorCode::PermissionDenied);
        let gone = map_error(&cpal::Error::new(ErrorKind::DeviceNotAvailable), "USB");
        assert_eq!(gone.code, AudioErrorCode::DeviceNotAvailable);
    }

    #[test]
    fn stream_errors_only_flag_telemetry() {
        let t = Telemetry::new();
        on_stream_error(&t, &cpal::Error::new(ErrorKind::Xrun));
        on_stream_error(&t, &cpal::Error::new(ErrorKind::DeviceChanged));
        on_stream_error(&t, &cpal::Error::new(ErrorKind::RealtimeDenied));
        assert_eq!(t.take_fault(), None);
        let s = t.snapshot().stats;
        assert_eq!((s.backend_xruns, s.stream_errors), (1, 1));
        assert!(s.realtime_denied);
        on_stream_error(&t, &cpal::Error::new(ErrorKind::DeviceNotAvailable));
        assert_eq!(t.take_fault(), Some(StreamFault::DeviceLost));
    }

    /// Lists whatever this machine has. Never fails: a CI runner without
    /// sound cards must still get a (possibly empty) list.
    #[test]
    fn listing_real_devices_never_panics() {
        let list = CpalBackend.list();
        assert!(!list.hosts.is_empty());
        for d in list.outputs.iter().chain(&list.inputs) {
            assert!(!d.id.is_empty());
        }
    }
}
