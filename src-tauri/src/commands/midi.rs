use crate::{
    error::{ErrorCode, IpcError, IpcResult},
    midi::MidiStatus,
    state::AppState,
};
use elyvon_project::{EditCommand, SessionView, TrackId};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
#[tauri::command]
pub async fn midi_status(app: AppHandle) -> IpcResult<MidiStatus> {
    Ok(app.state::<AppState>().midi.status())
}
#[tauri::command]
pub async fn midi_refresh(app: AppHandle) -> IpcResult<()> {
    app.state::<AppState>().midi.refresh()
}
#[tauri::command]
pub async fn midi_panic(app: AppHandle) -> IpcResult<()> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().midi.panic())
        .await
        .map_err(IpcError::from)?
}
#[tauri::command]
pub async fn midi_send(app: AppHandle, track_id: TrackId, bytes: Vec<u8>) -> IpcResult<()> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().midi.send(track_id, bytes))
        .await
        .map_err(IpcError::from)?
}
#[tauri::command]
pub async fn midi_record_start(app: AppHandle, from_tick: u64) -> IpcResult<MidiStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.capture.ensure_idle()?;
        state.midi.start_record(&state, from_tick)?;
        Ok(state.midi.status())
    })
    .await
    .map_err(IpcError::from)?
}
#[tauri::command]
pub async fn midi_record_stop(app: AppHandle, cancel: bool) -> IpcResult<SessionView> {
    tauri::async_runtime::spawn_blocking(move||{
 let state=app.state::<AppState>();let result=(||->IpcResult<SessionView>{
  let recorded=state.midi.stop_record(cancel)?;
  if let Some(error)=&recorded.error{log::warn!("MIDI recording: {error}");}
  if !recorded.regions.is_empty(){
   // Preserve the raw capture before the undoable project commit.
   let root=crate::recording::recording_dir(&state,recorded.project);std::fs::create_dir_all(&root).map_err(|e|IpcError::new(ErrorCode::Io,e.to_string()))?;
   let backup=root.join(format!("midi-{}.json",elyvon_project::RegionId::new()));
   let bytes=serde_json::to_vec_pretty(&recorded.regions).map_err(|e|IpcError::internal(e.to_string()))?;elyvon_core::fs::atomic_write(&backup,&bytes).map_err(|e|IpcError::new(ErrorCode::Io,e.to_string()))?;
   let mut session=state.session();if session.project().id!=recorded.project{return Err(IpcError::new(ErrorCode::Busy,"Captured MIDI belongs to a different project; the raw capture was preserved in recordings."));}
   session.apply(&EditCommand::ImportMidi{regions:recorded.regions,tempo:None,tempo_changes:vec![],time_signature:None,meter_changes:vec![]})?;
  }
  state.audio.sync(&app);Ok(state.session().view())
 })();if result.is_ok(){state.midi.finish_record();}result
}).await.map_err(IpcError::from)?
}
#[tauri::command]
pub async fn midi_import(
    app: AppHandle,
    path: String,
    start_tick: u64,
    use_file_tempo: bool,
) -> IpcResult<SessionView> {
    tauri::async_runtime::spawn_blocking(move||{
 let state=app.state::<AppState>();state.ensure_editable()?;let file=PathBuf::from(path);
 if std::fs::metadata(&file).map_err(|e|IpcError::new(ErrorCode::Io,e.to_string()))?.len()>16<<20{return Err(IpcError::new(ErrorCode::Media,"MIDI file exceeds 16 MiB"));}
 let bytes=std::fs::read(file).map_err(|e|IpcError::new(ErrorCode::Io,e.to_string()))?;
 let mut midi=elyvon_audio::midi::smf::decode(&bytes).map_err(|e|IpcError::new(ErrorCode::Media,e))?;
 for region in &mut midi.regions{region.start=region.start.checked_add(start_tick).ok_or_else(||IpcError::new(ErrorCode::InvalidEdit,"MIDI start overflow"))?;}
 let view={let mut session=state.session();state.ensure_editable()?;
  if use_file_tempo&&(!session.project().tracks.is_empty()||start_tick!=0){return Err(IpcError::new(ErrorCode::InvalidEdit,"Import the file tempo map into an empty project at the beginning, or keep the current project tempo."));}
  session.apply(&EditCommand::ImportMidi{regions:midi.regions,tempo:use_file_tempo.then_some(midi.tempo),tempo_changes:midi.tempo_changes,time_signature:use_file_tempo.then_some(midi.time_signature),meter_changes:midi.meter_changes})?;session.view()};
 state.audio.sync(&app);Ok(view)
}).await.map_err(IpcError::from)?
}
#[tauri::command]
pub async fn midi_export(app: AppHandle, path: String) -> IpcResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let project = app.state::<AppState>().session().project().clone();
        let bytes = elyvon_audio::midi::smf::encode(&project)
            .map_err(|e| IpcError::new(ErrorCode::Media, e))?;
        let path = PathBuf::from(path).with_extension("mid");
        elyvon_core::fs::atomic_write(&path, &bytes)
            .map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(IpcError::from)?
}
