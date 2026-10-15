// Prints the release notes for a version from CHANGELOG.md.
//
//   node scripts/release-notes.mjs 0.1.0               # full GitHub release body
//   node scripts/release-notes.mjs 0.1.0 --notes-only  # changelog section only (updater dialog)
//
// Exits with an error if the changelog has no section for the version, so a
// release cannot ship without notes.

import { readFileSync } from "node:fs";

const version = process.argv[2]?.replace(/^v/, "");
const notesOnly = process.argv.includes("--notes-only");
if (!version) {
  console.error("usage: release-notes.mjs <version> [--notes-only]");
  process.exit(2);
}

const changelog = readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8");
const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const match = changelog.match(new RegExp(`^## \\[${escaped}\\][^\\n]*\\n([\\s\\S]*?)(?=^## \\[|(?![\\s\\S]))`, "m"));
if (!match) {
  console.error(`CHANGELOG.md has no "## [${version}]" section.`);
  process.exit(1);
}
const notes = match[1].trim();

if (notesOnly) {
  console.log(notes);
  process.exit(0);
}

const v = `v${version}`;
const rows = [
  ["Windows 10/11 · x64 (Intel/AMD 64-bit)", `Elyvon-Studio-${v}-windows-x64-setup.exe`, `Elyvon-Studio-${v}-windows-x64.msi`],
  ["Windows 10/11 · x86 (32-bit)", `Elyvon-Studio-${v}-windows-x86-setup.exe`, `Elyvon-Studio-${v}-windows-x86.msi`],
  ["Windows 11 · ARM64", `Elyvon-Studio-${v}-windows-arm64-setup.exe`, "—"],
  ["macOS 11+ · Intel", `Elyvon-Studio-${v}-macos-intel.dmg`, "—"],
  ["macOS 11+ · Apple Silicon", `Elyvon-Studio-${v}-macos-arm64.dmg`, "—"],
  ["Ubuntu 22.04+ · x64", `Elyvon-Studio-${v}-ubuntu-amd64.deb`, `Elyvon-Studio-${v}-ubuntu-amd64.AppImage`],
  ["Ubuntu 22.04+ · ARM64", `Elyvon-Studio-${v}-ubuntu-arm64.deb`, `Elyvon-Studio-${v}-ubuntu-arm64.AppImage`],
];

console.log(`${notes}

## Downloads

Pick the installer for your system and processor. Each file contains a single platform and architecture.

| System | Installer | Alternative |
| --- | --- | --- |
${rows.map(([s, a, b]) => `| ${s} | \`${a}\` | ${b === "—" ? "—" : `\`${b}\``} |`).join("\n")}

## Installation

- **Windows:** run the \`-setup.exe\` (per-user install, no administrator rights needed). The \`.msi\` is for managed deployments.
- **macOS:** open the \`.dmg\` and drag Elyvon Studio to Applications. Builds without an Apple Developer ID are ad-hoc signed; on first launch use *Right-click › Open*.
- **Ubuntu:** \`sudo apt install ./Elyvon-Studio-${v}-ubuntu-<arch>.deb\`, or make the \`.AppImage\` executable (\`chmod +x\`) and run it.

Installed copies update themselves from this page: Elyvon Studio checks for updates (if enabled in Settings), shows these notes and installs only after you confirm. Every update is verified against the signing key built into the application.

## Verification

\`SHA256SUMS.txt\` lists the SHA-256 checksum of every file. On Linux/macOS run \`sha256sum -c SHA256SUMS.txt --ignore-missing\` (or \`shasum -a 256 -c\`); on Windows, \`Get-FileHash <file> -Algorithm SHA256\`.
`);
