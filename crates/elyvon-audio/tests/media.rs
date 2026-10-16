//! Decoding real files: formats, rates, channel layouts, failures.

// Test helpers outside `#[test]` functions may panic on failure.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::media::{MediaError, MediaStore, decode_file, fingerprint};

const RATE: u32 = 44_100;

fn expected_tone(ch: usize, i: usize) -> f32 {
    let (f, a) = if ch == 0 { (440.0, 0.5) } else { (660.0, 0.25) };
    let x = a * (2.0 * std::f64::consts::PI * f * i as f64 / f64::from(RATE)).sin();
    // Fixtures were quantised to 16 bits as round(x · 32767).
    (x * 32767.0).round() as f32 / 32768.0
}

#[test]
fn pcm_wav_variants_and_aiff_decode_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let planes = vec![sine(440.0, 0.5, RATE, 4410), sine(660.0, 0.25, RATE, 4410)];
    for (name, bits, tolerance) in [
        ("a16.wav", 16, 2.0 / 32768.0),
        ("a24.wav", 24, 1e-6),
        ("af.wav", 32, 0.0),
    ] {
        let path = dir.path().join(name);
        write_wav(&path, &planes, RATE, bits);
        let d = decode_file(&path, u64::MAX).unwrap();
        assert_eq!(d.info.sample_rate, RATE);
        assert_eq!(d.info.channels, 2);
        assert_eq!(d.info.frames, 4410);
        assert_eq!(d.info.format, "wave");
        for (ch, (got, want)) in d.planes.iter().zip(&planes).enumerate() {
            let max_err = got
                .iter()
                .zip(want)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(max_err <= tolerance + 1e-7, "{name} ch{ch}: {max_err}");
        }
    }
    let mono = vec![sine(1000.0, 0.8, 22_050, 2205)];
    let path = dir.path().join("m.aiff");
    write_aiff(&path, &mono, 22_050);
    let d = decode_file(&path, u64::MAX).unwrap();
    assert_eq!(
        (d.info.sample_rate, d.info.channels, d.info.frames),
        (22_050, 1, 2205)
    );
    assert_eq!(d.info.format, "aiff");
    let err = d.planes[0]
        .iter()
        .zip(&mono[0])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    // Written as round(x·32767), read back as n/32768.
    assert!(err <= 2.0 / 32768.0);
}

#[test]
fn flac_is_lossless() {
    let d = decode_file(&fixture("tone.flac"), u64::MAX).unwrap();
    assert_eq!(d.info.format, "flac");
    assert_eq!(
        (d.info.sample_rate, d.info.channels, d.info.frames),
        (RATE, 2, 22_050)
    );
    for ch in 0..2 {
        for (i, &s) in d.planes[ch].iter().enumerate() {
            assert_eq!(s, expected_tone(ch, i), "ch{ch} frame {i}");
        }
    }
}

#[test]
fn lossy_mp3_and_vorbis_keep_level_and_length() {
    for name in ["tone.mp3", "tone.ogg"] {
        let d = decode_file(&fixture(name), u64::MAX).unwrap();
        assert_eq!((d.info.sample_rate, d.info.channels), (RATE, 2), "{name}");
        // Gapless decoding trims encoder delay/padding: exact length.
        assert_eq!(d.info.frames, 22_050, "{name}");
        // Lossy codecs: level within ±0.5 dB of the source.
        let mid = |ch: usize| &d.planes[ch][4410..17_640];
        for (ch, amp) in [(0, 0.5), (1, 0.25)] {
            let db = 20.0 * (rms(mid(ch)) / (amp / 2f64.sqrt())).log10();
            assert!(db.abs() < 0.5, "{name} ch{ch}: {db:.3} dB");
        }
    }
}

#[test]
fn failures_are_reported_not_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let missing = decode_file(&dir.path().join("gone.wav"), u64::MAX).unwrap_err();
    assert!(matches!(missing, MediaError::NotFound(_)));
    let junk = dir.path().join("junk.wav");
    std::fs::write(&junk, b"definitely not audio").unwrap();
    assert!(matches!(
        decode_file(&junk, u64::MAX).unwrap_err(),
        MediaError::Unsupported(_)
    ));
    let empty = dir.path().join("empty.wav");
    write_wav(&empty, &[Vec::new()], RATE, 16);
    assert!(decode_file(&empty, u64::MAX).is_err());
    let big = dir.path().join("big.wav");
    write_wav(&big, &[vec![0.1; 10_000]], RATE, 16);
    assert_eq!(decode_file(&big, 1000).unwrap_err(), MediaError::TooLarge);
    assert_eq!(
        MediaStore::load_file(&big, 48_000, 1000).unwrap_err(),
        MediaError::TooLarge
    );
}

#[test]
fn load_resamples_to_engine_rate_and_never_writes_the_source() {
    let path = fixture("tone.flac");
    let before = std::fs::read(&path).unwrap();
    let (hash, size) = fingerprint(&path).unwrap();
    let buf = MediaStore::load_file(&path, 48_000, 1 << 30).unwrap();
    assert_eq!(buf.rate, 48_000);
    assert_eq!(buf.frames(), 24_000);
    assert_eq!(buf.info.fingerprint, format!("{hash:016x}"));
    assert_eq!(buf.info.size_bytes, size);
    assert_eq!(buf.engine_frames(22_050), 24_000);
    let mid = &buf.planes[0][6000..18_000];
    assert!((rms(mid) - 0.5 / 2f64.sqrt()).abs() < 0.005);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "source file untouched"
    );
}

#[test]
fn store_reuses_unchanged_files_and_detects_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.wav");
    write_wav(&path, &[sine(100.0, 0.5, RATE, 1000)], RATE, 16);
    let mut store = MediaStore::new(RATE, 1 << 30);
    let id = elyvon_project::MediaId::new();
    let buf = MediaStore::load_file(&path, RATE, store.remaining_bytes()).unwrap();
    store.insert(id, &path, Ok(buf.clone()));
    assert!(std::sync::Arc::ptr_eq(&store.cached(&path).unwrap(), &buf));
    // Rewriting the file (different size) invalidates the cache entry.
    write_wav(&path, &[sine(100.0, 0.5, RATE, 2000)], RATE, 16);
    assert!(store.cached(&path).is_none());
    let asset = elyvon_project::MediaAsset {
        id,
        name: "t.wav".into(),
        path: path.display().to_string(),
        info: Some(elyvon_project::MediaInfo {
            fingerprint: "deadbeef".into(),
            ..buf.info.clone()
        }),
    };
    assert!(matches!(
        store.statuses(&[asset])[0].state,
        elyvon_audio::media::MediaState::Ready { changed: true }
    ));
}
