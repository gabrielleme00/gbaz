pub mod regions;

use super::{Cartridge, IoDevices};
use regions::MemoryRegion;
use std::{cell::RefCell, rc::Rc};

const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;
const VRAM_SIZE: usize = 96 * 1024;
const PALETTE_SIZE: usize = 1 * 1024;

/// Central memory bus. Owns mapped memory blocks and routes reads/writes.
#[derive(Clone)]
pub struct Bus {
    bios: Vec<u8>,
    ewram: [u8; EWRAM_SIZE],
    iwram: [u8; IWRAM_SIZE],
    pub vram: [u8; VRAM_SIZE],
    pub pram: [u8; PALETTE_SIZE],
    pub io: Rc<RefCell<IoDevices>>,
    cartridge: Cartridge,
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
        if let Some(region) = MemoryRegion::from_addr(addr) {
            use MemoryRegion::*;
            match region {
                Bios => self.bios.get(addr as usize).copied().unwrap_or(0xFF),
                Wram => self.ewram[Self::wram_index(addr)],
                Iwram => self.iwram[Self::iwram_index(addr)],
                Io => self.io.borrow().read8(addr),
                Vram => self.vram[Self::vram_index(addr)],
                Palette => self.pram[Self::pram_index(addr)],
                Cartridge => self.cartridge.read8(addr),
                _ => 0,
            }
        } else {
            0
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        if let Some(region) = MemoryRegion::from_addr(addr) {
            use MemoryRegion::*;
            match region {
                Bios => {} // BIOS is read-only
                Wram => self.ewram[Self::wram_index(addr)] = value,
                Iwram => self.iwram[Self::iwram_index(addr)] = value,
                Io => self.io.borrow_mut().write8(addr, value),
                Vram => self.vram[Self::vram_index(addr)] = value,
                Palette => self.pram[Self::pram_index(addr)] = value,
                Cartridge => self.cartridge.write8(addr, value),
                _ => {}
            }
        }
    }

    fn wram_index(addr: u32) -> usize {
        (addr - 0x0200_0000) as usize % EWRAM_SIZE
    }

    fn iwram_index(addr: u32) -> usize {
        (addr - 0x0300_0000) as usize % IWRAM_SIZE
    }

    fn vram_index(addr: u32) -> usize {
        (addr - 0x0600_0000) as usize % VRAM_SIZE
    }

    fn pram_index(addr: u32) -> usize {
        (addr - 0x0500_0000) as usize % PALETTE_SIZE
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
}
