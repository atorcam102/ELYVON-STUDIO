#![allow(clippy::unwrap_used)]
mod common;
use common::*;
use elyvon_audio::media::{MediaBuffer, MediaStore};
use elyvon_audio::render::{self, RenderRequest};
use std::sync::Arc;

#[test]
fn streamed_pcm_is_bounded_and_sample_identical_after_seeks_loops_and_resampling() {
    let dir = tempfile::tempdir().unwrap();
    for (source, rate) in [(48000, 48000), (44100, 48000), (96000, 48000)] {
        let path = dir.path().join(format!("{source}.wav"));
        let planes = vec![
            sine(440.0, 0.3, source, source as usize * 4),
            sine(550.0, 0.2, source, source as usize * 4),
        ];
        write_wav(&path, &planes, source, 32);
        let original = std::fs::read(&path).unwrap();
        let resident = MediaStore::load_file(&path, rate, 32 << 20).unwrap();
        let streamed = Arc::new(MediaBuffer::load_streamed(&path, rate, 2 << 20).unwrap());
        assert!(streamed.planes.is_empty());
        assert!(streamed.memory_bytes() < 2 << 20);
        assert_eq!(streamed.frames(), resident.frames());
        for start in [0, 12000, 120000, 4000, 180000, 0] {
            streamed.prefetch(start, 512).unwrap();
            for frame in start as usize..start as usize + 512 {
                let a = streamed.sample(frame);
                let b = resident.sample(frame);
                assert!(
                    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5,
                    "{source}->{rate} at {frame}: {a:?} != {b:?}"
                );
            }
        }
        let mut session = Session::new(120.0);
        let track = session.audio_track();
        session.place(track, 0, &streamed, "streamed");
        let request = RenderRequest::whole_song(rate);
        let first = render::render(&session.project, &session.lookup(), request, None);
        let second = render::render(
            &session.project,
            &session.lookup(),
            RenderRequest {
                block_frames: 4096,
                ..request
            },
            None,
        );
        assert_eq!(first.report.streaming_errors, 0);
        assert_eq!(first.samples, second.samples);
        assert_eq!(original, std::fs::read(&path).unwrap());
    }
}

#[test]
fn compressed_streaming_uses_real_decoder_and_reports_cache_misses() {
    for name in ["tone.flac", "tone.mp3", "tone.ogg"] {
        let p = fixture(name);
        let resident = MediaStore::load_file(&p, 48000, 16 << 20).unwrap();
        let stream = MediaBuffer::load_streamed(&p, 48000, 2 << 20).unwrap();
        stream.prefetch(1000, 256).unwrap();
        for i in 1000..1256 {
            assert!((stream.sample(i)[0] - resident.sample(i)[0]).abs() < 1e-5);
        }
        assert!(!stream.stream.as_ref().unwrap().failed());
    }
    assert!(MediaBuffer::load_streamed(&fixture("tone.flac"), 48000, 1024).is_err());
}

#[test]
fn twelve_clips_from_one_stream_share_a_bounded_associative_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.wav");
    let planes = vec![sine(440.0, 0.1, 48000, 1_700_000)];
    write_wav(&path, &planes, 48000, 32);
    let streamed = Arc::new(MediaBuffer::load_streamed(&path, 48000, 2 << 20).unwrap());
    let resident = MediaStore::load_file(&path, 48000, 32 << 20).unwrap();
    let mut session = Session::new(120.0);
    for n in 0..12 {
        let track = session.audio_track();
        let id = session.place(track, 0, &streamed, "shared");
        let region = session
            .project
            .tracks
            .iter_mut()
            .flat_map(|t| &mut t.regions)
            .find(|r| r.id == id)
            .unwrap();
        if let elyvon_project::RegionContent::Audio { offset, .. } = &mut region.content {
            *offset = n * 131072;
        }
        region.length = 960;
    }
    let streamed_output = render::render(
        &session.project,
        &session.lookup(),
        RenderRequest::whole_song(48000),
        None,
    );
    assert_eq!(streamed_output.report.streaming_errors, 0);
    for buffer in session.media.values_mut() {
        *buffer = Arc::clone(&resident);
    }
    let resident_output = render::render(
        &session.project,
        &session.lookup(),
        RenderRequest::whole_song(48000),
        None,
    );
    assert_eq!(streamed_output.samples, resident_output.samples);
}
