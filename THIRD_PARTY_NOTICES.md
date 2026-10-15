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

## Audio engine dependencies (Phase 03)

| Crate | Version | Licence | Use |
| --- | --- | --- | --- |
| cpal | 0.18.2 | Apache-2.0 | Audio device I/O (WASAPI, CoreAudio, ALSA) |
| rtrb | 0.4.0 | MIT OR Apache-2.0 | Wait-free command ring buffer |
| audio_thread_priority | 0.35.1 | **MPL-2.0** | Real-time thread priority (CPAL `realtime` feature) |
| alsa / alsa-sys | 0.11.0 / 0.4.0 | Apache-2.0 OR MIT / MIT | Linux ALSA bindings |
| coreaudio-rs, objc2-core-audio, objc2-audio-toolbox, objc2-avf-audio | see Cargo.lock | MIT/Apache-2.0/Zlib | macOS CoreAudio bindings |
| dasp_sample, mach2, num-derive | see Cargo.lock | MIT/Apache-2.0/BSD-2-Clause | Transitive helpers |

MPL-2.0 is file-level copyleft: shipping binaries requires including its licence notice and making
the source of the (unmodified) MPL files available, e.g. by reference to the published crate. This
is a shipping obligation to discharge before stable distribution, not a legal clearance. The Linux
build links the system `libasound` (LGPL-2.1) dynamically. No ASIO SDK, JACK or proprietary audio
code is included; the test tone is generated mathematically and no audio samples are shipped.

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
