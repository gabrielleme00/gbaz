use crate::emulator::interrupt::signal_irq;
use std::{cell::RefCell, rc::Rc};

mod consts {
    use crate::emulator::interrupt::{Interrupt, Interrupt::*};

    pub const PRESCALER: [u64; 4] = [1, 64, 256, 1024];
    pub const TIMER_IRQ: [Interrupt; 4] = [Timer0, Timer1, Timer2, Timer3];
    pub const CTRL_PRESCALER_MASK: u16 = 0x03;
    pub const CTRL_COUNT_UP: u16 = 0x04;
    pub const CTRL_IRQ_EN: u16 = 0x40;
    pub const CTRL_START: u16 = 0x80;
}
use consts::*;

/// One GBA hardware timer channel.
struct Channel {
    /// Reload value - written to TMxCNT_L; copied to counter on start or overflow.
    reload: u16,
    /// Counter snapshot: the exact counter value at `timestamp`.
    counter: u16,
    /// Absolute cycle at which `counter` was last captured.
    timestamp: u64,
    /// TMxCNT_H control register (only low 8 bits are used by hardware).
    control: u16,
    /// Absolute cycle at which this timer will next overflow.
    /// `u64::MAX` when the timer is stopped or in count-up mode.
    deadline: u64,
}

impl Channel {
    fn new() -> Self {
        Self {
            reload: 0,
            counter: 0,
            timestamp: 0,
            control: 0,
            deadline: u64::MAX,
        }
    }

    fn is_started(&self) -> bool {
        self.control & CTRL_START != 0
    }

    fn is_count_up(&self) -> bool {
        self.control & CTRL_COUNT_UP != 0
    }

    fn irq_enabled(&self) -> bool {
        self.control & CTRL_IRQ_EN != 0
    }

    fn prescaler(&self) -> u64 {
        PRESCALER[(self.control & CTRL_PRESCALER_MASK) as usize]
    }

    /// Compute the current counter value from the snapshot and elapsed cycles.
    fn current_counter(&self, now: u64) -> u16 {
        if !self.is_started() || self.is_count_up() {
            return self.counter;
        }
        let ticks = now.saturating_sub(self.timestamp) / self.prescaler();
        self.counter.wrapping_add(ticks as u16)
    }

    /// Snapshot the counter at `now` and (re)compute the overflow deadline.
    fn reschedule(&mut self, now: u64) {
        if self.is_started() && !self.is_count_up() {
            let remaining = 0x1_0000u64 - self.counter as u64;
            self.deadline = now + remaining * self.prescaler();
        } else {
            self.deadline = u64::MAX;
        }
        self.timestamp = now;
    }
}

pub struct Timer {
    channels: [Channel; 4],
    cycles: u64,
    interrupt_flags: Rc<RefCell<u16>>,
    /// Bitmask of channels that overflowed since last call to `take_overflow_flags()`.
    overflow_pending: u8,
}

impl Timer {
    pub fn new(interrupt_flags: Rc<RefCell<u16>>) -> Self {
        Self {
            channels: std::array::from_fn(|_| Channel::new()),
            cycles: 0,
            interrupt_flags,
            overflow_pending: 0,
        }
    }

    pub fn reset(&mut self) {
        for ch in self.channels.iter_mut() {
            *ch = Channel::new();
        }
        self.cycles = 0;
        self.overflow_pending = 0;
    }

    /// Returns and clears the bitmask of channels that overflowed since the last call.
    pub fn take_overflow_flags(&mut self) -> u8 {
        let flags = self.overflow_pending;
        self.overflow_pending = 0;
        flags
    }

    /// Advance the timer block by `delta` clock cycles, firing overflows as needed.
    pub fn advance(&mut self, delta: u64) {
        self.cycles += delta;
        self.drain_overflows();
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let half = self.read_16(addr & !1);
        if addr & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        let (ch, reg) = Self::decode(addr);
        if ch >= 4 {
            return 0;
        }
        match reg {
            0 => self.channels[ch].current_counter(self.cycles),
            2 => self.channels[ch].control & 0x00FF,
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
        let merged = if addr & 1 == 0 {
            (half & 0xFF00) | value as u16
        } else {
            (half & 0x00FF) | ((value as u16) << 8)
        };
        self.write_16(addr & !1, merged);
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        let (ch, reg) = Self::decode(addr);
        if ch >= 4 {
            return;
        }
        match reg {
            // TMxCNT_L - reload register only (counter unchanged until start)
            0 => {
                self.channels[ch].reload = value;
            }
            // TMxCNT_H - control register; handle start/stop transitions
            2 => {
                let was_started = self.channels[ch].is_started();
                self.channels[ch].control = value & 0x00FF;
                let now_started = self.channels[ch].is_started();

                if !was_started && now_started {
                    // Start: reload counter and schedule next overflow.
                    let reload = self.channels[ch].reload;
                    self.channels[ch].counter = reload;
                    self.channels[ch].reschedule(self.cycles);
                } else if was_started && !now_started {
                    // Stop: freeze the counter at its current value.
                    let now = self.cycles;
                    let current = self.channels[ch].current_counter(now);
                    self.channels[ch].counter = current;
                    self.channels[ch].deadline = u64::MAX;
                } else if now_started && !self.channels[ch].is_count_up() {
                    // Prescaler / other field changed while running: re-snapshot.
                    let now = self.cycles;
                    let current = self.channels[ch].current_counter(now);
                    self.channels[ch].counter = current;
                    self.channels[ch].reschedule(now);
                }
            }
            _ => {}
        }
    }

    /// For 32-bit writes, reload and control are written atomically.
    /// Per the GBA docs, if the start bit transitions 0->1 in the same store that
    /// writes the reload value, the newly written reload is the initial counter.
    pub fn write_32(&mut self, addr: u32, value: u32) {
        // Write reload first so that the control write (start 0->1) picks it up.
        self.write_16(addr, (value & 0xFFFF) as u16);
        self.write_16(addr + 2, (value >> 16) as u16);
    }

    /// Drain all pending overflows, processing channels in order so that
    /// count-up cascades are handled correctly within the same pass.
    fn drain_overflows(&mut self) {
        loop {
            let mut any = false;
            for ch in 0..4usize {
                if !self.channels[ch].is_count_up() && self.channels[ch].deadline <= self.cycles {
                    self.fire_overflow(ch);
                    any = true;
                    break; // restart so cascades into higher channels are seen
                }
            }
            if !any {
                break;
            }
        }
    }

    /// Fire one overflow for channel `ch`: send IRQ, reload, reschedule,
    /// and cascade into the next count-up channel if applicable.
    fn fire_overflow(&mut self, ch: usize) {
        if self.channels[ch].irq_enabled() {
            signal_irq(&self.interrupt_flags, TIMER_IRQ[ch]);
        }

        // Record the overflow so APU can clock DMA FIFOs
        self.overflow_pending |= 1 << ch;

        self.channels[ch].counter = self.channels[ch].reload;
        let now = self.cycles;
        self.channels[ch].reschedule(now);

        // Cascade: tick the next count-up timer (ch+1 only; it cascades further
        // recursively if it overflows).
        if ch + 1 < 4 && self.channels[ch + 1].is_started() && self.channels[ch + 1].is_count_up() {
            let (new_val, overflowed) = self.channels[ch + 1].counter.overflowing_add(1);
            self.channels[ch + 1].counter = new_val;
            if overflowed {
                self.fire_overflow(ch + 1);
            }
        }
    }

    /// Decode a memory-mapped address into (channel_index, byte_offset_within_channel).
    fn decode(addr: u32) -> (usize, u32) {
        let offset = addr.wrapping_sub(0x0400_0100);
        ((offset / 4) as usize, offset % 4)
    }
}
