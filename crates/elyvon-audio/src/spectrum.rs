//! Spectrum analyser for the effect editors, computed **off the audio
//! thread**: the engine only pushes the selected strip's mono post-fader
//! samples into a lock-free queue (dropping them when it is full); this
//! control-side reader keeps the last [`FFT_SIZE`] samples, applies a Hann
//! window and returns levels on a logarithmic frequency axis.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use rtrb::Consumer;
use serde::Serialize;
use ts_rs::TS;

pub const FFT_SIZE: usize = 8192;
/// Points of the logarithmic axis (20 Hz … 20 kHz).
pub const BANDS: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Spectrum {
    pub sample_rate: u32,
    /// Band centre frequencies (Hz).
    pub freqs: Vec<f32>,
    /// Level per band in dBFS of a full-scale sine (≥ −140).
    pub db: Vec<f32>,
}

pub struct SpectrumAnalyzer {
    consumer: Consumer<f32>,
    history: Vec<f32>,
    at: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

impl std::fmt::Debug for SpectrumAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpectrumAnalyzer").finish_non_exhaustive()
    }
}

impl SpectrumAnalyzer {
    pub fn new(consumer: Consumer<f32>) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| {
                (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / FFT_SIZE as f64).cos()) as f32
            })
            .collect();
        Self {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            consumer,
            history: vec![0.0; FFT_SIZE],
            at: 0,
            window,
        }
    }

    /// Drains the queue and clears the history (a new strip was selected).
    pub fn reset(&mut self) {
        while self.consumer.pop().is_ok() {}
        self.history.fill(0.0);
        self.at = 0;
    }

    /// Takes every queued sample and analyses the latest window.
    pub fn read(&mut self, sample_rate: u32) -> Spectrum {
        while let Ok(v) = self.consumer.pop() {
            self.history[self.at] = v;
            self.at = (self.at + 1) % FFT_SIZE;
        }
        for i in 0..FFT_SIZE {
            self.input[i] = self.history[(self.at + i) % FFT_SIZE] * self.window[i];
        }
        let _ = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch);
        // A full-scale sine gives |X| = N/4 with a Hann window.
        let norm = 4.0 / FFT_SIZE as f32;
        let rate = sample_rate.max(1) as f32;
        let bin_hz = rate / FFT_SIZE as f32;
        let (lo, hi) = (20f32, 20_000f32.min(rate * 0.5));
        let mut freqs = Vec::with_capacity(BANDS);
        let mut db = Vec::with_capacity(BANDS);
        for b in 0..BANDS {
            let f0 = lo * (hi / lo).powf(b as f32 / BANDS as f32);
            let f1 = lo * (hi / lo).powf((b + 1) as f32 / BANDS as f32);
            let (k0, k1) = (
                (f0 / bin_hz).floor() as usize,
                ((f1 / bin_hz).ceil() as usize).max(1),
            );
            let peak = self.output[k0.min(self.output.len() - 1)..=k1.min(self.output.len() - 1)]
                .iter()
                .map(|c| c.norm() * norm)
                .fold(0.0f32, f32::max);
            freqs.push((f0 * f1).sqrt());
            db.push((20.0 * peak.max(1e-7).log10()).max(-140.0));
        }
        Spectrum {
            sample_rate,
            freqs,
            db,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    #[test]
    fn a_sine_peaks_in_its_band_at_its_level() {
        let (mut tx, rx) = RingBuffer::new(FFT_SIZE * 2);
        let rate = 48_000;
        for i in 0..FFT_SIZE {
            let v = 0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * i as f64 / f64::from(rate)).sin();
            let _ = tx.push(v as f32);
        }
        let mut a = SpectrumAnalyzer::new(rx);
        let s = a.read(rate);
        let (i, peak) =
            s.db.iter().enumerate().fold(
                (0, -200.0f32),
                |(bi, bv), (i, v)| if *v > bv { (i, *v) } else { (bi, bv) },
            );
        assert!((s.freqs[i] / 1_000.0 - 1.0).abs() < 0.06, "{}", s.freqs[i]);
        assert!((peak + 6.02).abs() < 1.5, "{peak}");
        // Far from the tone the level is low.
        assert!(s.db[5] < -80.0, "{}", s.db[5]);
    }
}
