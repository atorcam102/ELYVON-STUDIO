#!/usr/bin/env bash
# Linux x64 CI tooling only. Official archives, verified before extraction.
set -euo pipefail
output=$(realpath -m "${1:?usage: install-audit-tools.sh OUTPUT_DIRECTORY}")
mkdir -p "$output"
cd "$output"
cargo_archive=cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz
curl --fail --location --retry 3 "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/$cargo_archive" -o "$cargo_archive"
curl --fail --location --retry 3 "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/$cargo_archive.sha256" -o "$cargo_archive.sha256"
# Upstream's file contains only the hash, without a filename/newline.
printf '%s  %s\n' "$(cat "$cargo_archive.sha256")" "$cargo_archive" | sha256sum --check -
tar -xzf "$cargo_archive" --strip-components=1 cargo-deny-0.20.2-x86_64-unknown-linux-musl/cargo-deny
curl --fail --location --retry 3 https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_linux_amd64.tar.gz -o actionlint_1.7.12_linux_amd64.tar.gz
curl --fail --location --retry 3 https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_checksums.txt -o actionlint_1.7.12_checksums.txt
# Match exactly this platform; do not download/extract any unverified archive.
awk '$2 == "actionlint_1.7.12_linux_amd64.tar.gz" { print }' actionlint_1.7.12_checksums.txt | sha256sum --check -
tar -xzf actionlint_1.7.12_linux_amd64.tar.gz actionlint
./cargo-deny --version
./actionlint --version
