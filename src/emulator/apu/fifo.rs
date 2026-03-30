use std::collections::VecDeque;

/// DMA sound FIFO - 32-byte capacity, holds i8 PCM samples.
pub struct DmaFifo {
    queue: VecDeque<i8>,
    current_sample: i8,
}

impl DmaFifo {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::with_capacity(32),
            current_sample: 0,
        }
    }

    /// Write a 16-bit halfword (2 consecutive bytes, low byte first) into the FIFO.
    pub fn write_halfword(&mut self, hw: u16) {
        for byte in [(hw & 0xFF) as u8 as i8, (hw >> 8) as u8 as i8] {
            if self.queue.len() < 32 {
                self.queue.push_back(byte);
            }
        }
    }

    /// Write a 32-bit word (4 consecutive bytes, LSB first) into the FIFO.
    pub fn write_word(&mut self, word: u32) {
        for shift in [0u32, 8, 16, 24] {
            let byte = ((word >> shift) & 0xFF) as u8 as i8;
            if self.queue.len() < 32 {
                self.queue.push_back(byte);
            }
        }
    }

    /// Consume one sample on a timer overflow.
    pub fn tick(&mut self) {
        if let Some(s) = self.queue.pop_front() {
            self.current_sample = s;
        }
    }

    /// The last consumed sample, output to the DAC.
    pub fn sample(&self) -> i8 {
        self.current_sample
    }

    /// True when <=16 bytes remain - the hardware requests a DMA refill.
    pub fn needs_refill(&self) -> bool {
        self.queue.len() <= 16
    }

    pub fn reset(&mut self) {
        self.queue.clear();
        self.current_sample = 0;
    }
}
