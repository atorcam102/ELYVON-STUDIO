//! Engine status reported to the UI (audio contract version 2).
//!
//! The UI enables playback controls **only** when `state == running`. Every
//! other state carries an actionable message; nothing pretends to work.

use serde::Serialize;
use ts_rs::TS;

use crate::format::{BufferLayout, ChannelLayout};

/// Lifecycle of the audio engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EngineState {
    /// The user turned the engine off, or this build has no audio backend.
    Disabled,
    /// A device is being opened.
    Starting,
    /// A stream is open and the callback runs.
    Running,
    /// No usable output device exists right now.
    NoDevice,
    /// The open device disappeared; the engine retries automatically.
    DeviceLost,
    /// The device could not be opened (busy, permissions, configuration…).
    Error,
}

/// Stable error codes for UI decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AudioErrorCode {
    DeviceBusy,
    DeviceNotAvailable,
    PermissionDenied,
    UnsupportedConfig,
    UnsupportedFormat,
    HostUnavailable,
    Backend,
}

/// Non-fatal conditions the user should know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoticeCode {
    /// The preferred device is missing; the system default is used instead.
    PreferredDeviceMissing,
    /// The requested sample rate/buffer size was unsupported and adjusted.
    ConfigAdjusted,
    /// The selected input could not be opened; output keeps running.
    InputUnavailable,
    /// The OS refused real-time priority for the audio thread.
    RealtimeDenied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineNotice {
    pub code: NoticeCode,
    pub message: String,
}

/// A stream that is actually open.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamInfo {
    /// Audio API, e.g. "ALSA", "WASAPI", "CoreAudio" or "Headless".
    pub host: String,
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    /// Fixed buffer size requested from the device; `None` = device default.
    pub requested_buffer_frames: Option<u32>,
    /// The backend's own estimate of frames per callback, when known.
    pub reported_buffer_frames: Option<u32>,
    pub channels: u16,
    pub layout: ChannelLayout,
    pub buffer_layout: BufferLayout,
    /// Device sample format, e.g. "f32", "i16".
    pub sample_format: String,
    pub bits_per_sample: u32,
    /// Buffer duration (one callback) in milliseconds, when the size is known.
    pub nominal_latency_ms: Option<f32>,
    /// True for the driver-free headless clock (no sound reaches any device).
    pub headless: bool,
}

/// Complete engine status. `revision` increases on every change.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineStatus {
    pub state: EngineState,
    pub revision: u32,
    /// Actionable explanation for every state other than `running`.
    pub message: Option<String>,
    pub error_code: Option<AudioErrorCode>,
    pub notices: Vec<EngineNotice>,
    pub output: Option<StreamInfo>,
    pub input: Option<StreamInfo>,
}

impl EngineStatus {
    pub fn new(state: EngineState, message: Option<String>) -> Self {
        Self {
            state,
            revision: 0,
            message,
            error_code: None,
            notices: Vec::new(),
            output: None,
            input: None,
        }
    }

    /// Status of a build without a device backend.
    pub fn unavailable() -> Self {
        Self::new(
            EngineState::Disabled,
            Some("This build has no audio device backend.".into()),
        )
    }

    pub fn is_running(&self) -> bool {
        self.state == EngineState::Running
    }
}
