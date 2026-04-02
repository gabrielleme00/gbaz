use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy)]
pub enum Interrupt {
    VBlank = 1 << 0,
    HBlank = 1 << 1,
    VCount = 1 << 2,
    Timer0 = 1 << 3,
    Timer1 = 1 << 4,
    Timer2 = 1 << 5,
    Timer3 = 1 << 6,
    Serial = 1 << 7,
    Dma0 = 1 << 8,
    Dma1 = 1 << 9,
    Dma2 = 1 << 10,
    Dma3 = 1 << 11,
    Keypad = 1 << 12,
    GamePak = 1 << 13,
}

pub fn signal_irq(flag: &Rc<RefCell<u16>>, irq_bit: Interrupt) {
    *flag.borrow_mut() |= irq_bit as u16;
}

pub struct InterruptController {
    enable: u16,            // IE - Interrupt Enable Register
    flag: Rc<RefCell<u16>>, // IF - Interrupt Flag Register
    master_enable: bool,    // IME - Interrupt Master Enable Flag
}

impl InterruptController {
    pub fn new(flag: Rc<RefCell<u16>>) -> Self {
        Self {
            enable: 0,
            flag,
            master_enable: false,
        }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let half = self.read_16(addr & !1);
        if addr & 1 == 0 { half as u8 } else { (half >> 8) as u8 }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        match addr {
            0x0400_0200 => self.enable,
            0x0400_0202 => *self.flag.borrow(),
            0x0400_0208 => self.master_enable as u16,
            _ => 0,
        }
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        let lo = self.read_16(addr) as u32;
        let hi = self.read_16(addr + 2) as u32;
        lo | (hi << 16)
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        let half = self.read_16(addr & !1);
        let new = if addr & 1 == 0 {
            (half & 0xFF00) | (value as u16)
        } else {
            (half & 0x00FF) | ((value as u16) << 8)
        };
        self.write_16(addr & !1, new);
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        match addr {
            0x0400_0200 => self.enable = value,
            // IF is write-1-to-clear: writing a 1 acknowledges (clears) that interrupt bit.
            0x0400_0202 => *self.flag.borrow_mut() &= !value,
            0x0400_0208 => self.master_enable = value != 0,
            _ => {}
        }
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        self.write_16(addr, value as u16);
        self.write_16(addr + 2, (value >> 16) as u16);
    }

    /// Returns true if a maskable IRQ is pending and should be dispatched.
    /// Requires IME enabled, at least one enabled interrupt flagged, and the
    /// CPU I bit to be clear (checked separately in the CPU).
    pub fn irq_pending(&self) -> bool {
        self.master_enable && self.irq_asserted()
    }

    /// Returns true if any enabled interrupt flag is set, regardless of IME
    /// or the CPU I bit. Used as the HALT wake-up condition (GBA wakes from
    /// HALT when IE & IF != 0, independent of IME and the I bit).
    pub fn irq_asserted(&self) -> bool {
        (self.enable & *self.flag.borrow()) != 0
    }
}
