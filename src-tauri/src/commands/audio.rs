//! Audio engine commands. The engine runs in `elyvon-audio-device`; these
//! commands only translate between the UI's musical time (ticks) and the
//! engine's sample frames, persist preferences and forward requests.
//!
//! Nothing here can modify the project: transport state is engine state.

use std::time::Duration;

use elyvon_audio::command::TestTone;
use elyvon_audio::status::EngineStatus;
use elyvon_audio::telemetry::TelemetrySnapshot;
use elyvon_audio_device::types::{AudioSetup, DeviceList};
use elyvon_core::music::{Tempo, Ticks};
use tauri::{AppHandle, Manager, State};

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

/// Opening a device can take a few seconds on some drivers (and Bluetooth).
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(15);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Emitted with the new [`EngineStatus`] whenever it changes.
pub const AUDIO_STATUS_EVENT: &str = "audio://status";

fn not_available() -> IpcError {
    IpcError::new(
        ErrorCode::AudioUnavailable,
        "The audio engine has been shut down.",
    )
}

/// Musical ticks → frames at `sample_rate`, using the project tempo.
pub fn ticks_to_frames(tempo: Tempo, ticks: Ticks, sample_rate: u32) -> u64 {
    (tempo.ticks_to_seconds(ticks) * f64::from(sample_rate)).round() as u64
}

/// Frames → musical ticks (rounded down).
pub fn frames_to_ticks(tempo: Tempo, frames: u64, sample_rate: u32) -> Ticks {
    if sample_rate == 0 {
        return 0;
    }
    tempo.seconds_to_ticks(frames as f64 / f64::from(sample_rate))
}

fn tempo(state: &AppState) -> Tempo {
    state.session().project().transport.tempo
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> IpcResult<T> + Send + 'static,
) -> IpcResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(IpcError::from)?
}

#[tauri::command]
pub async fn audio_status(state: State<'_, AppState>) -> IpcResult<EngineStatus> {
    Ok(state
        .audio()
        .map_or_else(EngineStatus::unavailable, |m| m.status()))
}

/// Scans hosts and devices (on the audio manager thread).
#[tauri::command]
pub async fn audio_devices(app: AppHandle) -> IpcResult<DeviceList> {
    blocking(move || {
        let manager = app.state::<AppState>().audio().ok_or_else(not_available)?;
        Ok(manager.devices(LIST_TIMEOUT)?)
    })
    .await
}

/// Applies and persists the user's device choice, then reopens the streams.
#[tauri::command]
pub async fn audio_configure(app: AppHandle, setup: AudioSetup) -> IpcResult<EngineStatus> {
    blocking(move || {
        let state = app.state::<AppState>();
        let (prefs, settings) = {
            let mut settings = state.settings();
            settings.audio.apply_setup(setup);
            settings.audio = settings.audio.clone().sanitized();
            (settings.audio.clone(), settings.clone())
        };
        if let Err(e) = settings.save(&state.paths.config_dir) {
            log::warn!("could not save audio preferences: {e}");
            return Err(IpcError::new(
                ErrorCode::Io,
                format!("Could not save audio preferences: {e}"),
            ));
        }
        let manager = state.audio().ok_or_else(not_available)?;
        Ok(manager.apply(prefs, CONFIGURE_TIMEOUT)?)
    })
    .await
}

/// Reopens the saved configuration (e.g. "Use preferred device" / "Retry").
#[tauri::command]
pub async fn audio_reconnect(app: AppHandle) -> IpcResult<EngineStatus> {
    blocking(move || {
        let state = app.state::<AppState>();
        let prefs = state.settings().audio.clone();
        let manager = state.audio().ok_or_else(not_available)?;
        Ok(manager.apply(prefs, CONFIGURE_TIMEOUT)?)
    })
    .await
}

/// Meters, clocks and stream statistics. Clears held meter peaks.
#[tauri::command]
pub async fn audio_telemetry(state: State<'_, AppState>) -> IpcResult<TelemetrySnapshot> {
    let mut snapshot = state.audio().ok_or_else(not_available)?.telemetry();
    if snapshot.sample_rate > 0 {
        snapshot.position_ticks = Some(frames_to_ticks(
            tempo(&state),
            snapshot.transport_frames,
            snapshot.sample_rate,
        ));
    }
    Ok(snapshot)
}

fn with_running<T>(
    state: &AppState,
    f: impl FnOnce(&elyvon_audio_device::manager::AudioManager, u32) -> IpcResult<T>,
) -> IpcResult<T> {
    let manager = state.audio().ok_or_else(not_available)?;
    let status = manager.status();
    let Some(rate) = status.output.as_ref().map(|o| o.sample_rate) else {
        return Err(IpcError::new(
            ErrorCode::AudioUnavailable,
            status
                .message
                .unwrap_or_else(|| "The audio engine is not running.".into()),
        ));
    };
    f(&manager, rate)
}

/// Starts playback, optionally from a musical position.
#[tauri::command]
pub async fn audio_play(state: State<'_, AppState>, from_ticks: Option<Ticks>) -> IpcResult<()> {
    let tempo = tempo(&state);
    with_running(&state, |m, rate| {
        if let Some(ticks) = from_ticks {
            m.locate(ticks_to_frames(tempo, ticks, rate))?;
        }
        Ok(m.play()?)
    })
}

#[tauri::command]
pub async fn audio_stop(state: State<'_, AppState>) -> IpcResult<()> {
    with_running(&state, |m, _| Ok(m.stop()?))
}

#[tauri::command]
pub async fn audio_locate(state: State<'_, AppState>, ticks: Ticks) -> IpcResult<()> {
    let tempo = tempo(&state);
    with_running(&state, |m, rate| {
        Ok(m.locate(ticks_to_frames(tempo, ticks, rate))?)
    })
}

/// Sets the opt-in test tone; returns the sanitised settings actually used.
#[tauri::command]
pub async fn audio_test_tone(state: State<'_, AppState>, tone: TestTone) -> IpcResult<TestTone> {
    with_running(&state, |m, _| Ok(m.set_test_tone(tone)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_and_frames_round_trip_at_every_rate() {
        let tempo = Tempo::new(120.0).unwrap();
        for rate in [44_100, 48_000, 96_000] {
            // One beat at 120 BPM is half a second.
            assert_eq!(ticks_to_frames(tempo, 960, rate), u64::from(rate) / 2);
            assert_eq!(frames_to_ticks(tempo, u64::from(rate) / 2, rate), 960);
            for ticks in [0, 1, 959, 3840, 123_456] {
                let back = frames_to_ticks(tempo, ticks_to_frames(tempo, ticks, rate), rate);
                assert!(back.abs_diff(ticks) <= 1, "{ticks} -> {back} at {rate}");
            }
        }
        assert_eq!(frames_to_ticks(tempo, 1000, 0), 0);
    }
}
