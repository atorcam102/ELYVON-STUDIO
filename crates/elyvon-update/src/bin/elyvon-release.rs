//! Release pipeline helper. Keeps artifact naming and manifest rules in one
//! place (the `elyvon-update` crate) instead of duplicating them in YAML.
//!
//! ```text
//! elyvon-release name     --target <rust-triple> --kind <nsis|msi|dmg|app|deb|appimage> --version <x.y.z>
//! elyvon-release manifest --dir <dir> --version <x.y.z> --base-url <url> --out <file>
//!                         [--notes-file <file>] [--pub-date <rfc3339>] [--require-complete]
//!                         [--verify-config <tauri.conf.json>]
//! elyvon-release validate <latest.json> [--require-complete]
//! elyvon-release pubkey   --config <tauri.conf.json>        (fails if unconfigured)
//! elyvon-release verify   --config <tauri.conf.json> --file <f> --sig <f.sig> [--version <x.y.z>]
//! elyvon-release targets
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use elyvon_update::manifest::verify_signature;
use elyvon_update::version::parse_tag;
use elyvon_update::{ArtifactKind, Platform, STUDIO_NAMING, UpdateManifest};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  elyvon-release name --target <triple> --kind <nsis|msi|dmg|app|deb|appimage> --version <x.y.z>\n  \
         elyvon-release manifest --dir <dir> --version <x.y.z> --base-url <url> --out <file> [--notes-file <file>] [--pub-date <date>] [--require-complete]\n  \
         elyvon-release validate <latest.json> [--require-complete]\n  \
         elyvon-release pubkey --config <tauri.conf.json>\n  \
         elyvon-release verify --config <tauri.conf.json> --file <f> --sig <f.sig> [--version <x.y.z>]\n  \
         elyvon-release targets"
    );
    ExitCode::from(2)
}

struct Args {
    positional: Vec<String>,
    options: HashMap<String, String>,
    flags: Vec<String>,
}

fn parse_args(raw: impl Iterator<Item = String>) -> Args {
    let mut args = Args {
        positional: Vec::new(),
        options: HashMap::new(),
        flags: Vec::new(),
    };
    let mut raw = raw.peekable();
    while let Some(a) = raw.next() {
        if let Some(name) = a.strip_prefix("--") {
            match raw.peek() {
                Some(next) if !next.starts_with("--") && name != "require-complete" => {
                    let value = raw.next().unwrap_or_default();
                    args.options.insert(name.to_owned(), value);
                }
                _ => args.flags.push(name.to_owned()),
            }
        } else {
            args.positional.push(a);
        }
    }
    args
}

fn opt<'a>(args: &'a Args, name: &str) -> Result<&'a str, String> {
    args.options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing --{name}"))
}

fn version_arg(args: &Args) -> Result<String, String> {
    let raw = opt(args, "version")?;
    parse_tag(raw)
        .map(|v| v.to_string())
        .map_err(|e| format!("invalid --version {raw:?}: {e}"))
}

/// Reads `plugins.updater.pubkey` from a Tauri config and rejects the placeholder.
fn configured_pubkey(path: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let key = json["plugins"]["updater"]["pubkey"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if key.is_empty() || key == "UNCONFIGURED" {
        return Err(format!(
            "{path}: plugins.updater.pubkey is not configured; generate the release key (docs/RELEASES.md)"
        ));
    }
    Ok(key)
}

fn check_complete(m: &UpdateManifest, args: &Args) -> Result<(), String> {
    if args.flags.iter().any(|f| f == "require-complete") {
        let missing = m.missing_keys(&UpdateManifest::complete_key_set());
        if !missing.is_empty() {
            return Err(format!(
                "incomplete release, missing: {}",
                missing.join(", ")
            ));
        }
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    let cmd = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    match cmd {
        "name" => {
            let target = opt(&args, "target")?;
            let platform = Platform::from_rust_target(target)
                .ok_or_else(|| format!("unsupported target {target}"))?;
            let kind_name = opt(&args, "kind")?;
            let kind = ArtifactKind::from_cli_name(kind_name)
                .ok_or_else(|| format!("unknown kind {kind_name}"))?;
            if !platform.artifact_kinds().contains(&kind) {
                return Err(format!("{kind_name} is not built for {target}"));
            }
            println!(
                "{}",
                STUDIO_NAMING.file_name(&version_arg(&args)?, platform, kind)
            );
        }
        "manifest" => {
            let version = version_arg(&args)?;
            let notes = match args.options.get("notes-file") {
                Some(path) => {
                    Some(std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
                }
                None => None,
            };
            let manifest = UpdateManifest::assemble(
                &PathBuf::from(opt(&args, "dir")?),
                &STUDIO_NAMING,
                &version,
                opt(&args, "base-url")?,
                notes,
                args.options.get("pub-date").cloned(),
            )
            .map_err(|e| e.to_string())?;
            check_complete(&manifest, &args)?;
            if let Some(config) = args.options.get("verify-config") {
                let key = configured_pubkey(config)?;
                let dir = PathBuf::from(opt(&args, "dir")?);
                for (k, asset) in &manifest.platforms {
                    let file = asset.url.rsplit('/').next().unwrap_or_default();
                    let data = std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
                    verify_signature(&key, &data, &asset.signature, Some(&version))
                        .map_err(|e| format!("{k} ({file}): {e}"))?;
                    eprintln!("  verified {k}");
                }
            }
            let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
            let out = opt(&args, "out")?;
            std::fs::write(out, json + "\n").map_err(|e| format!("{out}: {e}"))?;
            eprintln!(
                "wrote {out} with {} platform entries:",
                manifest.platforms.len()
            );
            for key in manifest.platforms.keys() {
                eprintln!("  {key}");
            }
            let missing = manifest.missing_keys(&UpdateManifest::complete_key_set());
            if !missing.is_empty() {
                eprintln!("warning: not covered: {}", missing.join(", "));
            }
        }
        "validate" => {
            let path = args.positional.get(1).ok_or("missing manifest path")?;
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let manifest =
                UpdateManifest::parse(&bytes, &STUDIO_NAMING).map_err(|e| e.to_string())?;
            check_complete(&manifest, &args)?;
            println!(
                "{path}: valid manifest for {} ({} entries)",
                manifest.version,
                manifest.platforms.len()
            );
        }
        "pubkey" => {
            configured_pubkey(opt(&args, "config")?)?;
            println!("updater public key is configured");
        }
        "verify" => {
            let key = configured_pubkey(opt(&args, "config")?)?;
            let file = opt(&args, "file")?;
            let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let sig_path = opt(&args, "sig")?;
            let sig = std::fs::read_to_string(sig_path).map_err(|e| format!("{sig_path}: {e}"))?;
            let version = args.options.get("version").map(String::as_str);
            verify_signature(&key, &data, &sig, version)?;
            println!("{file}: signature valid for the embedded public key");
        }
        "targets" => {
            for p in Platform::SUPPORTED {
                println!("{}\t{}", p.rust_target(), p.release_label());
            }
        }
        _ => return Err(String::new()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(parse_args(std::env::args().skip(1))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => usage(),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
