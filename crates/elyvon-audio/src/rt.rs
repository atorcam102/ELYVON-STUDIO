//! Lock-free primitives for communicating with the real-time thread.

use std::sync::atomic::{AtomicU32, Ordering};

/// An `f32` that can be shared between the UI/control thread and the audio
/// thread without locks. Loads and stores are single atomic operations.
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    #[inline]
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn stores_and_loads_exact_values() {
        let a = AtomicF32::new(0.25);
        assert_eq!(a.load(), 0.25);
        a.store(-1.5);
        assert_eq!(a.load(), -1.5);
    }

    #[test]
    fn never_tears_under_contention() {
        let a = Arc::new(AtomicF32::new(1.0));
        let writer = {
            let a = Arc::clone(&a);
            std::thread::spawn(move || {
                for i in 0..10_000 {
                    a.store(if i % 2 == 0 { 1.0 } else { -1.0 });
                }
            })
        };
        for _ in 0..10_000 {
            let v = a.load();
            assert!(v == 1.0 || v == -1.0);
        }
        writer.join().unwrap();
    }
}
