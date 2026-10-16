use crate::{
    error::{ErrorCode, IpcError, IpcResult},
    recording::{CaptureResult, CaptureStatus},
    state::AppState,
};
use elyvon_audio::{
    device::DeviceInfo,
    recording::{RecordedTake, RecordingReport},
};
use tauri::{AppHandle, Manager};

#[tauri::command]
pub async fn recording_devices() -> IpcResult<Vec<DeviceInfo>> {
    tauri::async_runtime::spawn_blocking(elyvon_audio::input::input_devices)
        .await
        .map_err(IpcError::from)
}
#[tauri::command]
pub async fn recording_status(app: AppHandle) -> IpcResult<CaptureStatus> {
    Ok(app.state::<AppState>().capture.status())
}
#[tauri::command]
pub async fn recording_start(
    app: AppHandle,
    from_tick: u64,
    monitor_only: bool,
) -> IpcResult<CaptureStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .capture
            .start(&app, from_tick, monitor_only)
    })
    .await
    .map_err(IpcError::from)?
}
#[tauri::command]
pub async fn recording_stop(app: AppHandle, cancel: bool) -> IpcResult<CaptureResult> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().capture.stop(&app, cancel))
        .await
        .map_err(IpcError::from)?
}
#[tauri::command]
pub async fn recording_recoverable(app: AppHandle) -> IpcResult<Vec<RecordedTake>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let id = state.session().project().id;
        let dir = crate::recording::recording_dir(&state, id);
        let mut takes = Vec::new();
        if let Ok(files) = std::fs::read_dir(dir) {
            for f in files.take(4096).flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "json")
                    && f.metadata().is_ok_and(|m| m.len() < 64 * 1024)
                    && let Ok(data) = std::fs::read(&p)
                    && let Ok(t) = serde_json::from_slice::<RecordedTake>(&data)
                    && t.recoverable
                    && std::path::Path::new(&t.path) == p.with_extension("wav")
                {
                    takes.push(t);
                }
            }
        }
        takes
    })
    .await
    .map_err(IpcError::from)
}
#[tauri::command]
pub async fn recording_recover(app: AppHandle, path: String) -> IpcResult<CaptureResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.ensure_editable()?;
        let project = state.session().project().clone();
        let dir = crate::recording::recording_dir(&state, project.id);
        let actual = std::fs::canonicalize(&path)
            .map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))?;
        let base =
            std::fs::canonicalize(dir).map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))?;
        if actual.parent() != Some(base.as_path())
            || !actual
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".partial.wav"))
        {
            return Err(IpcError::new(
                ErrorCode::InvalidEdit,
                "Choose a recoverable take from this project.",
            ));
        }
        let take = elyvon_audio::recording::recover_take(&actual.with_extension("json"))
            .map_err(|e| IpcError::new(ErrorCode::Io, e))?;
        crate::recording::commit_takes(&app, &project, std::slice::from_ref(&take))?;
        Ok(CaptureResult {
            report: RecordingReport {
                state: elyvon_audio::recording::RecordingState::Finalized,
                takes: vec![take],
                error: None,
                input_peak: 0.0,
                clipped_frames: 0,
                overflow: false,
            },
            view: state.session().view(),
        })
    })
    .await
    .map_err(IpcError::from)?
}

#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LatencyMeasurement {
    pub sample_rate: u32,
    pub delay_frames: u32,
    pub correlation: f64,
}
/// Compare aligned files from a real loopback experiment, never the callback.
#[tauri::command]
pub async fn recording_measure_latency(
    reference_path: String,
    captured_path: String,
    max_delay: u32,
) -> IpcResult<LatencyMeasurement> {
    tauri::async_runtime::spawn_blocking(move || {
        if max_delay > 192000 {
            return Err(IpcError::new(
                ErrorCode::InvalidEdit,
                "Search at most 192000 frames.",
            ));
        }
        let reference =
            elyvon_audio::media::decode_file(std::path::Path::new(&reference_path), 8192)
                .map_err(|e| IpcError::new(ErrorCode::Media, e.to_string()))?;
        let captured = elyvon_audio::media::decode_file(
            std::path::Path::new(&captured_path),
            8192 + u64::from(max_delay),
        )
        .map_err(|e| IpcError::new(ErrorCode::Media, e.to_string()))?;
        if reference.info.sample_rate != captured.info.sample_rate {
            return Err(IpcError::new(
                ErrorCode::InvalidEdit,
                "Calibration files must have the same sample rate and aligned time origin.",
            ));
        }
        let measurement = elyvon_audio::latency::measure_loopback(
            &reference.planes[0],
            &captured.planes[0],
            (max_delay as usize).min(
                captured.planes[0]
                    .len()
                    .saturating_sub(reference.planes[0].len()),
            ),
        )
        .map_err(|e| IpcError::new(ErrorCode::InvalidEdit, e))?;
        Ok(LatencyMeasurement {
            sample_rate: reference.info.sample_rate,
            delay_frames: measurement.delay_frames,
            correlation: measurement.correlation,
        })
    })
    .await
    .map_err(IpcError::from)?
}
