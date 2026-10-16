# Third-party notices and remaining distribution work

Elyvon Studio has not yet selected a first-party distribution licence. This file does not grant
rights to first-party assets/code and does not claim that installer notice obligations are complete.

Dependency metadata and exact versions are reproducible with
`node scripts/dependency-inventory.mjs dependency-inventory.json` after npm ci and Rust setup;
CI uploads the result as audit-evidence. Rust policy is in deny.toml, npm metadata policy in
scripts/dependency-inventory.mjs. Optional/dev/platform dependencies are included in the inventory.

The source archive keeps these notices. Full licence texts/notices for shipped Rust/npm code and
bundled/system libraries must be collected and included appropriately in installers before stable
distribution. MPL/CC-BY and platform redistribution obligations require a shipping review; accepting
an identifier in a CI policy does not discharge those obligations. No commercial audio sample,
Apple instrument, private project format or proprietary DSP is included in the new test fixtures.
The origin/licensing of historical brand assets is not established by this technical audit.

## Audio engine dependencies (Phases 04–05)

Versions are pinned in Cargo.lock; `cargo deny --locked check` reports licences ok.

| Crate | Licence | Use | Notes |
| --- | --- | --- | --- |
| symphonia 0.6.1 (+ symphonia-core, -common, -metadata, -format-riff, -format-ogg, -bundle-flac, -bundle-mp3, -codec-pcm, -codec-vorbis) | MPL-2.0 | Decoding WAV/AIFF/FLAC/MP3/Ogg Vorbis | Used **unmodified** from crates.io. MPL-2.0 is file-level copyleft: distributions must keep the notice and make the Source Code Form of these files available (https://github.com/pdeljanov/Symphonia). No Elyvon file is MPL-licensed. |
| rubato 5.0.1, audioadapter 5.x, realfft, rustfft | MIT OR Apache-2.0 | Sample-rate conversion (FFT) | |
| cpal 0.18.2 | Apache-2.0 | Audio device output (WASAPI, CoreAudio, ALSA) | Linux builds link the system ALSA library (libasound, LGPL-2.1, dynamically linked). |
| rtrb 0.4.0 | MIT OR Apache-2.0 | Lock-free SPSC queues to the audio thread | |

Codec notes: MP3 patents have expired; FLAC and Vorbis are royalty-free open formats. No
proprietary codec (AAC, ALAC, Atmos) is enabled.

The test signals `crates/elyvon-audio/tests/fixtures/tone.{flac,mp3,ogg}` and every WAV/AIFF written
by the tests are synthetic sines created for this repository (see the fixtures README). The
metronome click is synthesised in code. No third-party audio is included.

## Phosphor Icons React

The interface uses @phosphor-icons/react 2.1.10 (MIT), from
https://github.com/phosphor-icons/react. Phosphor replaces the earlier bespoke icon map.

MIT License

Copyright (c) 2020 Phosphor Icons

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


## MIDI and recording dependencies

| Crate | Licence | Use | Source / notice |
| --- | --- | --- | --- |
| midir 0.11.1 | MIT | MIDI 1.0 native input/output through ALSA, CoreMIDI and Windows backends | https://github.com/Boddlnagg/midir — full notice: [midir-MIT.txt](docs/licenses/midir-MIT.txt), copyright 2015 Patrick Reisert and the RtMidi contributors |
| midly 0.5.3 | Unlicense | SMF type 0/1 parsing | https://github.com/negamartin/midly — the verified crates.io package declares SPDX `Unlicense` in Cargo.toml. A version-specific cargo-deny licence exception documents this permissive public-domain dedication; no other licence-policy entries are broadened. |
| tempfile 3.27.0 | MIT OR Apache-2.0 | Unique recording takes, bounded PCM cache and atomic export staging | https://github.com/Stebalien/tempfile — previously a workspace test dependency, now also used by native audio at runtime |

MIDI fixtures are original protocol messages and synthetic note sequences. They contain no
third-party performances, samples, instrument presets or proprietary formats. MIDI 2.0/UMP
compatibility is not claimed. The dependency inventory covers the full locked graph, including
platform bindings; the previously documented packaged-notice work remains outstanding.
