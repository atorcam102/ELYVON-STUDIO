//! Worker-decoded PCM disk cache with a bounded, nonblocking RT page cache.
//! Atomic samples plus a generation stamp prevent readers seeing torn pages.
//! A cache miss returns silence and increments an explicit underrun counter;
//! decoding, seeking, resampling, waiting and reclamation stay off the callback.
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{
    MediaError,
    decode::decode_blocks,
    peaks::{PeakBuilder, PeakPyramid},
};
use elyvon_project::MediaInfo;
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler};
use tempfile::NamedTempFile;

pub const PAGE_FRAMES: usize = 4096;
const PAGES: usize = 32;
const EMPTY: u64 = u64::MAX;

#[derive(Debug)]
struct Page {
    stamp: AtomicU64,
    index: AtomicU64,
    request: AtomicU64,
    samples: Box<[AtomicU32]>,
    touched: AtomicU64,
}

#[derive(Debug)]
struct Cache {
    pages: Vec<Page>,
    frames: u64,
    hint: AtomicUsize,
    clock: AtomicU64,
    read_ahead: AtomicBool,
    stop: AtomicBool,
    failed: AtomicBool,
    underruns: AtomicU64,
}

/// Owned only by immutable media plans. Dropped on the control/worker thread.
#[derive(Debug)]
pub struct DiskStream {
    cache: Arc<Cache>,
    worker: Option<JoinHandle<()>>,
    // Removed only after the worker has closed its file handle (also on Windows).
    _file: NamedTempFile,
}

impl Drop for DiskStream {
    fn drop(&mut self) {
        self.cache.stop.store(true, SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

fn io(e: std::io::Error) -> MediaError {
    MediaError::Io(e.to_string())
}

impl DiskStream {
    /// Packet decoding and FFT resampling use bounded working memory.
    pub fn prepare(
        path: &Path,
        rate: u32,
        budget: usize,
    ) -> Result<(Self, MediaInfo, PeakPyramid), MediaError> {
        if rate == 0 || budget < Self::cache_bytes() + 4096 {
            return Err(MediaError::TooLarge);
        }
        let mut raw = NamedTempFile::new().map_err(io)?;
        let mut peaks = PeakBuilder::default();
        // Finest peaks cost 4 bytes/channel per 256 source frames. Allow room
        // for all coarser levels and cap the decoded cache to classic WAV size.
        let max_frames =
            (((budget - Self::cache_bytes()) / 16) as u64 * 256).min((u32::MAX as u64) / 8);
        let info = decode_blocks(path, max_frames, &mut |planes, _| {
            peaks.push(planes);
            let n = planes.first().map_or(0, Vec::len);
            let mut bytes = Vec::with_capacity(n * planes.len() * 4);
            for i in 0..n {
                for p in planes {
                    bytes.extend_from_slice(&p[i].to_le_bytes());
                }
            }
            raw.write_all(&bytes).map_err(io)
        })?;
        raw.flush().map_err(io)?;
        let file = if info.sample_rate == rate {
            raw
        } else {
            resample_file(
                &mut raw,
                info.frames,
                usize::from(info.channels).min(2),
                info.sample_rate,
                rate,
            )?
        };
        let frames = elyvon_core::tempo::round_frame(
            info.frames as f64 * f64::from(rate) / f64::from(info.sample_rate),
        );
        let cache = Arc::new(Cache {
            pages: (0..PAGES)
                .map(|_| Page {
                    stamp: AtomicU64::new(0),
                    index: AtomicU64::new(EMPTY),
                    request: AtomicU64::new(EMPTY),
                    touched: AtomicU64::new(0),
                    samples: (0..PAGE_FRAMES * 2).map(|_| AtomicU32::new(0)).collect(),
                })
                .collect(),
            frames,
            hint: AtomicUsize::new(0),
            clock: AtomicU64::new(1),
            read_ahead: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
        });
        let mut reader = file.reopen().map_err(io)?;
        let c = Arc::clone(&cache);
        let channels = usize::from(info.channels).min(2);
        let worker = std::thread::Builder::new()
            .name("elyvon-disk-read".into())
            .spawn(move || {
                let mut bytes = vec![0u8; PAGE_FRAMES * channels * 4];
                while !c.stop.load(SeqCst) {
                    let mut busy = false;
                    for page in &c.pages {
                        let index = page.request.swap(EMPTY, SeqCst);
                        if index == EMPTY || c.pages.iter().any(|p| p.index.load(SeqCst) == index) {
                            continue;
                        }
                        let page = c
                            .pages
                            .iter()
                            .find(|p| p.index.load(SeqCst) == EMPTY)
                            .unwrap_or_else(|| {
                                c.pages
                                    .iter()
                                    .min_by_key(|p| p.touched.load(SeqCst))
                                    .unwrap_or(&c.pages[0])
                            });
                        busy = true;
                        let start = index * PAGE_FRAMES as u64;
                        let n = (c.frames.saturating_sub(start)).min(PAGE_FRAMES as u64) as usize;
                        if n == 0 {
                            continue;
                        }
                        if reader
                            .seek(SeekFrom::Start(start * channels as u64 * 4))
                            .and_then(|_| reader.read_exact(&mut bytes[..n * channels * 4]))
                            .is_err()
                        {
                            c.failed.store(true, SeqCst);
                            continue;
                        }
                        page.stamp.fetch_add(1, SeqCst); // odd: replacement in progress
                        for frame in 0..n {
                            for ch in 0..2 {
                                let at = (frame * channels + ch.min(channels - 1)) * 4;
                                let bits = u32::from_le_bytes(
                                    bytes[at..at + 4].try_into().unwrap_or([0; 4]),
                                );
                                page.samples[frame * 2 + ch].store(bits, SeqCst);
                            }
                        }
                        page.index.store(index, SeqCst);
                        page.touched.store(c.clock.fetch_add(1, SeqCst), SeqCst);
                        page.stamp.fetch_add(1, SeqCst); // even: complete
                    }
                    if !busy {
                        std::thread::park_timeout(Duration::from_millis(1));
                    }
                }
            })
            .map_err(io)?;
        let stream = Self {
            cache,
            worker: Some(worker),
            _file: file,
        };
        stream.prefetch(0, PAGE_FRAMES * 4)?;
        Ok((stream, info, peaks.finish()))
    }

    pub const fn cache_bytes() -> usize {
        PAGES * PAGE_FRAMES * 2 * 4
    }
    pub fn frames(&self) -> u64 {
        self.cache.frames
    }
    pub fn underruns(&self) -> u64 {
        self.cache.underruns.load(SeqCst)
    }
    pub fn set_read_ahead(&self, enabled: bool) -> bool {
        let old = self.cache.read_ahead.swap(enabled, SeqCst);
        if !enabled {
            for p in &self.cache.pages {
                p.request.store(EMPTY, SeqCst);
            }
        }
        old
    }
    pub fn failed(&self) -> bool {
        self.cache.failed.load(SeqCst)
    }

    fn find_page(&self, index: u64) -> Option<&Page> {
        let hint = self.cache.hint.load(SeqCst) % PAGES;
        if self.cache.pages[hint].index.load(SeqCst) == index {
            return Some(&self.cache.pages[hint]);
        }
        self.cache
            .pages
            .iter()
            .enumerate()
            .find(|(_, p)| p.index.load(SeqCst) == index)
            .map(|(i, p)| {
                self.cache.hint.store(i, SeqCst);
                p
            })
    }
    pub fn request(&self, frame: u64) {
        if frame >= self.cache.frames {
            return;
        }
        let index = frame / PAGE_FRAMES as u64;
        if let Some(page) = self.find_page(index) {
            page.touched
                .store(self.cache.clock.fetch_add(1, SeqCst), SeqCst);
            return;
        }
        for n in 0..PAGES {
            let slot = &self.cache.pages[(index as usize + n) % PAGES].request;
            if slot.load(SeqCst) == index
                || slot.compare_exchange(EMPTY, index, SeqCst, SeqCst).is_ok()
            {
                return;
            }
        }
    }

    /// RT safe; no IO, wait, ownership transfer, allocation or lock.
    pub fn sample(&self, frame: u64) -> [f32; 2] {
        if frame >= self.cache.frames {
            return [0.0; 2];
        }
        if self.cache.read_ahead.load(SeqCst) && frame.is_multiple_of(256) {
            self.request(frame.saturating_add(PAGE_FRAMES as u64));
            self.request(frame.saturating_add(2 * PAGE_FRAMES as u64));
        }
        let index = frame / PAGE_FRAMES as u64;
        let Some(page) = self.find_page(index) else {
            self.request(frame);
            self.cache.underruns.fetch_add(1, SeqCst);
            return [0.0; 2];
        };
        page.touched
            .store(self.cache.clock.fetch_add(1, SeqCst), SeqCst);
        let stamp = page.stamp.load(SeqCst);
        if stamp.is_multiple_of(2) && page.index.load(SeqCst) == index {
            let i = frame as usize % PAGE_FRAMES * 2;
            let pair = [
                f32::from_bits(page.samples[i].load(SeqCst)),
                f32::from_bits(page.samples[i + 1].load(SeqCst)),
            ];
            if page.stamp.load(SeqCst) == stamp {
                return pair;
            }
        }
        self.cache.underruns.fetch_add(1, SeqCst);
        [0.0; 2]
    }

    /// Control/offline only. Prepares a bounded range and surfaces IO failures.
    pub fn prefetch(&self, start: u64, frames: usize) -> Result<(), MediaError> {
        let end = start.saturating_add(frames as u64).min(self.cache.frames);
        if end <= start {
            return Ok(());
        }
        let first = start / PAGE_FRAMES as u64;
        let last = (end - 1) / PAGE_FRAMES as u64;
        if last - first >= PAGES as u64 {
            return Err(MediaError::TooLarge);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        for index in first..=last {
            self.request(index * PAGE_FRAMES as u64);
            while self
                .find_page(index)
                .is_none_or(|page| !page.stamp.load(SeqCst).is_multiple_of(2))
            {
                if self.failed() {
                    return Err(MediaError::Io("disk-cache read failed".into()));
                }
                if Instant::now() >= deadline {
                    return Err(MediaError::Io("disk-cache read timed out".into()));
                }
                self.request(index * PAGE_FRAMES as u64);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }
}

/// The same rubato FFT algorithm as in-memory preparation, pumped one chunk
/// at a time. Trim startup delay once; flush zeros and trim the final length.
fn resample_file(
    raw: &mut NamedTempFile,
    frames: u64,
    channels: usize,
    from: u32,
    to: u32,
) -> Result<NamedTempFile, MediaError> {
    let mut output = NamedTempFile::new().map_err(io)?;
    raw.seek(SeekFrom::Start(0)).map_err(io)?;
    let err = |e: &dyn std::fmt::Display| MediaError::Resample(e.to_string());
    let mut r = Fft::<f32>::new(from as usize, to as usize, 1024, channels, FixedSync::Input)
        .map_err(|e| err(&e))?;
    let mut input = vec![vec![0.0f32; r.input_frames_max()]; channels];
    let mut out = vec![vec![0.0f32; r.output_frames_max()]; channels];
    let mut bytes = vec![0u8; r.input_frames_max() * channels * 4];
    let mut remaining = frames;
    let expected = elyvon_core::tempo::round_frame(frames as f64 * f64::from(to) / f64::from(from));
    let mut written = 0u64;
    let mut trim = r.output_delay();
    while written < expected {
        let n = r.input_frames_next();
        let take = remaining.min(n as u64) as usize;
        raw.read_exact(&mut bytes[..take * channels * 4])
            .map_err(io)?;
        for (ch, p) in input.iter_mut().enumerate() {
            p.fill(0.0);
            for (i, s) in p.iter_mut().take(take).enumerate() {
                let at = (i * channels + ch) * 4;
                *s = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4]));
            }
        }
        remaining -= take as u64;
        let a = SequentialSliceOfVecs::new(&input, channels, n).map_err(|e| err(&e))?;
        let out_len = r.output_frames_max();
        let mut b =
            SequentialSliceOfVecs::new_mut(&mut out, channels, out_len).map_err(|e| err(&e))?;
        let (_, produced) = r
            .process_into_buffer(&a, &mut b, None)
            .map_err(|e| err(&e))?;
        let skip = trim.min(produced);
        trim -= skip;
        let use_frames = (produced - skip).min((expected - written) as usize);
        let mut encoded = Vec::with_capacity(use_frames * channels * 4);
        for i in skip..skip + use_frames {
            for p in &out {
                encoded.extend_from_slice(&p[i].to_le_bytes());
            }
        }
        output.write_all(&encoded).map_err(io)?;
        written += use_frames as u64;
    }
    output.flush().map_err(io)?;
    Ok(output)
}
