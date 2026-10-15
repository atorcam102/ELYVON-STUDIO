//! File decoding (control/worker threads only — never the audio callback).
//!
//! Supported through Symphonia (MPL-2.0, pure Rust): PCM WAV, AIFF, FLAC,
//! MP3 and Ogg Vorbis. Decoding always runs on a worker thread; the result is
//! planar `f32` at the file's own rate.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use elyvon_project::MediaInfo;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::MediaError;

/// Extensions offered by the import dialog.
pub const SUPPORTED_EXTENSIONS: [&str; 8] =
    ["wav", "wave", "aif", "aiff", "flac", "mp3", "ogg", "oga"];

/// Decoded audio at the file's sample rate.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub info: MediaInfo,
    /// One `Vec` per channel, all the same length.
    pub planes: Vec<Vec<f32>>,
}

/// 64-bit FNV-1a over a reader (used to detect changed source files).
pub fn fingerprint(path: &Path) -> Result<(u64, u64), MediaError> {
    let mut file = File::open(path).map_err(|e| MediaError::from_io(path, e))?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| MediaError::from_io(path, e))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        for &b in &buf[..n] {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok((hash, size))
}

/// Decodes a whole file. `max_frames` bounds memory before decoding starts
/// (when the container states its length) and while decoding.
pub fn decode_file(path: &Path, max_frames: u64) -> Result<Decoded, MediaError> {
    let (hash, size) = fingerprint(path)?;
    let file = File::open(path).map_err(|e| MediaError::from_io(path, e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| MediaError::Unsupported(format!("unrecognised audio file: {e}")))?;
    let format_name = format.format_info().short_name.to_owned();
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| MediaError::Unsupported("the file has no audio track".into()))?;
    let track_id = track.id;
    let stated_frames = track.num_frames;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| MediaError::Unsupported("unknown codec parameters".into()))?
        .clone();
    if let Some(n) = stated_frames
        && n > max_frames
    {
        return Err(MediaError::TooLarge);
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| MediaError::Unsupported(format!("unsupported codec: {e}")))?;

    let mut rate = params.sample_rate.unwrap_or(0);
    let mut planes: Vec<Vec<f32>> = Vec::new();
    let mut scratch: Vec<Vec<f32>> = Vec::new();
    let mut bad_packets = 0u32;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) | Err(SymError::ResetRequired) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(MediaError::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if rate == 0 {
                    rate = buf.spec().rate();
                }
                buf.copy_to_vecs_planar::<f32>(&mut scratch);
                if planes.is_empty() {
                    planes = vec![Vec::new(); scratch.len()];
                }
                for (dst, src) in planes.iter_mut().zip(&scratch) {
                    dst.extend_from_slice(src);
                }
                if planes.first().map_or(0, Vec::len) as u64 > max_frames {
                    return Err(MediaError::TooLarge);
                }
            }
            Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                bad_packets += 1;
                if bad_packets > 64 {
                    return Err(MediaError::Decode("too many corrupt packets".into()));
                }
            }
            Err(e) => return Err(MediaError::Decode(e.to_string())),
        }
    }
    let frames = planes.first().map_or(0, Vec::len);
    if frames == 0 || planes.is_empty() {
        return Err(MediaError::Empty);
    }
    if rate == 0 {
        return Err(MediaError::Unsupported("unknown sample rate".into()));
    }
    let channels = u16::try_from(planes.len()).unwrap_or(u16::MAX);
    Ok(Decoded {
        info: MediaInfo {
            sample_rate: rate,
            channels,
            frames: frames as u64,
            format: format_name.chars().take(16).collect(),
            size_bytes: size,
            fingerprint: format!("{hash:016x}"),
        },
        planes,
    })
}
