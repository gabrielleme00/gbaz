mod layer;
mod regs;
mod bg;
mod rgb15;
mod sfx;
mod window;
mod obj;

use crate::{
    emulator::{
        interrupt::{Interrupt, signal_irq},
        ppu::bg::Point,
    },
    index2d,
};
use core::panic;
pub use layer::*;
pub use regs::*;
pub use rgb15::*;
use std::{cell::RefCell, rc::Rc};
use window::*;

pub mod consts {
    pub const VRAM_SIZE: usize = 128 * 1024;
    pub const PRAM_SIZE: usize = 1 * 1024;
    pub const OAM_SIZE: usize = 1 * 1024;

    pub const SCREEN_WIDTH: usize = 240;
    pub const SCREEN_HEIGHT: usize = 160;

    pub const SCANLINES: usize = SCREEN_HEIGHT + 68;
    pub const CYCLES_PER_SCANLINE: usize = CYCLES_PER_FRAME as usize / SCANLINES;
    pub const CYCLES_PER_FRAME: u32 = 280_896;

    pub const TILE_SIZE: u32 = 0x20;

    pub const VRAM_OBJ_TILES_START_TEXT: u32 = 0x1_0000;
    pub const VRAM_OBJ_TILES_START_BITMAP: u32 = 0x1_4000;
}
pub use consts::*;

#[derive(Debug, Copy, Clone)]
pub enum PixelFormat {
    BPP4 = 0,
    BPP8 = 1,
}

bitfield! {
    struct TileMapEntry(u16);
    u16;
    u32, tile_index, _: 9, 0;
    x_flip, _ : 10;
    y_flip, _ : 11;
    palette_bank, _ : 15, 12;
}

#[derive(Debug, Copy, Clone)]
pub struct ObjBufferEntry {
    pub(super) window: bool,
    pub(super) alpha: bool,
    pub(super) color: Rgb15,
    pub(super) priority: u16,
}

impl Default for ObjBufferEntry {
    fn default() -> ObjBufferEntry {
        ObjBufferEntry {
            window: false,
            alpha: false,
            color: Rgb15::TRANSPARENT,
            priority: 4,
        }
    }
}

/// Pixel Processing Unit scaffold.
#[derive(Clone)]
pub struct Ppu {
    // Reference to interrupt flags for signaling VBlank and other events.
    interrupt_flags: Rc<RefCell<u16>>,

    // Frame timing
    frame_cycles: u32,
    frame_ready: bool,

    // General control registers
    dispcnt: DisplayControl,
    dispstat: DisplayStatus,
    vcount: u16,

    // Background control registers and internal affine reference registers for BG2/BG3
    bgcnt: [BgControl; 4],
    bg_hofs: [u16; 4],
    bg_vofs: [u16; 4],
    bg_aff: [BgAffine; 2],
    win0: Window,
    win1: Window,
    winout_flags: WindowFlags,
    winobj_flags: WindowFlags,
    mosaic: RegMosaic,
    bldcnt: BlendControl,
    bldalpha: BlendAlpha,
    bldy: u16,

    // Video memory: VRAM (128 KiB), palette RAM (1 KiB), OAM (1 KiB)
    pub vram: Box<[u8]>,
    pub pram: Box<[u8]>,
    pub oam: Box<[u8]>,

    // Intermediate buffer for sprite rendering:
    // Stores the highest-priority sprite pixel at each (x, y) for the current scanline
    obj_buffer: Box<[ObjBufferEntry]>,
    pub vram_obj_tiles_start: u32,

    // Intermediate buffer for background layers during rendering
    bg_line: [[Rgb15; SCREEN_WIDTH]; 4],

    // Pixel buffer for the current frame (15-bit BGR555, row-major, 240×160)
    frame_buffer: Box<[u32]>,

    // Edge-triggered DMA event signals, consumed by the emulator main loop.
    pub hblank_dma_trigger: bool,
    pub vblank_dma_trigger: bool,
}

impl Ppu {
    pub fn new(interrupt_flags: Rc<RefCell<u16>>) -> Self {
        Self {
            interrupt_flags,
            frame_cycles: 0,
            frame_ready: false,
            dispcnt: DisplayControl(0x80),
            dispstat: DisplayStatus(0),
            vcount: 0,
            bgcnt: [BgControl(0); 4],
            bg_hofs: [0; 4],
            bg_vofs: [0; 4],
            bg_aff: [BgAffine::default(); 2],
            win0: Window::default(),
            win1: Window::default(),
            winout_flags: WindowFlags::default(),
            winobj_flags: WindowFlags::default(),
            mosaic: RegMosaic::default(),
            bldcnt: BlendControl::default(),
            bldalpha: BlendAlpha::default(),
            bldy: 0,
            vram: vec![0u8; VRAM_SIZE].into_boxed_slice(),
            pram: vec![0u8; PRAM_SIZE].into_boxed_slice(),
            oam: vec![0u8; OAM_SIZE].into_boxed_slice(),
            obj_buffer: vec![ObjBufferEntry::default(); SCREEN_WIDTH * SCREEN_HEIGHT]
                .into_boxed_slice(),
            vram_obj_tiles_start: VRAM_OBJ_TILES_START_TEXT,
            bg_line: [[Rgb15::default(); SCREEN_WIDTH]; 4],
            frame_buffer: vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
            hblank_dma_trigger: false,
            vblank_dma_trigger: false,
        }
    }

    #[inline]
    fn obj_buffer_get_mut(&mut self, x: usize, y: usize) -> &mut ObjBufferEntry {
        &mut self.obj_buffer[index2d!(x, y, SCREEN_WIDTH)]
    }

    /// Returns the current scanline counter (VCOUNT).
    pub fn vcount(&self) -> u16 {
        self.vcount
    }

    /// Consumes and returns the HBlank DMA trigger flag.
    pub fn take_hblank_dma_trigger(&mut self) -> bool {
        let v = self.hblank_dma_trigger;
        self.hblank_dma_trigger = false;
        v
    }

    /// Consumes and returns the VBlank DMA trigger flag.
    pub fn take_vblank_dma_trigger(&mut self) -> bool {
        let v = self.vblank_dma_trigger;
        self.vblank_dma_trigger = false;
        v
    }

    pub fn skip_bios(&mut self) {
        // Set the identity affine matrix so BG2/3 display 1:1 without BIOS init.
        for i in 0..2 {
            self.bg_aff[i].pa = 0x100;
            self.bg_aff[i].pb = 0;
            self.bg_aff[i].pc = 0;
            self.bg_aff[i].pd = 0x100;
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame_cycles = 0;
        self.frame_ready = false;
        self.vcount = 0;
        self.dispstat.set_vblank_flag(false);
    }

    pub fn tick(&mut self) {
        // Increment the cycle count and check if we've completed a frame.
        self.frame_cycles += 1;
        if self.frame_cycles >= CYCLES_PER_FRAME {
            self.frame_ready = true;
            return;
        }

        // Handle per-scanline events at the appropriate cycle counts.
        let scanline_cycle = self.frame_cycles % CYCLES_PER_SCANLINE as u32;
        let v_count = (self.frame_cycles / CYCLES_PER_SCANLINE as u32) as u16;

        // Handle V-Count Changes
        if v_count != self.vcount {
            self.vcount = v_count;
            self.update_vcount_stats();

            if self.vcount == 160 {
                self.latch_affine_points();
            }
        }

        // Handle H-Phase Transitions (HDraw -> HBlank)
        // 960 cycles of HDraw, 272 cycles of HBlank, 1232 cycles total per scanline
        if scanline_cycle == 0 {
            self.dispstat.set_hblank_flag(false);
            self.obj_buffer_reset();

            if self.is_vdraw() {
                self.render_scanline();
            }
        } else if scanline_cycle == 960 {
            self.dispstat.set_hblank_flag(true);

            if self.dispstat.hblank_irq_enable() {
                signal_irq(&self.interrupt_flags, Interrupt::HBlank);
            }

            self.hblank_dma_trigger = true;

            if self.is_vdraw() {
                self.increment_affine_points();
            }
        }
    }

    pub fn obj_buffer_reset(&mut self) {
        for x in self.obj_buffer.iter_mut() {
            *x = Default::default();
        }
    }

    fn is_vdraw(&self) -> bool {
        (self.vcount as usize) < SCREEN_HEIGHT
    }

    #[inline]
    pub(super) fn obj_buffer_get(&self, x: usize, y: usize) -> &ObjBufferEntry {
        &self.obj_buffer[index2d!(x, y, SCREEN_WIDTH)]
    }

    /// Increments the internal affine reference points for BG2/BG3 by the B/D parameters.
    fn increment_affine_points(&mut self) {
        for i in 0..2 {
            self.bg_aff[i].increment();
        }
    }

    /// Latches the BG2/3 external reference points into the internal counters.
    /// Called at the start of VBlank; the internal registers are used during rendering.
    fn latch_affine_points(&mut self) {
        for i in 0..2 {
            // println!("BG{} Latched: X={}, Y={}", i+2, self.bg_aff[i].x, self.bg_aff[i].y);
            self.bg_aff[i].latch();
        }
    }

    fn update_vcount_stats(&mut self) {
        // VBlank Flag
        let is_vblank = self.vcount >= 160 && self.vcount < 227;
        self.dispstat.set_vblank_flag(is_vblank);

        if is_vblank && self.vcount == 160 {
            self.vblank_dma_trigger = true;
            if self.dispstat.vblank_irq_enable() {
                signal_irq(&self.interrupt_flags, Interrupt::VBlank);
            }
        }

        // V-Counter Match
        let vcount_match = self.vcount == self.dispstat.vcount_setting();
        self.dispstat.set_vcount_flag(vcount_match);
        if vcount_match && self.dispstat.vcount_irq_enable() {
            signal_irq(&self.interrupt_flags, Interrupt::VCount);
        }
    }

    fn render_scanline(&mut self) {
        for line in self.bg_line.iter_mut() {
            line.fill(Rgb15::TRANSPARENT);
        }

        if self.dispcnt.force_blank() {
            for x in self.frame_buffer[self.vcount as usize * SCREEN_WIDTH..]
                .iter_mut()
                .take(SCREEN_WIDTH)
            {
                *x = 0xf8f8f8;
            }
            return;
        }

        if self.dispcnt.obj_enable() {
            self.render_objs();
        }

        match self.dispcnt.mode() {
            0 => {
                self.render_reg_bg_if_enabled(0);
                self.render_reg_bg_if_enabled(1);
                self.render_reg_bg_if_enabled(2);
                self.render_reg_bg_if_enabled(3);
                self.finalize_scanline(0, 3);
            }
            1 => {
                self.render_aff_bg_if_enabled(2);
                self.render_reg_bg_if_enabled(1);
                self.render_reg_bg_if_enabled(0);
                self.finalize_scanline(0, 2);
            }
            2 => {
                self.render_aff_bg_if_enabled(3);
                self.render_aff_bg_if_enabled(2);
                self.finalize_scanline(2, 3);
            }
            3 => {
                self.render_mode3();
                self.finalize_scanline(2, 2);
            }
            4 => {
                self.render_mode4();
                self.finalize_scanline(2, 2);
            }
            5 => {
                self.render_mode5();
                self.finalize_scanline(2, 2);
            }
            _ => panic!("Invalid display mode {}", self.dispcnt.mode()),
        }
    }

    pub fn get_ref_point(&self, bg: usize) -> Point {
        assert!(bg == 2 || bg == 3);
        (
            self.bg_aff[bg - 2].internal_x,
            self.bg_aff[bg - 2].internal_y,
        )
    }

    fn is_bg_enabled(&self, bg_idx: usize) -> bool {
        // In bitmap modes (3/4/5), BG2 is implicitly active regardless of the enable bit.
        if matches!(self.dispcnt.mode(), 3 | 4 | 5) && bg_idx == 2 {
            return true;
        }
        match bg_idx {
            0 => self.dispcnt.bg0_enable(),
            1 => self.dispcnt.bg1_enable(),
            2 => self.dispcnt.bg2_enable(),
            3 => self.dispcnt.bg3_enable(),
            _ => false,
        }
    }

    #[inline(always)]
    pub fn get_palette_color(&mut self, index: u32, palette_bank: u32, offset: u32) -> Rgb15 {
        if index == 0 || (palette_bank != 0 && index % 16 == 0) {
            return Rgb15::TRANSPARENT;
        }
        let value = self.pram_read_16((offset + 2 * index + 0x20 * palette_bank) as usize);

        // Top bit is ignored
        Rgb15(value & 0x7FFF)
    }

    pub fn pram_read_16(&self, addr: usize) -> u16 {
        self.pram[addr..][..2]
            .try_into()
            .map(u16::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn pram_write_16(&mut self, addr: usize, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        if let Some(b) = self.pram.get_mut(addr) {
            *b = lo;
        }
        if let Some(b) = self.pram.get_mut(addr + 1) {
            *b = hi;
        }
    }

    #[inline]
    fn oam_index(addr: u32) -> usize {
        (addr & 0x3FF) as usize
    }

    pub fn oam_read_16(&self, addr: u32) -> u16 {
        let i = Self::oam_index(addr & !1);
        self.oam[i..][..2]
            .try_into()
            .map(u16::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn oam_write_16(&mut self, addr: u32, value: u16) {
        let i = Self::oam_index(addr & !1);
        let [lo, hi] = value.to_le_bytes();
        self.oam[i] = lo;
        self.oam[i + 1] = hi;
    }

    pub fn oam_read_8(&self, addr: u32) -> u8 {
        self.oam.get(Self::oam_index(addr)).copied().unwrap_or(0)
    }

    pub fn oam_write_8(&mut self, addr: u32, value: u8) {
        let i = Self::oam_index(addr);
        if let Some(b) = self.oam.get_mut(i) {
            *b = value;
        }
    }

    pub fn oam_read_32(&self, addr: u32) -> u32 {
        let i = Self::oam_index(addr & !3);
        self.oam[i..][..4]
            .try_into()
            .map(u32::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn oam_write_32(&mut self, addr: u32, value: u32) {
        let i = Self::oam_index(addr & !3);
        self.oam[i..i + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub fn vram_read_8(&self, addr: usize) -> u8 {
        self.vram.get(addr).copied().unwrap_or(0)
    }

    pub fn vram_read_16(&self, addr: usize) -> u16 {
        self.vram[addr..][..2]
            .try_into()
            .map(u16::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn vram_write_16(&mut self, addr: usize, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        if let Some(b) = self.vram.get_mut(addr) {
            *b = lo;
        }
        if let Some(b) = self.vram.get_mut(addr + 1) {
            *b = hi;
        }
    }

    pub fn read_pixel_index(&self, addr: u32, x: u32, y: u32, format: PixelFormat) -> usize {
        match format {
            PixelFormat::BPP4 => self.read_pixel_index_bpp4(addr, x, y),
            PixelFormat::BPP8 => self.read_pixel_index_bpp8(addr, x, y),
        }
    }

    #[inline]
    pub fn read_pixel_index_bpp4(&self, addr: u32, x: u32, y: u32) -> usize {
        let ofs = addr + index2d!(u32, x / 2, y, 4);
        let ofs = ofs as usize;
        let byte = self.vram[ofs];
        if x & 1 != 0 {
            (byte >> 4) as usize
        } else {
            (byte & 0xf) as usize
        }
    }

    #[inline]
    pub fn read_pixel_index_bpp8(&self, addr: u32, x: u32, y: u32) -> usize {
        let ofs = addr;
        self.vram[ofs as usize + index2d!(u32, x, y, 8) as usize] as usize
    }

    pub fn frame_ready(&self) -> bool {
        self.frame_ready
    }

    /// Returns the completed framebuffer as a flat BGR555 slice (240×160 pixels, row-major).
    pub fn get_frame_buffer(&self) -> &[u32] {
        &self.frame_buffer
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        match addr & !1 {
            0x4000000 => self.dispcnt.0,
            0x4000004 => self.dispstat.0,
            0x4000006 => self.vcount,
            0x4000008 => self.bgcnt[0].0,
            0x400000A => self.bgcnt[1].0,
            0x400000C => self.bgcnt[2].0,
            0x400000E => self.bgcnt[3].0,
            0x4000010 => self.bg_hofs[0],
            0x4000012 => self.bg_vofs[0],
            0x4000014 => self.bg_hofs[1],
            0x4000016 => self.bg_vofs[1],
            0x4000018 => self.bg_hofs[2],
            0x400001A => self.bg_vofs[2],
            0x400001C => self.bg_hofs[3],
            0x400001E => self.bg_vofs[3],
            0x4000050 => self.bldcnt.read(),
            0x4000052 => self.bldalpha.read(),
            _ => 0,
        }
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        match addr & !1 {
            0x4000000 => {
                let old_mode = self.dispcnt.mode();
                self.dispcnt.0 = value;
                let new_mode = self.dispcnt.mode();
                if old_mode != new_mode {
                    self.vram_obj_tiles_start = if new_mode >= 3 {
                        VRAM_OBJ_TILES_START_BITMAP
                    } else {
                        VRAM_OBJ_TILES_START_TEXT
                    };
                }
            },
            0x4000004 => {
                // Bits 2:0 are read-only status flags; preserve them.
                self.dispstat.0 = (self.dispstat.0 & 0x0007) | (value & 0xFFF8);
            }
            0x4000006 => {} // VCOUNT is read-only
            0x4000008 => self.bgcnt[0].0 = value,
            0x400000A => self.bgcnt[1].0 = value,
            0x400000C => self.bgcnt[2].0 = value,
            0x400000E => self.bgcnt[3].0 = value,
            0x4000010 => self.bg_hofs[0] = value,
            0x4000012 => self.bg_vofs[0] = value,
            0x4000014 => self.bg_hofs[1] = value,
            0x4000016 => self.bg_vofs[1] = value,
            0x4000018 => self.bg_hofs[2] = value,
            0x400001A => self.bg_vofs[2] = value,
            0x400001C => self.bg_hofs[3] = value,
            0x400001E => self.bg_vofs[3] = value,
            0x4000020 => self.bg_aff[0].pa = value as i16,
            0x4000022 => self.bg_aff[0].pb = value as i16,
            0x4000024 => self.bg_aff[0].pc = value as i16,
            0x4000026 => self.bg_aff[0].pd = value as i16,
            0x4000028 => {
                self.bg_aff[0].x = (self.bg_aff[0].x & !0x0000_FFFF) | (value as i32);
                self.bg_aff[0].internal_x = self.bg_aff[0].x;
            }
            0x400002A => {
                let b2 = (value & 0x00FF) as i32;
                let b3 = ((value >> 8) & 0x0F) as i32;
                let raw = (self.bg_aff[0].x & 0x0000_FFFF) | (b2 << 16) | (b3 << 24);
                self.bg_aff[0].x = (raw << 4) >> 4;
                self.bg_aff[0].internal_x = self.bg_aff[0].x;
            }
            0x400002C => {
                self.bg_aff[0].y = (self.bg_aff[0].y & !0x0000_FFFF) | (value as i32);
                self.bg_aff[0].internal_y = self.bg_aff[0].y;
            }
            0x400002E => {
                let b2 = (value & 0x00FF) as i32;
                let b3 = ((value >> 8) & 0x0F) as i32;
                let raw = (self.bg_aff[0].y & 0x0000_FFFF) | (b2 << 16) | (b3 << 24);
                self.bg_aff[0].y = (raw << 4) >> 4;
                self.bg_aff[0].internal_y = self.bg_aff[0].y;
            }
            0x4000030 => self.bg_aff[1].pa = value as i16,
            0x4000032 => self.bg_aff[1].pb = value as i16,
            0x4000034 => self.bg_aff[1].pc = value as i16,
            0x4000036 => self.bg_aff[1].pd = value as i16,
            0x4000038 => {
                self.bg_aff[1].x = (self.bg_aff[1].x & !0x0000_FFFF) | (value as i32);
                self.bg_aff[1].internal_x = self.bg_aff[1].x;
            }
            0x400003A => {
                let b2 = (value & 0x00FF) as i32;
                let b3 = ((value >> 8) & 0x0F) as i32;
                let raw = (self.bg_aff[1].x & 0x0000_FFFF) | (b2 << 16) | (b3 << 24);
                self.bg_aff[1].x = (raw << 4) >> 4;
                self.bg_aff[1].internal_x = self.bg_aff[1].x;
            }
            0x400003C => {
                self.bg_aff[1].y = (self.bg_aff[1].y & !0x0000_FFFF) | (value as i32);
                self.bg_aff[1].internal_y = self.bg_aff[1].y;
            }
            0x400003E => {
                let b2 = (value & 0x00FF) as i32;
                let b3 = ((value >> 8) & 0x0F) as i32;
                let raw = (self.bg_aff[1].y & 0x0000_FFFF) | (b2 << 16) | (b3 << 24);
                self.bg_aff[1].y = (raw << 4) >> 4;
                self.bg_aff[1].internal_y = self.bg_aff[1].y;
            }
            0x4000040 => {
                self.win0.right = (value & 0xFF) as u8;
                self.win0.left = (value >> 8) as u8;
            }
            0x4000042 => {
                self.win1.right = (value & 0xFF) as u8;
                self.win1.left = (value >> 8) as u8;
            }
            0x4000044 => {
                self.win0.bottom = (value & 0xFF) as u8;
                self.win0.top = (value >> 8) as u8;
            }
            0x4000046 => {
                self.win1.bottom = (value & 0xFF) as u8;
                self.win1.top = (value >> 8) as u8;
            }
            0x4000048 => {
                self.win0.flags = WindowFlags::from(value & 0x3F);
                self.win1.flags = WindowFlags::from((value >> 8) & 0x3F);
            }
            0x400004A => {
                self.winout_flags = WindowFlags::from(value & 0x3F);
                self.winobj_flags = WindowFlags::from((value >> 8) & 0x3F);
            }
            0x400004C => self.mosaic.0 = value,
            0x4000050 => self.bldcnt.write(value),
            0x4000052 => self.bldalpha.write(value),
            0x4000054 => self.bldy = value & 0x1F,
            _ => {}
        }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let half = self.read_16(addr);
        if addr & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        let half = self.read_16(addr);
        let new = if addr & 1 == 0 {
            (half & 0xFF00) | (value as u16)
        } else {
            (half & 0x00FF) | ((value as u16) << 8)
        };
        self.write_16(addr, new);
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        let lo = self.read_16(addr) as u32;
        let hi = self.read_16(addr + 2) as u32;
        lo | (hi << 16)
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        self.write_16(addr, value as u16);
        self.write_16(addr + 2, (value >> 16) as u16);
    }
}
