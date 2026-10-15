//! The seam between the device manager and concrete audio APIs.
//!
//! [`crate::cpal_backend`] talks to WASAPI/CoreAudio/ALSA, [`crate::headless`]
//! is a driver-free clock, and tests use a scripted fake. The manager only
//! ever sees this trait, so hot-unplug and failure handling is testable
//! without hardware.

use std::sync::Arc;

use elyvon_audio::render::EngineCore;
use elyvon_audio::status::{AudioErrorCode, EngineNotice, StreamInfo};
use elyvon_audio::telemetry::Telemetry;

use crate::types::DeviceList;

/// Which output to open. `None` fields mean "system default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputRequest {
    pub host: Option<String>,
    pub device: Option<String>,
    /// Display name of the preferred device, used in fallback messages.
    pub device_name: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
}

/// Which input to meter. Always an explicit device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRequest {
    pub host: Option<String>,
    pub device: String,
    /// Prefer the output's rate so a future duplex graph stays aligned.
    pub sample_rate: Option<u32>,
}

/// A failure to list or open a device, with an actionable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub code: AudioErrorCode,
    pub message: String,
}

impl BackendError {
    pub fn new(code: AudioErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for BackendError {}

/// An open stream. Dropping it stops the stream and joins its callback
/// thread, so no callback runs after the drop returns.
pub struct OpenStream {
    pub info: StreamInfo,
    pub notices: Vec<EngineNotice>,
    pub handle: Box<dyn Send>,
}

impl std::fmt::Debug for OpenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenStream")
            .field("info", &self.info)
            .field("notices", &self.notices)
            .finish_non_exhaustive()
    }
}

/// Builds the engine core once the backend has negotiated the format.
/// Arguments: sample rate, output channels.
pub type CoreFactory<'a> = &'a mut dyn FnMut(u32, u16) -> EngineCore;

/// A concrete audio API family. Runs on the manager thread only.
pub trait Backend: Send + 'static {
    /// Scans hosts and devices. Never fails as a whole.
    fn list(&self) -> DeviceList;

    /// Whether an output device with this id is currently present.
    fn output_exists(&self, host: Option<&str>, device: &str) -> bool;

    /// Opens an output stream that runs the engine core in its callback.
    fn open_output(
        &self,
        request: &OutputRequest,
        make_core: CoreFactory<'_>,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError>;

    /// Opens an input stream that only meters levels into `telemetry`.
    fn open_input(
        &self,
        request: &InputRequest,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError>;
}
