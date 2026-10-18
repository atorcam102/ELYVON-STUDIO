//! Convolution reverb: uniformly partitioned overlap-save convolution
//! (frequency-domain delay line) with impulse responses synthesised by
//! Elyvon Studio itself.
//!
//! # Impulse responses
//!
//! No third-party IR files are shipped. Each space is generated
//! deterministically from a fixed seed: sparse early reflections, then
//! exponentially decaying noise whose brightness falls over time; left and
//! right use independent noise. The responses are therefore original and
//! redistributable with the application. They are cached per
//! (space, length, rate) on the control thread.
//!
//! # Latency and tail
//!
//! Audio is processed in partitions of [`PARTITION`] frames, which is the
//! reported (and compensated) latency; the dry signal inside the effect is
//! delayed by the same amount. The tail is the IR length (0.2–6 s). In
//! economy mode (engine overload) only the first second is convolved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use elyvon_project::effects::{EffectKind, ParamValues};
use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

use super::delay::{Line, balanced_mix};
use super::filters::{Biquad, Coefs};
use super::utility::Rng;
use super::{Bank, FxContext, Ramp};

/// Partition length in frames (also the effect's latency).
pub const PARTITION: usize = 256;
const FFT: usize = 2 * PARTITION;
const BINS: usize = PARTITION + 1;
const ECONOMY_SECONDS: f64 = 1.0;

mod cvp {
    pub const SPACE: usize = 0;
    pub const LENGTH: usize = 1;
    pub const PREDELAY: usize = 2;
    pub const LOW_CUT: usize = 3;
    pub const HIGH_CUT: usize = 4;
    pub const WIDTH: usize = 5;
    pub const MIX: usize = 6;
}

/// Character of a synthesised space.
#[derive(Debug, Clone, Copy)]
struct Space {
    /// Early reflections: count and spread (s).
    reflections: usize,
    early: f64,
    /// Onset of the diffuse tail (s).
    onset: f64,
    /// Brightness at the start and end of the tail (low-pass Hz).
    bright: (f64, f64),
    seed: u32,
}

const SPACES: [Space; 5] = [
    Space {
        reflections: 14,
        early: 0.025,
        onset: 0.006,
        bright: (12_000.0, 3_000.0),
        seed: 11,
    },
    Space {
        reflections: 18,
        early: 0.045,
        onset: 0.012,
        bright: (10_000.0, 2_500.0),
        seed: 23,
    },
    Space {
        reflections: 24,
        early: 0.080,
        onset: 0.022,
        bright: (9_000.0, 1_800.0),
        seed: 37,
    },
    Space {
        reflections: 0,
        early: 0.0,
        onset: 0.001,
        bright: (16_000.0, 6_000.0),
        seed: 41,
    },
    Space {
        reflections: 30,
        early: 0.140,
        onset: 0.040,
        bright: (7_000.0, 1_200.0),
        seed: 53,
    },
];

/// Synthesises the stereo impulse response of `space` (index into
/// `IR_SPACES`), `seconds` long, at `rate`. Deterministic.
pub fn synthesise_ir(space: usize, seconds: f64, rate: u32) -> [Vec<f32>; 2] {
    let sp = SPACES[space.min(SPACES.len() - 1)];
    let r = f64::from(rate);
    let len = ((seconds * r) as usize).max(PARTITION);
    let rt60 = seconds.max(0.1);
    let mut out = [vec![0.0f32; len], vec![0.0f32; len]];
    for (ch, ir) in out.iter_mut().enumerate() {
        let mut rng = Rng::new(
            sp.seed
                .wrapping_mul(2_654_435_761)
                .wrapping_add(ch as u32 * 7_919 + 1),
        );
        // Early reflections.
        for k in 0..sp.reflections {
            let t = 0.002 + sp.early * f64::from(rng.next() * 0.5 + 0.5).powf(0.7);
            let i = (t * r) as usize;
            if i < len {
                let g = 0.6 * (1.0 - k as f64 / sp.reflections as f64 * 0.6);
                ir[i] += (g * f64::from(rng.next().signum())) as f32;
            }
        }
        // Diffuse tail with a time-varying low-pass (brighter early).
        let mut z = 0.0f64;
        let onset = (sp.onset * r) as usize;
        for (i, v) in ir.iter_mut().enumerate().skip(onset) {
            let t = (i - onset) as f64 / r;
            let env = 10f64.powf(-3.0 * t / rt60);
            let ramp = (t / 0.01).min(1.0);
            let frac = (t / rt60).min(1.0);
            let cutoff = sp.bright.0 * (sp.bright.1 / sp.bright.0).powf(frac);
            let a = (-2.0 * std::f64::consts::PI * cutoff.min(r * 0.45) / r).exp();
            let noise = f64::from(rng.next());
            z = noise + (z - noise) * a;
            *v += (z * env * ramp * 0.8) as f32;
        }
        // Fade out the last 5 % so truncation never clicks.
        let fade = (len / 20).max(1);
        for k in 0..fade {
            ir[len - 1 - k] *= k as f32 / fade as f32;
        }
    }
    // Normalise both channels together to unit energy.
    let energy: f64 = out
        .iter()
        .flatten()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        / 2.0;
    let g = (1.0 / energy.max(1e-12).sqrt()) as f32;
    for ch in &mut out {
        for v in ch.iter_mut() {
            *v *= g;
        }
    }
    out
}

/// Frequency-domain partitions of a stereo impulse response.
#[derive(Debug, Clone)]
pub struct IrSpectra {
    pub partitions: usize,
    /// `[channel][partition * BINS + bin]`.
    data: [Vec<Complex<f32>>; 2],
}

impl IrSpectra {
    pub fn new(ir: &[Vec<f32>; 2]) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT);
        let parts = ir[0].len().div_ceil(PARTITION).max(1);
        let mut data = [
            vec![Complex::new(0.0, 0.0); parts * BINS],
            vec![Complex::new(0.0, 0.0); parts * BINS],
        ];
        let mut input = fft.make_input_vec();
        let mut spectrum = fft.make_output_vec();
        for (ch, d) in data.iter_mut().enumerate() {
            for p in 0..parts {
                input.fill(0.0);
                let from = p * PARTITION;
                let to = (from + PARTITION).min(ir[ch].len());
                input[..to - from].copy_from_slice(&ir[ch][from..to]);
                // Scale for the unnormalised inverse transform.
                for v in &mut input {
                    *v /= FFT as f32;
                }
                let _ = fft.process(&mut input, &mut spectrum);
                d[p * BINS..(p + 1) * BINS].copy_from_slice(&spectrum);
            }
        }
        Self {
            partitions: parts,
            data,
        }
    }
}

type CacheKey = (usize, u64, u32);

fn cache() -> &'static Mutex<HashMap<CacheKey, Arc<IrSpectra>>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<IrSpectra>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Control thread: the (cached) spectra of a built-in space.
pub fn spectra(space: usize, seconds: f64, rate: u32) -> Arc<IrSpectra> {
    let key = (space, (seconds * 1000.0).round() as u64, rate);
    let mut c = match cache().lock() {
        Ok(c) => c,
        Err(p) => p.into_inner(),
    };
    if let Some(s) = c.get(&key) {
        return Arc::clone(s);
    }
    if c.len() > 32 {
        c.clear();
    }
    let s = Arc::new(IrSpectra::new(&synthesise_ir(space, seconds, rate)));
    c.insert(key, Arc::clone(&s));
    s
}

/// Uniformly partitioned convolver for two channels. Real-time safe after
/// construction.
#[derive(Clone)]
pub struct Convolver {
    ir: Arc<IrSpectra>,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    /// Last two partitions of input per channel (overlap-save window).
    window: [Vec<f32>; 2],
    /// Frequency delay line: `partitions` input spectra per channel.
    fdl: [Vec<Complex<f32>>; 2],
    fdl_at: usize,
    acc: Vec<Complex<f32>>,
    time: Vec<f32>,
    scratch_fwd: Vec<Complex<f32>>,
    scratch_inv: Vec<Complex<f32>>,
    /// Input collected for the next partition and output of the last one.
    fill: usize,
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
}

impl std::fmt::Debug for Convolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Convolver")
            .field("partitions", &self.ir.partitions)
            .finish_non_exhaustive()
    }
}

impl Convolver {
    pub fn new(ir: Arc<IrSpectra>) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FFT);
        let inv = planner.plan_fft_inverse(FFT);
        let parts = ir.partitions;
        Self {
            scratch_fwd: fwd.make_scratch_vec(),
            scratch_inv: inv.make_scratch_vec(),
            fwd,
            inv,
            window: [vec![0.0; FFT], vec![0.0; FFT]],
            fdl: [
                vec![Complex::new(0.0, 0.0); parts * BINS],
                vec![Complex::new(0.0, 0.0); parts * BINS],
            ],
            fdl_at: 0,
            acc: vec![Complex::new(0.0, 0.0); BINS],
            time: vec![0.0; FFT],
            fill: 0,
            input: [vec![0.0; PARTITION], vec![0.0; PARTITION]],
            output: [vec![0.0; PARTITION], vec![0.0; PARTITION]],
            ir,
        }
    }

    /// Takes over the running state when the IR is the same.
    pub fn adopt(&mut self, old: &mut Convolver) {
        if Arc::ptr_eq(&self.ir, &old.ir) {
            std::mem::swap(&mut self.window, &mut old.window);
            std::mem::swap(&mut self.fdl, &mut old.fdl);
            std::mem::swap(&mut self.input, &mut old.input);
            std::mem::swap(&mut self.output, &mut old.output);
            self.fdl_at = old.fdl_at;
            self.fill = old.fill;
        }
    }

    /// Pushes one stereo frame and returns the convolved frame from one
    /// partition earlier. `limit` caps the partitions used (economy mode).
    #[inline]
    pub fn push(&mut self, l: f32, r: f32, limit: usize) -> (f32, f32) {
        let out = (self.output[0][self.fill], self.output[1][self.fill]);
        self.input[0][self.fill] = l;
        self.input[1][self.fill] = r;
        self.fill += 1;
        if self.fill == PARTITION {
            self.fill = 0;
            self.partition(limit);
        }
        out
    }

    fn partition(&mut self, limit: usize) {
        let parts = self.ir.partitions;
        let used = limit.clamp(1, parts);
        self.fdl_at = (self.fdl_at + parts - 1) % parts;
        for ch in 0..2 {
            // Slide the window: [previous partition | new partition].
            let w = &mut self.window[ch];
            w.copy_within(PARTITION.., 0);
            w[PARTITION..].copy_from_slice(&self.input[ch]);
            self.time.copy_from_slice(w);
            let slot = &mut self.fdl[ch][self.fdl_at * BINS..(self.fdl_at + 1) * BINS];
            let _ = self
                .fwd
                .process_with_scratch(&mut self.time, slot, &mut self.scratch_fwd);
            self.acc.fill(Complex::new(0.0, 0.0));
            let h = &self.ir.data[ch];
            for p in 0..used {
                let x = (self.fdl_at + p) % parts;
                let xs = &self.fdl[ch][x * BINS..(x + 1) * BINS];
                let hs = &h[p * BINS..(p + 1) * BINS];
                for ((a, x), h) in self.acc.iter_mut().zip(xs).zip(hs) {
                    *a += x * h;
                }
            }
            self.acc[0].im = 0.0;
            self.acc[BINS - 1].im = 0.0;
            let _ =
                self.inv
                    .process_with_scratch(&mut self.acc, &mut self.time, &mut self.scratch_inv);
            // Overlap-save: the second half is the valid linear convolution.
            for (o, t) in self.output[ch].iter_mut().zip(&self.time[PARTITION..]) {
                *o = super::flush(*t);
            }
        }
    }

    pub fn partitions(&self) -> usize {
        self.ir.partitions
    }
}

#[derive(Debug, Clone)]
pub struct ConvolutionReverb {
    pub bank: Bank,
    pub out_peak: f32,
    rate: u32,
    conv: Convolver,
    economy_parts: usize,
    predelay: Line,
    dry_delay: Line,
    low: Biquad,
    high: Biquad,
    filters: (f64, f64),
    mix: [Ramp; 2],
    width: Ramp,
}

impl ConvolutionReverb {
    pub fn new(p: &ParamValues, rate: u32) -> Self {
        let space = p.get(cvp::SPACE).round() as usize;
        let ir = spectra(space, p.get(cvp::LENGTH), rate);
        let mut c = Self {
            bank: Bank::new(EffectKind::ConvolutionReverb, p, rate),
            out_peak: 0.0,
            rate,
            economy_parts: ((ECONOMY_SECONDS * f64::from(rate)) as usize).div_ceil(PARTITION),
            conv: Convolver::new(ir),
            predelay: Line::new((0.21 * f64::from(rate)) as usize + 8),
            dry_delay: Line::new(PARTITION + 8),
            low: Biquad::default(),
            high: Biquad::default(),
            filters: (0.0, 0.0),
            mix: [Ramp::hold(0.0); 2],
            width: Ramp::hold((p.get(cvp::WIDTH) / 100.0) as f32),
        };
        c.mix = c.mix_gains().map(Ramp::hold);
        c
    }

    fn mix_gains(&self) -> [f32; 2] {
        let (d, w) = balanced_mix((self.bank.get(cvp::MIX) / 100.0) as f32);
        [d, w]
    }

    pub fn latency(&self) -> usize {
        PARTITION
    }

    pub fn cost(&self) -> f32 {
        // Per frame: two FFT pairs amortised plus the spectral products.
        8.0 + self.conv.partitions() as f32 * BINS as f32 * 4.0 / PARTITION as f32
    }

    pub fn adopt(&mut self, old: &mut ConvolutionReverb) {
        self.bank.adopt(&old.bank);
        self.conv.adopt(&mut old.conv);
        self.predelay.adopt(&mut old.predelay);
        self.dry_delay.adopt(&mut old.dry_delay);
        self.low.set_state(old.low.state());
        self.high.set_state(old.high.state());
        self.mix = old.mix;
        self.width = old.width;
    }

    pub fn process(&mut self, buf: &mut [f32], cx: &FxContext<'_>) {
        let frames = buf.len() / 2;
        let rate = f64::from(self.rate);
        let limit = if cx.economy {
            self.economy_parts
        } else {
            usize::MAX
        };
        for ck in cx.chunks(frames) {
            let (off, n) = (ck.off, ck.n);
            let fresh = self.bank.enter(&ck);
            let b = &self.bank;
            let (lo, hi) = (b.get(cvp::LOW_CUT), b.get(cvp::HIGH_CUT));
            if (lo, hi) != self.filters {
                let q = std::f64::consts::FRAC_1_SQRT_2;
                self.low.c = if lo > 20.5 {
                    Coefs::high_pass(lo, q, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.high.c = if hi < 19_900.0 {
                    Coefs::low_pass(hi, q, self.rate)
                } else {
                    Coefs::IDENTITY
                };
                self.filters = (lo, hi);
            }
            let pre = (b.get(cvp::PREDELAY) * 0.001 * rate).max(2.0) as f32;
            let width_to = (b.get(cvp::WIDTH) / 100.0) as f32;
            let target = self.mix_gains();
            let dry = self.mix[0].step(fresh, target[0]);
            let wet = self.mix[1].step(fresh, target[1]);
            let width = self.width.step(fresh, width_to);
            for (i, f) in buf[off * 2..(off + n) * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                // Reads come before writes: a read of d frames is d frames old.
                let (dl, dr) = (
                    self.dry_delay.read_linear(PARTITION as f32, 0),
                    self.dry_delay.read_linear(PARTITION as f32, 1),
                );
                self.dry_delay.write([f[0], f[1]]);
                let (pl, pr) = if pre <= 2.0 {
                    (f[0], f[1])
                } else {
                    (
                        self.predelay.read_linear(pre, 0),
                        self.predelay.read_linear(pre, 1),
                    )
                };
                self.predelay.write([f[0], f[1]]);
                let (l, r) = self.conv.push(pl, pr, limit);
                let l = self.high.run(0, self.low.run(0, f64::from(l))) as f32;
                let r = self.high.run(1, self.low.run(1, f64::from(r))) as f32;
                let j = ck.idx(i);
                let w = width.at(j, ck.len);
                let m = 0.5 * (l + r);
                let s = 0.5 * (l - r) * w;
                let (dg, wg) = (dry.at(j, ck.len), wet.at(j, ck.len));
                f[0] = dl * dg + (m + s) * wg;
                f[1] = dr * dg + (m - s) * wg;
            }
            self.low.flush();
            self.high.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Effect;
    use super::super::testing::*;
    use super::*;

    /// Partitioned convolution equals direct time-domain convolution.
    #[test]
    fn partitioned_convolution_matches_direct_convolution() {
        let mut rng = Rng::new(99);
        let ir_len = PARTITION * 5 + 37;
        let ir = [
            (0..ir_len).map(|_| rng.next() * 0.1).collect::<Vec<f32>>(),
            (0..ir_len).map(|_| rng.next() * 0.1).collect::<Vec<f32>>(),
        ];
        let mut conv = Convolver::new(Arc::new(IrSpectra::new(&ir)));
        let n = PARTITION * 9;
        let x: Vec<[f32; 2]> = (0..n).map(|_| [rng.next(), rng.next()]).collect();
        let out: Vec<(f32, f32)> = x
            .iter()
            .map(|f| conv.push(f[0], f[1], usize::MAX))
            .collect();
        let mut worst = 0.0f32;
        for (t, o) in out.iter().enumerate().skip(PARTITION) {
            // Output at t is the convolution at t − PARTITION.
            let tt = t - PARTITION;
            for ch in 0..2 {
                let mut y = 0.0f64;
                for k in 0..ir_len.min(tt + 1) {
                    y += f64::from(ir[ch][k]) * f64::from(x[tt - k][ch]);
                }
                let got = if ch == 0 { o.0 } else { o.1 };
                worst = worst.max((got - y as f32).abs());
            }
        }
        assert!(worst < 2e-4, "max error {worst}");
    }

    #[test]
    fn reported_latency_aligns_dry_and_wet() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::ConvolutionReverb,
            &[("mix", 0.0), ("lengthSeconds", 0.3)],
        );
        let mut fx = Effect::new(&s, rate);
        assert_eq!(fx.latency(), PARTITION);
        assert_eq!(super::super::effect_latency(&s), PARTITION);
        let mut x = vec![0.0f32; 2_000 * 2];
        x[100 * 2] = 1.0;
        let out = run(&mut fx, &x, rate, 64, &m);
        let at = out.chunks(2).position(|f| f[0] > 0.5).unwrap_or(0);
        assert_eq!(at, 100 + PARTITION);
    }

    #[test]
    fn impulse_responses_are_deterministic_and_decay() {
        let a = synthesise_ir(2, 2.0, 48_000);
        let b = synthesise_ir(2, 2.0, 48_000);
        assert_eq!(a, b);
        assert_ne!(a[0], a[1], "decorrelated channels");
        let early = rms(&a[0][..24_000]);
        let late = rms(&a[0][72_000..]);
        assert!(late < early * 0.05, "{early} {late}");
        for space in 0..5 {
            let ir = synthesise_ir(space, 1.0, 44_100);
            assert!(ir.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn economy_mode_shortens_the_tail() {
        let rate = 48_000;
        let m = map(120.0);
        let s = slot(
            EffectKind::ConvolutionReverb,
            &[("mix", 100.0), ("lengthSeconds", 4.0), ("space", 4.0)],
        );
        let mut x = vec![0.0f32; 3 * rate as usize * 2];
        x[0] = 1.0;
        x[1] = 1.0;
        let full = run(&mut Effect::new(&s, rate), &x, rate, 128, &m);
        let mut fx = Effect::new(&s, rate);
        let mut eco = x.clone();
        let mut pos = 0;
        for piece in eco.chunks_mut(256) {
            let c = FxContext {
                economy: true,
                ..cx(&m, rate, pos)
            };
            fx.process(piece, &c);
            pos += (piece.len() / 2) as u64;
        }
        let late = |v: &[f32]| rms(&v[(3 * rate as usize / 2) * 2..]);
        assert!(late(&full) > 1e-4);
        assert!(late(&eco) < late(&full) * 0.01);
    }
}
