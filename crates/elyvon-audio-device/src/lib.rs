//! Audio device layer for Elyvon Studio, independent of Tauri.
//!
//! * [`cpal_backend`] — WASAPI / CoreAudio / ALSA through CPAL.
//! * [`headless`] — a driver-free clock for CI and machines without devices.
//! * [`manager`] — the control thread that opens, monitors, recovers and
//!   reconnects streams.
//! * [`types`] — device descriptions and persisted preferences.
//!
//! The engine core itself (`elyvon-audio`) never depends on this crate.

pub mod backend;
pub mod cpal_backend;
pub mod headless;
pub mod manager;
pub mod types;

use std::sync::Arc;

use elyvon_audio::telemetry::Telemetry;

use crate::backend::{Backend, BackendError, CoreFactory, InputRequest, OpenStream, OutputRequest};
use crate::cpal_backend::CpalBackend;
use crate::headless::{HEADLESS_DEVICE, HEADLESS_HOST, HeadlessBackend};
use crate::types::DeviceList;

/// The production backend: every CPAL host plus the explicit headless clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemBackend {
    cpal: CpalBackend,
    headless: HeadlessBackend,
}

fn is_headless(host: Option<&str>, device: Option<&str>) -> bool {
    host == Some(HEADLESS_HOST) || device == Some(HEADLESS_DEVICE)
}

impl Backend for SystemBackend {
    fn list(&self) -> DeviceList {
        let mut list = self.cpal.list();
        let headless = self.headless.list();
        list.hosts.extend(headless.hosts);
        list.outputs.extend(headless.outputs);
        list
    }

    fn output_exists(&self, host: Option<&str>, device: &str) -> bool {
        if is_headless(host, Some(device)) {
            self.headless.output_exists(host, device)
        } else {
            self.cpal.output_exists(host, device)
        }
    }

    fn open_output(
        &self,
        request: &OutputRequest,
        make_core: CoreFactory<'_>,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        if is_headless(request.host.as_deref(), request.device.as_deref()) {
            self.headless.open_output(request, make_core, telemetry)
        } else {
            self.cpal.open_output(request, make_core, telemetry)
        }
    }

    fn open_input(
        &self,
        request: &InputRequest,
        telemetry: Arc<Telemetry>,
    ) -> Result<OpenStream, BackendError> {
        if is_headless(request.host.as_deref(), Some(&request.device)) {
            self.headless.open_input(request, telemetry)
        } else {
            self.cpal.open_input(request, telemetry)
        }
    }
}
