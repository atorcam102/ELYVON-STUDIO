//! Automation writing during playback (Phase 10).
//!
//! The publisher thread reports every engine position here (~30 Hz). A
//! recorder starts when the transport starts playing and finishes when it
//! stops; the captured gestures become one `WriteAutomation` edit (a single
//! undo step) and the new view is pushed to the UI. Control gestures arrive
//! through `automation_touch`: they go to the recorder and, as live
//! overrides, straight to the audio callback through its lock-free queue.
//! Lock order: session → recorder.

use std::sync::{Mutex, MutexGuard, PoisonError};

use elyvon_audio::automation::{ParamOverride, StripKey};
use elyvon_audio::transport::{PlayState, TransportSnapshot};
use elyvon_project::automation::{AutomationParam, AutomationTarget, channel_automation, param_exists};
use elyvon_project::automation_record::{AutomationRecorder, TouchPhase};
use elyvon_project::mixer::ChannelRef;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// Pushed with the new `SessionView` after written automation was applied.
pub const PROJECT_EVENT: &str = "project://view";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
pub struct AutomationCapture {
    recorder: Mutex<Option<AutomationRecorder>>,
}

fn tick_of(s: &TransportSnapshot) -> u64 {
    s.ticks.max(0.0).round() as u64
}

pub fn strip_key(channel: ChannelRef) -> StripKey {
    match channel {
        ChannelRef::Track { id } => StripKey::Track(id.0.as_u128()),
        ChannelRef::Aux { id } => StripKey::Aux(id.0.as_u128()),
        ChannelRef::Master => StripKey::Master,
    }
}

impl AutomationCapture {
    /// Called with every published engine position.
    pub fn on_snapshot(&self, app: &AppHandle, snap: &TransportSnapshot) {
        let playing = snap.state == PlayState::Playing;
        let state = app.state::<AppState>();
        if playing {
            let mut rec = lock(&self.recorder);
            match rec.as_mut() {
                Some(r) => r.observe(tick_of(snap), snap.cycles),
                None => {
                    drop(rec);
                    let session = state.session();
                    let mut rec = lock(&self.recorder);
                    if rec.is_none() {
                        *rec = Some(AutomationRecorder::start(session.project(), tick_of(snap), snap.cycles));
                    }
                }
            }
            return;
        }
        if lock(&self.recorder).is_none() {
            return;
        }
        let view = {
            let mut session = state.session();
            let Some(rec) = lock(&self.recorder).take() else {
                return;
            };
            let Some(cmd) = rec.finish(tick_of(snap), snap.cycles, session.project()) else {
                return;
            };
            match session.apply(&cmd) {
                Ok(_) => session.view(),
                Err(e) => {
                    log::warn!("written automation was rejected: {e}");
                    return;
                }
            }
        };
        log::info!("applied automation written during playback");
        state.audio.sync(app);
        let _ = app.emit(PROJECT_EVENT, view);
    }

    /// A control gesture. Returns whether it was captured (the channel
    /// writes and the transport plays); otherwise the UI applies the value
    /// as an ordinary edit.
    pub fn touch(
        &self,
        app: &AppHandle,
        channel: ChannelRef,
        param: AutomationParam,
        value: f64,
        phase: TouchPhase,
    ) -> bool {
        let state = app.state::<AppState>();
        let Some(snap) = state.audio.snapshot() else {
            return false;
        };
        if snap.state != PlayState::Playing || !value.is_finite() {
            return false;
        }
        let session = state.session();
        let project = session.project();
        let Some(mode) = channel_automation(project, channel).map(|a| a.mode) else {
            return false;
        };
        if !mode.writes() || !param_exists(project, channel, &param) {
            return false;
        }
        let target = AutomationTarget {
            channel,
            region: None,
            param,
        };
        let captured = {
            let mut rec = lock(&self.recorder);
            let r = rec.get_or_insert_with(|| AutomationRecorder::start(project, tick_of(&snap), snap.cycles));
            r.touch(target, mode, phase, value, tick_of(&snap), snap.cycles)
        };
        drop(session);
        if captured {
            state.audio.send_override(ParamOverride {
                strip: strip_key(channel),
                param,
                value,
                phase,
            });
        }
        captured
    }

    pub fn is_recording(&self) -> bool {
        lock(&self.recorder).is_some()
    }

    /// Drops an unfinished recording (project replaced).
    pub fn discard(&self) {
        lock(&self.recorder).take();
    }
}
