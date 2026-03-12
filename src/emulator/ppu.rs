use crate::{
    emulator::interrupt::{Interrupt, signal_irq},
    utils::*,
};
use bitfield::bitfield;
use std::{cell::RefCell, rc::Rc};

const CYCLES_PER_FRAME: u32 = 280_896;

pub const SCREEN_WIDTH: usize = 240;
pub const SCREEN_HEIGHT: usize = 160;

const SCANLINES: usize = SCREEN_HEIGHT + 68;
const CYCLES_PER_SCANLINE: usize = CYCLES_PER_FRAME as usize / SCANLINES;

bitfield!(
    #[derive(Clone)]
    pub struct DispCnt(u16);
    impl Debug;
    pub mode, set_mode: 2, 0;
    pub cgb_mode, set_cgb_mode: 3;
    pub frame_select, set_frame_select: 4;
    pub hblank_interval_free, set_hblank_interval_free: 5;
    pub obj_character_mapping, set_obj_character_mapping: 6;
    pub forced_blank, set_forced_blank: 7;
    pub bg0_enable, set_bg0_enable: 8;
    pub bg1_enable, set_bg1_enable: 9;
    pub bg2_enable, set_bg2_enable: 10;
    pub bg3_enable, set_bg3_enable: 11;
    pub obj_enable, set_obj_enable: 12;
    pub win0_enable, set_win0_enable: 13;
    pub win1_enable, set_win1_enable: 14;
    pub obj_win_enable, set_obj_win_enable: 15;
);

bitfield!(
    #[derive(Clone)]
    pub struct DispStat(u16);
    impl Debug;
    pub vblank_flag, set_vblank_flag: 0;
    pub hblank_flag, set_hblank_flag: 1;
    pub vcount_flag, set_vcount_flag: 2;
    pub vblank_irq_enable, set_vblank_irq_enable: 3;
    pub hblank_irq_enable, set_hblank_irq_enable: 4;
    pub vcount_irq_enable, set_vcount_irq_enable: 5;
    pub vcount_setting, set_vcount_setting: 15, 8;
);

/// Pixel Processing Unit scaffold.
#[derive(Clone)]
pub struct Ppu {
    // Reference to interrupt flags for signaling VBlank and other events.
    interrupt_flags: Rc<RefCell<u16>>,

    // Frame timing
    frame_cycles: u32,
    frame_ready: bool,

    // Registers
    dispcnt: DispCnt,
    dispstat: DispStat,
    vcount: u16,

    // Pixel buffer for the current frame (15-bit BGR555, row-major, 240×160)
    buffer: [u16; SCREEN_WIDTH * SCREEN_HEIGHT],
}

impl Ppu {
    pub fn new(interrupt_flags: Rc<RefCell<u16>>) -> Self {
        Self {
            interrupt_flags,
            frame_cycles: 0,
            frame_ready: false,
            dispcnt: DispCnt(0),
            dispstat: DispStat(0),
            vcount: 0,
            buffer: [0; SCREEN_WIDTH * SCREEN_HEIGHT],
        }
    }

    pub fn reset(&mut self) {
        self.frame_cycles = 0;
        self.frame_ready = false;
        self.dispcnt = DispCnt(0);
        self.dispstat = DispStat(0);
        self.vcount = 0;
        self.buffer = [0; SCREEN_WIDTH * SCREEN_HEIGHT];
    }

    pub fn skip_bios(&mut self) {
        // self.vcount = 160;
        // self.frame_ready = true;
    }

    pub fn begin_frame(&mut self) {
        self.frame_cycles = 0;
        self.frame_ready = false;
        self.vcount = 0;
        self.dispstat.set_vblank_flag(false);
    }

    pub fn tick(&mut self, vram: &[u8], pram: &[u8]) {
        // Calculate the current scanline based on frame cycles before advancing.
        let prev_scanline = self.frame_cycles / CYCLES_PER_SCANLINE as u32;

        // Advance the frame cycle count by one tick
        self.frame_cycles = self.frame_cycles.saturating_add(1);

        // Check if we've completed the frame
        if self.frame_cycles >= CYCLES_PER_FRAME {
            self.frame_ready = true;
            return;
        }

        // Only update scanline and render when crossing into a new scanline
        let new_scanline = self.frame_cycles / CYCLES_PER_SCANLINE as u32;
        if new_scanline == prev_scanline {
            return;
        }

        // Update vcount and related flags/interrupts on scanline change
        self.vcount = new_scanline as u16;

        // Render the scanline that just became active (while still in visible area).
        if (self.vcount as usize) < SCREEN_HEIGHT {
            self.render_scanline(self.vcount as usize, vram, pram);
        }

        // Update VBlank and VCount flags/interrupts based on the new vcount value.
        if self.vcount == SCREEN_HEIGHT as u16 {
            // Entering VBlank
            self.dispstat.set_vblank_flag(true);
            if self.dispstat.vblank_irq_enable() {
                signal_irq(&self.interrupt_flags, Interrupt::VBlank);
            }
        } else if self.vcount == 0 {
            // VBlank ended
            self.dispstat.set_vblank_flag(false);
        }

        // Update VCount flag and interrupt if we've hit the specified VCount setting.
        if self.vcount == self.dispstat.vcount_setting() {
            self.dispstat.set_vcount_flag(true);
            if self.dispstat.vcount_irq_enable() {
                signal_irq(&self.interrupt_flags, Interrupt::VCount);
            }
        } else {
            self.dispstat.set_vcount_flag(false);
        }
    }

    fn render_scanline(&mut self, y: usize, vram: &[u8], pram: &[u8]) {
        if self.dispcnt.forced_blank() {
            let base = y * SCREEN_WIDTH;
            self.buffer[base..base + SCREEN_WIDTH].fill(0x7FFF);
            return;
        }
        match self.dispcnt.mode() {
            3 => self.render_mode3(y, vram),
            4 => self.render_mode4(y, vram, pram),
            _ => {
                // Unsupported modes: fill with black
                let base = y * SCREEN_WIDTH;
                self.buffer[base..base + SCREEN_WIDTH].fill(0);
            }
        }
    }

    /// Mode 3: 240×160 15-bit direct-colour bitmap.
    /// Each pixel is a 16-bit little-endian BGR555 value starting at VRAM offset 0.
    fn render_mode3(&mut self, y: usize, vram: &[u8]) {
        let row_base = y * SCREEN_WIDTH;
        let vram_base = y * SCREEN_WIDTH * 2;

        for x in 0..SCREEN_WIDTH {
            let off = vram_base + x * 2;

            let lo = vram.get(off).copied().unwrap_or(0) as u16;
            let hi = vram.get(off + 1).copied().unwrap_or(0) as u16;

            self.buffer[row_base + x] = lo | (hi << 8);
        }
    }

    /// Mode 4: 240×160 8-bit indexed-color bitmap with 256-entry palette.
    /// Each pixel is a single byte index into the palette, starting at VRAM offset 0.
    fn render_mode4(&mut self, y: usize, vram: &[u8], pram: &[u8]) {
        let row_base = y * SCREEN_WIDTH;
        let vram_row_start = y * SCREEN_WIDTH;

        for x in 0..SCREEN_WIDTH {
            let index = vram.get(vram_row_start + x).copied().unwrap_or(0) as usize;

            let pram_off = index * 2;
            let lo = pram.get(pram_off).copied().unwrap_or(0) as u16;
            let hi = pram.get(pram_off + 1).copied().unwrap_or(0) as u16;

            self.buffer[row_base + x] = lo | (hi << 8);
        }
    }

    pub fn frame_ready(&self) -> bool {
        self.frame_ready
    }

    /// Returns the completed framebuffer as a flat BGR555 slice (240×160 pixels, row-major).
    pub fn framebuffer(&self) -> &[u16] {
        &self.buffer
    }

    pub fn read8(&self, addr: u32) -> u8 {
        match addr {
            0x4000000 => get_lo(self.dispcnt.0),
            0x4000001 => get_hi(self.dispcnt.0),
            0x4000004 => get_lo(self.dispstat.0),
            0x4000005 => get_hi(self.dispstat.0),
            0x4000006 => get_lo(self.vcount),
            0x4000007 => get_hi(self.vcount),
            _ => 0,
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        match addr {
            0x4000000 => set_lo(&mut self.dispcnt.0, value),
            0x4000001 => set_hi(&mut self.dispcnt.0, value),
            0x4000004 => set_lo(&mut self.dispstat.0, value & 0xF8),
            0x4000005 => set_hi(&mut self.dispstat.0, value),
            0x4000006..=0x4000007 => {} // vcount is read-only
            _ => {}
        }
    }
}
