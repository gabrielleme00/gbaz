use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc,
};

#[derive(Debug, Clone, Copy)]
pub struct AudioControlsSnapshot {
    pub volume: f32,
    pub muted: bool,
    pub underflows: u64,
    pub overflows: u64,
}

#[derive(Default)]
pub struct AudioControls {
    volume_bits: AtomicU32,
    muted: AtomicBool,
    underflows: AtomicU64,
    overflows: AtomicU64,
}

impl AudioControls {
    pub fn new() -> Self {
        Self {
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            underflows: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
        }
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume_bits
            .store(volume.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn add_underflows(&self, count: u64) {
        self.underflows.fetch_add(count, Ordering::Relaxed);
    }

    pub fn add_overflows(&self, count: u64) {
        self.overflows.fetch_add(count, Ordering::Relaxed);
    }

    pub fn reset_diagnostics(&self) {
        self.underflows.store(0, Ordering::Relaxed);
        self.overflows.store(0, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> AudioControlsSnapshot {
        AudioControlsSnapshot {
            volume: self.volume(),
            muted: self.muted(),
            underflows: self.underflows.load(Ordering::Relaxed),
            overflows: self.overflows.load(Ordering::Relaxed),
        }
    }
}

pub type SharedAudioControls = Arc<AudioControls>;
