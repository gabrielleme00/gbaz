use super::{Apu, InterruptController, Ppu, Timer};
use super::bus::regions::IoRegisterRegion;

pub struct IoDevices {
    pub interrupt: InterruptController,
    pub ppu: Box<Ppu>,
    pub apu: Apu,
    pub timer: Timer,
}

impl IoDevices {
    pub fn new(interrupt: InterruptController, ppu: Box<Ppu>, apu: Apu, timer: Timer) -> Self {
        Self {
            interrupt,
            ppu,
            apu,
            timer,
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
                Interrupt => self.interrupt.read(addr),
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
                Interrupt => self.interrupt.write(addr, value),
            }
        }
    }
}
