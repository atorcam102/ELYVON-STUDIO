//! Offline analysis for non-destructive editing (Phase 11): region peaks
//! for normalisation, transient detection for markers and slices, and
//! single-track renders for freeze and bounce in place.
//!
//! Everything here runs on worker threads and reads decoded media; nothing
//! changes the source files.

use std::sync::Arc;

use elyvon_core::{TempoMap, Ticks};
use elyvon_project::mixer::{OutputTarget, PanLaw};
use elyvon_project::automation::AutomationParam;
use elyvon_project::{MediaId, Project, Region, RegionContent, RegionId, TrackId};

use crate::media::MediaBuffer;
use crate::render::{self, RenderRequest};

/// Engine-rate frames a region plays, in timeline order (`None` past the
/// end of a non-looped file).
fn heard_frames(map: &TempoMap, region: &Region, buffer: &MediaBuffer) -> Option<(u64, u64, Box<dyn Fn(u64) -> Option<usize>>)> {
    let RegionContent::Audio {
        offset,
        looped,
        reversed,
        ..
    } = &region.content
    else {
        return None;
    };
    let rate = buffer.rate;
    let start = map.sample_at_tick(region.start, rate);
    let end = map.sample_at_tick(region.end(), rate).max(start + 1);
    let offset = buffer.engine_frames(*offset);
    let frames = buffer.frames() as u64;
    let len = end - start;
    let (looped, reversed) = (*looped && !*reversed, *reversed);
    let at = move |d: u64| -> Option<usize> {
        let p = if reversed { offset + (len - 1 - d) } else { offset + d };
        if p < frames {
            Some(p as usize)
        } else if looped && frames > 0 {
            Some((p % frames) as usize)
        } else {
            None
        }
    };
    Some((start, len, Box::new(at)))
}

/// Peak (linear, both channels) of the source audio a region plays,
/// ignoring its gain and fades. `None` for MIDI or silent regions.
pub fn region_peak(map: &TempoMap, region: &Region, buffer: &MediaBuffer) -> Option<f32> {
    let (_, len, at) = heard_frames(map, region, buffer)?;
    prefetch(buffer, region, map);
    let mut peak = 0.0f32;
    for d in 0..len {
        let Some(src) = at(d) else {
            if !matches!(region.content, RegionContent::Audio { reversed: true, .. }) {
                break;
            }
            continue;
        };
        let [l, r] = buffer.sample(src);
        peak = peak.max(l.abs()).max(r.abs());
    }
    (peak > 0.0).then_some(peak)
}

fn prefetch(buffer: &MediaBuffer, region: &Region, map: &TempoMap) {
    if buffer.stream.is_none() {
        return;
    }
    let Some((_, len, at)) = heard_frames(map, region, buffer) else {
        return;
    };
    let page = crate::media::stream::PAGE_FRAMES as u64;
    let mut d = 0;
    while d < len {
        if let Some(src) = at(d) {
            let _ = buffer.prefetch((src as u64).saturating_sub(page / 2), page as usize);
        }
        d += page / 2;
    }
}

/// Region gain (dB) that makes the region's peak `target_db`, clamped to
/// the region gain range. `None` when the region is silent.
pub fn normalize_gain(peak: f32, target_db: f64) -> Option<f64> {
    (peak > 0.0).then(|| {
        (target_db - 20.0 * f64::from(peak).log10())
            .clamp(RegionContent::MIN_GAIN_DB, RegionContent::MAX_GAIN_DB)
    })
}

const HOP: usize = 128;

/// Onsets in a region's audio (absolute ticks, sorted), from rises of the
/// short-term energy. `sensitivity` 0…1 (higher finds softer onsets).
/// Onsets are refined to the first sample above half the hop's peak.
pub fn detect_transients(map: &TempoMap, region: &Region, buffer: &MediaBuffer, sensitivity: f64) -> Vec<Ticks> {
    let Some((start, len, at)) = heard_frames(map, region, buffer) else {
        return Vec::new();
    };
    prefetch(buffer, region, map);
    let rate = buffer.rate;
    let mono = |d: u64| -> f32 {
        at(d).map_or(0.0, |src| {
            let [l, r] = buffer.sample(src);
            0.5 * (l + r)
        })
    };
    let hops = (len as usize).div_ceil(HOP);
    let mut level = Vec::with_capacity(hops);
    for h in 0..hops {
        let mut e = 0.0f64;
        let a = (h * HOP) as u64;
        let b = (a + HOP as u64).min(len);
        for d in a..b {
            let x = f64::from(mono(d));
            e += x * x;
        }
        level.push(10.0 * (e / HOP as f64 + 1e-12).log10());
    }
    let s = sensitivity.clamp(0.0, 1.0);
    let rise_db = 14.0 - 10.0 * s;
    let gate_db = -50.0 - 20.0 * s;
    let min_gap = (0.05 * f64::from(rate)) as u64;
    let mut out = Vec::new();
    let mut last: Option<u64> = None;
    for h in 1..hops {
        let past = level[h - 1].max(if h >= 2 { level[h - 2] } else { level[h - 1] });
        if level[h] < gate_db || level[h] - past < rise_db {
            continue;
        }
        // Refine within this hop and the one before it.
        let a = ((h - 1) * HOP) as u64;
        let b = ((h + 1) * HOP).min(len as usize) as u64;
        let peak = (a..b).map(|d| mono(d).abs()).fold(0.0f32, f32::max);
        let onset = (a..b).find(|d| mono(*d).abs() >= 0.5 * peak).unwrap_or(a);
        if last.is_some_and(|l| onset < l + min_gap) {
            continue;
        }
        last = Some(onset);
        let tick = map.tick_at_sample(start + onset, rate).round() as Ticks;
        if tick > region.start && tick < region.end() {
            out.push(tick);
        }
    }
    out
}

/// A track rendered on its own (freeze, bounce in place).
#[derive(Debug, Clone)]
pub struct TrackRender {
    /// Interleaved stereo at the engine rate, aligned with the timeline.
    pub samples: Vec<f32>,
    pub rate: u32,
    /// Timeline position of the first frame.
    pub start: Ticks,
    /// Regions that stayed silent because their file is unavailable.
    pub missing_media: u64,
    pub streaming_errors: u64,
    pub peak: f32,
    pub checksum: String,
}

impl TrackRender {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
}

/// Renders `track` (only `regions` when given) through its inserts and
/// insert automation, before the fader unless `include_fader`. Insert
/// latency is removed so the result lines up with the timeline. `tail`
/// seconds after the last region keep effect and fade tails.
pub fn render_track(
    project: &Project,
    media: &dyn Fn(MediaId) -> Option<Arc<MediaBuffer>>,
    track_id: TrackId,
    regions: Option<&[RegionId]>,
    include_fader: bool,
    tail: f64,
    rate: u32,
) -> Result<TrackRender, String> {
    let mut p = project.clone();
    let mut track = p
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .cloned()
        .ok_or("the track no longer exists")?;
    if track.kind == elyvon_project::TrackKind::Midi {
        return Err("external MIDI tracks have no audio to render".into());
    }
    if let Some(ids) = regions {
        // A selected take renders its folder's comp (when it has one).
        let folders: std::collections::HashSet<RegionId> = track
            .regions
            .iter()
            .filter(|r| ids.contains(&r.id))
            .filter_map(|r| r.capture.as_ref().map(|c| c.folder))
            .collect();
        track.comps.retain(|c| folders.contains(&c.folder) && !c.flattened);
        let comped: std::collections::HashSet<RegionId> = track.comps.iter().map(|c| c.folder).collect();
        track.regions.retain(|r| {
            ids.contains(&r.id) || r.capture.as_ref().is_some_and(|c| comped.contains(&c.folder))
        });
    }
    track.freeze = None;
    track.muted = false;
    track.soloed = false;
    track.strip.sends.clear();
    track.strip.output = OutputTarget::Master;
    track.strip.group = None;
    track.folder = None;
    if track.automation.mode.writes() {
        track.automation.mode = elyvon_project::automation::AutomationMode::Read;
    }
    let pre_fader = |param: &AutomationParam| matches!(param, AutomationParam::Insert { .. });
    if !include_fader {
        track.volume_db = 0.0;
        track.pan = 0.0;
        track.automation.lanes.retain(|l| pre_fader(&l.param));
        for r in &mut track.regions {
            r.automation.retain(|l| pre_fader(&l.param));
        }
        // Unity at the centre for mono strips.
        p.mixer.pan_law = PanLaw::ConstantPowerCompensated;
    }
    let heard: Vec<(Ticks, Ticks)> = track
        .regions
        .iter()
        .filter(|r| !r.muted || r.capture.is_some())
        .map(|r| (r.start, r.end()))
        .collect();
    let start = heard.iter().map(|h| h.0).min().ok_or("nothing to render")?;
    let end = heard.iter().map(|h| h.1).max().unwrap_or(start);
    p.tracks = vec![track];
    p.mixer.auxes.clear();
    p.mixer.groups.clear();
    p.mixer.folders.clear();
    p.mixer.master = elyvon_project::mixer::MasterChannel::default();
    p.transport.loop_range.enabled = false;
    let map = p.transport.tempo_map().map_err(|e| e.to_string())?;
    let latency = crate::plan::build_plan(&p, media, rate, 1).stats.latency_frames;
    let end_secs = map.seconds_at(end as f64) + tail.max(0.0) + latency as f64 / f64::from(rate);
    let end_tick = map.tick_at_seconds(end_secs).ceil() as Ticks;
    let rendered = render::render(
        &p,
        media,
        RenderRequest {
            sample_rate: rate,
            start_tick: start,
            end_tick: Some(end_tick),
            block_frames: 1024,
            metronome: false,
        },
        None,
    );
    let mut samples = rendered.samples;
    // The render is delayed by the strip's look-ahead; shift it back.
    samples.drain(..(latency * 2).min(samples.len()));
    let wanted = (map.sample_at_tick(end_tick, rate) - map.sample_at_tick(start, rate)) as usize;
    let wanted = wanted.saturating_sub(latency).max(1);
    samples.resize(wanted * 2, 0.0);
    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &samples {
        for b in s.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    Ok(TrackRender {
        samples,
        rate,
        start,
        missing_media: rendered.report.missing_media,
        streaming_errors: rendered.report.streaming_errors,
        peak,
        checksum: format!("{h:016x}"),
    })
}

/// Writes a track render as a 32-bit float stereo WAV, atomically.
pub fn write_render(path: &std::path::Path, r: &TrackRender) -> std::io::Result<()> {
    if r.streaming_errors != 0 {
        return Err(std::io::Error::other("render aborted: disk-cache reads failed"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let wav = render::encode_wav_f32(&r.samples, 2, r.rate);
    elyvon_core::fs::atomic_write(path, &wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_gain_targets_the_peak() {
        assert!((normalize_gain(0.5, 0.0).unwrap() - 6.0206).abs() < 1e-3);
        assert!((normalize_gain(0.5, -1.0).unwrap() - 5.0206).abs() < 1e-3);
        assert_eq!(normalize_gain(1e-9, 0.0).unwrap(), 24.0, "clamped to +24 dB");
        assert!(normalize_gain(0.0, 0.0).is_none());
    }
}
