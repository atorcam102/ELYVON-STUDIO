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
use elyvon_project::mixer::{AuxId, ChannelFormat, OutputTarget, PanLaw};
use elyvon_project::{MediaId, Project, RegionContent, TrackKind};

use crate::dsp::{Insert, Smoothed, StereoDelay};
use crate::gain::{constant_power_pan, db_to_gain};
use crate::media::MediaBuffer;
use crate::synth::TestSynth;

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

/// Auxiliary channels the engine pre-allocates bus buffers for.
pub const MAX_AUXES: usize = elyvon_project::mixer::limits::MAX_AUXES;
/// Meter slots: tracks, then auxes, then the master.
pub const MAX_CHANNELS: usize = MAX_TRACKS + MAX_AUXES + 1;
pub const MASTER_SLOT: usize = MAX_TRACKS + MAX_AUXES;

pub fn aux_slot(index: usize) -> usize {
    MAX_TRACKS + index
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanNote {
    pub on: u64,
    pub off: u64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanEvent {
    pub on: u64,
    pub packet: crate::midi::Packet,
}

/// Where a strip's signal goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest {
    Master,
    /// Index into [`PlaybackPlan::auxes`].
    Aux(usize),
}

/// A send from a strip to an auxiliary channel.
#[derive(Debug, Clone)]
pub struct PlanSend {
    pub aux: usize,
    pub key: u128,
    pub pre_fader: bool,
    /// Zero when muted or disabled by solo; ramps like the fader.
    pub gain: Smoothed,
    /// Path compensation on this edge.
    pub delay: StereoDelay,
}

/// One channel strip of the processing graph.
#[derive(Debug, Clone)]
pub struct Strip {
    /// Track/aux UUID bits (0 for the master).
    pub key: u128,
    /// Sums its input to mono before the inserts.
    pub mono: bool,
    /// Volume × pan law/balance; zero when muted or silenced by solo.
    pub fader: Smoothed,
    pub inserts: Vec<Insert>,
    pub output: Dest,
    /// 1 when the output edge is enabled (solo/mute gating), else 0.
    pub out_gate: Smoothed,
    pub out_delay: StereoDelay,
    pub sends: Vec<PlanSend>,
    /// Frames of latency added by the inserts.
    pub latency: usize,
    pub meter_slot: usize,
    /// Anything can be heard from this strip (otherwise it is skipped once
    /// its ramps reach silence).
    pub active: bool,
}

impl Strip {
    /// True when processing would only produce silence.
    pub fn is_idle(&self) -> bool {
        !self.active
            && (self.fader.is_silent() || self.out_gate.is_silent())
            && self.sends.iter().all(|s| s.gain.is_silent())
    }

    /// Continue fader ramps, insert and delay state from the old graph.
    pub fn adopt(&mut self, old: &mut Strip) {
        self.fader.adopt(&old.fader);
        self.out_gate.adopt(&old.out_gate);
        for insert in &mut self.inserts {
            match old.inserts.iter_mut().find(|o| o.id == insert.id) {
                Some(o) => insert.adopt(o),
                None => insert.mark_new(),
            }
        }
        if old.output == self.output {
            self.out_delay.adopt(&mut old.out_delay);
        }
        for send in &mut self.sends {
            if let Some(o) = old.sends.iter_mut().find(|o| o.key == send.key) {
                send.gain.adopt(&o.gain);
                send.delay.adopt(&mut o.delay);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanTrack {
    pub key: u128,
    pub strip: Strip,
    pub events: Vec<PlanEvent>,
    /// Not muted and allowed by solo: notes play.
    pub audible: bool,
    pub segments: Vec<PlanSegment>,
    /// `max(end)` over `segments[..=i]`, for allocation-free range lookup.
    pub max_end: Vec<u64>,
    pub notes: Vec<PlanNote>,
    pub max_note_len: u64,
    /// Built-in test instrument of an instrument track.
    pub synth: Option<Box<TestSynth>>,
}

impl PlanTrack {
    /// Indices of segments overlapping `[p, q)`.
    pub fn segment_range(&self, p: u64, q: u64) -> std::ops::Range<usize> {
        let first = self.max_end.partition_point(|&e| e <= p);
        let last = self.segments.partition_point(|s| s.start < q);
        first..last.max(first)
    }

    /// Whether a note of `(channel, pitch)` sounds at frame `at`.
    pub fn covers(&self, at: u64, channel: u8, pitch: u8) -> bool {
        let first = self
            .notes
            .partition_point(|n| n.on < at.saturating_sub(self.max_note_len));
        self.notes[first..]
            .iter()
            .take_while(|n| n.on <= at)
            .any(|n| n.channel == channel && n.pitch == pitch && n.off > at)
    }
}

#[derive(Debug, Clone)]
pub struct PlanAux {
    pub strip: Strip,
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
    pub auxes: usize,
    pub inserts: usize,
    /// Insert latency on the longest path to the output (compensated on
    /// every parallel path).
    pub latency_frames: usize,
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
    /// `(key, index)` sorted by key, for allocation-free lookups.
    pub track_index: Vec<(u128, usize)>,
    pub auxes: Vec<PlanAux>,
    /// Aux processing order (sources before destinations).
    pub aux_order: Vec<usize>,
    pub master: Strip,
    /// End of the last region, in frames.
    pub end_frame: u64,
    pub stats: PlanStats,
}

impl PlaybackPlan {
    /// Worker/control only: warm streamed sources for a timeline span.
    pub fn prefetch(&self, position: u64, frames: usize) -> Result<(), crate::media::MediaError> {
        for track in &self.tracks {
            for seg in &track.segments
                [track.segment_range(position, position.saturating_add(frames as u64))]
            {
                let mut at = position.max(seg.start);
                let end = position.saturating_add(frames as u64).min(seg.end);
                while at < end {
                    let Some(src) = seg.source_frame(at) else {
                        break;
                    };
                    let n = ((end - at) as usize)
                        .min(seg.media.frames() - src)
                        .min(crate::media::stream::PAGE_FRAMES);
                    if n == 0 {
                        break;
                    }
                    seg.media.prefetch(src as u64, n)?;
                    at += n as u64;
                }
            }
        }
        Ok(())
    }

    pub fn find_track(&self, key: u128) -> Option<usize> {
        self.track_index
            .binary_search_by_key(&key, |&(k, _)| k)
            .ok()
            .map(|i| self.track_index[i].1)
    }

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
            track_index: Vec::new(),
            auxes: Vec::new(),
            aux_order: Vec::new(),
            master: master_strip(&Project::new("empty"), sample_rate),
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

/// Left/right gains of a strip for `pan` under `law` (mono) or balance (stereo).
pub fn pan_gains(mono: bool, pan: f64, law: PanLaw) -> (f64, f64) {
    let p = pan.clamp(-1.0, 1.0);
    if !mono {
        return if p <= 0.0 {
            (1.0, 1.0 + p)
        } else {
            (1.0 - p, 1.0)
        };
    }
    match law {
        PanLaw::ConstantPower => constant_power_pan(p),
        PanLaw::ConstantPowerCompensated => {
            let (l, r) = constant_power_pan(p);
            (l * std::f64::consts::SQRT_2, r * std::f64::consts::SQRT_2)
        }
        PanLaw::Linear => ((1.0 - p) / 2.0, (1.0 + p) / 2.0),
    }
}

fn inserts(slots: &[elyvon_project::mixer::InsertSlot], rate: u32) -> Vec<Insert> {
    slots
        .iter()
        .map(|s| Insert::new(s.id, s.bypass, &s.processor, rate))
        .collect()
}

fn gain_pair(g: f64) -> (f32, f32) {
    (g as f32, g as f32)
}

fn master_strip(project: &Project, rate: u32) -> Strip {
    let m = &project.mixer.master;
    let ins = inserts(&m.inserts, rate);
    let own: usize = ins.iter().map(Insert::latency).sum();
    let g = if m.muted {
        0.0
    } else {
        db_to_gain(m.volume_db)
    };
    Strip {
        key: 0,
        mono: false,
        fader: Smoothed::new(gain_pair(g)),
        inserts: ins,
        output: Dest::Master,
        out_gate: Smoothed::new((1.0, 1.0)),
        out_delay: StereoDelay::new(0),
        sends: Vec::new(),
        latency: own,
        meter_slot: MASTER_SLOT,
        active: true,
    }
}

/// Graph node index: tracks `0..t`, auxes `t..t + a`.
struct Graph {
    t: usize,
    /// Output destination and sends `(aux index, pre-fader)` per node.
    out: Vec<Dest>,
    sends: Vec<Vec<usize>>,
}

impl Graph {
    fn targets(&self, n: usize) -> impl Iterator<Item = Option<usize>> + '_ {
        let out = match self.out[n] {
            Dest::Master => None,
            Dest::Aux(a) => Some(self.t + a),
        };
        std::iter::once(out).chain(self.sends[n].iter().map(|&a| Some(self.t + a)))
    }
}

/// Which nodes are processed and which edges carry signal under solo.
struct SoloState {
    soloed: bool,
    /// In the solo set, downstream of it, or solo-safe (all edges on).
    full: Vec<bool>,
    /// Upstream of a soloed aux: only edges towards it carry signal.
    upstream: Vec<bool>,
}

impl SoloState {
    fn edge(&self, from: usize, to: Option<usize>) -> bool {
        if !self.soloed || self.full[from] {
            return true;
        }
        match to {
            Some(d) => self.upstream[from] && (self.upstream[d] || self.full[d]),
            None => false,
        }
    }

    fn processed(&self, n: usize) -> bool {
        !self.soloed || self.full[n] || self.upstream[n]
    }
}

fn solo_state(graph: &Graph, soloed: &[bool], safe: &[bool]) -> SoloState {
    let n = soloed.len();
    let any = soloed.iter().any(|s| *s);
    let mut full = vec![false; n];
    let mut upstream = vec![false; n];
    if any {
        // Downstream closure of soloed and solo-safe channels.
        let mut stack: Vec<usize> = (0..n).filter(|&i| soloed[i] || safe[i]).collect();
        while let Some(i) = stack.pop() {
            if full[i] {
                continue;
            }
            full[i] = true;
            stack.extend(graph.targets(i).flatten());
        }
        // Sources feeding a soloed aux keep feeding it.
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..n {
                if upstream[i] {
                    continue;
                }
                if graph
                    .targets(i)
                    .flatten()
                    .any(|d| (d >= graph.t && soloed[d]) || upstream[d])
                {
                    upstream[i] = true;
                    changed = true;
                }
            }
        }
    }
    SoloState {
        soloed: any,
        full,
        upstream,
    }
}

/// Aux processing order (sources first). An invalid cycle (rejected by
/// project validation) is broken by sending the offending auxes to master.
fn aux_order(graph: &mut Graph) -> Vec<usize> {
    let a = graph.out.len() - graph.t;
    let mut indegree = vec![0usize; a];
    for i in 0..a {
        for d in graph.targets(graph.t + i).flatten() {
            indegree[d - graph.t] += 1;
        }
    }
    let mut ready: Vec<usize> = (0..a).rev().filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(a);
    while let Some(i) = ready.pop() {
        order.push(i);
        let targets: Vec<usize> = graph.targets(graph.t + i).flatten().collect();
        for d in targets {
            indegree[d - graph.t] -= 1;
            if indegree[d - graph.t] == 0 {
                ready.push(d - graph.t);
            }
        }
    }
    for i in 0..a {
        if !order.contains(&i) {
            graph.out[graph.t + i] = Dest::Master;
            graph.sends[graph.t + i].clear();
            order.push(i);
        }
    }
    order
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
    let mixer = &project.mixer;
    let law = mixer.pan_law;
    let mut stats = PlanStats::default();

    let tracks_in: Vec<&elyvon_project::Track> = project.tracks.iter().take(MAX_TRACKS).collect();
    let auxes_in: Vec<&elyvon_project::mixer::AuxChannel> =
        mixer.auxes.iter().take(MAX_AUXES).collect();
    let aux_index: HashMap<AuxId, usize> = auxes_in
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();
    let dest = |o: OutputTarget| match o {
        OutputTarget::Master => Dest::Master,
        OutputTarget::Aux { id } => aux_index.get(&id).map_or(Dest::Master, |&i| Dest::Aux(i)),
    };
    let strips_in: Vec<&elyvon_project::mixer::StripRouting> = tracks_in
        .iter()
        .map(|t| &t.strip)
        .chain(auxes_in.iter().map(|a| &a.strip))
        .collect();
    let mut graph = Graph {
        t: tracks_in.len(),
        out: strips_in.iter().map(|s| dest(s.output)).collect(),
        sends: strips_in
            .iter()
            .map(|s| {
                s.sends
                    .iter()
                    .filter_map(|x| aux_index.get(&x.target).copied())
                    .collect()
            })
            .collect(),
    };
    let order = aux_order(&mut graph);
    let soloed: Vec<bool> = tracks_in
        .iter()
        .map(|t| t.soloed)
        .chain(auxes_in.iter().map(|a| a.soloed))
        .collect();
    let muted: Vec<bool> = tracks_in
        .iter()
        .map(|t| t.muted)
        .chain(auxes_in.iter().map(|a| a.muted))
        .collect();
    let safe: Vec<bool> = strips_in.iter().map(|s| s.solo_safe).collect();
    let solo = solo_state(&graph, &soloed, &safe);

    // Sources: audio segments, notes, events and instruments per track.
    let mut tracks = Vec::with_capacity(tracks_in.len());
    for (ti, track) in tracks_in.iter().enumerate() {
        let mut clips = Vec::new();
        let mut notes = Vec::new();
        let mut events = Vec::new();
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
                    for event in region
                        .midi_events
                        .iter()
                        .filter(|e| e.tick <= region.length)
                    {
                        if let Some(packet) = crate::midi::Packet::parse(&event.bytes) {
                            events.push(PlanEvent {
                                on: at(region.start.saturating_add(event.tick)),
                                packet,
                            });
                        }
                    }
                    for n in region_notes
                        .iter()
                        .filter(|n| !n.muted && n.start < region.length)
                    {
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
                            channel: n.channel & 0x0f,
                        });
                    }
                }
                _ => {}
            }
        }
        stats.clips += clips.len();
        let all_mono = !clips.is_empty() && clips.iter().all(|c| c.media.channels() <= 1);
        let segments = resolve_overlaps(clips);
        stats.segments += segments.len();
        let mut max_end = Vec::with_capacity(segments.len());
        let mut m = 0;
        for s in &segments {
            m = m.max(s.end);
            max_end.push(m);
        }
        notes.sort_by_key(|n| (n.on, n.pitch, n.channel));
        stats.notes += notes.len();
        let max_note_len = notes.iter().map(|n| n.off - n.on).max().unwrap_or(0);
        events.sort_by_key(|e| e.on);
        let mono = match track.strip.format {
            ChannelFormat::Mono => true,
            ChannelFormat::Stereo => false,
            ChannelFormat::Auto => track.kind == TrackKind::Audio && all_mono,
        };
        let synth =
            (track.kind == TrackKind::Instrument).then(|| Box::new(TestSynth::new(sample_rate)));
        let audible = solo.processed(ti) && !muted[ti];
        let has_audio = !segments.is_empty() || synth.is_some();
        tracks.push(PlanTrack {
            key: track.id.0.as_u128(),
            strip: Strip {
                key: track.id.0.as_u128(),
                mono,
                fader: Smoothed::new((0.0, 0.0)),
                inserts: inserts(&track.strip.inserts, sample_rate),
                output: graph.out[ti],
                out_gate: Smoothed::new((0.0, 0.0)),
                out_delay: StereoDelay::new(0),
                sends: Vec::new(),
                latency: 0,
                meter_slot: ti,
                active: audible && has_audio,
            },
            events,
            audible,
            segments,
            max_end,
            notes,
            max_note_len,
            synth,
        });
    }
    let mut auxes: Vec<PlanAux> = auxes_in
        .iter()
        .enumerate()
        .map(|(i, a)| PlanAux {
            strip: Strip {
                key: a.id.0.as_u128(),
                mono: a.strip.format == ChannelFormat::Mono,
                fader: Smoothed::new((0.0, 0.0)),
                inserts: inserts(&a.strip.inserts, sample_rate),
                output: graph.out[graph.t + i],
                out_gate: Smoothed::new((0.0, 0.0)),
                out_delay: StereoDelay::new(0),
                sends: Vec::new(),
                latency: 0,
                meter_slot: aux_slot(i),
                active: solo.processed(graph.t + i) && !muted[graph.t + i],
            },
        })
        .collect();
    stats.auxes = auxes.len();

    // Faders, gates and sends.
    let n_nodes = strips_in.len();
    for node in 0..n_nodes {
        let (volume, pan) = if node < graph.t {
            (tracks_in[node].volume_db, tracks_in[node].pan)
        } else {
            let a = auxes_in[node - graph.t];
            (a.volume_db, a.pan)
        };
        let strip = if node < graph.t {
            &mut tracks[node].strip
        } else {
            &mut auxes[node - graph.t].strip
        };
        let on = solo.processed(node) && !muted[node];
        let (l, r) = pan_gains(strip.mono, pan, law);
        let v = if muted[node] { 0.0 } else { db_to_gain(volume) };
        strip.fader = Smoothed::new(((v * l) as f32, (v * r) as f32));
        let out_target = match graph.out[node] {
            Dest::Master => None,
            Dest::Aux(a) => Some(graph.t + a),
        };
        let gate = if on && solo.edge(node, out_target) {
            1.0
        } else {
            0.0
        };
        strip.out_gate = Smoothed::new((gate, gate));
        strip.sends = strips_in[node]
            .sends
            .iter()
            .filter_map(|s| {
                let a = *aux_index.get(&s.target)?;
                if !graph.sends[node].contains(&a) {
                    return None;
                }
                let enabled = on && !s.muted && solo.edge(node, Some(graph.t + a));
                let g = if enabled { db_to_gain(s.level_db) } else { 0.0 };
                Some(PlanSend {
                    aux: a,
                    key: s.target.0.as_u128(),
                    pre_fader: s.pre_fader,
                    gain: Smoothed::new(gain_pair(g)),
                    delay: StereoDelay::new(0),
                })
            })
            .collect();
        stats.inserts += strip.inserts.len();
        strip.latency = strip.inserts.iter().map(Insert::latency).sum();
    }

    // Path latency compensation: every input of a summing point arrives with
    // the latency of its slowest input.
    let mut in_lat = vec![0usize; auxes.len()];
    let out_lat = |node: usize, tracks: &[PlanTrack], auxes: &[PlanAux], in_lat: &[usize]| {
        if node < graph.t {
            tracks[node].strip.latency
        } else {
            in_lat[node - graph.t] + auxes[node - graph.t].strip.latency
        }
    };
    let mut master_in = 0usize;
    // Tracks feed auxes and master; then auxes in processing order.
    let feeds =
        |node: usize, lat: usize, in_lat: &mut [usize], master_in: &mut usize, g: &Graph| {
            for d in g.targets(node) {
                match d {
                    Some(d) => in_lat[d - g.t] = in_lat[d - g.t].max(lat),
                    None => *master_in = (*master_in).max(lat),
                }
            }
        };
    for (node, track) in tracks.iter().enumerate() {
        let lat = track.strip.latency;
        feeds(node, lat, &mut in_lat, &mut master_in, &graph);
    }
    for &a in &order {
        let lat = in_lat[a] + auxes[a].strip.latency;
        feeds(graph.t + a, lat, &mut in_lat, &mut master_in, &graph);
    }
    for node in 0..n_nodes {
        let lat = out_lat(node, &tracks, &auxes, &in_lat);
        let arrival = |d: Dest| match d {
            Dest::Master => master_in,
            Dest::Aux(a) => in_lat[a],
        };
        let strip = if node < graph.t {
            &mut tracks[node].strip
        } else {
            &mut auxes[node - graph.t].strip
        };
        strip.out_delay = StereoDelay::new(arrival(strip.output).saturating_sub(lat));
        for s in &mut strip.sends {
            s.delay = StereoDelay::new(in_lat[s.aux].saturating_sub(lat));
        }
    }
    let master = master_strip(project, sample_rate);
    stats.inserts += master.inserts.len();
    stats.latency_frames = master_in + master.latency;

    let mut track_index: Vec<(u128, usize)> =
        tracks.iter().enumerate().map(|(i, t)| (t.key, i)).collect();
    track_index.sort_unstable();
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
        track_index,
        auxes,
        aux_order: order,
        master,
        end_frame,
        stats,
    }
}

impl PlaybackPlan {
    /// Moves running state (fader ramps, insert memory, delay lines and
    /// sounding instrument voices) from `old` into this plan. Control or
    /// audio thread; never allocates or frees.
    pub fn adopt(&mut self, old: &mut PlaybackPlan) {
        for track in &mut self.tracks {
            if let Some(i) = old.find_track(track.key) {
                let o = &mut old.tracks[i];
                track.strip.adopt(&mut o.strip);
                if let (Some(s), Some(os)) = (track.synth.as_mut(), o.synth.as_ref()) {
                    s.adopt(os);
                }
            }
        }
        for aux in &mut self.auxes {
            if let Some(o) = old.auxes.iter_mut().find(|o| o.strip.key == aux.strip.key) {
                aux.strip.adopt(&mut o.strip);
            }
        }
        self.master.adopt(&mut old.master);
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
