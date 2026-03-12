use std::{cell::RefCell, rc::Rc};

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
    enable: u16,         // IE - Interrupt Enable Register
    flag: Rc<RefCell<u16>>, // IF - Interrupt Flag Register
    master_enable: bool, // IME - Interrupt Master Enable Flag
}

impl InterruptController {
    pub fn new(flag: Rc<RefCell<u16>>) -> Self {
        Self {
            enable: 0,
            flag,
            master_enable: false,
        }
    }

    pub fn read(&self, addr: u32) -> u8 {
        match addr {
            0x4000_0200 => (self.enable & 0xFF) as u8,
            0x4000_0201 => (self.enable >> 8) as u8,
            0x4000_0202 => (*self.flag.borrow() & 0xFF) as u8,
            0x4000_0203 => (*self.flag.borrow() >> 8) as u8,
            0x4000_0208 => self.master_enable as u8,
            _ => 0,
        }
    }

    pub fn write(&mut self, addr: u32, value: u8) {
        match addr {
            0x4000_0200 => self.enable = (self.enable & 0xFF00) | value as u16,
            0x4000_0201 => self.enable = (self.enable & 0x00FF) | ((value as u16) << 8),
            // IF is write-1-to-clear: writing a 1 acknowledges (clears) that interrupt bit.
            0x4000_0202 => *self.flag.borrow_mut() &= !(value as u16),
            0x4000_0203 => *self.flag.borrow_mut() &= !((value as u16) << 8),
            0x4000_0208 => self.master_enable = value != 0,
            _ => {}
        }
    }

    /// Returns true if a maskable IRQ is pending and should be dispatched.
    /// Requires IME enabled, at least one enabled interrupt flagged, and the
    /// CPU I bit to be clear (checked separately in the CPU).
    pub fn irq_pending(&self) -> bool {
        self.master_enable && (self.enable & *self.flag.borrow()) != 0
    }
}
