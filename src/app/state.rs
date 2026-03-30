use crate::audio::AudioBackend;
use crate::emulator::{Emulator, InputState};
use std::path::PathBuf;
use std::time::Instant;

const GBA_FPS: f64 = 59.7275;
const MAX_FRAMES_PER_UPDATE_TICK: usize = 8;
const AUDIO_TARGET_BUFFER_MS: usize = 50;
const AUDIO_LOW_WATERMARK_MS: usize = 30;

pub struct EmulatorState {
    pub emulator: Option<Emulator>,
    pub running: bool,
    pub error_message: Option<String>,
    pub bios_path: Option<PathBuf>,
    pub audio: Option<AudioBackend>,
    pub audio_debug: bool,

    timing_anchor: Option<Instant>,
    frame_accumulator: f64,
    audio_samples: Vec<f32>,
}

impl EmulatorState {
    pub fn new(rom_data: Option<Vec<u8>>, bios_path: Option<PathBuf>) -> Self {
        let audio = match AudioBackend::new(40) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("Audio disabled: {e}");
                None
            }
        };

        let bios_data = Self::read_bios(bios_path.as_deref());
        let mut emulator = rom_data.map(|rom| Emulator::new(rom, bios_data));

        if let (Some(emu), Some(a)) = (&mut emulator, &audio) {
            emu.set_audio_sample_rate(a.sample_rate());
        }

        let running = emulator.is_some();
        Self {
            emulator,
            running,
            error_message: None,
            bios_path,
            audio,
            audio_debug: false,
            timing_anchor: None,
            frame_accumulator: 0.0,
            audio_samples: Vec::new(),
        }
    }

    pub fn load_rom(&mut self, rom_data: Vec<u8>) {
        let bios_data = Self::read_bios(self.bios_path.as_deref());
        let mut emu = Emulator::new(rom_data, bios_data);
        if let Some(a) = &self.audio {
            emu.set_audio_sample_rate(a.sample_rate());
            a.clear_buffer();
        }
        self.emulator = Some(emu);
        self.running = true;
        self.error_message = None;
        self.reset_timing();
    }

    pub fn toggle_pause(&mut self) {
        if self.emulator.is_some() {
            self.running = !self.running;
            if self.running {
                self.reset_timing();
            } else if let Some(a) = &self.audio {
                a.clear_buffer();
            }
        }
    }

    pub fn set_input(&mut self, input: InputState) {
        if let Some(emu) = &mut self.emulator {
            emu.set_input(input);
        }
    }

    /// Returns a snapshot of audio pipeline health for the debug overlay.
    pub fn audio_diag(&self) -> Option<AudioDiag> {
        let audio = self.audio.as_ref()?;
        let snap = audio.controls().snapshot();
        let buf_frames = audio.buffer_len();
        let buf_cap = audio.buffer_capacity();
        let buf_ms = buf_frames as f32 / audio.sample_rate() as f32 * 1000.0;
        Some(AudioDiag {
            buf_frames,
            buf_cap,
            buf_ms,
            underflows: snap.underflows,
            overflows: snap.overflows,
        })
    }

    pub fn step_frame(&mut self) {
        if self.emulator.is_none() || !self.running { return; }

        let now = Instant::now();
        if self.timing_anchor.is_none() {
            self.timing_anchor = Some(now);
        }
        let elapsed = now
            .saturating_duration_since(self.timing_anchor.unwrap())
            .as_secs_f64()
            .min(0.25);
        self.timing_anchor = Some(now);

        self.frame_accumulator += elapsed * GBA_FPS;
        let base_frames = self.frame_accumulator.floor() as usize;
        self.frame_accumulator -= base_frames as f64;

        let extra = self.audio_catchup_frames();
        // Do NOT clamp to a minimum of 1 unconditionally — that would run the emulator
        // at the display refresh rate (~60 Hz) instead of the GBA's 59.7275 Hz, producing
        // ~0.45% more audio samples than the device can consume (systematic overflow/cracks).
        // Only enforce the minimum-1 floor when the audio buffer is low and needs catchup.
        let frames = if extra > 0 {
            base_frames.saturating_add(extra).clamp(1, MAX_FRAMES_PER_UPDATE_TICK)
        } else {
            base_frames.min(MAX_FRAMES_PER_UPDATE_TICK)
        };

        for _ in 0..frames {
            if let Some(emu) = &mut self.emulator {
                emu.run_frame();
                // drain APU samples
                self.audio_samples.clear();
                emu.drain_audio_samples(&mut self.audio_samples);
            }
            if let Some(audio) = &self.audio {
                if !self.audio_samples.is_empty() {
                    audio.push_samples(&self.audio_samples);
                }
            }
        }
    }

    fn audio_catchup_frames(&self) -> usize {
        let Some(audio) = &self.audio else { return 0 };
        let low = (audio.sample_rate() as usize * AUDIO_LOW_WATERMARK_MS / 1000).max(1024);
        if audio.buffer_len() >= low { return 0; }
        let target = (audio.sample_rate() as usize * AUDIO_TARGET_BUFFER_MS / 1000).max(1024);
        let spf = (audio.sample_rate() as f64 / GBA_FPS).max(1.0);
        let deficit = target.saturating_sub(audio.buffer_len());
        ((deficit as f64 / spf).ceil() as usize).min(4)
    }

    fn reset_timing(&mut self) {
        self.timing_anchor = None;
        self.frame_accumulator = 0.0;
    }
    fn read_bios(path: Option<&std::path::Path>) -> Option<Vec<u8>> {
        let path = path?;
        match std::fs::read(path) {
            Ok(data) => Some(data),
            Err(e) => {
                eprintln!("Failed to read BIOS '{}': {e}", path.display());
                None
            }
        }
    }
}

/// Snapshot of audio pipeline health for the debug overlay.
#[derive(Debug, Clone, Copy)]
pub struct AudioDiag {
    pub buf_frames: usize,
    pub buf_cap: usize,
    pub buf_ms: f32,
    pub underflows: u64,
    pub overflows: u64,
}
