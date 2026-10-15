//! Device descriptions and persisted audio preferences (IPC types).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Sample rates offered in the UI when a device supports them.
pub const STANDARD_SAMPLE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
/// Buffer sizes offered in the UI when a device supports them.
pub const STANDARD_BUFFER_SIZES: [u32; 8] = [32, 64, 128, 256, 512, 1024, 2048, 4096];
/// Remembered per-device profiles; the oldest is dropped beyond this.
pub const MAX_DEVICE_PROFILES: usize = 32;

/// An audio API available on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostInfo {
    /// Stable identifier, e.g. "alsa", "wasapi", "coreaudio", "headless".
    pub id: String,
    /// Display name, e.g. "ALSA".
    pub name: String,
    pub is_default: bool,
    /// True for the driver-free clock that produces no sound.
    pub headless: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Direction {
    Input,
    Output,
}

/// One supported configuration range reported by a device ("profile").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConfigRange {
    pub channels: u16,
    pub min_sample_rate: u32,
    pub max_sample_rate: u32,
    /// e.g. "f32", "i16", "i24".
    pub sample_format: String,
    pub bits_per_sample: u32,
    /// Whether the engine can stream this format.
    pub supported_by_engine: bool,
    /// Buffer size range in frames; `None` when the backend cannot tell.
    pub min_buffer_frames: Option<u32>,
    pub max_buffer_frames: Option<u32>,
}

/// The configuration the system would choose by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefaultConfig {
    pub channels: u16,
    pub sample_rate: u32,
    pub sample_format: String,
    pub bits_per_sample: u32,
}

/// A device and everything the UI needs to configure it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceInfo {
    pub host: String,
    /// Persistable identifier (e.g. "alsa:hw:CARD=USB,DEV=0").
    pub id: String,
    pub name: String,
    pub manufacturer: Option<String>,
    pub driver: Option<String>,
    pub direction: Direction,
    pub is_default: bool,
    pub default_config: Option<DefaultConfig>,
    pub ranges: Vec<ConfigRange>,
    /// Standard sample rates inside at least one usable range.
    pub sample_rates: Vec<u32>,
    /// Standard buffer sizes inside the reported range (empty = unknown).
    pub buffer_sizes: Vec<u32>,
    pub max_channels: u16,
    /// Why details could not be read (device busy, disappeared…).
    pub error: Option<String>,
}

/// Result of a device scan. Errors of one host never hide the others.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceList {
    pub hosts: Vec<HostInfo>,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    pub errors: Vec<String>,
}

/// Remembered settings for one device, applied whenever it is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceProfile {
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
}

/// Persisted audio preferences (part of `settings.json`). Missing fields take
/// defaults, so older settings files load unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AudioPreferences {
    /// Open the audio device at start-up.
    pub enabled: bool,
    /// Audio API id; `None` = platform default.
    pub host: Option<String>,
    /// Output device id; `None` = system default output.
    pub output_device: Option<String>,
    /// Display name of the preferred output, for messages when it is missing.
    pub output_device_name: Option<String>,
    /// Input device id for level metering; `None` = no input stream.
    pub input_device: Option<String>,
    pub profiles: Vec<DeviceProfile>,
}

impl Default for AudioPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            host: None,
            output_device: None,
            output_device_name: None,
            input_device: None,
            profiles: Vec::new(),
        }
    }
}

/// What the user picked in Settings › Audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioSetup {
    pub enabled: bool,
    pub host: Option<String>,
    pub output_device: Option<String>,
    pub output_device_name: Option<String>,
    pub input_device: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
}

fn valid_rate(rate: Option<u32>) -> Option<u32> {
    rate.filter(|r| (8_000..=384_000).contains(r))
}

fn valid_buffer(frames: Option<u32>) -> Option<u32> {
    frames.filter(|f| (16..=8192).contains(f))
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty() && s.len() <= 512)
}

impl AudioPreferences {
    /// Clamps values from disk or IPC into valid ranges.
    pub fn sanitized(mut self) -> Self {
        self.host = non_empty(self.host);
        self.output_device = non_empty(self.output_device);
        self.output_device_name = non_empty(self.output_device_name);
        self.input_device = non_empty(self.input_device);
        let mut seen = std::collections::HashSet::new();
        self.profiles
            .retain(|p| !p.device_id.is_empty() && seen.insert(p.device_id.clone()));
        for p in &mut self.profiles {
            p.sample_rate = valid_rate(p.sample_rate);
            p.buffer_frames = valid_buffer(p.buffer_frames);
        }
        self.profiles.truncate(MAX_DEVICE_PROFILES);
        self
    }

    /// Profile key for the default device of a host.
    pub fn default_key(host: Option<&str>) -> String {
        format!("default:{}", host.unwrap_or("system"))
    }

    /// The remembered profile for an output selection (explicit or default).
    pub fn profile_for(&self, device: Option<&str>) -> Option<&DeviceProfile> {
        let key = device
            .map(str::to_owned)
            .unwrap_or_else(|| Self::default_key(self.host.as_deref()));
        self.profiles.iter().find(|p| p.device_id == key)
    }

    /// Applies a setup from the UI, remembering its rate/buffer for that device.
    pub fn apply_setup(&mut self, setup: AudioSetup) {
        self.enabled = setup.enabled;
        self.host = non_empty(setup.host);
        self.output_device = non_empty(setup.output_device);
        self.output_device_name = non_empty(setup.output_device_name);
        self.input_device = non_empty(setup.input_device);
        let key = self
            .output_device
            .clone()
            .unwrap_or_else(|| Self::default_key(self.host.as_deref()));
        let profile = DeviceProfile {
            device_id: key.clone(),
            device_name: self
                .output_device_name
                .clone()
                .unwrap_or_else(|| "System default output".into()),
            sample_rate: valid_rate(setup.sample_rate),
            buffer_frames: valid_buffer(setup.buffer_frames),
        };
        self.profiles.retain(|p| p.device_id != key);
        self.profiles.insert(0, profile);
        self.profiles.truncate(MAX_DEVICE_PROFILES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_without_audio_fields_get_defaults() {
        let p: AudioPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(p, AudioPreferences::default());
        assert!(p.enabled);
        let p: AudioPreferences =
            serde_json::from_str(r#"{"enabled":false,"unknownFutureField":1}"#).unwrap();
        assert!(!p.enabled);
    }

    #[test]
    fn profiles_are_remembered_per_device_and_sanitized() {
        let mut p = AudioPreferences::default();
        p.apply_setup(AudioSetup {
            enabled: true,
            host: Some("alsa".into()),
            output_device: Some("alsa:hw:1".into()),
            output_device_name: Some("USB".into()),
            input_device: None,
            sample_rate: Some(96_000),
            buffer_frames: Some(128),
        });
        p.apply_setup(AudioSetup {
            enabled: true,
            host: Some("alsa".into()),
            output_device: None,
            output_device_name: None,
            input_device: Some("  ".into()),
            sample_rate: Some(1),
            buffer_frames: Some(1_000_000),
        });
        let usb = p.profile_for(Some("alsa:hw:1")).unwrap();
        assert_eq!(
            (usb.sample_rate, usb.buffer_frames),
            (Some(96_000), Some(128))
        );
        let default = p.profile_for(None).unwrap();
        assert_eq!(default.device_id, "default:alsa");
        assert_eq!((default.sample_rate, default.buffer_frames), (None, None));
        assert_eq!(p.input_device, None);
        // Re-selecting a device replaces its profile, never duplicates it.
        p.profiles.push(p.profiles[0].clone());
        p.profiles.push(DeviceProfile {
            device_id: "x".into(),
            device_name: "x".into(),
            sample_rate: Some(5),
            buffer_frames: Some(64),
        });
        let p = p.sanitized();
        assert_eq!(p.profiles.len(), 3);
        assert_eq!(p.profiles[2].sample_rate, None);
    }
}
