//! Native insert processors and mixer building blocks (Phase 09), and the
//! insert wrapper of the Phase 12–13 effects ([`crate::fx`]).
//!
//! Everything here is built on the control thread (allocation happens in
//! `new`) and processed on the audio thread without allocating, locking or
//! performing IO. State that must survive a graph swap (filter memory,
//! limiter look-ahead, delay lines) is moved from the old instance with
//! `adopt`, which only swaps buffers — the replaced plan is freed later on
//! the control thread.

use elyvon_project::automation::InsertParam;
use elyvon_project::mixer::{FilterMode, Processor};

use crate::fx::{Effect, FxContext};
use crate::gain::db_to_gain;

/// Linear gain pair that ramps to a new target over one block, so fader,
/// pan and send changes do not click. A freshly built value starts at its
/// target, which keeps renders without parameter changes bit-exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Smoothed {
    pub current: (f32, f32),
    pub target: (f32, f32),
}

impl Smoothed {
    pub fn new(target: (f32, f32)) -> Self {
        Self {
            current: target,
            target,
        }
    }

    /// Continues from the previously heard value.
    pub fn adopt(&mut self, old: &Smoothed) {
        self.current = old.current;
    }

    pub fn is_steady(&self) -> bool {
        self.current == self.target
    }

    /// Gain for frame `i` of an `n`-frame block (linear ramp).
    #[inline]
    pub fn at(&self, i: usize, n: usize) -> (f32, f32) {
        if self.is_steady() {
            return self.target;
        }
        let t = (i + 1) as f32 / n.max(1) as f32;
        (
            self.current.0 + (self.target.0 - self.current.0) * t,
            self.current.1 + (self.target.1 - self.current.1) * t,
        )
    }

    /// Ends the block: the ramp has reached its target.
    #[inline]
    pub fn settle(&mut self) {
        self.current = self.target;
    }

    pub fn is_silent(&self) -> bool {
        self.current == (0.0, 0.0) && self.target == (0.0, 0.0)
    }
}

/// Stereo delay line with a fixed length (path latency compensation).
#[derive(Debug, Clone)]
pub struct StereoDelay {
    buffer: Vec<[f32; 2]>,
    at: usize,
}

impl StereoDelay {
    pub fn new(frames: usize) -> Self {
        Self {
            buffer: vec![[0.0; 2]; frames],
            at: 0,
        }
    }

    pub fn frames(&self) -> usize {
        self.buffer.len()
    }

    /// Keeps the delayed audio when the length is unchanged.
    pub fn adopt(&mut self, old: &mut StereoDelay) {
        if old.buffer.len() == self.buffer.len() {
            std::mem::swap(&mut self.buffer, &mut old.buffer);
            self.at = old.at;
        }
    }

    /// Delays interleaved stereo `buf` in place.
    #[inline]
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.buffer.is_empty() {
            return;
        }
        for frame in buf.as_chunks_mut::<2>().0.iter_mut() {
            let out = self.buffer[self.at];
            self.buffer[self.at] = [frame[0], frame[1]];
            frame[0] = out[0];
            frame[1] = out[1];
            self.at += 1;
            if self.at == self.buffer.len() {
                self.at = 0;
            }
        }
    }

    pub fn clear(&mut self) {
        self.buffer.fill([0.0; 2]);
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    /// Transposed direct form II state per channel.
    z: [[f32; 2]; 2],
}

impl Biquad {
    /// RBJ cookbook 2nd-order high/low-pass, band-pass and notch.
    fn new(mode: FilterMode, cutoff: f64, q: f64, rate: u32) -> Self {
        let nyquist = f64::from(rate) * 0.5;
        let f = cutoff.clamp(10.0, nyquist * 0.95);
        let w = 2.0 * std::f64::consts::PI * f / f64::from(rate);
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match mode {
            FilterMode::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            FilterMode::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
            FilterMode::BandPass => (alpha, 0.0, -alpha),
            FilterMode::Notch => (1.0, -2.0 * cos, 1.0),
        };
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (-2.0 * cos / a0) as f32,
            a2: ((1.0 - alpha) / a0) as f32,
            z: [[0.0; 2]; 2],
        }
    }

    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b0 * x + z[0];
        z[0] = self.b1 * x - self.a1 * y + z[1];
        z[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Look-ahead peak limiter: the gain needed by every sample in the window is
/// applied before the sample leaves the delay line, so peaks never pass the
/// ceiling. The look-ahead is the processor's latency.
#[derive(Debug, Clone)]
struct Limiter {
    ceiling: f32,
    release: f32,
    delay: Vec<[f32; 2]>,
    at: usize,
    /// Monotonic queue of (sample index, required gain) for the sliding minimum.
    queue: Vec<(u64, f32)>,
    head: usize,
    len: usize,
    index: u64,
    gain: f32,
    /// Automated bypass amount: 0 limits, 1 passes the delayed signal.
    pass: f32,
    /// Largest gain reduction (dB) since the meter last read it.
    reduction: f32,
}

impl Limiter {
    fn new(ceiling_db: f64, lookahead_ms: f64, release_ms: f64, rate: u32) -> Self {
        let look = (lookahead_ms * f64::from(rate) / 1000.0).round() as usize;
        let release_frames = (release_ms * f64::from(rate) / 1000.0).max(1.0);
        Self {
            ceiling: db_to_gain(ceiling_db) as f32,
            release: (1.0 - (-1.0 / release_frames).exp()) as f32,
            delay: vec![[0.0; 2]; look],
            at: 0,
            queue: vec![(0, 1.0); look + 1],
            head: 0,
            len: 0,
            index: 0,
            gain: 1.0,
            pass: 0.0,
            reduction: 0.0,
        }
    }

    fn latency(&self) -> usize {
        self.delay.len()
    }

    fn adopt(&mut self, old: &mut Limiter) {
        if old.delay.len() == self.delay.len() {
            std::mem::swap(&mut self.delay, &mut old.delay);
            std::mem::swap(&mut self.queue, &mut old.queue);
            self.at = old.at;
            self.head = old.head;
            self.len = old.len;
            self.index = old.index;
            self.gain = old.gain;
        }
    }

    #[inline]
    fn push(&mut self, g: f32) {
        let cap = self.queue.len();
        // Drop larger-or-equal requirements from the back: they can never be
        // the minimum again.
        while self.len > 0 {
            let back = (self.head + self.len - 1) % cap;
            if self.queue[back].1 >= g {
                self.len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.head + self.len) % cap;
        self.queue[slot] = (self.index, g);
        self.len += 1;
        let window = self.delay.len() as u64;
        while self.len > 0 && self.queue[self.head].0 + window < self.index {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
    }

    #[inline]
    fn process(&mut self, l: &mut f32, r: &mut f32) {
        let peak = l.abs().max(r.abs());
        let needed = if peak > self.ceiling {
            self.ceiling / peak
        } else {
            1.0
        };
        self.push(needed);
        let target = self.queue[self.head].1;
        // Instant attack (the look-ahead already saw the peak), smooth release.
        self.gain = if target < self.gain {
            target
        } else {
            self.gain + (target - self.gain) * self.release
        };
        let (dl, dr) = if self.delay.is_empty() {
            (*l, *r)
        } else {
            let out = self.delay[self.at];
            self.delay[self.at] = [*l, *r];
            self.at = (self.at + 1) % self.delay.len();
            (out[0], out[1])
        };
        let g = self.gain + (1.0 - self.gain) * self.pass;
        *l = dl * g;
        *r = dr * g;
        self.index += 1;
    }
}

#[derive(Debug, Clone)]
enum Kind {
    Trim(Smoothed),
    Utility {
        invert: (bool, bool),
        swap: bool,
        mono: bool,
    },
    Filter {
        biquad: Biquad,
        mode: FilterMode,
        cutoff: f64,
        q: f64,
    },
    Limiter(Box<Limiter>),
    Effect(Box<Effect>),
}

/// Crossfade between the dry and processed signal over one block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fade {
    None,
    In,
    Out,
}

/// A prepared insert processor.
#[derive(Debug, Clone)]
pub struct Insert {
    pub id: elyvon_project::mixer::InsertId,
    pub bypass: bool,
    kind: Kind,
    fade: Fade,
    rate: u32,
    /// Bypass is automated: the processor always runs and its output is
    /// blended with the dry signal by `amount` (0 processed … 1 dry).
    automated_bypass: bool,
    amount: f32,
    amount_target: f32,
    /// Key-signal slot of an external sidechain (see `plan::PlaybackPlan::keys`).
    pub key_slot: Option<usize>,
}

impl Insert {
    pub fn new(
        id: elyvon_project::mixer::InsertId,
        bypass: bool,
        p: &Processor,
        rate: u32,
    ) -> Self {
        let kind = match *p {
            Processor::Trim { gain_db } => {
                let g = db_to_gain(gain_db) as f32;
                Kind::Trim(Smoothed::new((g, g)))
            }
            Processor::Utility {
                invert_left,
                invert_right,
                swap,
                mono,
            } => Kind::Utility {
                invert: (invert_left, invert_right),
                swap,
                mono,
            },
            Processor::Filter { mode, cutoff_hz, q } => Kind::Filter {
                biquad: Biquad::new(mode, cutoff_hz, q, rate),
                mode,
                cutoff: cutoff_hz,
                q,
            },
            Processor::Limiter {
                ceiling_db,
                lookahead_ms,
                release_ms,
            } => Kind::Limiter(Box::new(Limiter::new(
                ceiling_db,
                lookahead_ms,
                release_ms,
                rate,
            ))),
            Processor::Effect(ref slot) => Kind::Effect(Box::new(Effect::new(slot, rate))),
        };
        Self {
            id,
            bypass,
            kind,
            fade: Fade::None,
            rate,
            automated_bypass: false,
            amount: f32::from(u8::from(bypass)),
            amount_target: f32::from(u8::from(bypass)),
            key_slot: None,
        }
    }

    /// Lets automation drive the bypass as a continuous dry/processed blend
    /// (the look-ahead latency is then always present).
    pub fn set_bypass_automated(&mut self) {
        self.automated_bypass = true;
        self.bypass = false;
    }

    /// Sets an automated parameter for the next processed span. Real-time
    /// safe: filter coefficients are recomputed in place, state is kept.
    pub fn automate(&mut self, param: InsertParam, value: f32) {
        match (param, &mut self.kind) {
            (InsertParam::Bypass, _) => self.amount_target = value.clamp(0.0, 1.0),
            (InsertParam::TrimGain, Kind::Trim(g)) => {
                let v = db_to_gain(f64::from(value)) as f32;
                g.target = (v, v);
            }
            (
                InsertParam::FilterCutoff | InsertParam::FilterQ,
                Kind::Filter {
                    biquad,
                    mode,
                    cutoff,
                    q,
                },
            ) => {
                let v = f64::from(value);
                let changed = if param == InsertParam::FilterCutoff {
                    std::mem::replace(cutoff, v) != v
                } else {
                    std::mem::replace(q, v) != v
                };
                if changed {
                    let z = biquad.z;
                    *biquad = Biquad::new(*mode, *cutoff, *q, self.rate);
                    biquad.z = z;
                }
            }
            (InsertParam::LimiterCeiling, Kind::Limiter(l)) => {
                l.ceiling = db_to_gain(f64::from(value)) as f32;
            }
            (InsertParam::LimiterRelease, Kind::Limiter(l)) => {
                let frames = (f64::from(value) * f64::from(self.rate) / 1000.0).max(1.0);
                l.release = (1.0 - (-1.0 / frames).exp()) as f32;
            }
            (InsertParam::Effect { kind, index }, Kind::Effect(e)) if e.kind() == kind => {
                e.set(usize::from(index), f64::from(value));
            }
            _ => {}
        }
    }

    /// A processor added while audio runs fades in instead of switching on.
    pub fn mark_new(&mut self) {
        if !self.bypass {
            self.fade = Fade::In;
        }
    }

    /// Frames of delay this processor adds (0 when bypassed).
    pub fn latency(&self) -> usize {
        match &self.kind {
            Kind::Limiter(l) if !self.bypass => l.latency(),
            Kind::Effect(e) if !self.bypass => e.latency(),
            _ => 0,
        }
    }

    /// The processor may still produce sound with silent input (delay and
    /// reverb tails, a test generator): its strip keeps running while the
    /// transport is stopped.
    pub fn ringing(&self) -> bool {
        match &self.kind {
            Kind::Effect(e) if !self.bypass => {
                e.ringing() || e.kind() == elyvon_project::effects::EffectKind::TestGenerator
            }
            _ => false,
        }
    }

    /// Gain reduction (dB) since the previous call, for dynamics processors
    /// and the limiter.
    pub fn take_reduction(&mut self) -> Option<f32> {
        match &mut self.kind {
            Kind::Effect(e) => e.take_reduction(),
            Kind::Limiter(l) => Some(std::mem::take(&mut l.reduction)),
            _ => None,
        }
    }

    /// The prepared effect, if this insert is one (inspection and tests).
    pub fn effect(&self) -> Option<&Effect> {
        match &self.kind {
            Kind::Effect(e) => Some(e),
            _ => None,
        }
    }

    /// Takes over the running state of the same insert in the old graph.
    pub fn adopt(&mut self, old: &mut Insert) {
        self.fade = match (old.bypass, self.bypass) {
            _ if self.automated_bypass || old.automated_bypass => Fade::None,
            (true, false) => Fade::In,
            (false, true) => Fade::Out,
            _ => Fade::None,
        };
        if self.automated_bypass && old.automated_bypass {
            self.amount = old.amount;
        }
        match (&mut self.kind, &mut old.kind) {
            (Kind::Trim(a), Kind::Trim(b)) => a.adopt(b),
            (Kind::Filter { biquad: a, .. }, Kind::Filter { biquad: b, .. }) => a.z = b.z,
            (Kind::Limiter(a), Kind::Limiter(b)) => a.adopt(b),
            (Kind::Effect(a), Kind::Effect(b)) => a.adopt(b),
            _ => {}
        }
    }

    /// Processes interleaved stereo `buf` in place. Real-time safe.
    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        if self.automated_bypass {
            return self.process_blended(buf, cx);
        }
        let fade = std::mem::replace(&mut self.fade, Fade::None);
        if fade == Fade::None {
            if !self.bypass {
                self.run(buf, cx);
            }
            return;
        }
        // Mix dry and processed signal in small pieces (no allocation).
        let n = (buf.len() / 2).max(1);
        let mut dry = [0.0f32; 128];
        let mut at = 0;
        while at < buf.len() {
            let end = (at + dry.len()).min(buf.len());
            let piece = &mut buf[at..end];
            dry[..piece.len()].copy_from_slice(piece);
            self.run(piece, &cx.sub(at / 2, piece.len() / 2));
            for (i, (w, d)) in piece.iter_mut().zip(&dry).enumerate() {
                let t = ((at + i) / 2 + 1) as f32 / n as f32;
                let wet = if fade == Fade::In { t } else { 1.0 - t };
                *w = *d + (*w - *d) * wet;
            }
            at = end;
        }
    }

    /// Processes with per-frame automated trim gain (dB) and/or bypass
    /// amount (0 processed … 1 dry), so the result does not depend on how
    /// a span is split. Real-time safe.
    pub fn process_levels(
        &mut self,
        buf: &mut [f32],
        trim_db: Option<&[f32]>,
        bypass: Option<&[f32]>,
        cx: &FxContext<'_>,
    ) {
        if trim_db.is_none() && bypass.is_none() {
            return self.process(buf, cx);
        }
        self.fade = Fade::None;
        let at = |s: Option<&[f32]>, i: usize, default: f32| {
            s.and_then(|s| s.get(i)).copied().unwrap_or(default)
        };
        let static_amount = self.amount_target;
        if let Kind::Limiter(l) = &mut self.kind {
            for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                l.pass = at(bypass, i, static_amount).clamp(0.0, 1.0);
                let (a, b) = f.split_at_mut(1);
                l.process(&mut a[0], &mut b[0]);
            }
            self.amount = l.pass;
            return;
        }
        let mut dry = [0.0f32; 128];
        let mut start = 0;
        while start < buf.len() {
            let end = (start + dry.len()).min(buf.len());
            let piece = &mut buf[start..end];
            dry[..piece.len()].copy_from_slice(piece);
            match (&mut self.kind, trim_db) {
                (Kind::Trim(g), Some(levels)) => {
                    for (j, f) in piece.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                        let v = levels.get(start / 2 + j).copied().unwrap_or(0.0);
                        let k = db_to_gain(f64::from(v)) as f32;
                        f[0] *= k;
                        f[1] *= k;
                    }
                    let last = levels.get(end / 2 - 1).copied().unwrap_or(0.0);
                    let k = db_to_gain(f64::from(last)) as f32;
                    *g = Smoothed::new((k, k));
                }
                _ => self.run(piece, &cx.sub(start / 2, piece.len() / 2)),
            }
            if bypass.is_some() || self.automated_bypass {
                for (j, (w, d)) in piece.iter_mut().zip(&dry).enumerate() {
                    let x = at(bypass, (start + j) / 2, static_amount).clamp(0.0, 1.0);
                    *w += (*d - *w) * x;
                }
            }
            start = end;
        }
        if let Some(b) = bypass {
            self.amount = b.last().copied().unwrap_or(self.amount);
            self.amount_target = self.amount;
        }
    }

    /// Automated bypass: ramps the dry amount from its last value to the
    /// target over `buf`.
    fn process_blended(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let (from, to) = (self.amount, self.amount_target);
        self.amount = to;
        let n = (buf.len() / 2).max(1) as f32;
        if let Kind::Limiter(l) = &mut self.kind {
            // The dry path must stay behind the same look-ahead delay: blend
            // the limiter's gain towards unity instead of the signals.
            for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                l.pass = from + (to - from) * (i + 1) as f32 / n;
                let (a, b) = f.split_at_mut(1);
                l.process(&mut a[0], &mut b[0]);
            }
            return;
        }
        if from == 0.0 && to == 0.0 {
            return self.run(buf, cx);
        }
        let mut dry = [0.0f32; 128];
        let mut at = 0;
        while at < buf.len() {
            let end = (at + dry.len()).min(buf.len());
            let piece = &mut buf[at..end];
            dry[..piece.len()].copy_from_slice(piece);
            self.run(piece, &cx.sub(at / 2, piece.len() / 2));
            for (i, (w, d)) in piece.iter_mut().zip(&dry).enumerate() {
                let x = from + (to - from) * ((at + i) / 2 + 1) as f32 / n;
                *w += (*d - *w) * x;
            }
            at = end;
        }
    }

    fn run(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let n = buf.len() / 2;
        match &mut self.kind {
            Kind::Trim(g) => {
                for (i, f) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    let (a, _) = g.at(i, n);
                    f[0] *= a;
                    f[1] *= a;
                }
                g.settle();
            }
            Kind::Utility { invert, swap, mono } => {
                for f in buf.as_chunks_mut::<2>().0.iter_mut() {
                    let (mut l, mut r) = (f[0], f[1]);
                    if invert.0 {
                        l = -l;
                    }
                    if invert.1 {
                        r = -r;
                    }
                    if *swap {
                        std::mem::swap(&mut l, &mut r);
                    }
                    if *mono {
                        let m = 0.5 * (l + r);
                        l = m;
                        r = m;
                    }
                    f[0] = l;
                    f[1] = r;
                }
            }
            Kind::Filter { biquad: b, .. } => {
                for f in buf.as_chunks_mut::<2>().0.iter_mut() {
                    f[0] = b.run(0, f[0]);
                    f[1] = b.run(1, f[1]);
                }
                // Denormal guard: tiny feedback values cost CPU on x86.
                for z in &mut b.z {
                    for v in z.iter_mut() {
                        if v.abs() < 1e-20 {
                            *v = 0.0;
                        }
                    }
                }
            }
            Kind::Limiter(l) => {
                for f in buf.as_chunks_mut::<2>().0.iter_mut() {
                    let (a, b) = f.split_at_mut(1);
                    l.process(&mut a[0], &mut b[0]);
                }
                let gr = -20.0 * l.gain.max(1e-6).log10();
                l.reduction = l.reduction.max(gr);
            }
            Kind::Effect(e) => e.process(buf, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elyvon_project::mixer::InsertId;

    fn sine(freq: f64, rate: u32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let v = amp
                    * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32;
                [v, v]
            })
            .collect()
    }

    fn run(insert: &mut Insert, x: &mut [f32]) {
        let map = crate::fx::testing::map(120.0);
        insert.process(x, &crate::fx::testing::cx(&map, 48_000, 0));
    }

    #[test]
    fn band_pass_and_notch_filters() {
        let rate = 48_000;
        for (mode, centre_kept) in [(FilterMode::BandPass, true), (FilterMode::Notch, false)] {
            let p = Processor::Filter {
                mode,
                cutoff_hz: 1000.0,
                q: 2.0,
            };
            let at = |freq: f64| {
                let mut f = Insert::new(InsertId::new(), false, &p, rate);
                let mut x = sine(freq, rate, 48_000, 0.5);
                run(&mut f, &mut x);
                rms(&x[24_000..]) / (0.5 / 2f32.sqrt())
            };
            let (centre, far) = (at(1000.0), at(100.0));
            if centre_kept {
                assert!(
                    (centre - 1.0).abs() < 0.02 && far < 0.1,
                    "bp {centre} {far}"
                );
            } else {
                assert!(
                    centre < 0.02 && (far - 1.0).abs() < 0.02,
                    "notch {centre} {far}"
                );
            }
        }
    }

    #[test]
    fn effects_wrap_into_inserts_and_report_latency_and_reduction() {
        use elyvon_project::effects::{EffectKind, EffectSlot};
        let mut conv = EffectSlot::new(EffectKind::ConvolutionReverb);
        conv.set("lengthSeconds", 0.3).expect("key");
        let ins = Insert::new(InsertId::new(), false, &Processor::Effect(conv), 48_000);
        assert_eq!(ins.latency(), crate::fx::convolution::PARTITION);
        let bypassed = Insert::new(InsertId::new(), true, &Processor::Effect(conv), 48_000);
        assert_eq!(bypassed.latency(), 0);
        let mut comp = EffectSlot::new(EffectKind::Compressor);
        comp.set("thresholdDb", -40.0).expect("key");
        let mut c = Insert::new(InsertId::new(), false, &Processor::Effect(comp), 48_000);
        let mut x = sine(200.0, 48_000, 4_800, 0.5);
        run(&mut c, &mut x);
        assert!(c.take_reduction().expect("dynamics") > 10.0);
        // Automation reaches the effect's parameter.
        c.automate(
            InsertParam::Effect {
                kind: EffectKind::Compressor,
                index: 0,
            },
            0.0,
        );
        let mut x = sine(200.0, 48_000, 48_000, 0.1);
        run(&mut c, &mut x);
        let _ = c.take_reduction();
        let mut x = sine(200.0, 48_000, 4_800, 0.1);
        run(&mut c, &mut x);
        assert!(c.take_reduction().expect("dynamics") < 0.5);
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn smoothing_ramps_once_and_settles() {
        let mut s = Smoothed::new((1.0, 1.0));
        s.target = (0.0, 0.0);
        assert_eq!(s.at(0, 4), (0.75, 0.75));
        assert_eq!(s.at(3, 4), (0.0, 0.0));
        s.settle();
        assert!(s.is_steady() && s.is_silent());
    }

    #[test]
    fn filters_pass_and_stop_as_expected() {
        let rate = 48_000;
        let lp = Processor::Filter {
            mode: FilterMode::LowPass,
            cutoff_hz: 1000.0,
            q: 0.707,
        };
        let hp = Processor::Filter {
            mode: FilterMode::HighPass,
            cutoff_hz: 1000.0,
            q: 0.707,
        };
        for (p, low_kept) in [(lp, true), (hp, false)] {
            let mut f = Insert::new(InsertId::new(), false, &p, rate);
            let mut low = sine(100.0, rate, 48_000, 0.5);
            let mut high = sine(10_000.0, rate, 48_000, 0.5);
            run(&mut f, &mut low);
            let mut f = Insert::new(InsertId::new(), false, &p, rate);
            run(&mut f, &mut high);
            let (l, h) = (rms(&low[9600..]), rms(&high[9600..]));
            let pass = 0.5 / 2f32.sqrt();
            if low_kept {
                assert!((l - pass).abs() < 0.01 && h < 0.01, "lp {l} {h}");
            } else {
                assert!(l < 0.01 && (h - pass).abs() < 0.01, "hp {l} {h}");
            }
        }
    }

    #[test]
    fn limiter_holds_the_ceiling_and_reports_its_lookahead() {
        let rate = 48_000;
        let p = Processor::Limiter {
            ceiling_db: -6.0,
            lookahead_ms: 5.0,
            release_ms: 50.0,
        };
        let mut lim = Insert::new(InsertId::new(), false, &p, rate);
        assert_eq!(lim.latency(), 240);
        let mut x = sine(440.0, rate, 9600, 1.0);
        // An isolated full-scale spike.
        x[2 * 5000] = 1.0;
        let input = x.clone();
        run(&mut lim, &mut x);
        let ceiling = db_to_gain(-6.0) as f32;
        assert!(x.iter().all(|v| v.abs() <= ceiling + 1e-6));
        // Delayed by exactly the look-ahead: the output tracks the input
        // shifted by 240 frames (scaled by the gain).
        let lag = |d: usize| -> f32 {
            (0..4000)
                .map(|i| x[2 * (i + 3000 + d)] * input[2 * (i + 3000)])
                .sum()
        };
        assert!(lag(240) > lag(239) && lag(240) > lag(241));
        let mut bypassed = Insert::new(InsertId::new(), true, &p, rate);
        assert_eq!(bypassed.latency(), 0);
        let mut y = input.clone();
        run(&mut bypassed, &mut y);
        assert_eq!(y, input);
    }

    #[test]
    fn utility_inverts_swaps_and_sums() {
        let p = Processor::Utility {
            invert_left: true,
            invert_right: false,
            swap: true,
            mono: false,
        };
        let mut u = Insert::new(InsertId::new(), false, &p, 48_000);
        let mut x = vec![0.5, 0.25];
        run(&mut u, &mut x);
        assert_eq!(x, [0.25, -0.5]);
        let mono = Processor::Utility {
            invert_left: false,
            invert_right: false,
            swap: false,
            mono: true,
        };
        let mut m = Insert::new(InsertId::new(), false, &mono, 48_000);
        let mut x = vec![1.0, 0.0];
        run(&mut m, &mut x);
        assert_eq!(x, [0.5, 0.5]);
    }

    #[test]
    fn delay_line_shifts_by_its_length() {
        let mut d = StereoDelay::new(3);
        let mut x: Vec<f32> = (1..=6).flat_map(|v| [v as f32, -(v as f32)]).collect();
        d.process(&mut x);
        assert_eq!(x, [0., 0., 0., 0., 0., 0., 1., -1., 2., -2., 3., -3.]);
    }
}
