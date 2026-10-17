# Security policy

## Supported versions

Only the latest published Windows preview receives fixes. Previews are unsigned (no Authenticode,
no updater signature); verify downloads against the release's `SHA256SUMS.txt`.

## Reporting a vulnerability

Please report vulnerabilities privately through
[GitHub security advisories](https://github.com/atorcam102/ELYVON-STUDIO/security/advisories/new),
not in a public issue. Include the version, the platform, steps to reproduce and, if possible, a
project or media file that triggers the problem. You will receive an answer within seven days.

Areas of particular interest: parsing of `.elyvon` projects, audio and MIDI files, the update
check and manifest verification, and file-system operations such as collect media and backups.
