//! Native effect processors (Phases 12–13): descriptions, parameters and
//! presets.
//!
//! Every effect is an [`EffectSlot`]: an [`EffectKind`] plus a fixed array
//! of `f64` parameter values described by the kind's [`ParamDef`] table.
//! Like plug-in parameters, all values are plain numbers in their own unit
//! (dB, Hz, ms, %, or the index of a choice). The table is **append-only**:
//! a parameter's index is its automation id ([`crate::automation::InsertParam::Effect`]),
//! so existing parameters are never reordered or removed.
//!
//! On disk a slot stores its values by key (`{"thresholdDb": -18, …}`):
//! unknown keys are ignored and missing keys take the default, so adding
//! parameters keeps old projects and presets loading unchanged.
//!
//! The DSP lives in `elyvon-audio::fx`; this module has no audio code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::TrackId;

/// Upper bound of parameters per effect (the 8-band equaliser uses 49).
pub const MAX_PARAMS: usize = 64;

/// Distinct tracks that may key external sidechains in one project.
pub const MAX_SIDECHAIN_SOURCES: usize = 16;

/// How a parameter is shown and interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ParamScale {
    /// Linear in the unit.
    Linear,
    /// Logarithmic (frequency, time, ratio).
    Log,
    /// An index into `choices`.
    Choice,
    /// 0 = off, 1 = on.
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ParamUnit {
    None,
    Db,
    Hz,
    Ms,
    Seconds,
    Percent,
    Ratio,
    Semitones,
    Cents,
    Degrees,
    Bits,
    Factor,
}

/// Static description of one effect parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamDef {
    /// Stable storage key (camelCase).
    pub key: &'static str,
    /// Spanish UI label.
    pub label: &'static str,
    pub unit: ParamUnit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: ParamScale,
    pub choices: &'static [&'static str],
    /// Changing it rebuilds the processor (impulse responses, oversampling):
    /// it can be edited but not automated.
    pub automatable: bool,
}

impl ParamDef {
    const fn new(
        key: &'static str,
        label: &'static str,
        unit: ParamUnit,
        min: f64,
        max: f64,
        default: f64,
        scale: ParamScale,
    ) -> Self {
        Self {
            key,
            label,
            unit,
            min,
            max,
            default,
            scale,
            choices: &[],
            automatable: true,
        }
    }

    const fn lin(
        key: &'static str,
        label: &'static str,
        unit: ParamUnit,
        min: f64,
        max: f64,
        default: f64,
    ) -> Self {
        Self::new(key, label, unit, min, max, default, ParamScale::Linear)
    }

    const fn log(
        key: &'static str,
        label: &'static str,
        unit: ParamUnit,
        min: f64,
        max: f64,
        default: f64,
    ) -> Self {
        Self::new(key, label, unit, min, max, default, ParamScale::Log)
    }

    const fn toggle(key: &'static str, label: &'static str, default: bool) -> Self {
        Self::new(
            key,
            label,
            ParamUnit::None,
            0.0,
            1.0,
            if default { 1.0 } else { 0.0 },
            ParamScale::Toggle,
        )
    }

    const fn choice(
        key: &'static str,
        label: &'static str,
        choices: &'static [&'static str],
        default: usize,
    ) -> Self {
        Self {
            key,
            label,
            unit: ParamUnit::None,
            min: 0.0,
            max: (choices.len() - 1) as f64,
            default: default as f64,
            scale: ParamScale::Choice,
            choices,
            automatable: true,
        }
    }

    const fn fixed(mut self) -> Self {
        self.automatable = false;
        self
    }

    /// Whether only whole values are valid.
    pub fn discrete(&self) -> bool {
        matches!(self.scale, ParamScale::Choice | ParamScale::Toggle)
    }

    /// Clamps (and for choices rounds) a value; non-finite values become the default.
    pub fn clamp(&self, v: f64) -> f64 {
        if !v.is_finite() {
            return self.default;
        }
        let v = v.clamp(self.min, self.max);
        if self.discrete() { v.round() } else { v }
    }
}

/// Musical note values for tempo-synchronised times and rates, in quarter
/// notes. Shared by delay and modulation effects.
pub const DIVISIONS: &[&str] = &[
    "1/64", "1/32", "1/16T", "1/16", "1/16.", "1/8T", "1/8", "1/8.", "1/4T", "1/4", "1/4.", "1/2T",
    "1/2", "1/2.", "1/1", "2/1", "4/1",
];

/// Length in quarter notes of `DIVISIONS[index]`.
pub fn division_quarters(index: usize) -> f64 {
    const Q: [f64; 17] = [
        1.0 / 16.0,
        1.0 / 8.0,
        1.0 / 6.0,
        1.0 / 4.0,
        3.0 / 8.0,
        1.0 / 3.0,
        1.0 / 2.0,
        3.0 / 4.0,
        2.0 / 3.0,
        1.0,
        1.5,
        4.0 / 3.0,
        2.0,
        3.0,
        4.0,
        8.0,
        16.0,
    ];
    Q[index.min(Q.len() - 1)]
}

/// Equaliser band shapes (index stored in `bNType`).
pub const EQ_TYPES: &[&str] = &[
    "Paso alto",
    "Estantería grave",
    "Campana",
    "Estantería aguda",
    "Paso bajo",
    "Paso banda",
    "Notch",
];

pub mod eq_type {
    pub const HIGH_PASS: usize = 0;
    pub const LOW_SHELF: usize = 1;
    pub const PEAK: usize = 2;
    pub const HIGH_SHELF: usize = 3;
    pub const LOW_PASS: usize = 4;
    pub const BAND_PASS: usize = 5;
    pub const NOTCH: usize = 6;
}

/// Filter slopes of high/low-pass bands, in dB/oct.
pub const EQ_SLOPES: &[&str] = &["12 dB/oct", "24 dB/oct", "36 dB/oct", "48 dB/oct"];

/// Bands of the parametric equaliser.
pub const EQ_BANDS: usize = 8;
/// Parameters per equaliser band: on, type, frequency, gain, Q, slope.
pub const EQ_BAND_PARAMS: usize = 6;

macro_rules! band {
    ($n:literal, $on:expr, $ty:expr, $freq:expr, $q:expr) => {
        [
            ParamDef::toggle(
                concat!("b", $n, "On"),
                concat!("Banda ", $n, " activa"),
                $on,
            ),
            ParamDef::choice(
                concat!("b", $n, "Type"),
                concat!("Banda ", $n, " tipo"),
                EQ_TYPES,
                $ty,
            ),
            ParamDef::log(
                concat!("b", $n, "Freq"),
                concat!("Banda ", $n, " frecuencia"),
                ParamUnit::Hz,
                20.0,
                20_000.0,
                $freq,
            ),
            ParamDef::lin(
                concat!("b", $n, "Gain"),
                concat!("Banda ", $n, " ganancia"),
                ParamUnit::Db,
                -24.0,
                24.0,
                0.0,
            ),
            ParamDef::log(
                concat!("b", $n, "Q"),
                concat!("Banda ", $n, " Q"),
                ParamUnit::None,
                0.1,
                18.0,
                $q,
            ),
            ParamDef::choice(
                concat!("b", $n, "Slope"),
                concat!("Banda ", $n, " pendiente"),
                EQ_SLOPES,
                1,
            ),
        ]
    };
}

/// What the equaliser processes: both channels, the mid (L+R) or the side
/// (L−R) signal (index stored in `processing`).
pub const EQ_PROCESSING: &[&str] = &["Estéreo", "Medio (M)", "Lateral (S)"];

/// Equaliser parameters after the bands: output gain, processing.
const EQ_EXTRA: usize = 2;

const fn flatten_eq(
    bands: [[ParamDef; EQ_BAND_PARAMS]; EQ_BANDS],
    extra: [ParamDef; EQ_EXTRA],
) -> [ParamDef; EQ_BANDS * EQ_BAND_PARAMS + EQ_EXTRA] {
    let mut out = [extra[0]; EQ_BANDS * EQ_BAND_PARAMS + EQ_EXTRA];
    let mut b = 0;
    while b < EQ_BANDS {
        let mut p = 0;
        while p < EQ_BAND_PARAMS {
            out[b * EQ_BAND_PARAMS + p] = bands[b][p];
            p += 1;
        }
        b += 1;
    }
    let mut e = 0;
    while e < EQ_EXTRA {
        out[EQ_BANDS * EQ_BAND_PARAMS + e] = extra[e];
        e += 1;
    }
    out
}

static EQ: [ParamDef; EQ_BANDS * EQ_BAND_PARAMS + EQ_EXTRA] = flatten_eq(
    [
        band!("1", false, eq_type::HIGH_PASS, 30.0, 0.707),
        band!("2", true, eq_type::LOW_SHELF, 100.0, 0.707),
        band!("3", true, eq_type::PEAK, 250.0, 1.0),
        band!("4", true, eq_type::PEAK, 800.0, 1.0),
        band!("5", true, eq_type::PEAK, 2_500.0, 1.0),
        band!("6", true, eq_type::PEAK, 6_000.0, 1.0),
        band!("7", true, eq_type::HIGH_SHELF, 10_000.0, 0.707),
        band!("8", false, eq_type::LOW_PASS, 18_000.0, 0.707),
    ],
    [
        ParamDef::lin("outputDb", "Salida", ParamUnit::Db, -24.0, 24.0, 0.0),
        ParamDef::choice("processing", "Procesado", EQ_PROCESSING, 0).fixed(),
    ],
);

const DETECTORS: &[&str] = &["Pico", "RMS"];

static COMPRESSOR: [ParamDef; 12] = [
    ParamDef::lin("thresholdDb", "Umbral", ParamUnit::Db, -60.0, 0.0, -18.0),
    ParamDef::log("ratio", "Ratio", ParamUnit::Ratio, 1.0, 20.0, 4.0),
    ParamDef::log("attackMs", "Ataque", ParamUnit::Ms, 0.05, 200.0, 10.0),
    ParamDef::log(
        "releaseMs",
        "Liberación",
        ParamUnit::Ms,
        5.0,
        2_000.0,
        120.0,
    ),
    ParamDef::lin("kneeDb", "Rodilla", ParamUnit::Db, 0.0, 24.0, 6.0),
    ParamDef::lin(
        "makeupDb",
        "Ganancia de compensación",
        ParamUnit::Db,
        0.0,
        24.0,
        0.0,
    ),
    ParamDef::toggle("autoMakeup", "Compensación automática", false),
    ParamDef::choice("detector", "Detector", DETECTORS, 0),
    ParamDef::log(
        "scHighPassHz",
        "Filtro sidechain (paso alto)",
        ParamUnit::Hz,
        20.0,
        500.0,
        20.0,
    ),
    ParamDef::toggle("external", "Sidechain externo", false),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 100.0),
    ParamDef::toggle("stereoLink", "Enlace estéreo", true),
];

const GATE_MODES: &[&str] = &["Puerta", "Expansor"];

static GATE: [ParamDef; 11] = [
    ParamDef::choice("mode", "Modo", GATE_MODES, 0),
    ParamDef::lin("thresholdDb", "Umbral", ParamUnit::Db, -90.0, 0.0, -40.0),
    ParamDef::lin("rangeDb", "Rango", ParamUnit::Db, -90.0, 0.0, -60.0),
    ParamDef::log(
        "ratio",
        "Ratio (expansor)",
        ParamUnit::Ratio,
        1.0,
        10.0,
        2.0,
    ),
    ParamDef::log("attackMs", "Ataque", ParamUnit::Ms, 0.01, 50.0, 0.5),
    ParamDef::lin("holdMs", "Mantener", ParamUnit::Ms, 0.0, 500.0, 20.0),
    ParamDef::log(
        "releaseMs",
        "Liberación",
        ParamUnit::Ms,
        5.0,
        2_000.0,
        100.0,
    ),
    ParamDef::lin("hysteresisDb", "Histéresis", ParamUnit::Db, 0.0, 12.0, 3.0),
    ParamDef::log(
        "scHighPassHz",
        "Filtro sidechain (paso alto)",
        ParamUnit::Hz,
        20.0,
        2_000.0,
        20.0,
    ),
    ParamDef::log(
        "scLowPassHz",
        "Filtro sidechain (paso bajo)",
        ParamUnit::Hz,
        500.0,
        20_000.0,
        20_000.0,
    ),
    ParamDef::toggle("external", "Sidechain externo", false),
];

static STEREO_TOOL: [ParamDef; 8] = [
    ParamDef::lin("gainDb", "Ganancia", ParamUnit::Db, -48.0, 24.0, 0.0),
    ParamDef::lin("balance", "Balance", ParamUnit::Percent, -100.0, 100.0, 0.0),
    ParamDef::lin(
        "width",
        "Anchura estéreo",
        ParamUnit::Percent,
        0.0,
        200.0,
        100.0,
    ),
    ParamDef::log(
        "monoBelowHz",
        "Mono por debajo de",
        ParamUnit::Hz,
        20.0,
        500.0,
        20.0,
    ),
    ParamDef::toggle("dcBlock", "Bloqueo de DC", false),
    ParamDef::toggle("invertLeft", "Invertir fase izquierda", false),
    ParamDef::toggle("invertRight", "Invertir fase derecha", false),
    ParamDef::toggle("swap", "Intercambiar L/R", false),
];

const NOISE_TYPES: &[&str] = &["Ruido blanco", "Ruido rosa", "Seno", "Barrido logarítmico"];
const NOISE_MODES: &[&str] = &["Reemplazar entrada", "Sumar a la entrada"];

static NOISE: [ParamDef; 5] = [
    ParamDef::choice("signal", "Señal", NOISE_TYPES, 1),
    ParamDef::lin("levelDb", "Nivel", ParamUnit::Db, -96.0, 0.0, -18.0),
    ParamDef::log(
        "freqHz",
        "Frecuencia (seno)",
        ParamUnit::Hz,
        20.0,
        20_000.0,
        1_000.0,
    ),
    ParamDef::choice("mode", "Modo", NOISE_MODES, 0),
    ParamDef::log(
        "sweepSeconds",
        "Duración del barrido",
        ParamUnit::Seconds,
        1.0,
        60.0,
        10.0,
    ),
];

const DELAY_MODES: &[&str] = &["Digital", "Cinta"];

static DELAY: [ParamDef; 16] = [
    ParamDef::toggle("sync", "Sincronizar con el tempo", true),
    ParamDef::choice("divisionLeft", "División izquierda", DIVISIONS, 9),
    ParamDef::choice("divisionRight", "División derecha", DIVISIONS, 7),
    ParamDef::log(
        "timeLeftMs",
        "Tiempo izquierdo",
        ParamUnit::Ms,
        1.0,
        4_000.0,
        375.0,
    ),
    ParamDef::log(
        "timeRightMs",
        "Tiempo derecho",
        ParamUnit::Ms,
        1.0,
        4_000.0,
        500.0,
    ),
    ParamDef::lin(
        "feedback",
        "Realimentación",
        ParamUnit::Percent,
        0.0,
        100.0,
        35.0,
    ),
    ParamDef::toggle("pingPong", "Ping-pong", false),
    ParamDef::log(
        "lowCutHz",
        "Corte de graves",
        ParamUnit::Hz,
        20.0,
        2_000.0,
        80.0,
    ),
    ParamDef::log(
        "highCutHz",
        "Corte de agudos",
        ParamUnit::Hz,
        1_000.0,
        20_000.0,
        12_000.0,
    ),
    ParamDef::choice("mode", "Carácter", DELAY_MODES, 0),
    ParamDef::lin(
        "wowFlutter",
        "Wow y flutter",
        ParamUnit::Percent,
        0.0,
        100.0,
        20.0,
    ),
    ParamDef::lin(
        "saturation",
        "Saturación",
        ParamUnit::Percent,
        0.0,
        100.0,
        25.0,
    ),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 30.0),
    ParamDef::lin("width", "Anchura", ParamUnit::Percent, 0.0, 100.0, 100.0),
    ParamDef::toggle("mono", "Entrada mono", false),
    ParamDef::lin("outputDb", "Salida", ParamUnit::Db, -24.0, 12.0, 0.0),
];

static REVERB: [ParamDef; 10] = [
    ParamDef::lin("predelayMs", "Predelay", ParamUnit::Ms, 0.0, 250.0, 20.0),
    ParamDef::log(
        "decaySeconds",
        "Decaimiento (RT60)",
        ParamUnit::Seconds,
        0.1,
        20.0,
        2.0,
    ),
    ParamDef::lin("size", "Tamaño", ParamUnit::Percent, 10.0, 100.0, 60.0),
    ParamDef::log(
        "dampingHz",
        "Amortiguación de agudos",
        ParamUnit::Hz,
        1_000.0,
        20_000.0,
        7_000.0,
    ),
    ParamDef::log(
        "lowCutHz",
        "Corte de graves",
        ParamUnit::Hz,
        20.0,
        1_000.0,
        100.0,
    ),
    ParamDef::lin(
        "diffusion",
        "Difusión",
        ParamUnit::Percent,
        0.0,
        100.0,
        70.0,
    ),
    ParamDef::lin(
        "modulation",
        "Modulación",
        ParamUnit::Percent,
        0.0,
        100.0,
        20.0,
    ),
    ParamDef::lin("width", "Anchura", ParamUnit::Percent, 0.0, 100.0, 100.0),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 25.0),
    ParamDef::toggle("freeze", "Congelar cola", false),
];

/// Built-in impulse-response spaces of the convolution reverb. They are
/// synthesised by `elyvon-audio` from a fixed seed: original, redistributable.
pub const IR_SPACES: &[&str] = &[
    "Habitación pequeña",
    "Cámara",
    "Sala de conciertos",
    "Placa",
    "Catedral",
];

static CONVOLUTION: [ParamDef; 7] = [
    ParamDef::choice("space", "Espacio (IR original)", IR_SPACES, 2).fixed(),
    ParamDef::log(
        "lengthSeconds",
        "Longitud de la IR",
        ParamUnit::Seconds,
        0.2,
        6.0,
        2.5,
    )
    .fixed(),
    ParamDef::lin("predelayMs", "Predelay", ParamUnit::Ms, 0.0, 200.0, 0.0),
    ParamDef::log(
        "lowCutHz",
        "Corte de graves",
        ParamUnit::Hz,
        20.0,
        1_000.0,
        20.0,
    ),
    ParamDef::log(
        "highCutHz",
        "Corte de agudos",
        ParamUnit::Hz,
        1_000.0,
        20_000.0,
        20_000.0,
    ),
    ParamDef::lin("width", "Anchura", ParamUnit::Percent, 0.0, 100.0, 100.0),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 25.0),
];

const VOICES: &[&str] = &["1", "2", "3", "4"];

static CHORUS: [ParamDef; 9] = [
    ParamDef::log("rateHz", "Velocidad", ParamUnit::Hz, 0.02, 10.0, 0.8),
    ParamDef::toggle("sync", "Sincronizar con el tempo", false),
    ParamDef::choice("division", "División", DIVISIONS, 14),
    ParamDef::lin("depthMs", "Profundidad", ParamUnit::Ms, 0.0, 10.0, 3.0),
    ParamDef::lin("delayMs", "Retardo base", ParamUnit::Ms, 5.0, 40.0, 15.0),
    ParamDef::choice("voices", "Voces", VOICES, 1),
    ParamDef::lin(
        "feedback",
        "Realimentación",
        ParamUnit::Percent,
        0.0,
        70.0,
        0.0,
    ),
    ParamDef::lin(
        "spread",
        "Separación estéreo",
        ParamUnit::Percent,
        0.0,
        100.0,
        100.0,
    ),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 50.0),
];

static FLANGER: [ParamDef; 8] = [
    ParamDef::log("rateHz", "Velocidad", ParamUnit::Hz, 0.02, 10.0, 0.25),
    ParamDef::toggle("sync", "Sincronizar con el tempo", false),
    ParamDef::choice("division", "División", DIVISIONS, 15),
    ParamDef::lin("depthMs", "Profundidad", ParamUnit::Ms, 0.0, 5.0, 2.0),
    ParamDef::lin("delayMs", "Retardo base", ParamUnit::Ms, 0.1, 10.0, 1.0),
    ParamDef::lin(
        "feedback",
        "Realimentación",
        ParamUnit::Percent,
        -95.0,
        95.0,
        50.0,
    ),
    ParamDef::lin(
        "spread",
        "Separación estéreo",
        ParamUnit::Percent,
        0.0,
        100.0,
        50.0,
    ),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 50.0),
];

const STAGES: &[&str] = &["2", "4", "6", "8", "10", "12"];

static PHASER: [ParamDef; 9] = [
    ParamDef::log("rateHz", "Velocidad", ParamUnit::Hz, 0.02, 10.0, 0.4),
    ParamDef::toggle("sync", "Sincronizar con el tempo", false),
    ParamDef::choice("division", "División", DIVISIONS, 14),
    ParamDef::choice("stages", "Etapas", STAGES, 1),
    ParamDef::log(
        "centerHz",
        "Frecuencia central",
        ParamUnit::Hz,
        100.0,
        5_000.0,
        800.0,
    ),
    ParamDef::lin("depth", "Profundidad", ParamUnit::Percent, 0.0, 100.0, 70.0),
    ParamDef::lin(
        "feedback",
        "Realimentación",
        ParamUnit::Percent,
        -95.0,
        95.0,
        40.0,
    ),
    ParamDef::lin(
        "spread",
        "Separación estéreo",
        ParamUnit::Percent,
        0.0,
        100.0,
        50.0,
    ),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 50.0),
];

const SHAPES: &[&str] = &["Seno", "Triángulo", "Cuadrada suave"];

static TREMOLO: [ParamDef; 6] = [
    ParamDef::log("rateHz", "Velocidad", ParamUnit::Hz, 0.05, 20.0, 4.0),
    ParamDef::toggle("sync", "Sincronizar con el tempo", true),
    ParamDef::choice("division", "División", DIVISIONS, 6),
    ParamDef::lin("depth", "Profundidad", ParamUnit::Percent, 0.0, 100.0, 50.0),
    ParamDef::choice("shape", "Forma", SHAPES, 0),
    ParamDef::lin(
        "stereoPhase",
        "Desfase estéreo (autopan)",
        ParamUnit::Degrees,
        0.0,
        180.0,
        0.0,
    ),
];

const ROTOR_SPEEDS: &[&str] = &["Lento", "Rápido", "Freno"];

static ROTARY: [ParamDef; 7] = [
    ParamDef::choice("speed", "Velocidad", ROTOR_SPEEDS, 0),
    ParamDef::log("slowHz", "Rotación lenta", ParamUnit::Hz, 0.3, 2.0, 0.8),
    ParamDef::log("fastHz", "Rotación rápida", ParamUnit::Hz, 3.0, 9.0, 6.7),
    ParamDef::log(
        "accelSeconds",
        "Aceleración",
        ParamUnit::Seconds,
        0.1,
        5.0,
        1.2,
    ),
    ParamDef::lin("drive", "Saturación", ParamUnit::Percent, 0.0, 100.0, 10.0),
    ParamDef::lin(
        "spread",
        "Separación de micrófonos",
        ParamUnit::Percent,
        0.0,
        100.0,
        80.0,
    ),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 100.0),
];

const CURVES: &[&str] = &["Suave (tanh)", "Dura", "Válvula (asimétrica)", "Plegado"];
const OVERSAMPLING: &[&str] = &["1×", "2×", "4×"];

static DISTORTION: [ParamDef; 6] = [
    ParamDef::choice("curve", "Curva", CURVES, 0),
    ParamDef::lin("driveDb", "Drive", ParamUnit::Db, 0.0, 48.0, 12.0),
    ParamDef::log("toneHz", "Tono", ParamUnit::Hz, 500.0, 20_000.0, 8_000.0),
    ParamDef::lin("outputDb", "Salida", ParamUnit::Db, -36.0, 12.0, -6.0),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 100.0),
    ParamDef::choice("oversampling", "Sobremuestreo", OVERSAMPLING, 1).fixed(),
];

static BITCRUSHER: [ParamDef; 5] = [
    ParamDef::lin("bits", "Resolución", ParamUnit::Bits, 1.0, 24.0, 8.0),
    ParamDef::log(
        "downsample",
        "Reducción de frecuencia",
        ParamUnit::Factor,
        1.0,
        64.0,
        4.0,
    ),
    ParamDef::lin("driveDb", "Drive", ParamUnit::Db, 0.0, 24.0, 0.0),
    ParamDef::toggle("dither", "Dither", false),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 100.0),
];

static PITCH: [ParamDef; 4] = [
    ParamDef::lin(
        "semitones",
        "Semitonos",
        ParamUnit::Semitones,
        -24.0,
        24.0,
        0.0,
    ),
    ParamDef::lin("cents", "Centésimas", ParamUnit::Cents, -100.0, 100.0, 0.0),
    ParamDef::lin("windowMs", "Ventana", ParamUnit::Ms, 10.0, 100.0, 40.0),
    ParamDef::lin("mix", "Mezcla", ParamUnit::Percent, 0.0, 100.0, 100.0),
];

/// The native effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EffectKind {
    ParametricEq,
    Compressor,
    Gate,
    StereoTool,
    TestGenerator,
    Delay,
    Reverb,
    ConvolutionReverb,
    Chorus,
    Flanger,
    Phaser,
    Tremolo,
    Rotary,
    Distortion,
    Bitcrusher,
    PitchShifter,
}

/// Every effect, in menu order.
pub const ALL_EFFECTS: &[EffectKind] = &[
    EffectKind::ParametricEq,
    EffectKind::Compressor,
    EffectKind::Gate,
    EffectKind::StereoTool,
    EffectKind::TestGenerator,
    EffectKind::Delay,
    EffectKind::Reverb,
    EffectKind::ConvolutionReverb,
    EffectKind::Chorus,
    EffectKind::Flanger,
    EffectKind::Phaser,
    EffectKind::Tremolo,
    EffectKind::Rotary,
    EffectKind::Distortion,
    EffectKind::Bitcrusher,
    EffectKind::PitchShifter,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EffectCategory {
    Eq,
    Dynamics,
    Utility,
    Delay,
    Reverb,
    Modulation,
    Distortion,
    Pitch,
}

impl EffectKind {
    pub fn params(self) -> &'static [ParamDef] {
        match self {
            Self::ParametricEq => &EQ,
            Self::Compressor => &COMPRESSOR,
            Self::Gate => &GATE,
            Self::StereoTool => &STEREO_TOOL,
            Self::TestGenerator => &NOISE,
            Self::Delay => &DELAY,
            Self::Reverb => &REVERB,
            Self::ConvolutionReverb => &CONVOLUTION,
            Self::Chorus => &CHORUS,
            Self::Flanger => &FLANGER,
            Self::Phaser => &PHASER,
            Self::Tremolo => &TREMOLO,
            Self::Rotary => &ROTARY,
            Self::Distortion => &DISTORTION,
            Self::Bitcrusher => &BITCRUSHER,
            Self::PitchShifter => &PITCH,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ParametricEq => "Ecualizador paramétrico",
            Self::Compressor => "Compresor",
            Self::Gate => "Puerta / expansor",
            Self::StereoTool => "Utilidad e imagen estéreo",
            Self::TestGenerator => "Generador de prueba",
            Self::Delay => "Delay estéreo",
            Self::Reverb => "Reverb algorítmica",
            Self::ConvolutionReverb => "Reverb de convolución",
            Self::Chorus => "Chorus",
            Self::Flanger => "Flanger",
            Self::Phaser => "Phaser",
            Self::Tremolo => "Trémolo / autopan",
            Self::Rotary => "Altavoz rotatorio",
            Self::Distortion => "Distorsión / saturación",
            Self::Bitcrusher => "Bitcrusher",
            Self::PitchShifter => "Cambio de tono",
        }
    }

    /// Short slot label.
    pub fn short(self) -> &'static str {
        match self {
            Self::ParametricEq => "EQ",
            Self::Compressor => "Comp",
            Self::Gate => "Gate",
            Self::StereoTool => "Util",
            Self::TestGenerator => "Gen",
            Self::Delay => "Delay",
            Self::Reverb => "Verb",
            Self::ConvolutionReverb => "Conv",
            Self::Chorus => "Chorus",
            Self::Flanger => "Flanger",
            Self::Phaser => "Phaser",
            Self::Tremolo => "Trem",
            Self::Rotary => "Rotor",
            Self::Distortion => "Dist",
            Self::Bitcrusher => "Crush",
            Self::PitchShifter => "Pitch",
        }
    }

    pub fn category(self) -> EffectCategory {
        match self {
            Self::ParametricEq => EffectCategory::Eq,
            Self::Compressor | Self::Gate => EffectCategory::Dynamics,
            Self::StereoTool | Self::TestGenerator => EffectCategory::Utility,
            Self::Delay => EffectCategory::Delay,
            Self::Reverb | Self::ConvolutionReverb => EffectCategory::Reverb,
            Self::Chorus | Self::Flanger | Self::Phaser | Self::Tremolo | Self::Rotary => {
                EffectCategory::Modulation
            }
            Self::Distortion | Self::Bitcrusher => EffectCategory::Distortion,
            Self::PitchShifter => EffectCategory::Pitch,
        }
    }

    /// Accepts a key signal from another track.
    pub fn has_sidechain(self) -> bool {
        matches!(self, Self::Compressor | Self::Gate)
    }

    /// Index of the parameter stored under `key`.
    pub fn index_of(self, key: &str) -> Option<usize> {
        self.params().iter().position(|p| p.key == key)
    }

    /// Default values of every parameter.
    pub fn defaults(self) -> ParamValues {
        let mut v = [0.0; MAX_PARAMS];
        for (slot, def) in v.iter_mut().zip(self.params()) {
            *slot = def.default;
        }
        ParamValues(v)
    }
}

/// Parameter values of one effect, by index into [`EffectKind::params`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamValues(pub [f64; MAX_PARAMS]);

impl ParamValues {
    pub fn get(&self, index: usize) -> f64 {
        self.0.get(index).copied().unwrap_or(0.0)
    }

    /// Keyed form used on disk, in presets and over IPC.
    pub fn to_map(&self, kind: EffectKind) -> BTreeMap<String, f64> {
        kind.params()
            .iter()
            .enumerate()
            .map(|(i, d)| (d.key.to_string(), self.0[i]))
            .collect()
    }

    /// Reads keyed values: missing keys take the default, unknown keys are
    /// ignored and values are clamped into range.
    pub fn from_map(kind: EffectKind, map: &BTreeMap<String, f64>) -> Self {
        let mut v = kind.defaults();
        for (i, d) in kind.params().iter().enumerate() {
            if let Some(x) = map.get(d.key) {
                v.0[i] = d.clamp(*x);
            }
        }
        v
    }
}

/// A native effect in an insert slot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(from = "EffectSlotWire", into = "EffectSlotWire")]
#[ts(export)]
pub struct EffectSlot {
    pub kind: EffectKind,
    #[ts(as = "BTreeMap<String, f64>")]
    pub params: ParamValues,
    /// Track whose post-insert, pre-fader signal keys the detector
    /// (compressor and gate with `external` on).
    #[ts(optional)]
    pub sidechain: Option<TrackId>,
    /// The other setting of the A/B comparison, when one was stored.
    #[ts(as = "Option<BTreeMap<String, f64>>", optional)]
    pub alternate: Option<ParamValues>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectSlotWire {
    kind: EffectKind,
    #[serde(default)]
    params: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sidechain: Option<TrackId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alternate: Option<BTreeMap<String, f64>>,
}

impl From<EffectSlotWire> for EffectSlot {
    fn from(w: EffectSlotWire) -> Self {
        Self {
            kind: w.kind,
            params: ParamValues::from_map(w.kind, &w.params),
            sidechain: w.sidechain,
            alternate: w.alternate.map(|m| ParamValues::from_map(w.kind, &m)),
        }
    }
}

impl From<EffectSlot> for EffectSlotWire {
    fn from(s: EffectSlot) -> Self {
        Self {
            kind: s.kind,
            params: s.params.to_map(s.kind),
            sidechain: s.sidechain,
            alternate: s.alternate.map(|v| v.to_map(s.kind)),
        }
    }
}

impl EffectSlot {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            params: kind.defaults(),
            sidechain: None,
            alternate: None,
        }
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.kind.index_of(key).map(|i| self.params.0[i])
    }

    /// Sets a parameter by key (clamped). Unknown keys are an error.
    pub fn set(&mut self, key: &str, value: f64) -> Result<(), String> {
        let i = self
            .kind
            .index_of(key)
            .ok_or_else(|| format!("{} has no parameter «{key}»", self.kind.label()))?;
        self.params.0[i] = self.kind.params()[i].clamp(value);
        Ok(())
    }

    /// Swaps the current setting with the stored A/B alternative (storing
    /// a copy first when there is none).
    pub fn toggle_ab(&mut self) {
        let other = self.alternate.unwrap_or(self.params);
        self.alternate = Some(self.params);
        self.params = other;
    }

    pub fn validate(&self) -> Result<(), String> {
        let check = |v: &ParamValues| -> Result<(), String> {
            for (i, d) in self.kind.params().iter().enumerate() {
                let x = v.0[i];
                if !x.is_finite() || x < d.min || x > d.max {
                    return Err(format!(
                        "{}: {} must be between {} and {}",
                        self.kind.label(),
                        d.label,
                        d.min,
                        d.max
                    ));
                }
                if d.discrete() && x.fract() != 0.0 {
                    return Err(format!(
                        "{}: {} must be a whole value",
                        self.kind.label(),
                        d.label
                    ));
                }
            }
            Ok(())
        };
        check(&self.params)?;
        if let Some(a) = &self.alternate {
            check(a)?;
        }
        if self.sidechain.is_some() && !self.kind.has_sidechain() {
            return Err(format!("{} has no sidechain input", self.kind.label()));
        }
        Ok(())
    }
}

// ---- presets ----

/// Format tag of preset files.
pub const PRESET_FORMAT: &str = "elyvon-effect-preset";
/// Current preset file version. Readers accept every version up to this one.
pub const PRESET_VERSION: u32 = 1;
/// File extension of user presets.
pub const PRESET_EXTENSION: &str = "elyvonpreset";

/// A preset on disk or in the factory set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectPreset {
    pub format: String,
    pub version: u32,
    pub kind: EffectKind,
    pub name: String,
    pub params: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresetError {
    #[error("not an Elyvon effect preset")]
    Format,
    #[error(
        "preset version {found} is newer than this version of Elyvon Studio supports ({supported})"
    )]
    TooNew { found: u32, supported: u32 },
    #[error("preset for {found} cannot be loaded into {expected}")]
    WrongKind { found: String, expected: String },
    #[error("invalid preset: {0}")]
    Invalid(String),
}

impl EffectPreset {
    pub fn new(kind: EffectKind, name: impl Into<String>, params: &ParamValues) -> Self {
        Self {
            format: PRESET_FORMAT.into(),
            version: PRESET_VERSION,
            kind,
            name: name.into(),
            params: params.to_map(kind),
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec_pretty(self).unwrap_or_default();
        out.push(b'\n');
        out
    }

    /// Parses and checks a preset file.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PresetError> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| PresetError::Invalid(e.to_string()))?;
        if value.get("format").and_then(|f| f.as_str()) != Some(PRESET_FORMAT) {
            return Err(PresetError::Format);
        }
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or(PresetError::Format)?;
        let version = u32::try_from(version).unwrap_or(u32::MAX);
        if version > PRESET_VERSION {
            return Err(PresetError::TooNew {
                found: version,
                supported: PRESET_VERSION,
            });
        }
        let preset: Self =
            serde_json::from_value(value).map_err(|e| PresetError::Invalid(e.to_string()))?;
        if preset.name.trim().is_empty() || preset.name.len() > 128 {
            return Err(PresetError::Invalid(
                "the name must be 1–128 characters".into(),
            ));
        }
        if preset.params.values().any(|v| !v.is_finite()) {
            return Err(PresetError::Invalid("values must be finite".into()));
        }
        Ok(preset)
    }

    /// Values for an effect of `kind` (missing keys default, out-of-range clamp).
    pub fn values_for(&self, kind: EffectKind) -> Result<ParamValues, PresetError> {
        if self.kind != kind {
            return Err(PresetError::WrongKind {
                found: self.kind.label().into(),
                expected: kind.label().into(),
            });
        }
        Ok(ParamValues::from_map(kind, &self.params))
    }
}

/// Factory presets: original settings written for Elyvon Studio.
pub fn factory_presets(kind: EffectKind) -> Vec<EffectPreset> {
    let make = |name: &str, values: &[(&str, f64)]| {
        let mut slot = EffectSlot::new(kind);
        for (k, v) in values {
            // Keys are checked by the factory preset test.
            let _ = slot.set(k, *v);
        }
        EffectPreset::new(kind, name, &slot.params)
    };
    let list: Vec<(&str, Vec<(&str, f64)>)> = match kind {
        EffectKind::ParametricEq => vec![
            ("Neutro", vec![]),
            (
                "Voz: presencia y aire",
                vec![
                    ("b1On", 1.0),
                    ("b1Freq", 90.0),
                    ("b3Gain", -2.0),
                    ("b3Freq", 300.0),
                    ("b5Gain", 2.5),
                    ("b5Freq", 3_200.0),
                    ("b7Gain", 3.0),
                    ("b7Freq", 11_000.0),
                ],
            ),
            (
                "Bombo: pegada",
                vec![
                    ("b1On", 1.0),
                    ("b1Freq", 35.0),
                    ("b2Gain", 3.0),
                    ("b2Freq", 60.0),
                    ("b3Gain", -4.0),
                    ("b3Freq", 350.0),
                    ("b3Q", 1.4),
                    ("b5Gain", 3.0),
                    ("b5Freq", 3_500.0),
                ],
            ),
            (
                "Limpieza de graves",
                vec![("b1On", 1.0), ("b1Freq", 120.0), ("b1Slope", 2.0)],
            ),
            (
                "Teléfono",
                vec![
                    ("b1On", 1.0),
                    ("b1Freq", 400.0),
                    ("b1Slope", 3.0),
                    ("b8On", 1.0),
                    ("b8Freq", 3_000.0),
                    ("b8Slope", 3.0),
                    ("b4Gain", 6.0),
                    ("b4Freq", 1_500.0),
                ],
            ),
        ],
        EffectKind::Compressor => vec![
            (
                "Suave 2:1",
                vec![
                    ("thresholdDb", -20.0),
                    ("ratio", 2.0),
                    ("attackMs", 20.0),
                    ("releaseMs", 200.0),
                    ("kneeDb", 10.0),
                    ("autoMakeup", 1.0),
                ],
            ),
            (
                "Voz al frente",
                vec![
                    ("thresholdDb", -24.0),
                    ("ratio", 4.0),
                    ("attackMs", 5.0),
                    ("releaseMs", 80.0),
                    ("kneeDb", 6.0),
                    ("makeupDb", 6.0),
                    ("detector", 1.0),
                ],
            ),
            (
                "Bus de batería en paralelo",
                vec![
                    ("thresholdDb", -30.0),
                    ("ratio", 8.0),
                    ("attackMs", 1.0),
                    ("releaseMs", 60.0),
                    ("kneeDb", 3.0),
                    ("makeupDb", 10.0),
                    ("mix", 40.0),
                ],
            ),
            (
                "Ducking por sidechain",
                vec![
                    ("thresholdDb", -30.0),
                    ("ratio", 10.0),
                    ("attackMs", 2.0),
                    ("releaseMs", 150.0),
                    ("external", 1.0),
                    ("scHighPassHz", 40.0),
                ],
            ),
        ],
        EffectKind::Gate => vec![
            (
                "Puerta de tom",
                vec![
                    ("thresholdDb", -35.0),
                    ("rangeDb", -40.0),
                    ("attackMs", 0.2),
                    ("holdMs", 60.0),
                    ("releaseMs", 150.0),
                    ("scHighPassHz", 80.0),
                ],
            ),
            (
                "Expansor de ruido suave",
                vec![
                    ("mode", 1.0),
                    ("thresholdDb", -50.0),
                    ("ratio", 2.0),
                    ("rangeDb", -20.0),
                    ("releaseMs", 250.0),
                ],
            ),
        ],
        EffectKind::StereoTool => vec![
            ("Graves en mono (120 Hz)", vec![("monoBelowHz", 120.0)]),
            ("Ensanchar 140 %", vec![("width", 140.0)]),
            ("Mono total", vec![("width", 0.0)]),
            (
                "Reparar DC y polaridad",
                vec![("dcBlock", 1.0), ("invertLeft", 1.0), ("invertRight", 1.0)],
            ),
        ],
        EffectKind::TestGenerator => vec![
            ("Ruido rosa −18 dBFS", vec![]),
            (
                "Seno 1 kHz −12 dBFS",
                vec![("signal", 2.0), ("levelDb", -12.0)],
            ),
            (
                "Barrido 20 Hz – 20 kHz",
                vec![("signal", 3.0), ("levelDb", -12.0), ("sweepSeconds", 10.0)],
            ),
        ],
        EffectKind::Delay => vec![
            (
                "Corchea con puntillo",
                vec![
                    ("divisionLeft", 7.0),
                    ("divisionRight", 7.0),
                    ("feedback", 40.0),
                ],
            ),
            (
                "Slapback",
                vec![
                    ("sync", 0.0),
                    ("timeLeftMs", 95.0),
                    ("timeRightMs", 110.0),
                    ("feedback", 10.0),
                    ("mix", 25.0),
                    ("mode", 1.0),
                    ("highCutHz", 6_000.0),
                ],
            ),
            (
                "Cinta gastada",
                vec![
                    ("mode", 1.0),
                    ("wowFlutter", 60.0),
                    ("saturation", 60.0),
                    ("feedback", 55.0),
                    ("highCutHz", 4_500.0),
                    ("lowCutHz", 200.0),
                ],
            ),
            (
                "Ping-pong a negras",
                vec![
                    ("divisionLeft", 9.0),
                    ("divisionRight", 9.0),
                    ("pingPong", 1.0),
                    ("feedback", 45.0),
                ],
            ),
        ],
        EffectKind::Reverb => vec![
            (
                "Habitación",
                vec![
                    ("decaySeconds", 0.6),
                    ("size", 35.0),
                    ("predelayMs", 5.0),
                    ("mix", 20.0),
                ],
            ),
            (
                "Sala grande",
                vec![
                    ("decaySeconds", 2.8),
                    ("size", 85.0),
                    ("predelayMs", 30.0),
                    ("dampingHz", 6_000.0),
                ],
            ),
            (
                "Placa brillante",
                vec![
                    ("decaySeconds", 1.8),
                    ("size", 55.0),
                    ("diffusion", 95.0),
                    ("dampingHz", 14_000.0),
                    ("predelayMs", 10.0),
                ],
            ),
            (
                "Ambiente infinito",
                vec![
                    ("decaySeconds", 20.0),
                    ("size", 100.0),
                    ("modulation", 60.0),
                    ("mix", 40.0),
                ],
            ),
        ],
        EffectKind::ConvolutionReverb => vec![
            ("Cámara corta", vec![("space", 1.0), ("lengthSeconds", 1.2)]),
            (
                "Sala de conciertos",
                vec![("space", 2.0), ("lengthSeconds", 3.0), ("predelayMs", 15.0)],
            ),
            (
                "Placa vocal",
                vec![("space", 3.0), ("lengthSeconds", 2.0), ("lowCutHz", 150.0)],
            ),
            (
                "Catedral",
                vec![("space", 4.0), ("lengthSeconds", 6.0), ("mix", 30.0)],
            ),
        ],
        EffectKind::Chorus => vec![
            ("Chorus clásico", vec![]),
            (
                "Doblaje ancho",
                vec![
                    ("voices", 3.0),
                    ("depthMs", 1.5),
                    ("delayMs", 22.0),
                    ("rateHz", 0.3),
                ],
            ),
            (
                "Chorus a negras",
                vec![("sync", 1.0), ("division", 9.0), ("depthMs", 4.0)],
            ),
        ],
        EffectKind::Flanger => vec![
            (
                "Avión",
                vec![("rateHz", 0.1), ("feedback", 80.0), ("depthMs", 3.0)],
            ),
            (
                "Flanger a compás",
                vec![("sync", 1.0), ("division", 14.0), ("feedback", 60.0)],
            ),
        ],
        EffectKind::Phaser => vec![
            ("Phaser lento", vec![("rateHz", 0.2), ("stages", 2.0)]),
            (
                "Phaser a corcheas",
                vec![
                    ("sync", 1.0),
                    ("division", 6.0),
                    ("stages", 3.0),
                    ("feedback", 60.0),
                ],
            ),
        ],
        EffectKind::Tremolo => vec![
            ("Trémolo a corcheas", vec![]),
            (
                "Autopan a negras",
                vec![("division", 9.0), ("stereoPhase", 180.0), ("depth", 80.0)],
            ),
            (
                "Pulsos cuadrados",
                vec![("shape", 2.0), ("division", 3.0), ("depth", 100.0)],
            ),
        ],
        EffectKind::Rotary => vec![
            ("Coral (lento)", vec![]),
            ("Trémolo (rápido)", vec![("speed", 1.0)]),
        ],
        EffectKind::Distortion => vec![
            (
                "Calidez de cinta",
                vec![("driveDb", 6.0), ("outputDb", -3.0), ("curve", 0.0)],
            ),
            (
                "Válvula sucia",
                vec![
                    ("curve", 2.0),
                    ("driveDb", 24.0),
                    ("toneHz", 5_000.0),
                    ("outputDb", -14.0),
                ],
            ),
            (
                "Fuzz plegado",
                vec![
                    ("curve", 3.0),
                    ("driveDb", 30.0),
                    ("outputDb", -18.0),
                    ("oversampling", 2.0),
                ],
            ),
        ],
        EffectKind::Bitcrusher => vec![
            ("8 bits", vec![]),
            (
                "Videojuego antiguo",
                vec![("bits", 4.0), ("downsample", 8.0)],
            ),
            (
                "Textura sutil",
                vec![("bits", 12.0), ("downsample", 2.0), ("mix", 40.0)],
            ),
        ],
        EffectKind::PitchShifter => vec![
            ("Octava arriba", vec![("semitones", 12.0)]),
            ("Octava abajo", vec![("semitones", -12.0)]),
            (
                "Quinta para armonías",
                vec![("semitones", 7.0), ("mix", 50.0)],
            ),
            (
                "Desafinado (doblaje)",
                vec![("cents", 12.0), ("mix", 50.0), ("windowMs", 30.0)],
            ),
        ],
    };
    list.into_iter().map(|(n, v)| make(n, &v)).collect()
}

/// Validates the sidechain sources of every effect: they must be tracks of
/// the project, not the track itself, and track-to-track keys must not form
/// a cycle (the source is processed before the destination).
pub(crate) fn check_sidechains(
    project: &crate::model::Project,
) -> Result<(), crate::ValidationError> {
    use crate::ValidationError;
    use crate::mixer::Processor;
    let ids: std::collections::HashSet<TrackId> = project.tracks.iter().map(|t| t.id).collect();
    let mut edges: Vec<(TrackId, TrackId)> = Vec::new();
    let mut external_sources: Vec<TrackId> = Vec::new();
    let mut check = |owner: Option<TrackId>,
                     slots: &[crate::mixer::InsertSlot]|
     -> Result<(), ValidationError> {
        for s in slots {
            if let Processor::Effect(e) = &s.processor
                && let Some(src) = e.sidechain
            {
                if !ids.contains(&src) {
                    return Err(ValidationError::new(
                        "mixer",
                        "a sidechain refers to a missing track",
                    ));
                }
                if owner == Some(src) {
                    return Err(ValidationError::new(
                        "mixer",
                        "a track cannot key its own sidechain",
                    ));
                }
                match owner {
                    Some(o) => edges.push((src, o)),
                    None => external_sources.push(src),
                }
            }
        }
        Ok(())
    };
    for t in &project.tracks {
        check(Some(t.id), &t.strip.inserts)?;
    }
    for a in &project.mixer.auxes {
        check(None, &a.strip.inserts)?;
    }
    check(None, &project.mixer.master.inserts)?;
    let mut sources: Vec<TrackId> = edges.iter().map(|e| e.0).collect();
    sources.extend(external_sources.iter().copied());
    sources.sort_unstable();
    sources.dedup();
    if sources.len() > MAX_SIDECHAIN_SOURCES {
        return Err(ValidationError::new(
            "mixer",
            format!("at most {MAX_SIDECHAIN_SOURCES} tracks can key sidechains"),
        ));
    }
    if !edges.is_empty() && sidechain_order(project).is_none() {
        return Err(ValidationError::new(
            "mixer",
            "sidechain cycle: two tracks key each other",
        ));
    }
    Ok(())
}

/// Track processing order that puts every sidechain source before the
/// tracks it keys (stable: project order otherwise). `None` on a cycle.
pub fn sidechain_order(project: &crate::model::Project) -> Option<Vec<usize>> {
    use crate::mixer::Processor;
    let n = project.tracks.len();
    let index: std::collections::HashMap<TrackId, usize> = project
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id, i))
        .collect();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, t) in project.tracks.iter().enumerate() {
        for s in &t.strip.inserts {
            if let Processor::Effect(e) = &s.processor
                && let Some(src) = e.sidechain.and_then(|id| index.get(&id).copied())
                && src != i
            {
                deps[i].push(src);
            }
        }
    }
    let mut order = Vec::with_capacity(n);
    let mut state = vec![0u8; n]; // 0 new, 1 visiting, 2 done
    fn visit(i: usize, deps: &[Vec<usize>], state: &mut [u8], order: &mut Vec<usize>) -> bool {
        match state[i] {
            2 => return true,
            1 => return false,
            _ => {}
        }
        state[i] = 1;
        for &d in &deps[i] {
            if !visit(d, deps, state, order) {
                return false;
            }
        }
        state[i] = 2;
        order.push(i);
        true
    }
    for i in 0..n {
        if !visit(i, &deps, &mut state, &mut order) {
            return None;
        }
    }
    Some(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_consistent() {
        for kind in ALL_EFFECTS {
            let defs = kind.params();
            assert!(defs.len() <= MAX_PARAMS, "{kind:?}");
            let mut keys = std::collections::HashSet::new();
            for d in defs {
                assert!(keys.insert(d.key), "{kind:?} duplicates {}", d.key);
                assert!(d.min < d.max, "{kind:?} {}", d.key);
                assert!((d.min..=d.max).contains(&d.default), "{kind:?} {}", d.key);
                if d.scale == ParamScale::Log {
                    assert!(d.min > 0.0, "{kind:?} {}", d.key);
                }
                if d.scale == ParamScale::Choice {
                    assert_eq!(d.max as usize + 1, d.choices.len());
                }
            }
            EffectSlot::new(*kind)
                .validate()
                .expect("defaults are valid");
        }
        assert_eq!(EQ.len(), 50);
        assert_eq!(EffectKind::ParametricEq.index_of("outputDb"), Some(48));
        assert_eq!(EffectKind::ParametricEq.index_of("b3Freq"), Some(2 * 6 + 2));
        assert_eq!(DIVISIONS.len(), 17);
        assert!((division_quarters(7) - 0.75).abs() < 1e-12);
    }

    #[test]
    fn factory_presets_use_known_keys_and_valid_values() {
        for kind in ALL_EFFECTS {
            let presets = factory_presets(*kind);
            assert!(!presets.is_empty(), "{kind:?}");
            let mut names = std::collections::HashSet::new();
            for p in presets {
                assert!(names.insert(p.name.clone()));
                let mut slot = EffectSlot::new(*kind);
                slot.params = p.values_for(*kind).expect("same kind");
                slot.validate().expect("preset values are valid");
            }
        }
        // A misspelt key would be dropped silently by `set`; check the source list.
        let mut eq = EffectSlot::new(EffectKind::ParametricEq);
        assert!(eq.set("b9Freq", 100.0).is_err());
    }

    #[test]
    fn slots_round_trip_by_key_and_tolerate_schema_growth() {
        let mut slot = EffectSlot::new(EffectKind::Compressor);
        slot.set("thresholdDb", -30.0).expect("key");
        slot.toggle_ab();
        slot.set("ratio", 8.0).expect("key");
        let json = serde_json::to_string(&slot).expect("serialise");
        let back: EffectSlot = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, slot);
        // Unknown keys are ignored, missing keys default, out-of-range clamps.
        let old = r#"{"kind":"compressor","params":{"ratio":3,"future":1,"thresholdDb":-500}}"#;
        let s: EffectSlot = serde_json::from_str(old).expect("parse");
        assert_eq!(s.get("ratio"), Some(3.0));
        assert_eq!(s.get("thresholdDb"), Some(-60.0));
        assert_eq!(s.get("attackMs"), Some(10.0));
    }

    #[test]
    fn ab_toggles_between_two_settings() {
        let mut slot = EffectSlot::new(EffectKind::Delay);
        let a = slot.params;
        slot.toggle_ab();
        slot.set("feedback", 80.0).expect("key");
        let b = slot.params;
        slot.toggle_ab();
        assert_eq!(slot.params, a);
        slot.toggle_ab();
        assert_eq!(slot.params, b);
    }

    #[test]
    fn presets_are_versioned() {
        let slot = EffectSlot::new(EffectKind::Reverb);
        let p = EffectPreset::new(EffectKind::Reverb, "Mi sala", &slot.params);
        let back = EffectPreset::from_json(&p.to_json()).expect("parse");
        assert_eq!(
            back.values_for(EffectKind::Reverb).expect("kind"),
            slot.params
        );
        assert!(matches!(
            back.values_for(EffectKind::Delay),
            Err(PresetError::WrongKind { .. })
        ));
        let newer = br#"{"format":"elyvon-effect-preset","version":99,"kind":"reverb","name":"x","params":{}}"#;
        assert!(matches!(
            EffectPreset::from_json(newer),
            Err(PresetError::TooNew { .. })
        ));
        assert!(matches!(
            EffectPreset::from_json(b"{}"),
            Err(PresetError::Format)
        ));
    }
}
