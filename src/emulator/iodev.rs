use super::bus::regions::IoRegisterRegion;
use super::{Apu, InterruptController, Ppu, Timer};
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
    // Modules
    pub interrupt: InterruptController,
    pub ppu: Box<Ppu>,
    pub apu: Apu,
    pub timer: Timer,

    // Registers
    waitcnt: WaitCnt,
    /// Set by HALTCNT writes; cleared when an IRQ wakes the CPU.
    pub halted: bool,
}

impl IoDevices {
    pub fn new(interrupt: InterruptController, ppu: Box<Ppu>, apu: Apu, timer: Timer) -> Self {
        Self {
            interrupt,
            ppu,
            apu,
            timer,
            waitcnt: WaitCnt(0),
            halted: false,
        }
    }

    pub fn read8(&self, addr: u32) -> u8 {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.read8(addr),
                Sound => 0,
                Dma => 0,
                Timer => 0,
                Keypad => 0,
                Serial => 0,
                Interrupt => match addr {
                    0x0400_0204 => (self.waitcnt.0 & 0xFF) as u8,
                    0x0400_0205 => (self.waitcnt.0 >> 8) as u8,
                    _ => self.interrupt.read(addr),
                },
            }
        } else {
            0
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        if let Some(region) = IoRegisterRegion::from_addr(addr) {
            use IoRegisterRegion::*;
            match region {
                Lcd => self.ppu.write8(addr, value),
                Sound => {}
                Dma => {}
                Timer => {}
                Keypad => {} // KEYINPUT is read-only hardware
                Serial => {}
                Interrupt => match addr {
                    0x0400_0204 => self.waitcnt.0 = (self.waitcnt.0 & 0xFF00) | value as u16,
                    0x0400_0205 => self.waitcnt.0 = (self.waitcnt.0 & 0x00FF) | (value as u16) << 8,
                    0x0400_0300 => {} // POSTFLG (read-only in practice)
                    0x0400_0301 => self.halted = true, // HALTCNT: any write halts the CPU
                    _ => self.interrupt.write(addr, value),
                },
            }
        }
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let low = self.read8(addr) as u16;
        let high = self.read8(addr + 1) as u16;
        (high << 8) | low
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
