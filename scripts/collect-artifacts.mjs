// Collects the bundles produced by `tauri build` for one target, renames them
// to the canonical release names and copies them (with their updater
// signatures) into an output directory.
//
//   node scripts/collect-artifacts.mjs --target x86_64-unknown-linux-gnu --version 0.1.0 --out dist-release
//
// Naming comes from the `elyvon-release` tool so the convention lives in one
// place (crates/elyvon-update). Fails if an expected artifact is missing.

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0 || !process.argv[i + 1]) throw new Error(`missing --${name}`);
  return process.argv[i + 1];
}

const target = arg("target");
const version = arg("version").replace(/^v/, "");
const out = resolve(arg("out"));
const root = resolve(new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1"));
const bundle = join(root, "target", target, "release", "bundle");
const tool = join(root, "target", "release", process.platform === "win32" ? "elyvon-release.exe" : "elyvon-release");

// kind → [bundle subdirectory, file suffix, has updater signature]
// `app` (.app.tar.gz) exists only as an updater payload.
const KINDS = {
  nsis: ["nsis", "-setup.exe", true],
  msi: ["msi", ".msi", true],
  dmg: ["dmg", ".dmg", false],
  app: ["macos", ".app.tar.gz", true],
  deb: ["deb", ".deb", true],
  appimage: ["appimage", ".AppImage", true],
};

const PER_TARGET = {
  "x86_64-pc-windows-msvc": ["nsis", "msi"],
  "i686-pc-windows-msvc": ["nsis", "msi"],
  "aarch64-pc-windows-msvc": ["nsis"],
  "x86_64-apple-darwin": ["dmg", "app"],
  "aarch64-apple-darwin": ["dmg", "app"],
  "x86_64-unknown-linux-gnu": ["deb", "appimage"],
  "aarch64-unknown-linux-gnu": ["deb", "appimage"],
};

const kinds = PER_TARGET[target];
if (!kinds) throw new Error(`unsupported target ${target}`);
mkdirSync(out, { recursive: true });

// UPDATER_ARTIFACTS=0: build made with createUpdaterArtifacts=false (no
// signing key) — there are no signatures and no updater-only payloads.
const updaterArtifacts = process.env.UPDATER_ARTIFACTS !== "0";
let copied = 0;
for (const kind of kinds) {
  if (kind === "app" && !updaterArtifacts) continue;
  const [dir, suffix, signed] = KINDS[kind];
  const from = join(bundle, dir);
  const files = existsSync(from) ? readdirSync(from).filter((f) => f.endsWith(suffix)) : [];
  if (files.length !== 1) {
    throw new Error(`expected exactly one ${suffix} in ${from}, found ${files.length}: ${files.join(", ")}`);
  }
  const name = execFileSync(tool, ["name", "--target", target, "--kind", kind, "--version", version], { encoding: "utf8" }).trim();
  copyFileSync(join(from, files[0]), join(out, name));
  console.log(`${files[0]} -> ${name}`);
  copied++;
  if (signed) {
    const sig = join(from, `${files[0]}.sig`);
    if (existsSync(sig)) {
      copyFileSync(sig, join(out, `${name}.sig`));
      console.log(`  + ${name}.sig`);
    } else if (updaterArtifacts) {
      throw new Error(`missing updater signature ${sig} (is TAURI_SIGNING_PRIVATE_KEY set?)`);
    }
  }
}
console.log(`collected ${copied} artifacts for ${target}`);
