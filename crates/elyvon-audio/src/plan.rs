//! Playback plan: an immutable, sample-accurate description of what the engine
//! plays, built from a [`Project`] on the control thread.
//!
//! Building a plan allocates; the real-time thread only reads it. A new plan
//! is published after every edit and swapped in at a block boundary.
//!
//! # Overlapping regions
//!
//! On one track, the region that starts later wins (ties: later in the list).
//! A covered region is heard again where nothing covers it. When the winning
//! region has a fade-in, the covered region keeps playing underneath for that
//! long with the complementary fade-out (equal-power crossfade); symmetrically
//! for a fade-out where a covering region ends.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use elyvon_core::{Tempo, TempoMap, Ticks, TimeSignature};
use elyvon_project::{MediaId, Project, RegionContent, TrackKind};

use crate::gain::{constant_power_pan, db_to_gain};
use crate::media::MediaBuffer;

/// Upper bound on tracks the engine pre-allocates state for.
pub const MAX_TRACKS: usize = elyvon_project::validate::limits::MAX_TRACKS;

/// One contiguous piece of a clip as heard on the timeline (engine frames).
#[derive(Debug, Clone)]
pub struct PlanSegment {
    /// Heard range `[start, end)`.
    pub start: u64,
    pub end: u64,
    /// Bounds of the whole region (for its own fades and source position).
    pub clip_start: u64,
    pub clip_end: u64,
    pub media: Arc<MediaBuffer>,
    /// Engine-rate source frame at `clip_start`.
    pub offset: u64,
    pub looped: bool,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    /// Equal-power ramps at the segment edges (crossfades with covering clips).
    pub xfade_in: u64,
    pub xfade_out: u64,
}

impl PlanSegment {
    /// Envelope at absolute frame `t` (inside the segment). Fade-ins use
    /// `sin(d/F)` and fade-outs `sin(remaining/F)`, so a fade-out over the same
    /// frames as a fade-in is its exact equal-power complement.
    #[inline]
    pub fn envelope(&self, t: u64) -> f32 {
        let mut g = self.gain;
        let d = t - self.clip_start;
        if d < self.fade_in {
            g *= quarter_sine(d as f32 / self.fade_in as f32);
        }
        let rem = self.clip_end - t;
        if rem <= self.fade_out && self.fade_out > 0 {
            g *= quarter_sine(rem as f32 / self.fade_out as f32);
        }
        let e = t - self.start;
        if e < self.xfade_in {
            g *= quarter_sine(e as f32 / self.xfade_in as f32);
        }
        let rem = self.end - t;
        if rem <= self.xfade_out && self.xfade_out > 0 {
            g *= quarter_sine(rem as f32 / self.xfade_out as f32);
        }
        g
    }

    /// Engine-rate source frame for absolute frame `t`, or `None` past the end.
    #[inline]
    pub fn source_frame(&self, t: u64) -> Option<usize> {
        let frames = self.media.frames() as u64;
        let p = self.offset + (t - self.clip_start);
        if p < frames {
            Some(p as usize)
        } else if self.looped && frames > 0 {
            Some((p % frames) as usize)
        } else {
            None
        }
    }
}

/// `sin(x·π/2)` for x in [0, 1]: equal-power fade shape.
#[inline]
pub fn quarter_sine(x: f32) -> f32 {
    (x.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanNote {
    pub on: u64,
    pub off: u64,
    pub pitch: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone)]
pub struct PlanTrack {
    pub audible: bool,
    /// Volume × constant-power pan, for mono sources.
    pub mono_gain: (f32, f32),
    /// Volume × balance, for stereo sources.
    pub stereo_gain: (f32, f32),
    pub segments: Vec<PlanSegment>,
    /// `max(end)` over `segments[..=i]`, for allocation-free range lookup.
    pub max_end: Vec<u64>,
    pub notes: Vec<PlanNote>,
    pub max_note_len: u64,
}

impl PlanTrack {
    /// Indices of segments overlapping `[p, q)`.
    pub fn segment_range(&self, p: u64, q: u64) -> std::ops::Range<usize> {
        let first = self.max_end.partition_point(|&e| e <= p);
        let last = self.segments.partition_point(|s| s.start < q);
        first..last.max(first)
    }
}

/// Pre-rendered metronome sounds at the plan rate.
#[derive(Debug, Clone)]
pub struct ClickSounds {
    pub accent: Vec<f32>,
    pub normal: Vec<f32>,
}

impl ClickSounds {
    pub const SECONDS: f64 = 0.03;

    /// Short decaying sine bursts (synthesised, no sample files).
    pub fn synthesise(rate: u32) -> Self {
        let n = (Self::SECONDS * f64::from(rate)).round() as usize;
        let make = |freq: f64, amp: f64| -> Vec<f32> {
            (0..n)
                .map(|i| {
                    let t = i as f64 / f64::from(rate);
                    let attack = (t / 0.0005).min(1.0);
                    (amp * attack
                        * (-t / 0.006).exp()
                        * (2.0 * std::f64::consts::PI * freq * t).sin()) as f32
                })
                .collect()
        };
        Self {
            accent: make(1760.0, 0.7),
            normal: make(1320.0, 0.5),
        }
    }

    pub fn len(&self) -> usize {
        self.normal.len()
    }

    pub fn is_empty(&self) -> bool {
        self.normal.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanStats {
    pub clips: usize,
    pub segments: usize,
    pub missing_media: usize,
    pub notes: usize,
}

#[derive(Debug, Clone)]
pub struct PlaybackPlan {
    pub generation: u64,
    pub sample_rate: u32,
    pub map: TempoMap,
    /// Changes whenever the tempo/metre map changes (position remapping).
    pub map_key: u64,
    /// Cycle `[start, end)` in frames when enabled.
    pub cycle: Option<(u64, u64)>,
    pub metronome: bool,
    pub metronome_gain: f32,
    pub accent: bool,
    pub clicks: ClickSounds,
    pub count_in_bars: u8,
    pub pre_roll_bars: u8,
    pub tracks: Vec<PlanTrack>,
    /// End of the last region, in frames.
    pub end_frame: u64,
    pub stats: PlanStats,
}

impl PlaybackPlan {
    /// A silent plan (no project loaded yet).
    pub fn empty(sample_rate: u32) -> Self {
        let map = TempoMap::constant(Tempo::DEFAULT, TimeSignature::default());
        Self {
            generation: 0,
            sample_rate,
            map_key: map_key(&map),
            map,
            cycle: None,
            metronome: false,
            metronome_gain: 0.0,
            accent: true,
            clicks: ClickSounds::synthesise(sample_rate),
            count_in_bars: 0,
            pre_roll_bars: 0,
            tracks: Vec::new(),
            end_frame: 0,
            stats: PlanStats::default(),
        }
    }
}

fn map_key(map: &TempoMap) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Sample a few defining values: every change point and its tempo.
    map.initial_bpm().to_bits().hash(&mut h);
    for (tick, bpm) in map.tempo_changes() {
        tick.hash(&mut h);
        bpm.to_bits().hash(&mut h);
    }
    h.finish()
}

fn project_map_key(project: &Project) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let t = &project.transport;
    t.tempo.bpm().to_bits().hash(&mut h);
    (t.time_signature.numerator, t.time_signature.denominator).hash(&mut h);
    for c in &t.tempo_changes {
        c.tick.hash(&mut h);
        c.bpm.bpm().to_bits().hash(&mut h);
    }
    for m in &t.meter_changes {
        (
            m.bar,
            m.time_signature.numerator,
            m.time_signature.denominator,
        )
            .hash(&mut h);
    }
    h.finish()
}

struct Clip {
    start: u64,
    end: u64,
    media: Arc<MediaBuffer>,
    offset: u64,
    looped: bool,
    gain: f32,
    fade_in: u64,
    fade_out: u64,
}

/// Subtracts sorted disjoint `covered` intervals from `[s, e)`.
fn subtract(covered: &[(u64, u64)], s: u64, e: u64, out: &mut Vec<(u64, u64)>) {
    out.clear();
    let mut cur = s;
    let first = covered.partition_point(|&(_, ce)| ce <= s);
    for &(cs, ce) in &covered[first..] {
        if cs >= e {
            break;
        }
        if cs > cur {
            out.push((cur, cs));
        }
        cur = cur.max(ce);
        if cur >= e {
            break;
        }
    }
    if cur < e {
        out.push((cur, e));
    }
}

/// Adds `[s, e)` to sorted disjoint `covered`, merging neighbours.
fn cover(covered: &mut Vec<(u64, u64)>, s: u64, e: u64) {
    let i = covered.partition_point(|&(_, ce)| ce < s);
    let mut j = i;
    let (mut ns, mut ne) = (s, e);
    while j < covered.len() && covered[j].0 <= e {
        ns = ns.min(covered[j].0);
        ne = ne.max(covered[j].1);
        j += 1;
    }
    covered.splice(i..j, [(ns, ne)]);
}

fn resolve_overlaps(mut clips: Vec<Clip>) -> Vec<PlanSegment> {
    // Priority: later start wins; ties keep list order (later wins).
    clips.sort_by_key(|c| c.start);
    let mut fade_at_start: HashMap<u64, u64> = HashMap::new();
    let mut fade_at_end: HashMap<u64, u64> = HashMap::new();
    for c in &clips {
        let f = fade_at_start.entry(c.start).or_default();
        *f = (*f).max(c.fade_in);
        let f = fade_at_end.entry(c.end).or_default();
        *f = (*f).max(c.fade_out);
    }
    let mut covered: Vec<(u64, u64)> = Vec::new();
    let mut pieces = Vec::new();
    let mut segments = Vec::new();
    for c in clips.into_iter().rev() {
        subtract(&covered, c.start, c.end, &mut pieces);
        for &(a, b) in &pieces {
            let mut start = a;
            let mut end = b;
            let mut xfade_in = 0;
            let mut xfade_out = 0;
            if b < c.end {
                let ext = fade_at_start.get(&b).copied().unwrap_or(0);
                end = (b + ext).min(c.end);
                xfade_out = end - b;
            }
            if a > c.start {
                let ext = fade_at_end.get(&a).copied().unwrap_or(0);
                start = a.saturating_sub(ext).max(c.start);
                xfade_in = a - start;
            }
            segments.push(PlanSegment {
                start,
                end,
                clip_start: c.start,
                clip_end: c.end,
                media: Arc::clone(&c.media),
                offset: c.offset,
                looped: c.looped,
                gain: c.gain,
                fade_in: c.fade_in,
                fade_out: c.fade_out,
                xfade_in,
                xfade_out,
            });
        }
        cover(&mut covered, c.start, c.end);
    }
    segments.sort_by_key(|s| (s.start, s.end));
    segments
}

/// Builds the plan for `project` at `sample_rate`. `media` returns prepared
/// buffers (at `sample_rate`) for pool ids; regions without one are silent and
/// counted in [`PlanStats::missing_media`].
pub fn build_plan(
    project: &Project,
    media: &dyn Fn(MediaId) -> Option<Arc<MediaBuffer>>,
    sample_rate: u32,
    generation: u64,
) -> PlaybackPlan {
    let t = &project.transport;
    let map = t
        .tempo_map()
        .unwrap_or_else(|_| TempoMap::constant(t.tempo, t.time_signature));
    let at = |tick: Ticks| map.sample_at_tick(tick, sample_rate);
    let any_solo = project.any_solo();
    let mut stats = PlanStats::default();
    let mut tracks = Vec::with_capacity(project.tracks.len().min(MAX_TRACKS));
    for track in project.tracks.iter().take(MAX_TRACKS) {
        let volume = db_to_gain(track.volume_db) as f32;
        let (pl, pr) = constant_power_pan(track.pan);
        let (bl, br) = if track.pan <= 0.0 {
            (1.0, 1.0 + track.pan)
        } else {
            (1.0 - track.pan, 1.0)
        };
        let mut clips = Vec::new();
        let mut notes = Vec::new();
        for region in track.regions.iter().filter(|r| !r.muted) {
            let start = at(region.start);
            let end = at(region.end()).max(start + 1);
            match &region.content {
                RegionContent::Audio {
                    media: Some(id),
                    offset,
                    gain_db,
                    fade_in,
                    fade_out,
                    looped,
                } if track.kind == TrackKind::Audio => {
                    let Some(buffer) = media(*id) else {
                        stats.missing_media += 1;
                        continue;
                    };
                    let len = end - start;
                    let mut fi = buffer.engine_frames(*fade_in);
                    let mut fo = buffer.engine_frames(*fade_out);
                    if fi + fo > len {
                        // Scale both fades to fit the region.
                        let scale = len as f64 / (fi + fo) as f64;
                        fi = (fi as f64 * scale) as u64;
                        fo = len - fi;
                    }
                    clips.push(Clip {
                        start,
                        end,
                        offset: buffer.engine_frames(*offset),
                        media: buffer,
                        looped: *looped,
                        gain: db_to_gain(*gain_db) as f32,
                        fade_in: fi,
                        fade_out: fo,
                    });
                }
                RegionContent::Midi {
                    notes: region_notes,
                } => {
                    for n in region_notes.iter().filter(|n| n.start < region.length) {
                        let on_tick = region.start + n.start;
                        let off_tick =
                            region.start + n.start.saturating_add(n.length).min(region.length);
                        let on = at(on_tick);
                        let off = at(off_tick).max(on + 1);
                        notes.push(PlanNote {
                            on,
                            off,
                            pitch: n.pitch.min(127),
                            velocity: n.velocity.clamp(1, 127),
                        });
                    }
                }
                _ => {}
            }
        }
        stats.clips += clips.len();
        let segments = resolve_overlaps(clips);
        stats.segments += segments.len();
        let mut max_end = Vec::with_capacity(segments.len());
        let mut m = 0;
        for s in &segments {
            m = m.max(s.end);
            max_end.push(m);
        }
        notes.sort_by_key(|n| (n.on, n.pitch));
        stats.notes += notes.len();
        let max_note_len = notes.iter().map(|n| n.off - n.on).max().unwrap_or(0);
        tracks.push(PlanTrack {
            audible: track.is_audible(any_solo),
            mono_gain: (volume * pl as f32, volume * pr as f32),
            stereo_gain: (volume * bl as f32, volume * br as f32),
            segments,
            max_end,
            notes,
            max_note_len,
        });
    }
    let lr = &t.loop_range;
    let cycle = lr
        .enabled
        .then(|| (at(lr.start), at(lr.end)))
        .filter(|(s, e)| e > s);
    let end_frame = at(project.content_end());
    let key = project_map_key(project);
    PlaybackPlan {
        generation,
        sample_rate,
        map,
        map_key: key,
        cycle,
        metronome: t.metronome.enabled,
        metronome_gain: db_to_gain(t.metronome.volume_db) as f32,
        accent: t.metronome.accent,
        clicks: ClickSounds::synthesise(sample_rate),
        count_in_bars: t.count_in_bars,
        pre_roll_bars: t.pre_roll_bars,
        tracks,
        end_frame,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_and_cover() {
        let mut covered = Vec::new();
        cover(&mut covered, 10, 20);
        cover(&mut covered, 30, 40);
        cover(&mut covered, 20, 25);
        assert_eq!(covered, [(10, 25), (30, 40)]);
        let mut out = Vec::new();
        subtract(&covered, 0, 50, &mut out);
        assert_eq!(out, [(0, 10), (25, 30), (40, 50)]);
        subtract(&covered, 12, 24, &mut out);
        assert!(out.is_empty());
        cover(&mut covered, 0, 100);
        assert_eq!(covered, [(0, 100)]);
    }

    #[test]
    fn quarter_sine_is_equal_power() {
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let a = quarter_sine(x);
            let b = quarter_sine(1.0 - x);
            assert!((a * a + b * b - 1.0).abs() < 1e-6);
        }
    }
}
