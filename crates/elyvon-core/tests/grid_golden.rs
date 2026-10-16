//! Golden vectors shared with the UI mirror (`src/lib/tempoMap.ts`).
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p elyvon-core --test grid_golden`.
#![allow(clippy::unwrap_used)]

use elyvon_core::grid::{GridDivision, GridSpec};
use elyvon_core::{MeterChange, Tempo, TempoChange, TempoMap, TimeSignature};
use serde_json::{Value, json};

fn golden() -> Value {
    let tempo = Tempo::new(110.0).unwrap();
    let tempo_changes = vec![
        TempoChange {
            tick: 5000,
            bpm: Tempo::new(87.5).unwrap(),
        },
        TempoChange {
            tick: 20_000,
            bpm: Tempo::new(160.0).unwrap(),
        },
    ];
    let meter_changes = vec![
        MeterChange {
            bar: 2,
            time_signature: TimeSignature::new(7, 8).unwrap(),
        },
        MeterChange {
            bar: 4,
            time_signature: TimeSignature::new(6, 8).unwrap(),
        },
        MeterChange {
            bar: 6,
            time_signature: TimeSignature::new(3, 4).unwrap(),
        },
    ];
    let ts = TimeSignature::new(4, 4).unwrap();
    let map = TempoMap::new(tempo, &tempo_changes, ts, &meter_changes).unwrap();
    let ticks: Vec<u64> = vec![
        0, 1, 239, 240, 479, 959, 1000, 3839, 3840, 7000, 7679, 7680, 9000, 11_039, 11_040, 13_500,
        16_800, 20_000, 25_000, 40_001,
    ];
    let divisions = [
        GridDivision::Bar,
        GridDivision::Beat,
        GridDivision::Half,
        GridDivision::Quarter,
        GridDivision::Eighth,
        GridDivision::Sixteenth,
        GridDivision::ThirtySecond,
    ];
    let mut snaps = Vec::new();
    for division in divisions {
        for (triplet, swing) in [(false, 0u8), (true, 0), (false, 60)] {
            let spec = GridSpec {
                division,
                triplet,
                swing,
            };
            let values: Vec<u64> = ticks.iter().map(|t| spec.snap(&map, *t)).collect();
            let mut points = Vec::new();
            spec.points(&map, 0, 12_000, |p| points.push(p));
            snaps.push(json!({ "spec": spec, "snapped": values, "points": points }));
        }
    }
    json!({
        "tempo": tempo,
        "timeSignature": ts,
        "tempoChanges": tempo_changes,
        "meterChanges": meter_changes,
        "ticks": ticks,
        "barStarts": (0..10).map(|b| map.bar_start(b)).collect::<Vec<_>>(),
        "positions": ticks.iter().map(|t| map.position(*t).to_string()).collect::<Vec<_>>(),
        "seconds": ticks.iter().map(|t| map.seconds_at(*t as f64)).collect::<Vec<_>>(),
        "beats": map.beats(0, 12_000).map(|b| json!([b.tick, b.downbeat])).collect::<Vec<_>>(),
        "snaps": snaps,
    })
}

#[test]
fn grid_and_tempo_map_golden_vectors() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/grid_golden.json");
    let value = golden();
    let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let stored =
        std::fs::read_to_string(&path).expect("run with UPDATE_GOLDEN=1 to create the fixture");
    // Git may check text fixtures out with CRLF on Windows (core.autocrlf).
    let stored = stored.replace("\r\n", "\n");
    assert_eq!(
        stored, text,
        "golden vectors changed; regenerate and update the UI mirror"
    );
}
