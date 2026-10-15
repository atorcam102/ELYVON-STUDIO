//! Multi-resolution min/max peaks for waveform drawing.
//!
//! Built once per decoded file on a worker thread. Level `k` stores, for
//! every bucket of `BASE × 4^k` source frames, the minimum and maximum of each
//! displayed channel quantised to `i16`. A query for any frame range picks the
//! coarsest level that still has several buckets in it, so scrolling and
//! zooming cost O(columns) regardless of file length.

/// Frames per bucket at the finest level.
pub const BASE: u64 = 256;
const FACTOR: u64 = 4;

#[derive(Debug, Clone)]
struct Level {
    bucket: u64,
    /// `[min, max]` per displayed channel per bucket.
    data: Vec<i16>,
}

#[derive(Debug, Clone)]
pub struct PeakPyramid {
    channels: usize,
    frames: u64,
    levels: Vec<Level>,
}

fn quantise(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

pub fn dequantise(v: i16) -> f32 {
    f32::from(v) / 32767.0
}

impl PeakPyramid {
    /// Builds peaks for up to two channels of `planes` (source rate).
    pub fn build(planes: &[Vec<f32>]) -> Self {
        let channels = planes.len().clamp(1, 2);
        let frames = planes.first().map_or(0, Vec::len) as u64;
        let mut levels = Vec::new();
        let base_buckets = frames.div_ceil(BASE).max(1) as usize;
        let mut data = vec![0i16; base_buckets * channels * 2];
        for ch in 0..channels {
            let plane = planes.get(ch).map_or(&[][..], Vec::as_slice);
            for (b, chunk) in plane.chunks(BASE as usize).enumerate() {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for &s in chunk {
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
                let i = (b * channels + ch) * 2;
                data[i] = quantise(lo);
                data[i + 1] = quantise(hi);
            }
        }
        levels.push(Level { bucket: BASE, data });
        while levels
            .last()
            .is_some_and(|l| l.data.len() / (channels * 2) > 1)
        {
            let prev = &levels[levels.len() - 1];
            let n = prev.data.len() / (channels * 2);
            let m = n.div_ceil(FACTOR as usize);
            let mut data = vec![0i16; m * channels * 2];
            for b in 0..m {
                for ch in 0..channels {
                    let (mut lo, mut hi) = (i16::MAX, i16::MIN);
                    for k in b * FACTOR as usize..((b + 1) * FACTOR as usize).min(n) {
                        let i = (k * channels + ch) * 2;
                        lo = lo.min(prev.data[i]);
                        hi = hi.max(prev.data[i + 1]);
                    }
                    let i = (b * channels + ch) * 2;
                    data[i] = lo;
                    data[i + 1] = hi;
                }
            }
            let bucket = prev.bucket * FACTOR;
            levels.push(Level { bucket, data });
        }
        Self {
            channels,
            frames,
            levels,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Bytes held by all levels.
    pub fn memory_bytes(&self) -> usize {
        self.levels.iter().map(|l| l.data.len() * 2).sum()
    }

    /// Min/max of `[start, end)` source frames per channel, using whole
    /// buckets. Returns `None` when the range is finer than [`BASE`]; callers
    /// then read samples directly.
    pub fn range(&self, start: u64, end: u64, out: &mut [(f32, f32); 2]) -> bool {
        let end = end.min(self.frames);
        if start >= end {
            *out = [(0.0, 0.0); 2];
            return true;
        }
        let len = end - start;
        if len < BASE {
            return false;
        }
        // Coarsest level with at least two buckets inside the range.
        let level = self
            .levels
            .iter()
            .rev()
            .find(|l| l.bucket * 2 <= len)
            .unwrap_or(&self.levels[0]);
        let first = start / level.bucket;
        let last = (end - 1) / level.bucket;
        for (ch, slot) in out.iter_mut().enumerate().take(self.channels) {
            let (mut lo, mut hi) = (i16::MAX, i16::MIN);
            for b in first..=last {
                let i = (b as usize * self.channels + ch) * 2;
                lo = lo.min(level.data[i]);
                hi = hi.max(level.data[i + 1]);
            }
            *slot = (dequantise(lo), dequantise(hi));
        }
        if self.channels == 1 {
            out[1] = out[0];
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(plane: &[f32], a: usize, b: usize) -> (f32, f32) {
        plane[a..b]
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)))
    }

    #[test]
    fn coarse_levels_bound_the_exact_extremes() {
        // Deterministic noise-like signal with a single spike.
        let mut state = 12345u32;
        let mut plane: Vec<f32> = (0..200_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 24) as f32 * 0.5 - 0.25
            })
            .collect();
        plane[123_456] = 0.9;
        let p = PeakPyramid::build(&[plane.clone()]);
        let mut out = [(0.0, 0.0); 2];
        for (a, b) in [
            (0, 200_000),
            (1000, 50_000),
            (123_000, 124_000),
            (199_000, 200_000),
        ] {
            assert!(p.range(a, b, &mut out));
            let (lo, hi) = brute(&plane, a as usize, b as usize);
            // Whole-bucket coverage can only widen the range, by at most one bucket each side.
            assert!(out[0].0 <= lo + 1e-4 && out[0].1 >= hi - 1e-4, "{a}..{b}");
        }
        assert!(p.range(123_000, 124_000, &mut out));
        assert!((out[0].1 - 0.9).abs() < 1e-4);
        assert!(
            !p.range(10, 100, &mut out),
            "finer than a bucket reads samples"
        );
    }

    #[test]
    fn stereo_channels_are_independent() {
        let l = vec![0.5f32; 4096];
        let r = vec![-0.25f32; 4096];
        let p = PeakPyramid::build(&[l, r]);
        let mut out = [(0.0, 0.0); 2];
        assert!(p.range(0, 4096, &mut out));
        assert!((out[0].1 - 0.5).abs() < 1e-4 && (out[1].0 + 0.25).abs() < 1e-4);
        assert!(p.memory_bytes() < 4096);
    }
}
