pub mod regions;

use super::{Cartridge, IoDevices};
use regions::MemoryRegion;
use std::{cell::Cell, cell::RefCell, rc::Rc};

const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;
const VRAM_SIZE: usize = 96 * 1024;
const PALETTE_SIZE: usize = 1 * 1024;

#[rustfmt::skip]#[derive(Clone, Copy, PartialEq)]
pub enum AccessWidth { Byte, Half, Word }

/// Central memory bus. Owns mapped memory blocks and routes reads/writes.
#[derive(Clone)]
pub struct Bus {
    bios: Vec<u8>,
    ewram: [u8; EWRAM_SIZE],
    iwram: [u8; IWRAM_SIZE],
    vram: [u8; VRAM_SIZE],
    pram: [u8; PALETTE_SIZE],
    pub io: Rc<RefCell<IoDevices>>,
    cartridge: Cartridge,
    last_addr: Cell<u32>, // For timing calculations of sequential accesses
}

impl Bus {
    pub fn new(
        cartridge: Cartridge,
        io_devs: Rc<RefCell<IoDevices>>,
        bios: Option<Vec<u8>>,
    ) -> Self {
        Self {
            bios: bios.unwrap_or_default(),
            ewram: [0; EWRAM_SIZE],
            iwram: [0; IWRAM_SIZE],
            vram: [0; VRAM_SIZE],
            pram: [0; PALETTE_SIZE],
            io: io_devs,
            cartridge,
            last_addr: Cell::new(0),
        }
    }

    pub fn has_bios(&self) -> bool {
        !self.bios.is_empty()
    }

    pub fn reset(&mut self) {
        self.ewram.fill(0);
        self.iwram.fill(0);
        self.vram.fill(0);
        self.pram.fill(0);
    }

    pub fn tick(&mut self) {
        self.io.borrow_mut().ppu.tick(&self.vram, &self.pram);
    }

    pub fn read8(&self, addr: u32) -> u8 {
        let data = if let Some(region) = MemoryRegion::from_addr(addr) {
            use MemoryRegion::*;
            match region {
                Bios => self.bios.get(addr as usize).copied().unwrap_or(0xFF),
                Ewram => self.ewram[Self::ewram_index(addr)],
                Iwram => self.iwram[Self::iwram_index(addr)],
                Io => self.io.borrow().read8(addr),
                Vram => self.vram[Self::vram_index(addr)],
                Palette => self.pram[Self::pram_index(addr)],
                CartridgeWs0 | CartridgeWs1 | CartridgeWs2 => self.cartridge.read8(addr),
                CartridgeSram => 0xFF,
                _ => 0,
            }
        } else {
            0
        };
        data
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        if let Some(region) = MemoryRegion::from_addr(addr) {
            use MemoryRegion::*;
            match region {
                Bios => {} // BIOS is read-only
                Ewram => self.ewram[Self::ewram_index(addr)] = value,
                Iwram => self.iwram[Self::iwram_index(addr)] = value,
                Io => self.io.borrow_mut().write8(addr, value),
                Vram => self.vram[Self::vram_index(addr)] = value,
                Palette => self.pram[Self::pram_index(addr)] = value,
                CartridgeWs0 | CartridgeWs1 | CartridgeWs2 => self.cartridge.write8(addr, value),
                CartridgeSram => {}
                _ => {}
            }
        }
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let low = self.read8(addr) as u16;
        let high = self.read8(addr + 1) as u16;
        (high << 8) | low
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        self.write8(addr, (value & 0xFF) as u8);
        self.write8(addr + 1, (value >> 8) as u8);
    }

    pub fn read32(&self, addr: u32) -> u32 {
        let low = self.read16(addr) as u32;
        let high = self.read16(addr + 2) as u32;
        (high << 16) | low
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        self.write16(addr, (value & 0xFFFF) as u16);
        self.write16(addr + 2, (value >> 16) as u16);
    }

    pub fn cartridge_size(&self) -> usize {
        self.cartridge.size()
    }

    /// Sequential (S) cycle count for a memory access at `addr` with `width`.
    fn seq_cycles(&self, addr: u32, width: AccessWidth) -> u32 {
        use AccessWidth::*;
        use MemoryRegion::*;
        match MemoryRegion::from_addr(addr) {
            // 32-bit internal buses: 1 cycle regardless of width.
            Some(Bios) | Some(Iwram) | Some(Io) | Some(Oam) => 1,
            // 16-bit buses, 1 cycle for byte/half; treated as 1 cycle for word too
            // (PPU-side contention is not modelled here).
            Some(Palette) | Some(Vram) => 1,
            // EWRAM: 16-bit bus, default 3S / 6N for 16-bit; double for 32-bit.
            Some(Ewram) => {
                if width == Word {
                    6
                } else {
                    3
                }
            }
            // Cartridge ROM windows: 16-bit bus; Word = S + S.
            Some(CartridgeWs0) => {
                let s = self.io.borrow().ws0_s();
                if width == Word { s + s } else { s }
            }
            Some(CartridgeWs1) => {
                let s = self.io.borrow().ws1_s();
                if width == Word { s + s } else { s }
            }
            Some(CartridgeWs2) => {
                let s = self.io.borrow().ws2_s();
                if width == Word { s + s } else { s }
            }
            // SRAM: 8-bit bus; timing is the same for all widths (hardware does
            // multiple byte accesses, but games always use byte instructions).
            Some(CartridgeSram) => self.io.borrow().sram_cycles(),
            None => 1,
        }
    }

    /// Non-sequential (N) cycle count for a memory access at `addr` with `width`.
    ///
    /// For 32-bit accesses on 16-bit buses the first half is N and the second is S.
    fn nonseq_cycles(&self, addr: u32, width: AccessWidth) -> u32 {
        use AccessWidth::*;
        use MemoryRegion::*;
        match MemoryRegion::from_addr(addr) {
            Some(Bios) | Some(Iwram) | Some(Io) | Some(Oam) => 1,
            Some(Palette) | Some(Vram) => 1,
            // EWRAM: 16-bit N=6; Word = N + S = 6 + 3 = 9.
            Some(Ewram) => {
                if width == Word {
                    9
                } else {
                    6
                }
            }
            Some(CartridgeWs0) => {
                let (n, s) = (self.io.borrow().ws0_n(), self.io.borrow().ws0_s());
                if width == Word { n + s } else { n }
            }
            Some(CartridgeWs1) => {
                let (n, s) = (self.io.borrow().ws1_n(), self.io.borrow().ws1_s());
                if width == Word { n + s } else { n }
            }
            Some(CartridgeWs2) => {
                let (n, s) = (self.io.borrow().ws2_n(), self.io.borrow().ws2_s());
                if width == Word { n + s } else { n }
            }
            Some(CartridgeSram) => self.io.borrow().sram_cycles(),
            None => 1,
        }
    }

    /// Returns the cycle cost for a memory access at `addr` with `width`.
    /// Pass `sequential = true` when the address is the natural continuation of
    /// the previous bus access (S-cycle), or `false` for a new address (N-cycle).
    pub fn access_cycles(&self, addr: u32, width: AccessWidth) -> u32 {
        let seq = match width {
            AccessWidth::Word => addr == self.last_addr.get().wrapping_add(4),
            AccessWidth::Half => addr == self.last_addr.get().wrapping_add(2),
            AccessWidth::Byte => addr == self.last_addr.get().wrapping_add(1),
        };
        self.last_addr.set(addr);
        if seq {
            self.seq_cycles(addr, width)
        } else {
            self.nonseq_cycles(addr, width)
        }
    }
}

#[rustfmt::skip]
impl Bus {
    // Helper functions to calculate indices into memory blocks based on address.
    // TODO: Not use % as it has a performance cost
    fn ewram_index(addr: u32) -> usize { (addr - 0x0200_0000) as usize}
    fn iwram_index(addr: u32) -> usize { ((addr - 0x0300_0000) as usize) % IWRAM_SIZE }
    fn pram_index(addr: u32) -> usize { (addr - 0x0500_0000) as usize }
    fn vram_index(addr: u32) -> usize { (addr - 0x0600_0000) as usize }
}
