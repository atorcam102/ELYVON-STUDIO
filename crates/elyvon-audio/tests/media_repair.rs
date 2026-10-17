//! Phase 11: missing-media repair, collect media for portable projects and
//! backed-up clean-up. Source files are never modified.
#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use elyvon_audio::media::repair;
use elyvon_audio::media::{MediaStore, fingerprint, resolve_path};
use elyvon_project::{
    AudioPlacement, EditCommand, MediaAsset, MediaId, PlacementTrack, ProjectSession, TrackKind,
};

fn import(s: &mut ProjectSession, path: &std::path::Path) -> MediaId {
    let buffer = MediaStore::load_file(path, 48_000, 1 << 26).unwrap();
    let track = s
        .apply(&EditCommand::AddTrack {
            kind: TrackKind::Audio,
            name: None,
            index: None,
        })
        .unwrap()
        .created_tracks[0];
    let id = MediaId::new();
    s.apply(&EditCommand::ImportAudio {
        assets: vec![MediaAsset {
            id,
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: path.display().to_string(),
            info: Some(buffer.info.clone()),
        }],
        placements: vec![AudioPlacement {
            media_id: id,
            track: PlacementTrack::Existing { id: track },
            start: 0,
        }],
    })
    .unwrap();
    id
}

#[test]
fn moved_files_are_found_by_fingerprint_and_lookalikes_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("orig").join("kick.wav");
    let b = root.path().join("orig").join("snare.wav");
    std::fs::create_dir_all(a.parent().unwrap()).unwrap();
    write_wav(&a, &[sine(60.0, 0.5, 48_000, 4800)], 48_000, 16);
    write_wav(&b, &[sine(200.0, 0.5, 48_000, 4800)], 48_000, 16);
    let mut s = ProjectSession::new_empty();
    let ka = import(&mut s, &a);
    let kb = import(&mut s, &b);
    // The originals move away; a different file named snare.wav appears.
    let moved = root.path().join("library").join("drums");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(&a, moved.join("kick.wav")).unwrap();
    std::fs::remove_file(&b).unwrap();
    write_wav(&moved.join("snare.wav"), &[sine(201.0, 0.5, 48_000, 4800)], 48_000, 16);
    assert_eq!(repair::missing_media(s.project(), None).len(), 2);
    let report = repair::find_missing(s.project(), None, &root.path().join("library"), 10_000);
    assert_eq!(report.relinked.len(), 1);
    assert_eq!(report.relinked[0].id, ka);
    assert_eq!(report.not_found, vec![kb], "same name, different content is not relinked");
    s.apply(&EditCommand::RelinkMediaFiles { items: report.relinked }).unwrap();
    assert_eq!(repair::missing_media(s.project(), None), vec![kb]);
    assert!(s.undo());
    assert_eq!(repair::missing_media(s.project(), None).len(), 2);
}

#[test]
fn collect_media_makes_the_project_folder_portable_without_touching_sources() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("elsewhere").join("vox.wav");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    write_wav(&src, &[sine(330.0, 0.4, 48_000, 9600)], 48_000, 24);
    let before = fingerprint(&src).unwrap();
    let project_dir = root.path().join("Song");
    std::fs::create_dir_all(&project_dir).unwrap();
    let mut s = ProjectSession::new_empty();
    import(&mut s, &src);
    // A different file with the same name already in Media is kept.
    std::fs::create_dir_all(project_dir.join("Media")).unwrap();
    std::fs::write(project_dir.join("Media").join("vox.wav"), b"not this one").unwrap();
    let relinks = repair::collect(s.project(), &project_dir).unwrap();
    assert_eq!(relinks.len(), 1);
    assert_eq!(relinks[0].path, "Media/vox (2).wav");
    assert_eq!(std::fs::read(project_dir.join("Media").join("vox.wav")).unwrap(), b"not this one");
    s.apply(&EditCommand::RelinkMediaFiles { items: relinks }).unwrap();
    let file = project_dir.join("Song.elyvon");
    s.save(Some(&file), "test").unwrap();
    assert_eq!(fingerprint(&src).unwrap(), before, "source untouched");
    // Move the whole folder: every file still resolves and matches.
    let moved = root.path().join("Moved Song");
    std::fs::rename(&project_dir, &moved).unwrap();
    std::fs::remove_file(&src).unwrap();
    let reopened = ProjectSession::open(&moved.join("Song.elyvon")).unwrap();
    assert!(repair::missing_media(reopened.project(), Some(&moved)).is_empty());
    for a in &reopened.project().media {
        let path = resolve_path(&a.path, Some(&moved));
        let (hash, _) = fingerprint(&path).unwrap();
        assert_eq!(format!("{hash:016x}"), a.info.as_ref().unwrap().fingerprint);
    }
    // Collecting again changes nothing.
    assert!(repair::collect(reopened.project(), &moved).unwrap().is_empty());
}

#[test]
fn unused_files_are_moved_to_a_backup_not_deleted() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("Song");
    std::fs::create_dir_all(dir.join("Bounces")).unwrap();
    std::fs::create_dir_all(dir.join("Media")).unwrap();
    let used = dir.join("Media").join("used.wav");
    write_wav(&used, &[vec![0.1; 480]], 48_000, 16);
    let old = dir.join("Bounces").join("old bounce.wav");
    write_wav(&old, &[vec![0.2; 480]], 48_000, 16);
    let mut s = ProjectSession::new_empty();
    let id = import(&mut s, &used);
    s.apply(&EditCommand::RelinkMediaFiles {
        items: vec![elyvon_project::region_edit::MediaRelink {
            id,
            path: "Media/used.wav".into(),
            info: None,
        }],
    })
    .unwrap();
    let unused = repair::unused_files(s.project(), &dir);
    assert_eq!(unused, vec![old.clone()]);
    let backup = repair::move_to_backup(&unused, &dir, "20261017-120000").unwrap();
    assert!(!old.exists());
    assert!(backup.join("Bounces").join("old bounce.wav").is_file());
    assert!(used.is_file());
}
