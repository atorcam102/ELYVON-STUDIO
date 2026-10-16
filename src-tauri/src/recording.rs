//! Desktop recording lifecycle. Devices/files are owned off the UI and RT
//! threads. Project edits remain locked until takes have been committed.
use crate::{
    error::{ErrorCode, IpcError, IpcResult},
    state::AppState,
};
use elyvon_audio::{
    device::RunningOutput,
    input::{self, InputConfig},
    media::MediaStore,
    recording::{
        self, CaptureConfig, CaptureHealth, CaptureRoute, RecordingReport, RecordingState,
        RecordingWriter,
    },
    transport::TransportCommand,
};
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, PlacementTrack, Project, ProjectId,
    SessionView, TrackKind,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Manager};
use ts_rs::TS;
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn err(s: impl Into<String>) -> IpcError {
    IpcError::new(ErrorCode::AudioUnavailable, s)
}
#[derive(Debug)]
struct Active {
    input: RunningOutput,
    writer: RecordingWriter,
    project: Project,
    health: Arc<CaptureHealth>,
    monitor_only: bool,
    shared: Arc<elyvon_audio::transport::EngineShared>,
}
#[derive(Debug, Default)]
pub struct CaptureService {
    active: Mutex<Option<Active>>,
    busy: AtomicBool,
    last: Mutex<Option<RecordingReport>>,
}
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureStatus {
    pub active: bool,
    pub monitoring: bool,
    pub input_peak: f32,
    #[ts(type = "number")]
    pub clipped_frames: u64,
    pub interrupted: bool,
    #[ts(type = "number")]
    pub estimated_output_latency_frames: u64,
    #[ts(type = "number")]
    pub estimated_input_latency_frames: u64,
    pub last: Option<RecordingReport>,
}
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureResult {
    pub report: RecordingReport,
    pub view: SessionView,
}
struct ResetBusy<'a>(&'a AtomicBool);
impl Drop for ResetBusy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl CaptureService {
    pub fn is_active(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
    pub fn ensure_idle(&self) -> IpcResult<()> {
        if self.is_active() {
            Err(IpcError::new(
                ErrorCode::Busy,
                "Stop recording/input monitoring before changing the project, transport or audio device.",
            ))
        } else {
            Ok(())
        }
    }
    pub fn status(&self) -> CaptureStatus {
        let active = lock(&self.active);
        CaptureStatus {
            active: self.is_active(),
            monitoring: active.as_ref().is_some_and(|a| a.monitor_only),
            input_peak: active.as_ref().map_or(0.0, |a| a.health.peak.load()),
            clipped_frames: active
                .as_ref()
                .map_or(0, |a| a.health.clipped.load(Ordering::Relaxed)),
            interrupted: active.as_ref().is_some_and(|a| {
                a.input.has_failed()
                    || a.health.disconnected.load(Ordering::Relaxed)
                    || a.health.overflow.load(Ordering::Relaxed)
            }),
            estimated_output_latency_frames: active
                .as_ref()
                .map_or(0, |a| a.shared.output_latency()),
            estimated_input_latency_frames: active
                .as_ref()
                .map_or(0, |a| a.health.input_latency_frames.load(Ordering::Relaxed)),
            last: lock(&self.last).clone(),
        }
    }
    pub fn start(
        &self,
        app: &AppHandle,
        from_tick: u64,
        monitor_only: bool,
    ) -> IpcResult<CaptureStatus> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(IpcError::new(
                ErrorCode::Busy,
                "An input session is already running.",
            ));
        }
        let guard = ResetBusy(&self.busy);
        let state = app.state::<AppState>();
        state.midi.ensure_idle()?;
        let project = {
            let session = state.session();
            state.midi.ensure_idle()?;
            if state.rendering.load(Ordering::Acquire) {
                return Err(err("Wait for the diagnostic render to finish."));
            }
            session.project().clone()
        };
        if from_tick > elyvon_project::validate::limits::MAX_TICKS {
            return Err(err("capture start is outside the timeline"));
        }
        let settings = state.settings().audio.clone();
        let routes: Vec<_> = project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio && (t.io.armed || t.io.monitor))
            .map(|t| CaptureRoute {
                track: t.id,
                input: t.io.input_channel,
                channels: t.io.input_channels,
                armed: t.io.armed,
                monitor: t.io.monitor,
                gain: elyvon_audio::gain::db_to_gain(t.volume_db) as f32,
            })
            .collect();
        let channels = routes
            .iter()
            .map(|r| r.input + r.channels)
            .max()
            .ok_or_else(|| err("Arm an audio track or enable its input monitor first."))?;
        let (rate, shared, monitor) = state
            .audio
            .capture_context()
            .ok_or_else(|| err("The audio engine is not running."))?;
        let request = InputConfig {
            device: settings.input_device.clone(),
            rate,
            minimum_channels: channels,
            buffer: if settings.buffer_frames == 0 {
                256
            } else {
                settings.buffer_frames
            },
        };
        let (cfg, format) = input::negotiate(&request).map_err(err)?;
        let map = project
            .transport
            .tempo_map()
            .map_err(|e| err(e.to_string()))?;
        let t = &project.transport;
        let config = CaptureConfig {
            rate,
            input_channels: cfg.input_channels,
            start_frame: map.sample_at_tick(from_tick, rate),
            punch: t.punch.enabled.then(|| {
                (
                    map.sample_at_tick(t.punch.start, rate),
                    map.sample_at_tick(t.punch.end, rate),
                )
            }),
            cycle: t.loop_range.enabled.then(|| {
                (
                    map.sample_at_tick(t.loop_range.start, rate),
                    map.sample_at_tick(t.loop_range.end, rate),
                )
            }),
            compensation_frames: i64::from(settings.recording_offset_frames),
            monitor_only,
            max_bytes: None,
        };
        let dir = recording_dir(&state, project.id);
        let original_path = state.session().path().map(Path::to_path_buf);
        elyvon_project::recovery::write(
            &state.paths.recovery_dir,
            &project,
            original_path.as_deref(),
            &app.package_info().version.to_string(),
        )
        .map_err(IpcError::from)?;

        let (tap, writer) = RecordingWriter::start(&dir, config, routes, monitor).map_err(err)?;
        let health = Arc::clone(&tap.health);
        let input = match input::start(request, cfg, format, tap, Arc::clone(&shared)) {
            Ok(i) => i,
            Err(e) => {
                let _ = writer.finish(true);
                return Err(err(e));
            }
        };
        let active = Active {
            input,
            writer,
            project: project.clone(),
            health,
            monitor_only,
            shared,
        };
        *lock(&self.active) = Some(active);
        // Forget only the flag guard; the active input now owns this busy state.
        std::mem::forget(guard);
        if !monitor_only
            && state
                .audio
                .command(TransportCommand::Play {
                    from_tick: Some(from_tick),
                    count_in: t.count_in_bars > 0,
                    pre_roll: t.pre_roll_bars > 0,
                })
                .is_none()
        {
            self.interrupt();
            return Err(err(
                "The transport could not start; partial takes remain recoverable.",
            ));
        }
        Ok(self.status())
    }
    pub fn interrupt(&self) {
        if let Some(active) = lock(&self.active).take() {
            let _guard = ResetBusy(&self.busy);
            active.health.disconnected.store(true, Ordering::Release);
            drop(active.input);
            *lock(&self.last) = Some(active.writer.finish(false));
        }
    }
    pub fn stop(&self, app: &AppHandle, cancel: bool) -> IpcResult<CaptureResult> {
        let active = lock(&self.active)
            .take()
            .ok_or_else(|| IpcError::new(ErrorCode::Busy, "No input session is running."))?;
        let _guard = ResetBusy(&self.busy);
        drop(active.input); // stop callbacks, then drain and finalize the disk queue
        let state = app.state::<AppState>();
        state.audio.command(TransportCommand::Stop);
        let mut report = active.writer.finish(cancel);
        if report.state == RecordingState::Finalized
            && !report.takes.is_empty()
            && let Err(e) = commit_takes(app, &active.project, &report.takes)
        {
            report.state = RecordingState::Failed;
            report.error = Some(format!(
                "Takes are intact on disk but could not be added to the project: {e}"
            ));
        }
        *lock(&self.last) = Some(report.clone());
        Ok(CaptureResult {
            report,
            view: state.session().view(),
        })
    }
}
pub fn recording_dir(state: &AppState, id: ProjectId) -> PathBuf {
    state
        .paths
        .recovery_dir
        .parent()
        .unwrap_or(&state.paths.recovery_dir)
        .join("recordings")
        .join(id.to_string())
}
pub fn commit_takes(
    app: &AppHandle,
    project: &Project,
    takes: &[recording::RecordedTake],
) -> IpcResult<()> {
    let state = app.state::<AppState>();
    let rate = state.audio.rate();
    let mut budget = state.audio.remaining_budget();
    let mut assets = Vec::new();
    let mut placements = Vec::new();
    let mut buffers = Vec::new();
    let map = project
        .transport
        .tempo_map()
        .map_err(|e| err(e.to_string()))?;
    for (n, take) in takes.iter().enumerate() {
        if take.frames == 0 {
            continue;
        }
        let path = Path::new(&take.path);
        let buffer = MediaStore::load_file(path, rate, budget)
            .map_err(|e| IpcError::new(ErrorCode::Media, e.to_string()))?;
        budget = budget.saturating_sub(buffer.memory_bytes());
        let id = MediaId::new();
        assets.push(MediaAsset {
            id,
            name: format!("Take {}", n + 1),
            path: take.path.clone(),
            info: Some(buffer.info.clone()),
        });
        placements.push(AudioPlacement {
            media_id: id,
            track: PlacementTrack::Existing { id: take.track },
            start: map
                .tick_at_sample(take.start_frame, take.sample_rate)
                .round() as u64,
        });
        buffers.push((id, path.to_owned(), buffer));
    }
    {
        let mut session = state.session();
        if session.project().id != project.id {
            return Err(IpcError::new(
                ErrorCode::Busy,
                "The capture belongs to a different project.",
            ));
        }
        session.apply(&EditCommand::ImportTakes {
            assets,
            placements,
            recorded_at: takes.first().map_or(0, |t| t.recorded_at),
        })?;
    }
    for (id, path, buffer) in buffers {
        state.audio.insert_loaded(id, &path, buffer);
    }
    state.audio.sync(app);
    Ok(())
}
