//! Native effects (Phases 12–13): the parameter catalogue the editors are
//! drawn from, versioned presets (factory and user files), the exact
//! equaliser response, latency details and the off-real-time spectrum
//! analyser. Parameter edits themselves are ordinary undoable project
//! edits (`setInsert`), so presets, A/B and automation share one path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use elyvon_audio::spectrum::Spectrum;
use elyvon_project::effects::{
    ALL_EFFECTS, EffectCategory, EffectKind, EffectPreset, EffectSlot, PRESET_EXTENSION,
    ParamScale, ParamUnit, ParamValues, factory_presets,
};
use elyvon_project::mixer::ChannelRef;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectParamInfo {
    /// Automation index (append-only).
    pub index: u8,
    pub key: String,
    pub label: String,
    pub unit: ParamUnit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: ParamScale,
    pub choices: Vec<String>,
    pub automatable: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectInfo {
    pub kind: EffectKind,
    pub label: String,
    pub short: String,
    pub category: EffectCategory,
    pub has_sidechain: bool,
    pub params: Vec<EffectParamInfo>,
}

/// Every native effect and its parameters.
#[tauri::command]
pub async fn effects_catalog() -> IpcResult<Vec<EffectInfo>> {
    Ok(ALL_EFFECTS
        .iter()
        .map(|&kind| EffectInfo {
            kind,
            label: kind.label().into(),
            short: kind.short().into(),
            category: kind.category(),
            has_sidechain: kind.has_sidechain(),
            params: kind
                .params()
                .iter()
                .enumerate()
                .map(|(i, d)| EffectParamInfo {
                    index: i as u8,
                    key: d.key.into(),
                    label: d.label.into(),
                    unit: d.unit,
                    min: d.min,
                    max: d.max,
                    default: d.default,
                    scale: d.scale,
                    choices: d.choices.iter().map(|c| (*c).to_string()).collect(),
                    automatable: d.automatable,
                })
                .collect(),
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PresetSource {
    Factory,
    User,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetEntry {
    pub name: String,
    pub source: PresetSource,
    pub version: u32,
    /// Values for every parameter of the kind (missing keys defaulted).
    pub params: BTreeMap<String, f64>,
}

fn kind_dir(root: &Path, kind: EffectKind) -> PathBuf {
    let tag = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "effect".into());
    root.join(tag)
}

/// File-name stem for a preset name: letters, digits, spaces, `-`, `_`.
fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim().chars().take(96).collect()
}

fn entry(p: &EffectPreset, kind: EffectKind, source: PresetSource) -> Option<PresetEntry> {
    let values = p.values_for(kind).ok()?;
    Some(PresetEntry {
        name: p.name.clone(),
        source,
        version: p.version,
        params: values.to_map(kind),
    })
}

/// Factory presets followed by the user's saved presets of `kind`.
/// Unreadable or incompatible user files are skipped and logged.
#[tauri::command]
pub async fn effect_presets(app: AppHandle, kind: EffectKind) -> IpcResult<Vec<PresetEntry>> {
    let mut out: Vec<PresetEntry> = factory_presets(kind)
        .iter()
        .filter_map(|p| entry(p, kind, PresetSource::Factory))
        .collect();
    let dir = kind_dir(&app.state::<AppState>().paths.presets_dir, kind);
    let mut user = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        for e in read.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some(PRESET_EXTENSION) {
                continue;
            }
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| EffectPreset::from_json(&b).map_err(|e| e.to_string()))
            {
                Ok(p) => {
                    if let Some(x) = entry(&p, kind, PresetSource::User) {
                        user.push(x);
                    }
                }
                Err(e) => log::warn!("skipping preset {}: {e}", path.display()),
            }
        }
    }
    user.sort_by_key(|a| a.name.to_lowercase());
    out.extend(user);
    Ok(out)
}

/// Saves the current values of an effect as a user preset (atomic write;
/// a preset with the same name is replaced).
#[tauri::command]
pub async fn effect_preset_save(
    app: AppHandle,
    kind: EffectKind,
    name: String,
    params: BTreeMap<String, f64>,
) -> IpcResult<PresetEntry> {
    let name = name.trim().to_string();
    let stem = file_stem(&name);
    if name.is_empty() || stem.is_empty() || name.chars().count() > 96 {
        return Err(IpcError::new(
            ErrorCode::InvalidEdit,
            "El nombre del preset debe tener entre 1 y 96 caracteres.",
        ));
    }
    let values = ParamValues::from_map(kind, &params);
    let preset = EffectPreset::new(kind, &name, &values);
    let dir = kind_dir(&app.state::<AppState>().paths.presets_dir, kind);
    std::fs::create_dir_all(&dir).map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))?;
    let path = dir.join(format!("{stem}.{PRESET_EXTENSION}"));
    elyvon_core::fs::atomic_write(&path, &preset.to_json())
        .map_err(|e| IpcError::new(ErrorCode::Io, e.to_string()))?;
    entry(&preset, kind, PresetSource::User)
        .ok_or_else(|| IpcError::internal("the saved preset could not be read back"))
}

/// Deletes a user preset file (factory presets cannot be deleted).
#[tauri::command]
pub async fn effect_preset_delete(app: AppHandle, kind: EffectKind, name: String) -> IpcResult<()> {
    let dir = kind_dir(&app.state::<AppState>().paths.presets_dir, kind);
    let path = dir.join(format!("{}.{PRESET_EXTENSION}", file_stem(&name)));
    std::fs::remove_file(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            IpcError::new(ErrorCode::NotFound, "Ese preset de usuario no existe.")
        } else {
            IpcError::new(ErrorCode::Io, e.to_string())
        }
    })
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EqResponse {
    pub sample_rate: u32,
    pub freqs: Vec<f64>,
    pub db: Vec<f64>,
}

/// Exact magnitude response of an equaliser setting at the engine rate
/// (the same biquad sections the audio thread runs).
#[tauri::command]
pub async fn effect_eq_response(
    app: AppHandle,
    params: BTreeMap<String, f64>,
    points: u32,
) -> IpcResult<EqResponse> {
    let rate = app.state::<AppState>().audio.rate();
    let n = points.clamp(16, 1024) as usize;
    let nyquist = f64::from(rate) * 0.5;
    let hi = 20_000f64.min(nyquist * 0.999);
    let freqs: Vec<f64> = (0..n)
        .map(|i| 20.0 * (hi / 20.0).powf(i as f64 / (n - 1) as f64))
        .collect();
    let values = ParamValues::from_map(EffectKind::ParametricEq, &params);
    let db = elyvon_audio::fx::eq_response_db(&values, rate, &freqs);
    Ok(EqResponse {
        sample_rate: rate,
        freqs,
        db,
    })
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectDetails {
    pub sample_rate: u32,
    /// Fixed delay the effect adds (compensated by the mixer).
    pub latency_frames: u32,
    pub latency_ms: f64,
    /// Tail after the input stops, in seconds (0: none, `null`: unbounded
    /// while frozen or at full feedback).
    pub tail_seconds: Option<f64>,
}

/// Latency and tail of an effect setting at the engine rate.
#[tauri::command]
pub async fn effect_details(app: AppHandle, slot: EffectSlot) -> IpcResult<EffectDetails> {
    let rate = app.state::<AppState>().audio.rate();
    let latency = elyvon_audio::fx::effect_latency(&slot) as u32;
    let get = |k: &str| slot.get(k).unwrap_or(0.0);
    let tail = match slot.kind {
        EffectKind::Reverb if get("freeze") >= 0.5 => None,
        EffectKind::Reverb => Some(get("decaySeconds") + get("predelayMs") / 1000.0),
        EffectKind::ConvolutionReverb => Some(get("lengthSeconds") + get("predelayMs") / 1000.0),
        EffectKind::Delay if get("feedback") >= 100.0 => None,
        EffectKind::Delay => {
            // Echoes fall below −60 dB after log(0.001)/log(feedback) repeats.
            let fb = (get("feedback") / 100.0 * 0.99).max(1e-3);
            let repeats = (0.001f64.ln() / fb.ln()).max(1.0);
            let time = if get("sync") >= 0.5 {
                2.0
            } else {
                get("timeLeftMs").max(get("timeRightMs")) / 1000.0
            };
            Some(repeats * time)
        }
        EffectKind::Chorus | EffectKind::Flanger => Some(0.05),
        _ => Some(0.0),
    };
    Ok(EffectDetails {
        sample_rate: rate,
        latency_frames: latency,
        latency_ms: f64::from(latency) * 1000.0 / f64::from(rate.max(1)),
        tail_seconds: tail,
    })
}

/// Selects the strip the spectrum analyser listens to (`null`: off).
#[tauri::command]
pub async fn analyzer_select(app: AppHandle, channel: Option<ChannelRef>) -> IpcResult<()> {
    let state = app.state::<AppState>();
    let slot = match channel {
        None => None,
        Some(ChannelRef::Master) => Some(elyvon_audio::plan::MASTER_SLOT),
        Some(ChannelRef::Track { id }) => {
            let s = state.session();
            s.project().tracks.iter().position(|t| t.id == id)
        }
        Some(ChannelRef::Aux { id }) => {
            let s = state.session();
            s.project()
                .mixer
                .auxes
                .iter()
                .position(|a| a.id == id)
                .map(elyvon_audio::plan::aux_slot)
        }
    };
    if channel.is_some() && slot.is_none() {
        return Err(IpcError::new(ErrorCode::NotFound, "Ese canal no existe."));
    }
    state.audio.select_analyzer(slot);
    Ok(())
}

/// The latest spectrum of the selected strip (`null` without an engine).
#[tauri::command]
pub async fn analyzer_spectrum(app: AppHandle) -> IpcResult<Option<Spectrum>> {
    Ok(app.state::<AppState>().audio.spectrum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_file_names_are_safe() {
        assert_eq!(file_stem("Voz: brillo / aire"), "Voz_ brillo _ aire");
        assert_eq!(file_stem("../../etc"), "______etc");
        assert!(file_stem("   ").is_empty());
        assert_eq!(
            kind_dir(Path::new("p"), EffectKind::ParametricEq),
            Path::new("p").join("parametricEq")
        );
    }
}
