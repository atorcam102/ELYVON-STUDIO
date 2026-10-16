//! Native insert processors and mixer building blocks (Phase 09).
//!
//! Everything here is built on the control thread (allocation happens in
//! `new`) and processed on the audio thread without allocating, locking or
//! performing IO. State that must survive a graph swap (filter memory,
//! limiter look-ahead, delay lines) is moved from the old instance with
//! `adopt`, which only swaps buffers — the replaced plan is freed later on
//! the control thread.

use elyvon_project::mixer::{FilterMode, Processor};

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
    /// RBJ cookbook 2nd-order high/low-pass.
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
        *l = dl * self.gain;
        *r = dr * self.gain;
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
    Filter(Biquad),
    Limiter(Box<Limiter>),
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
            Processor::Filter { mode, cutoff_hz, q } => {
                Kind::Filter(Biquad::new(mode, cutoff_hz, q, rate))
            }
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
        };
        Self {
            id,
            bypass,
            kind,
            fade: Fade::None,
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
            _ => 0,
        }
    }

    /// Takes over the running state of the same insert in the old graph.
    pub fn adopt(&mut self, old: &mut Insert) {
        self.fade = match (old.bypass, self.bypass) {
            (true, false) => Fade::In,
            (false, true) => Fade::Out,
            _ => Fade::None,
        };
        match (&mut self.kind, &mut old.kind) {
            (Kind::Trim(a), Kind::Trim(b)) => a.adopt(b),
            (Kind::Filter(a), Kind::Filter(b)) => a.z = b.z,
            (Kind::Limiter(a), Kind::Limiter(b)) => a.adopt(b),
            _ => {}
        }
    }

    /// Processes interleaved stereo `buf` in place. Real-time safe.
    pub fn process(&mut self, buf: &mut [f32]) {
        let fade = std::mem::replace(&mut self.fade, Fade::None);
        if fade == Fade::None {
            if !self.bypass {
                self.run(buf);
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
            self.run(piece);
            for (i, (w, d)) in piece.iter_mut().zip(&dry).enumerate() {
                let t = ((at + i) / 2 + 1) as f32 / n as f32;
                let wet = if fade == Fade::In { t } else { 1.0 - t };
                *w = *d + (*w - *d) * wet;
            }
            at = end;
        }
    }

    fn run(&mut self, buf: &mut [f32]) {
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
            Kind::Filter(b) => {
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
            }
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
            f.process(&mut low);
            let mut f = Insert::new(InsertId::new(), false, &p, rate);
            f.process(&mut high);
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
        lim.process(&mut x);
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
        bypassed.process(&mut y);
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
        u.process(&mut x);
        assert_eq!(x, [0.25, -0.5]);
        let mono = Processor::Utility {
            invert_left: false,
            invert_right: false,
            swap: false,
            mono: true,
        };
        let mut m = Insert::new(InsertId::new(), false, &mono, 48_000);
        let mut x = vec![1.0, 0.0];
        m.process(&mut x);
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
