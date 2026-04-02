pub mod regions;

use super::{Cartridge, IoDevices};
use MemoryRegion::*;
use regions::{MemoryRegion, consts::*};
use std::{cell::Cell, cell::RefCell, rc::Rc};

const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;

#[rustfmt::skip]#[derive(Clone, Copy, PartialEq)]
pub enum AccessWidth { Byte, Half, Word }

/// Central memory bus. Owns mapped memory blocks and routes reads/writes.
pub struct Bus {
    bios: Vec<u8>,
    ewram: [u8; EWRAM_SIZE],
    iwram: [u8; IWRAM_SIZE],
    pub io: Rc<RefCell<IoDevices>>,
    cartridge: Cartridge,
    last_addr: Cell<u32>,
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
        self.io.borrow_mut().ppu.vram.fill(0);
        self.io.borrow_mut().ppu.pram.fill(0);
        self.io.borrow_mut().ppu.oam.fill(0);
    }

    pub fn tick(&mut self) {
        self.io.borrow_mut().tick();
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Bios => self.bios.get(addr as usize).copied().unwrap_or(0xFF),
                Ewram => self.ewram[Self::ewram_index(addr)],
                Iwram => self.iwram[Self::iwram_index(addr)],
                Io => self.io.borrow().read_8(addr),
                // PRAM/VRAM/OAM are 16-bit. Read 16 and pick the byte.
                Vram | Pram | Oam => {
                    let val = self.read_16(addr & !1);
                    if addr & 1 == 0 {
                        (val & 0xFF) as u8
                    } else {
                        (val >> 8) as u8
                    }
                }
                CartWs0 | CartWs1 | CartWs2 | CartSram => self.cartridge.read_8(addr),
            }
        } else {
            0
        }
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Bios => {}
                Ewram => self.ewram[Self::ewram_index(addr)] = value,
                Iwram => self.iwram[Self::iwram_index(addr)] = value,
                Vram => {
                    // Hardware quirk: 8-bit writes to BG VRAM expand the byte to both halves of
                    // the aligned 16-bit word. 8-bit writes to OBJ VRAM are ignored.
                    let ofs = Self::vram_index(addr);
                    if ofs < self.io.borrow().ppu.vram_obj_tiles_start as usize {
                        let expanded = (value as u16) | ((value as u16) << 8);
                        self.io.borrow_mut().ppu.vram_write_16(ofs & !1, expanded);
                    }
                }
                Oam => { /* 8-bit OAM writes are ignored on GBA hardware */ }
                Pram => {
                    // Hardware quirk: 8-bit writes to PRAM replicate the byte to both halves of the 16-bit entry.
                    self.io.borrow_mut().ppu.pram_write_16(
                        Self::pram_index(addr),
                        value as u16 | ((value as u16) << 8),
                    );
                }
                Io => self.io.borrow_mut().write_8(addr, value),
                CartWs0 | CartWs1 | CartWs2 | CartSram => self.cartridge.write_8(addr, value),
            }
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        // Force alignment for 16-bit reads
        let addr = addr & !1;
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Bios => {
                    self.bios.get(addr as usize).copied().unwrap_or(0xFF) as u16
                        | ((self.bios.get(addr as usize + 1).copied().unwrap_or(0xFF) as u16) << 8)
                }
                Ewram => {
                    let idx = Self::ewram_index(addr);
                    u16::from_le_bytes(self.ewram[idx..idx + 2].try_into().unwrap())
                }
                Iwram => {
                    let idx = Self::iwram_index(addr);
                    u16::from_le_bytes(self.iwram[idx..idx + 2].try_into().unwrap())
                }
                Pram => self.io.borrow().ppu.pram_read_16(Self::pram_index(addr)),
                Vram => self.io.borrow().ppu.vram_read_16(Self::vram_index(addr)),
                Io => self.io.borrow().read_16(addr),
                Oam => self.io.borrow().ppu.oam_read_16(addr),
                CartWs0 | CartWs1 | CartWs2 | CartSram => self.cartridge.read_16(addr),
            }
        } else {
            0
        }
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        let addr = addr & !1;
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Bios => {}
                Ewram => {
                    let idx = Self::ewram_index(addr);
                    self.ewram[idx..idx + 2].copy_from_slice(&value.to_le_bytes());
                }
                Iwram => {
                    let idx = Self::iwram_index(addr);
                    self.iwram[idx..idx + 2].copy_from_slice(&value.to_le_bytes());
                }
                Pram => {
                    self.io
                        .borrow_mut()
                        .ppu
                        .pram_write_16(Self::pram_index(addr), value);
                }
                Vram => {
                    self.io
                        .borrow_mut()
                        .ppu
                        .vram_write_16(Self::vram_index(addr), value);
                }
                Io => self.io.borrow_mut().write_16(addr, value),
                Oam => self.io.borrow_mut().ppu.oam_write_16(addr, value),
                CartWs0 | CartWs1 | CartWs2 | CartSram => self.cartridge.write_16(addr, value),
            }
        }
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        let addr = addr & !3;
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Ewram => {
                    let idx = Self::ewram_index(addr);
                    return u32::from_le_bytes(self.ewram[idx..idx + 4].try_into().unwrap());
                }
                Iwram => {
                    let idx = Self::iwram_index(addr);
                    return u32::from_le_bytes(self.iwram[idx..idx + 4].try_into().unwrap());
                }
                CartWs0 | CartWs1 | CartWs2 | CartSram => return self.cartridge.read_32(addr),
                _ => {}
            }
        }
        let low = self.read_16(addr) as u32;
        let high = self.read_16(addr + 2) as u32;
        (high << 16) | low
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        let addr = addr & !3;
        if let Some(region) = MemoryRegion::from_addr(addr) {
            match region {
                Ewram => {
                    let idx = Self::ewram_index(addr);
                    self.ewram[idx..idx + 4].copy_from_slice(&value.to_le_bytes());
                    return;
                }
                Iwram => {
                    let idx = Self::iwram_index(addr);
                    self.iwram[idx..idx + 4].copy_from_slice(&value.to_le_bytes());
                    return;
                }
                CartWs0 | CartWs1 | CartWs2 | CartSram => {
                    return self.cartridge.write_32(addr, value);
                }
                _ => {}
            }
        }
        self.write_16(addr, (value & 0xFFFF) as u16);
        self.write_16(addr.wrapping_add(2), (value >> 16) as u16);
    }

    pub fn cartridge_size(&self) -> usize {
        self.cartridge.size()
    }

    pub fn cartridge_save_data(&self) -> Option<Vec<u8>> {
        self.cartridge.save_data()
    }

    pub fn load_cartridge_save(&mut self, data: &[u8]) {
        self.cartridge.load_save_data(data);
    }

    pub fn is_save_dirty(&self) -> bool {
        self.cartridge.is_save_dirty()
    }

    pub fn clear_save_dirty(&mut self) {
        self.cartridge.clear_save_dirty();
    }

    /// Sequential (S) cycle count for a memory access at `addr` with `width`.
    fn seq_cycles(&self, addr: u32, width: AccessWidth) -> u32 {
        use AccessWidth::*;
        match MemoryRegion::from_addr(addr) {
            // 32-bit internal buses: 1 cycle regardless of width.
            Some(Bios) | Some(Iwram) | Some(Io) | Some(Oam) => 1,
            // 16-bit buses, 1 cycle for byte/half; treated as 1 cycle for word too
            // (PPU-side contention is not modelled here).
            Some(Pram) | Some(Vram) => 1,
            // EWRAM: 16-bit bus, default 3S / 6N for 16-bit; double for 32-bit.
            Some(Ewram) => {
                if width == Word {
                    6
                } else {
                    3
                }
            }
            // Cartridge ROM windows: 16-bit bus; Word = S + S.
            Some(CartWs0) => {
                let s = self.io.borrow().ws0_s();
                if width == Word { s + s } else { s }
            }
            Some(CartWs1) => {
                let s = self.io.borrow().ws1_s();
                if width == Word { s + s } else { s }
            }
            Some(CartWs2) => {
                let s = self.io.borrow().ws2_s();
                if width == Word { s + s } else { s }
            }
            // SRAM: 8-bit bus; timing is the same for all widths (hardware does
            // multiple byte accesses, but games always use byte instructions).
            Some(CartSram) => self.io.borrow().sram_cycles(),
            None => 1,
        }
    }

    /// Non-sequential (N) cycle count for a memory access at `addr` with `width`.
    ///
    /// For 32-bit accesses on 16-bit buses the first half is N and the second is S.
    fn nonseq_cycles(&self, addr: u32, width: AccessWidth) -> u32 {
        use AccessWidth::*;
        match MemoryRegion::from_addr(addr) {
            Some(Bios) | Some(Iwram) | Some(Io) | Some(Oam) => 1,
            Some(Pram) | Some(Vram) => 1,
            // EWRAM: 16-bit N=6; Word = N + S = 6 + 3 = 9.
            Some(Ewram) => {
                if width == Word {
                    9
                } else {
                    6
                }
            }
            Some(CartWs0) => {
                let (n, s) = (self.io.borrow().ws0_n(), self.io.borrow().ws0_s());
                if width == Word { n + s } else { n }
            }
            Some(CartWs1) => {
                let (n, s) = (self.io.borrow().ws1_n(), self.io.borrow().ws1_s());
                if width == Word { n + s } else { n }
            }
            Some(CartWs2) => {
                let (n, s) = (self.io.borrow().ws2_n(), self.io.borrow().ws2_s());
                if width == Word { n + s } else { n }
            }
            Some(CartSram) => self.io.borrow().sram_cycles(),
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
    fn ewram_index(addr: u32) -> usize { (addr - EWRAM_ADDR) as usize}
    fn iwram_index(addr: u32) -> usize { ((addr - IWRAM_ADDR) as usize) & 0x7FFF }
    fn pram_index(addr: u32) -> usize { ((addr - PRAM_ADDR) as usize) & 0x3FF }
    fn vram_index(addr: u32) -> usize {
        // VRAM is 96KB (0x18000 bytes) mapped in a 128KB window.
        // The last 32KB of the window (0x18000..0x1FFFF) mirrors 0x10000..0x17FFF.
        let ofs = (addr - VRAM_ADDR) as usize & 0x1FFFF;
        if ofs >= 0x18000 { ofs - 0x8000 } else { ofs }
    }
}
