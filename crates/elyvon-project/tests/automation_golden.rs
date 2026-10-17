//! Golden vectors shared with the UI mirror (`src/lib/automation.ts`), which
//! draws lanes and shows automated control values.
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p elyvon-project --test automation_golden`.
#![allow(clippy::unwrap_used)]

use elyvon_project::automation::{
    AutomationLane, AutomationParam, AutomationPoint, Curve, InsertParam, lane_pieces, overlay,
    pieces_value,
};
use elyvon_project::mixer::InsertId;
use serde_json::{Value, json};

fn lane(param: AutomationParam, pts: &[(u64, f64, Curve)]) -> AutomationLane {
    AutomationLane {
        param,
        points: pts
            .iter()
            .enumerate()
            .map(|(i, &(tick, value, curve))| AutomationPoint {
                id: i as u32 + 1,
                tick,
                value,
                curve,
            })
            .collect(),
    }
}

fn golden() -> Value {
    let cutoff = AutomationParam::Insert {
        id: InsertId(uuid::Uuid::nil()),
        param: InsertParam::FilterCutoff,
    };
    let lanes = vec![
        lane(
            AutomationParam::Volume,
            &[
                (100, -40.0, Curve::Bezier { c1: 0.0, c2: 0.2 }),
                (2000, 0.0, Curve::Step),
                (5000, -12.0, Curve::Linear),
                (5000, -6.0, Curve::Linear),
                (9000, -3.0, Curve::Bezier { c1: 1.0, c2: 1.0 }),
                (12_000, -96.0, Curve::Linear),
            ],
        ),
        lane(AutomationParam::Pan, &[(0, -1.0, Curve::Linear), (3840, 1.0, Curve::Linear)]),
        lane(AutomationParam::Mute, &[(500, 0.0, Curve::Step), (1500, 1.0, Curve::Step)]),
        lane(cutoff, &[(0, 20.0, Curve::Linear), (8000, 20_000.0, Curve::Bezier { c1: 0.3, c2: 0.9 })]),
    ];
    let ticks: Vec<f64> = vec![
        0.0, 50.0, 100.0, 700.0, 1999.0, 2000.0, 3000.0, 4999.0, 5000.0, 7000.0, 8000.0, 10_500.0,
        12_000.0, 20_000.0,
    ];
    let values: Vec<Vec<f64>> = lanes
        .iter()
        .map(|l| ticks.iter().map(|t| l.value_at(*t).unwrap()).collect())
        .collect();
    let before: Vec<Vec<f64>> = lanes
        .iter()
        .map(|l| ticks.iter().map(|t| l.value_before(*t).unwrap()).collect())
        .collect();
    // A region lane over [3000, 6000) on the volume lane (normalised values).
    let base = lane_pieces(&lanes[0], 0.0).unwrap();
    let region = lane(AutomationParam::Volume, &[(0, -20.0, Curve::Linear), (1000, -10.0, Curve::Linear)]);
    let over = lane_pieces(&region, 3000.0).unwrap();
    let merged = overlay(&base, &over, 3000.0, 6000.0);
    let overlaid: Vec<f64> = ticks.iter().map(|t| pieces_value(&merged, *t).unwrap()).collect();
    json!({
        "lanes": lanes,
        "ticks": ticks,
        "values": values,
        "before": before,
        "region": region,
        "regionStart": 3000,
        "regionEnd": 6000,
        "overlaidNorm": overlaid,
    })
}

#[test]
fn automation_golden_vectors() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/automation_golden.json");
    let text = serde_json::to_string_pretty(&golden()).unwrap() + "\n";
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let stored = std::fs::read_to_string(&path).expect("run with UPDATE_GOLDEN=1 to create the fixture");
    assert_eq!(stored.replace("\r\n", "\n"), text, "golden vectors changed; update the UI mirror");
}
