use crate::utils::*;

use super::bus::regions::IoRegisterRegion;
use super::{Apu, InputState, InterruptController, Ppu, Timer, Dma};
use bitfield::bitfield;

const SRAM_CYCLES: [u32; 4] = [4, 3, 2, 8];
const WS0_CYCLES_N: [u32; 4] = [4, 3, 2, 8];
const WS0_CYCLES_S: [u32; 2] = [2, 1];
const WS1_CYCLES_N: [u32; 4] = [4, 3, 2, 8];
const WS1_CYCLES_S: [u32; 2] = [4, 1];
const WS2_CYCLES_N: [u32; 4] = [4, 3, 2, 8];
const WS2_CYCLES_S: [u32; 2] = [8, 1];

bitfield!(
    pub struct WaitCnt(u16);
    impl Debug;
    pub sram, set_sram: 1, 0;
    pub ws0_n, set_ws0_n: 3, 2;
    pub ws0_s, set_ws0_s: 4;
    pub ws1_n, set_ws1_n: 6, 5;
    pub ws1_s, set_ws1_s: 7;
    pub ws2_n, set_ws2_n: 9, 8;
    pub ws2_s, set_ws2_s: 10;
    pub phinter, set_phinter: 12, 11;
    pub gamepak_prefetch, set_gamepak_prefetch: 14;
    pub gamepak_type, set_gamepak_type: 15;
);

pub struct IoDevices {
    pub interrupt: InterruptController,

    // Modules
    pub ppu: Ppu,
    pub apu: Apu,
    pub dma: Dma,
    pub timer: Timer,

    // Registers
    waitcnt: WaitCnt,
    /// Set by HALTCNT writes; cleared when an IRQ wakes the CPU.
    pub halted: bool,
    /// Current button state, updated each frame by the frontend.
    pub input: InputState,
}

impl IoDevices {
    pub fn new(interrupt: InterruptController, ppu: Ppu, apu: Apu, dma: Dma, timer: Timer) -> Self {
        Self {
            interrupt,
            ppu,
            dma,
            apu,
            timer,
            waitcnt: WaitCnt(0),
            halted: false,
            input: InputState::default(),
        }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.read_8(addr),
                Sound => self.apu.read_8(addr),
                Dma => self.dma.read_8(addr),
                Timer => self.timer.read_8(addr),
                Keypad => self.input.read_8(addr),
                Serial => 0,
                Interrupt => match addr {
                    0x0400_0204 => get_lo(self.waitcnt.0),
                    0x0400_0205 => get_hi(self.waitcnt.0),
                    _ => self.interrupt.read_8(addr),
                },
            }
        } else {
            0
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.read_16(addr),
                Sound => self.apu.read_16(addr),
                Dma => self.dma.read_16(addr),
                Timer => self.timer.read_16(addr),
                Keypad => self.input.read_16(),
                Serial => 0,
                Interrupt => match addr {
                    0x0400_0204 => get_lo(self.waitcnt.0) as u16 | ((get_hi(self.waitcnt.0) as u16) << 8),
                    _ => self.interrupt.read_16(addr),
                },
            }
        } else {
            0
        }
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.read_32(addr),
                Sound => self.apu.read_32(addr),
                Dma => self.dma.read_32(addr),
                Timer => self.timer.read_32(addr),
                Keypad => self.input.read_32(),
                Serial => 0,
                Interrupt => match addr {
                    0x0400_0204 => self.waitcnt.0 as u32,
                    _ => self.interrupt.read_32(addr),
                },
            }
        } else {
            0
        }
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.write_8(addr, value),
                Sound => self.apu.write_8(addr, value),
                Dma => self.dma.write_8(addr, value),
                Timer => self.timer.write_8(addr, value),
                Keypad => {}
                Serial => {}
                Interrupt => match addr {
                    0x0400_0204 => set_lo(&mut self.waitcnt.0, value),
                    0x0400_0205 => set_hi(&mut self.waitcnt.0, value),
                    0x0400_0301 => self.halted = true,
                    _ => self.interrupt.write_8(addr, value),
                },
            }
        }
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.write_16(addr, value),
                Sound => self.apu.write_16(addr, value),
                Dma => self.dma.write_16(addr, value),
                Timer => self.timer.write_16(addr, value),
                Keypad => {}
                Serial => {}
                Interrupt => match addr {
                    0x0400_0204 => set_lo(&mut self.waitcnt.0, value as u8),
                    0x0400_0205 => set_hi(&mut self.waitcnt.0, (value >> 8) as u8),
                    _ => self.interrupt.write_16(addr, value),
                },
            }
        }
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.write_32(addr, value),
                Sound => self.apu.write_32(addr, value),
                Dma => self.dma.write_32(addr, value),
                Timer => self.timer.write_32(addr, value),
                Keypad => {}
                Serial => {}
                Interrupt => match addr {
                    0x0400_0204 => self.waitcnt.0 = value as u16,
                    _ => self.interrupt.write_32(addr, value),
                },
            }
        }
    }
}

#[rustfmt::skip]
impl IoDevices {
    // WAITCNT helpers - decode total cycle counts for each cartridge region.
    // Values are total cycles (1 base + wait) as defined in GBATEK §4.1
    pub fn sram_cycles(&self) -> u32 { SRAM_CYCLES[self.waitcnt.sram() as usize] }
    pub fn ws0_n(&self) -> u32 { WS0_CYCLES_N[self.waitcnt.ws0_n() as usize] }
    pub fn ws0_s(&self) -> u32 { WS0_CYCLES_S[self.waitcnt.ws0_s() as usize] }
    pub fn ws1_n(&self) -> u32 { WS1_CYCLES_N[self.waitcnt.ws1_n() as usize] }
    pub fn ws1_s(&self) -> u32 { WS1_CYCLES_S[self.waitcnt.ws1_s() as usize] }
    pub fn ws2_n(&self) -> u32 { WS2_CYCLES_N[self.waitcnt.ws2_n() as usize] }
    pub fn ws2_s(&self) -> u32 { WS2_CYCLES_S[self.waitcnt.ws2_s() as usize] }
}

impl IoDevices {
    /// Advance all hardware by one CPU cycle. Called from `Bus::tick()`.
    pub fn tick(&mut self) {
        self.ppu.tick();
        self.timer.advance(1);
        // Notify APU of any timer overflows that just occurred.
        let overflow_flags = self.timer.take_overflow_flags();
        if overflow_flags != 0 {
            for ch in 0..2usize {
                if overflow_flags & (1 << ch) != 0 {
                    self.apu.on_timer_overflow(ch);
                }
            }
        }
        self.apu.advance(1);
    }

    /// Returns and clears the DMA-refill request flags from the APU FIFOs
    /// (bit 0 = FIFO A wants DMA, bit 1 = FIFO B).
    pub fn take_sound_dma_flags(&mut self) -> u8 {
        self.apu.take_fifo_dma_flags()
    }
}
