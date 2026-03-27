use crate::emulator::{Emulator, InputState};
use std::path::PathBuf;
use std::time::Instant;

const GBA_FPS: f64 = 59.7275;
const MAX_FRAMES_PER_UPDATE_TICK: usize = 8;

pub struct EmulatorState {
    pub emulator: Option<Emulator>,
    pub running: bool,
    pub error_message: Option<String>,

    pub bios_path: Option<PathBuf>,
    timing_anchor: Option<Instant>,
    frame_accumulator: f64,
}

impl EmulatorState {
    pub fn new(rom_data: Option<Vec<u8>>, bios_path: Option<PathBuf>) -> Self {
        let bios_data = Self::read_bios(bios_path.as_deref());
        let emulator = rom_data.map(|rom| Emulator::new(rom, bios_data));
        let running = emulator.is_some();
        Self {
            emulator,
            running,
            error_message: None,
            bios_path,
            timing_anchor: None,
            frame_accumulator: 0.0,
        }
    }

    pub fn load_rom(&mut self, rom_data: Vec<u8>) {
        let bios_data = Self::read_bios(self.bios_path.as_deref());
        self.emulator = Some(Emulator::new(rom_data, bios_data));
        self.running = true;
        self.error_message = None;
        self.reset_timing();
    }

    pub fn toggle_pause(&mut self) {
        if self.emulator.is_some() {
            self.running = !self.running;
            if self.running {
                self.reset_timing();
            }
        }
    }

    pub fn set_input(&mut self, input: InputState) {
        if let Some(emu) = &mut self.emulator {
            emu.set_input(input);
        }
    }

    pub fn step_frame(&mut self) {
        let Some(emu) = &mut self.emulator else { return };
        if !self.running {
            return;
        }

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
        let frames = (self.frame_accumulator.floor() as usize)
            .clamp(1, MAX_FRAMES_PER_UPDATE_TICK);
        self.frame_accumulator -= frames as f64;

        for _ in 0..frames {
            emu.run_frame();
        }
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
