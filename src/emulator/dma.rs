use crate::emulator::interrupt::{Interrupt, signal_irq};
use std::{cell::RefCell, rc::Rc};

// Source address masks per channel (DMA0 limited to 27-bit; others 28-bit)
const SRC_ADDR_MASK: [u32; 4] = [0x07FF_FFFF, 0x0FFF_FFFF, 0x0FFF_FFFF, 0x0FFF_FFFF];
// Destination address masks per channel
const DST_ADDR_MASK: [u32; 4] = [0x07FF_FFFF, 0x07FF_FFFF, 0x07FF_FFFF, 0x0FFF_FFFF];
// Maximum unit count when count register == 0
const MAX_UNITS: [u32; 4] = [0x4000, 0x4000, 0x4000, 0x1_0000];

const FIFO_A: u32 = 0x0400_00A0;
const FIFO_B: u32 = 0x0400_00A4;

/// Which hardware event can trigger a DMA burst.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DmaEvent {
    Immediate,
    VBlank,
    HBlank,
    Special,
}

/// Fully-resolved parameters for executing one DMA burst.
#[derive(Clone, Copy)]
pub struct DmaRunParams {
    pub src: u32,
    pub dst: u32,
    /// Number of units (halfwords or words) to transfer.
    pub count: u32,
    /// `true` = 32-bit words, `false` = 16-bit halfwords.
    pub is_word: bool,
    /// Signed byte delta applied to `src` after each unit.
    pub src_step: i32,
    /// Signed byte delta applied to `dst` after each unit.
    pub dst_step: i32,
}

pub struct Dma {
    channels: [Channel; 4],
    interrupt_flags: Rc<RefCell<u16>>,
}

impl Dma {
    pub fn new(interrupt_flags: Rc<RefCell<u16>>) -> Self {
        Self {
            channels: [Channel::default(); 4],
            interrupt_flags,
        }
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        let (ch_idx, reg) = Self::decode(addr & !3);
        if ch_idx >= 4 {
            return 0;
        }
        let ch = &self.channels[ch_idx];
        match reg {
            0 => ch.source,
            4 => ch.dest,
            8 => (ch.control.0 as u32) << 16 | ch.count as u32,
            _ => 0,
        }
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        let (ch_idx, reg) = Self::decode(addr & !3);
        if ch_idx >= 4 {
            return;
        }
        let ch = &mut self.channels[ch_idx];
        match reg {
            0 => ch.source = value,
            4 => ch.dest = value,
            8 => {
                ch.count = value as u16;
                let was_enabled = ch.control.enable();
                ch.control.0 = (value >> 16) as u16;
                if !was_enabled && ch.control.enable() {
                    let raw_count = ch.count as u32;
                    ch.internal_source = ch.source & SRC_ADDR_MASK[ch_idx];
                    ch.internal_dest   = ch.dest   & DST_ADDR_MASK[ch_idx];
                    ch.internal_count  =
                        if raw_count == 0 { MAX_UNITS[ch_idx] } else { raw_count };
                }
            }
            _ => {}
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        let word = self.read_32(addr & !3);
        if addr & 2 == 0 { word as u16 } else { (word >> 16) as u16 }
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        let aligned = addr & !3;
        let word = self.read_32(aligned);
        let new = if addr & 2 == 0 {
            (word & 0xFFFF_0000) | (value as u32)
        } else {
            (word & 0x0000_FFFF) | ((value as u32) << 16)
        };
        self.write_32(aligned, new);
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let word = self.read_32(addr & !3);
        (word >> ((addr & 3) * 8)) as u8
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        let aligned = addr & !3;
        let word = self.read_32(aligned);
        let shift = (addr & 3) * 8;
        let new = (word & !(0xFF_u32 << shift)) | ((value as u32) << shift);
        self.write_32(aligned, new);
    }

    #[inline]
    fn decode(addr: u32) -> (usize, u32) {
        let offset = addr.wrapping_sub(0x0400_00B0);
        ((offset / 12) as usize, offset % 12)
    }

    /// Returns `true` if channel `ch` is enabled and waiting for `event`.
    pub fn channel_wants_run(&self, ch: usize, event: DmaEvent) -> bool {
        let c = &self.channels[ch];
        if !c.control.enable() {
            return false;
        }
        match (event, c.control.get_start_timing()) {
            (DmaEvent::Immediate, DmaStartTiming::Immediate) => true,
            (DmaEvent::VBlank, DmaStartTiming::VBlank) => true,
            (DmaEvent::HBlank, DmaStartTiming::HBlank) => true,
            // DMA0 is prohibited in Special mode
            (DmaEvent::Special, DmaStartTiming::Special) => ch >= 1,
            _ => false,
        }
    }

    /// Build the burst parameters for channel `ch` using its current internal state.
    pub fn run_params(&self, ch: usize) -> DmaRunParams {
        let c = &self.channels[ch];
        let is_word = c.control.get_transfer_type() == DmaTransferType::Word;
        let unit = if is_word { 4_u32 } else { 2_u32 };

        // FIFO mode: DMA1/DMA2 with Special timing targeting a FIFO address.
        // Transfer type bit and word count are both ignored; always 4×32-bit words.
        let is_fifo = (ch == 1 || ch == 2)
            && c.control.get_start_timing() == DmaStartTiming::Special
            && (c.internal_dest == FIFO_A || c.internal_dest == FIFO_B);

        if is_fifo {
            return DmaRunParams {
                src: c.internal_source,
                dst: c.internal_dest,
                count: 4,
                is_word: true,
                src_step: 4,
                dst_step: 0, // dest address is fixed in FIFO mode
            };
        }

        let src_step = match c.control.get_src_addr_control() {
            DmaAddrControl::Increment | DmaAddrControl::IncrReload => unit as i32,
            DmaAddrControl::Decrement => -(unit as i32),
            DmaAddrControl::Fixed => 0,
        };
        let dst_step = match c.control.get_dst_addr_control() {
            DmaAddrControl::Increment | DmaAddrControl::IncrReload => unit as i32,
            DmaAddrControl::Decrement => -(unit as i32),
            DmaAddrControl::Fixed => 0,
        };

        DmaRunParams {
            src: c.internal_source,
            dst: c.internal_dest,
            count: c.internal_count,
            is_word,
            src_step,
            dst_step,
        }
    }

    /// Update internal state after a burst. `new_src`/`new_dst` are the pointer
    /// values *after* the final unit was transferred (i.e. already advanced).
    pub fn finish_channel(&mut self, ch: usize, new_src: u32, new_dst: u32) {
        let c = &mut self.channels[ch];
        c.internal_source = new_src;

        // IncrReload reloads the destination address at every repeat.
        if c.control.get_dst_addr_control() == DmaAddrControl::IncrReload {
            c.internal_dest = c.dest & DST_ADDR_MASK[ch];
        } else {
            c.internal_dest = new_dst;
        }

        if c.control.get_repeat() {
            // Reload word count; enable bit stays set for next trigger.
            let raw_count = c.count as u32;
            c.internal_count = if raw_count == 0 {
                MAX_UNITS[ch]
            } else {
                raw_count
            };
        } else {
            c.control.set_enable(false);
        }

        if c.control.get_irq_on_end() {
            let irq = match ch {
                0 => Interrupt::Dma0,
                1 => Interrupt::Dma1,
                2 => Interrupt::Dma2,
                3 => Interrupt::Dma3,
                _ => unreachable!(),
            };
            signal_irq(&self.interrupt_flags, irq);
        }
    }

    /// Auto-disable DMA3 video-capture mode when VCOUNT reaches 162.
    pub fn stop_video_capture(&mut self) {
        let c = &mut self.channels[3];
        if c.control.enable() && c.control.get_start_timing() == DmaStartTiming::Special {
            c.control.set_enable(false);
        }
    }
}

#[derive(Default, Copy, Clone, Debug)]
struct Channel {
    // CPU-visible registers
    source: u32,
    dest: u32,
    count: u16,
    control: DmaControl,
    // Internal transfer pointers - not CPU-readable; survive repeated bursts.
    internal_source: u32,
    internal_dest: u32,
    internal_count: u32,
}

bitfield::bitfield! {
    #[derive(Default, Copy, Clone, Debug)]
    pub struct DmaControl(u16);
    dst_addr_control, set_dst_addr_control: 6, 5;
    src_addr_control, set_src_addr_control: 8, 7;
    repeat, set_repeat: 9;
    transfer_type, set_transfer_type: 10;
    gamepak_drq, set_gamepak_drq: 11;
    start_timing, set_start_timing: 13, 12;
    irq_on_end, set_irq_on_end: 14;
    enable, set_enable: 15;
}

impl DmaControl {
    pub fn get_dst_addr_control(&self) -> DmaAddrControl {
        DmaAddrControl::from_u16(self.dst_addr_control())
    }

    pub fn get_src_addr_control(&self) -> DmaAddrControl {
        DmaAddrControl::from_u16(self.src_addr_control())
    }

    pub fn get_repeat(&self) -> bool {
        self.repeat()
    }

    pub fn get_gamepak_drq(&self) -> bool {
        self.gamepak_drq()
    }

    pub fn get_transfer_type(&self) -> DmaTransferType {
        DmaTransferType::from_bool(self.transfer_type())
    }

    pub fn get_start_timing(&self) -> DmaStartTiming {
        DmaStartTiming::from_u16(self.start_timing())
    }

    pub fn get_irq_on_end(&self) -> bool {
        self.irq_on_end()
    }

    pub fn get_enable(&self) -> bool {
        self.enable()
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum DmaAddrControl {
    Increment = 0,
    Decrement = 1,
    Fixed = 2,
    IncrReload = 3,
}

impl DmaAddrControl {
    fn from_u16(v: u16) -> Self {
        match v & 0b11 {
            0 => Self::Increment,
            1 => Self::Decrement,
            2 => Self::Fixed,
            3 => Self::IncrReload,
            _ => unreachable!(),
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum DmaTransferType {
    HalfWord = 0,
    Word = 1,
}

impl DmaTransferType {
    fn from_bool(v: bool) -> Self {
        if v { Self::Word } else { Self::HalfWord }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum DmaStartTiming {
    Immediate = 0,
    VBlank = 1,
    HBlank = 2,
    Special = 3,
}

impl DmaStartTiming {
    fn from_u16(v: u16) -> Self {
        match v & 0b11 {
            0 => Self::Immediate,
            1 => Self::VBlank,
            2 => Self::HBlank,
            3 => Self::Special,
            _ => unreachable!(),
        }
    }
}
