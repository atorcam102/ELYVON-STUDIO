//! Sample-format conversion and channel layout helpers.
//!
//! The engine renders **planar `f32`** internally (one slice per channel,
//! nominal range −1.0…+1.0). Devices usually want **interleaved** frames in a
//! native sample type. These helpers convert between the two without
//! allocating, so they are safe to call from the real-time callback.
//!
//! Output conversion always clamps to the representable range and reports how
//! many samples had to be clipped; integer formats use symmetric scaling so
//! +1.0 and −1.0 map to the same magnitude and never overflow.

use serde::Serialize;
use ts_rs::TS;

/// A device sample type the engine can write to.
pub trait OutputSample: Copy + Send + 'static {
    /// The value that represents digital silence.
    const SILENCE: Self;
    /// Converts a sample that is already clamped to −1.0…+1.0.
    fn from_unit(v: f32) -> Self;
}

/// A device sample type the engine can read from (input metering).
pub trait InputSample: Copy + Send + 'static {
    /// Converts to `f32` in −1.0…+1.0 (approximately, for integer formats).
    fn to_unit(self) -> f32;
}

impl OutputSample for f32 {
    const SILENCE: Self = 0.0;
    #[inline]
    fn from_unit(v: f32) -> Self {
        v
    }
}

impl OutputSample for f64 {
    const SILENCE: Self = 0.0;
    #[inline]
    fn from_unit(v: f32) -> Self {
        f64::from(v)
    }
}

macro_rules! signed_output {
    ($t:ty, $max:expr) => {
        impl OutputSample for $t {
            const SILENCE: Self = 0;
            #[inline]
            fn from_unit(v: f32) -> Self {
                // `as` saturates on out-of-range floats; inputs are clamped anyway.
                (f64::from(v) * $max).round() as $t
            }
        }
    };
}

macro_rules! unsigned_output {
    ($t:ty, $signed:ty, $max:expr, $offset:expr) => {
        impl OutputSample for $t {
            const SILENCE: Self = $offset;
            #[inline]
            fn from_unit(v: f32) -> Self {
                let s = (f64::from(v) * $max).round() as $signed;
                (s as i128 + $offset as i128) as $t
            }
        }
    };
}

signed_output!(i8, 127.0);
signed_output!(i16, 32_767.0);
signed_output!(i32, 2_147_483_647.0);
signed_output!(i64, 9_223_372_036_854_775_807.0);
unsigned_output!(u8, i8, 127.0, 128u8);
unsigned_output!(u16, i16, 32_767.0, 32_768u16);
unsigned_output!(u32, i32, 2_147_483_647.0, 2_147_483_648u32);
unsigned_output!(
    u64,
    i64,
    9_223_372_036_854_775_807.0,
    9_223_372_036_854_775_808u64
);

impl InputSample for f32 {
    #[inline]
    fn to_unit(self) -> f32 {
        self
    }
}

impl InputSample for f64 {
    #[inline]
    fn to_unit(self) -> f32 {
        self as f32
    }
}

macro_rules! signed_input {
    ($t:ty, $scale:expr) => {
        impl InputSample for $t {
            #[inline]
            fn to_unit(self) -> f32 {
                (self as f64 / $scale) as f32
            }
        }
    };
}

macro_rules! unsigned_input {
    ($t:ty, $offset:expr, $scale:expr) => {
        impl InputSample for $t {
            #[inline]
            fn to_unit(self) -> f32 {
                ((self as f64 - $offset) / $scale) as f32
            }
        }
    };
}

signed_input!(i8, 128.0);
signed_input!(i16, 32_768.0);
signed_input!(i32, 2_147_483_648.0);
signed_input!(i64, 9_223_372_036_854_775_808.0);
unsigned_input!(u8, 128.0, 128.0);
unsigned_input!(u16, 32_768.0, 32_768.0);
unsigned_input!(u32, 2_147_483_648.0, 2_147_483_648.0);
unsigned_input!(
    u64,
    9_223_372_036_854_775_808.0,
    9_223_372_036_854_775_808.0
);

/// Clamps one float sample to −1.0…+1.0. NaN becomes silence. Returns the
/// clamped value and whether it had to be changed (clipping).
#[inline]
pub fn clamp_unit(v: f32) -> (f32, bool) {
    if v.is_nan() {
        (0.0, true)
    } else if v > 1.0 {
        (1.0, true)
    } else if v < -1.0 {
        (-1.0, true)
    } else {
        (v, false)
    }
}

/// Writes `frames` frames of planar audio into an interleaved device buffer
/// with `out_channels` channels, clamping and converting each sample.
///
/// Planar channels beyond the device's channel count are dropped; device
/// channels without a planar source are written as silence. Returns the
/// number of clipped samples. Never allocates.
pub fn interleave_into<S: OutputSample>(
    planar: &[&[f32]],
    frames: usize,
    out: &mut [S],
    out_channels: usize,
) -> u64 {
    if out_channels == 0 {
        return 0;
    }
    let frames = frames.min(out.len() / out_channels);
    let mut clipped = 0u64;
    for (frame_index, frame) in out.chunks_exact_mut(out_channels).take(frames).enumerate() {
        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = match planar.get(channel).and_then(|c| c.get(frame_index)) {
                Some(v) => {
                    let (v, was_clipped) = clamp_unit(*v);
                    clipped += u64::from(was_clipped);
                    S::from_unit(v)
                }
                None => S::SILENCE,
            };
        }
    }
    clipped
}

/// Splits interleaved samples into planar `f32` channels (input side, tests).
/// Writes at most `min(frames available, planar channel length)` frames and
/// returns that frame count. Never allocates.
pub fn deinterleave_into<S: InputSample>(
    interleaved: &[S],
    in_channels: usize,
    planar: &mut [&mut [f32]],
) -> usize {
    if in_channels == 0 {
        return 0;
    }
    let available = interleaved.len() / in_channels;
    let frames = planar
        .iter()
        .map(|c| c.len())
        .min()
        .unwrap_or(0)
        .min(available);
    for (frame_index, frame) in interleaved
        .chunks_exact(in_channels)
        .take(frames)
        .enumerate()
    {
        for (channel, out) in planar.iter_mut().enumerate() {
            out[frame_index] = frame.get(channel).map_or(0.0, |s| s.to_unit());
        }
    }
    frames
}

/// How the channels of a stream are arranged, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Multichannel,
}

impl ChannelLayout {
    pub fn from_channels(channels: u16) -> Self {
        match channels {
            1 => Self::Mono,
            2 => Self::Stereo,
            _ => Self::Multichannel,
        }
    }
}

/// How samples are laid out in a device buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BufferLayout {
    /// Frames of `[ch0, ch1, …]` one after another (all current backends).
    Interleaved,
    /// One contiguous buffer per channel (engine-internal processing).
    Planar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_extremes_are_symmetric_and_never_overflow() {
        assert_eq!(i16::from_unit(1.0), 32_767);
        assert_eq!(i16::from_unit(-1.0), -32_767);
        assert_eq!(i16::from_unit(0.0), 0);
        assert_eq!(i8::from_unit(-1.0), -127);
        assert_eq!(i32::from_unit(1.0), i32::MAX);
        assert_eq!(i32::from_unit(-1.0), -i32::MAX);
        assert_eq!(i64::from_unit(1.0), i64::MAX);
        assert_eq!(u8::from_unit(0.0), 128);
        assert_eq!(u8::from_unit(1.0), 255);
        assert_eq!(u8::from_unit(-1.0), 1);
        assert_eq!(u16::from_unit(0.0), 32_768);
        assert_eq!(u16::from_unit(1.0), 65_535);
        assert_eq!(u32::from_unit(0.0), 2_147_483_648);
        assert_eq!(u64::from_unit(0.0), u64::SILENCE);
        assert_eq!(f64::from_unit(0.5), 0.5);
    }

    #[test]
    fn input_conversion_round_trips_within_two_lsb() {
        // Output scales by 2^(n-1)-1 (symmetric), input by 2^(n-1) (conventional).
        for v in [-1.0f32, -0.5, -0.001, 0.0, 0.25, 0.999] {
            assert!((i16::from_unit(v).to_unit() - v).abs() <= 2.0 / 32_767.0);
            assert!((u16::from_unit(v).to_unit() - v).abs() <= 2.0 / 32_767.0);
            assert!((i32::from_unit(v).to_unit() - v).abs() <= 1e-6);
            assert!((u8::from_unit(v).to_unit() - v).abs() <= 2.0 / 127.0);
        }
        assert_eq!(i16::MIN.to_unit(), -1.0);
        assert_eq!(0u8.to_unit(), -1.0);
    }

    #[test]
    fn clamp_reports_clipping_and_rejects_nan() {
        assert_eq!(clamp_unit(0.5), (0.5, false));
        assert_eq!(clamp_unit(1.0), (1.0, false));
        assert_eq!(clamp_unit(1.5), (1.0, true));
        assert_eq!(clamp_unit(-3.0), (-1.0, true));
        assert_eq!(clamp_unit(f32::NAN), (0.0, true));
        assert_eq!(clamp_unit(f32::INFINITY), (1.0, true));
    }

    #[test]
    fn interleave_mono_stereo_and_extra_device_channels() {
        let left = [0.1f32, 0.2, 0.3];
        let right = [-0.1f32, -0.2, 2.0];
        // Stereo to a 2-channel device: L R L R L R, last right sample clips.
        let mut out = [0.0f32; 6];
        let clipped = interleave_into(&[&left, &right], 3, &mut out, 2);
        assert_eq!(out, [0.1, -0.1, 0.2, -0.2, 0.3, 1.0]);
        assert_eq!(clipped, 1);
        // Mono engine to a 3-channel device: extra channels are silent.
        let mut out = [7i16; 6];
        interleave_into(&[&left], 2, &mut out, 3);
        assert_eq!(out, [3277, 0, 0, 6553, 0, 0]);
        // Stereo engine to a mono device: second channel is dropped.
        let mut out = [0.0f32; 3];
        interleave_into(&[&left, &right], 3, &mut out, 1);
        assert_eq!(out, left);
    }

    #[test]
    fn deinterleave_is_the_inverse_of_interleave() {
        let left = [0.5f32, -0.25, 0.125, 0.0];
        let right = [-0.5f32, 0.25, -0.125, 1.0];
        let mut device = [0i32; 8];
        interleave_into(&[&left, &right], 4, &mut device, 2);
        let mut l = [0.0f32; 4];
        let mut r = [0.0f32; 4];
        let frames = deinterleave_into(&device, 2, &mut [&mut l, &mut r]);
        assert_eq!(frames, 4);
        for i in 0..4 {
            assert!((l[i] - left[i]).abs() < 1e-6 && (r[i] - right[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn layouts() {
        assert_eq!(ChannelLayout::from_channels(1), ChannelLayout::Mono);
        assert_eq!(ChannelLayout::from_channels(2), ChannelLayout::Stereo);
        assert_eq!(ChannelLayout::from_channels(8), ChannelLayout::Multichannel);
    }
}
