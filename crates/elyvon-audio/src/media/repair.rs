//! Media repair and portability (Phase 11).
//!
//! * [`find_missing`] looks for moved or renamed files under a folder the
//!   user chooses and matches them by name, size and content fingerprint —
//!   never by name alone when the original was measured.
//! * [`collect`] copies every referenced file into the project's `Media`
//!   folder (verified copies, never overwriting) so the project folder is
//!   portable. Source files are left where they are.
//! * [`unused_files`] and [`move_to_backup`] support the only destructive
//!   clean-up: files the project no longer uses are *moved* into a dated
//!   backup folder inside the project, after explicit confirmation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use elyvon_project::region_edit::MediaRelink;
use elyvon_project::{MediaId, Project};
use serde::Serialize;
use ts_rs::TS;

use super::{fingerprint, resolve_path};

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairReport {
    /// Files found and verified.
    pub relinked: Vec<MediaRelink>,
    /// Files with several candidates that could not be told apart.
    pub ambiguous: Vec<MediaId>,
    pub not_found: Vec<MediaId>,
    /// Files scanned under the search folder.
    pub scanned: usize,
    /// The scan stopped at its file limit.
    pub truncated: bool,
}

/// Pool entries whose file is missing on disk.
pub fn missing_media(project: &Project, project_dir: Option<&Path>) -> Vec<MediaId> {
    project
        .media
        .iter()
        .filter(|a| !resolve_path(&a.path, project_dir).is_file())
        .map(|a| a.id)
        .collect()
}

fn file_name_key(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase())
}

fn walk(root: &Path, max_files: usize, out: &mut HashMap<String, Vec<PathBuf>>) -> (usize, bool) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut scanned = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let hidden = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'));
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !hidden && depth < 16 {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file() {
                scanned += 1;
                if scanned > max_files {
                    return (scanned - 1, true);
                }
                if let Some(k) = file_name_key(&path) {
                    out.entry(k).or_default().push(path);
                }
            }
        }
    }
    (scanned, false)
}

/// Path to store for `file`: relative (forward slashes) when it lies inside
/// the project folder, otherwise absolute.
pub fn stored_path(file: &Path, project_dir: Option<&Path>) -> String {
    if let Some(dir) = project_dir
        && let Ok(rel) = file.strip_prefix(dir)
    {
        return rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
    }
    file.display().to_string()
}

/// Searches `root` for the project's missing files.
pub fn find_missing(
    project: &Project,
    project_dir: Option<&Path>,
    root: &Path,
    max_files: usize,
) -> RepairReport {
    let missing = missing_media(project, project_dir);
    let mut report = RepairReport::default();
    if missing.is_empty() {
        return report;
    }
    let mut by_name = HashMap::new();
    let (scanned, truncated) = walk(root, max_files, &mut by_name);
    report.scanned = scanned;
    report.truncated = truncated;
    for id in missing {
        let Some(asset) = project.media_asset(id) else {
            continue;
        };
        let name =
            file_name_key(Path::new(&asset.path)).unwrap_or_else(|| asset.name.to_lowercase());
        let candidates = by_name.get(&name).cloned().unwrap_or_default();
        let matches: Vec<PathBuf> = match &asset.info {
            Some(info) => candidates
                .into_iter()
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() == info.size_bytes))
                .filter(|p| {
                    fingerprint(p).is_ok_and(|(h, _)| format!("{h:016x}") == info.fingerprint)
                })
                .collect(),
            None => candidates,
        };
        match matches.as_slice() {
            [] => report.not_found.push(id),
            [one] => report.relinked.push(MediaRelink {
                id,
                path: stored_path(one, project_dir),
                info: asset.info.clone(),
            }),
            // Identical content in several places: any copy is the file.
            [first, ..] if asset.info.is_some() => report.relinked.push(MediaRelink {
                id,
                path: stored_path(first, project_dir),
                info: asset.info.clone(),
            }),
            _ => report.ambiguous.push(id),
        }
    }
    report
}

/// Copies every referenced file outside `project_dir` into
/// `project_dir/Media` and returns the relinks to their relative paths.
/// Files are copied to a temporary name, verified and then renamed; an
/// existing different file is never overwritten.
pub fn collect(project: &Project, project_dir: &Path) -> Result<Vec<MediaRelink>, String> {
    let media_dir = project_dir.join("Media");
    std::fs::create_dir_all(&media_dir)
        .map_err(|e| format!("could not create {}: {e}", media_dir.display()))?;
    let mut relinks = Vec::new();
    let mut missing = Vec::new();
    for asset in &project.media {
        let src = resolve_path(&asset.path, Some(project_dir));
        if !src.is_file() {
            missing.push(asset.name.clone());
            continue;
        }
        if src.starts_with(project_dir) {
            let stored = stored_path(&src, Some(project_dir));
            if stored != asset.path {
                relinks.push(MediaRelink {
                    id: asset.id,
                    path: stored,
                    info: None,
                });
            }
            continue;
        }
        let (hash, _) = fingerprint(&src).map_err(|e| format!("{}: {e}", src.display()))?;
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{}.wav", asset.id));
        let (stem, ext) = match name.rfind('.') {
            Some(i) => (name[..i].to_owned(), name[i..].to_owned()),
            None => (name.clone(), String::new()),
        };
        let mut dest = media_dir.join(&name);
        let mut n = 2;
        while dest.exists() {
            if fingerprint(&dest).is_ok_and(|(h, _)| h == hash) {
                break; // the same file was collected before
            }
            dest = media_dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        if !dest.exists() {
            let tmp = media_dir.join(format!(".{}.collecting", asset.id));
            std::fs::copy(&src, &tmp)
                .map_err(|e| format!("could not copy {}: {e}", src.display()))?;
            let copied = fingerprint(&tmp).map(|(h, _)| h).ok();
            if copied != Some(hash) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!(
                    "the copy of {} does not match the original",
                    src.display()
                ));
            }
            std::fs::rename(&tmp, &dest)
                .map_err(|e| format!("could not place {}: {e}", dest.display()))?;
        }
        relinks.push(MediaRelink {
            id: asset.id,
            path: stored_path(&dest, Some(project_dir)),
            info: None,
        });
    }
    if !missing.is_empty() {
        return Err(format!(
            "{} file(s) are missing and were not collected: {}; locate them first",
            missing.len(),
            missing.join(", ")
        ));
    }
    Ok(relinks)
}

/// Folders inside a project directory that Elyvon writes media into.
pub const PROJECT_MEDIA_FOLDERS: [&str; 3] = ["Media", "Bounces", "Freeze"];

/// Files in the project's media folders that no pool entry references.
pub fn unused_files(project: &Project, project_dir: &Path) -> Vec<PathBuf> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let used: Vec<PathBuf> = project
        .media
        .iter()
        .map(|a| canon(&resolve_path(&a.path, Some(project_dir))))
        .collect();
    let mut out = Vec::new();
    for folder in PROJECT_MEDIA_FOLDERS {
        let Ok(entries) = std::fs::read_dir(project_dir.join(folder)) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                && !used.contains(&canon(&p))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Moves `files` (inside `project_dir`) into `project_dir/.elyvon-backup/<stamp>/`,
/// keeping their relative paths. Returns the backup folder.
pub fn move_to_backup(
    files: &[PathBuf],
    project_dir: &Path,
    stamp: &str,
) -> Result<PathBuf, String> {
    let backup = project_dir.join(".elyvon-backup").join(stamp);
    for f in files {
        let rel = f
            .strip_prefix(project_dir)
            .map_err(|_| format!("{} is outside the project folder", f.display()))?;
        let dest = backup.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if std::fs::rename(f, &dest).is_err() {
            std::fs::copy(f, &dest)
                .map_err(|e| format!("could not back up {}: {e}", f.display()))?;
            std::fs::remove_file(f)
                .map_err(|e| format!("could not remove {}: {e}", f.display()))?;
        }
    }
    Ok(backup)
}
