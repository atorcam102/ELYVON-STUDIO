# Test-only signing fixtures

These files exercise signature verification in `elyvon-update`.

* `test-only.pub.b64` — public half of a throwaway key pair generated with
  `tauri signer generate` for these tests. The private key was **discarded**;
  it is not, and never was, a release signing key.
* `payload.bin` / `payload.bin.sig` — a payload signed with
  `tauri signer sign --app-version 0.2.0`.
* `payload-unversioned.bin.sig` — the same payload signed without
  `--app-version` (no `version:` field in the trusted comment).
