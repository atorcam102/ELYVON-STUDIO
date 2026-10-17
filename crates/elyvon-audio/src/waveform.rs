//! Waveform columns for a region on screen, from the media peak pyramid.
//!
//! The UI asks for `columns` evenly spaced in *ticks* between two timeline
//! positions; this maps every column through the tempo map, the region offset
//! and looping to source frames, so what is drawn is exactly what plays.

use elyvon_core::TempoMap;
use elyvon_project::{Region, RegionContent};

use crate::gain::db_to_gain;
use crate::media::MediaBuffer;

/// Hard cap on columns per request.
pub const MAX_COLUMNS: usize = 8192;

/// Min/max per column for up to two channels (`[(min, max); 2]`).
pub fn region_columns(
    map: &TempoMap,
    region: &Region,
    media: &MediaBuffer,
    from_tick: f64,
    to_tick: f64,
    columns: usize,
) -> Vec<[(f32, f32); 2]> {
    let columns = columns.clamp(1, MAX_COLUMNS);
    let RegionContent::Audio {
        offset,
        gain_db,
        looped,
        reversed,
        ..
    } = &region.content
    else {
        return vec![[(0.0, 0.0); 2]; columns];
    };
    let gain = db_to_gain(*gain_db) as f32;
    let rate = f64::from(media.info.sample_rate);
    let frames = media.info.frames;
    let region_start = region.start as f64;
    let region_end = region.end() as f64;
    let start_secs = map.seconds_at(region_start);
    // Reversed regions read their source span from the top down.
    let span = elyvon_project::timeline_edit::abs_frame(map, region.end(), media.info.sample_rate)
        - elyvon_project::timeline_edit::abs_frame(map, region.start, media.info.sample_rate);
    let from = from_tick.max(region_start);
    let to = to_tick.min(region_end);
    let mut out = Vec::with_capacity(columns);
    let step = (to_tick - from_tick) / columns as f64;
    for c in 0..columns {
        let ta = (from_tick + step * c as f64).max(from);
        let tb = (from_tick + step * (c + 1) as f64).min(to);
        if tb <= ta {
            out.push([(0.0, 0.0); 2]);
            continue;
        }
        let (ea, eb) = (
            (map.seconds_at(ta) - start_secs) * rate,
            (map.seconds_at(tb) - start_secs) * rate,
        );
        let (fa, fb) = if *reversed {
            let top = (*offset + span) as f64;
            (top - eb, top - ea)
        } else {
            (*offset as f64 + ea, *offset as f64 + eb)
        };
        let (a, b) = (
            fa.floor() as u64,
            (fb.ceil() as u64).max(fa.floor() as u64 + 1),
        );
        let mut cell = if !*looped || frames == 0 {
            media.peak_range(a.min(frames), b.min(frames))
        } else if b - a >= frames {
            media.peak_range(0, frames)
        } else {
            let (wa, wb) = (a % frames, b % frames);
            if wa < wb || wb == 0 {
                media.peak_range(wa, if wb == 0 { frames } else { wb })
            } else {
                let x = media.peak_range(wa, frames);
                let y = media.peak_range(0, wb);
                [
                    (x[0].0.min(y[0].0), x[0].1.max(y[0].1)),
                    (x[1].0.min(y[1].0), x[1].1.max(y[1].1)),
                ]
            }
        };
        for ch in &mut cell {
            *ch = (ch.0 * gain, ch.1 * gain);
        }
        out.push(cell);
    }
    out
}
