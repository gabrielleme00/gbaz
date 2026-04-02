/// Channel 3 - Wave output, playing 4-bit samples from Wave RAM.
pub struct WaveChannel {
    // Latched register fields
    pub two_banks: bool,     // NR30 bit 5: 0=one bank/32 samples, 1=two banks/64 samples
    pub bank_select: u8,     // NR30 bit 6: which bank plays (0 or 1)
    pub dac_on: bool,        // NR30 bit 7: 0=off, 1=playback
    pub length_load: u16,    // bits 7-0 of NR31 (length = 256 - n)
    pub volume_shift: u8,    // bits 14-13 of NR32 (0=mute, 1=100%, 2=50%, 3=25%)
    pub force_volume: bool,  // bit 15 of NR32 (force 75% = shift by 1 after masking)
    pub freq: u16,           // bits 10-0 of NR33+NR34
    pub length_enable: bool, // bit 14 of NR34

    // Runtime state
    pub enabled: bool,
    pub wave_ram: [[u8; 16]; 2], // 2 banks × 16 bytes (each byte = two 4-bit samples)
    wave_pos: u8,           // sample position within the playing bank (0-31) or (0-63)
    freq_timer: u32,        // CPU cycles remaining until next sample
    length_counter: u16,
}

impl WaveChannel {
    pub fn new() -> Self {
        Self {
            two_banks: false,
            bank_select: 0,
            dac_on: false,
            length_load: 0,
            volume_shift: 0,
            force_volume: false,
            freq: 0,
            length_enable: false,
            enabled: false,
            wave_ram: [[0; 16]; 2],
            wave_pos: 0,
            freq_timer: 0,
            length_counter: 0,
        }
    }

    /// Sample timer period: (2048 - n) × 8 CPU cycles per sample.
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 8
    }

    fn total_samples(&self) -> u8 {
        if self.two_banks { 64 } else { 32 }
    }

    pub fn trigger(&mut self) {
        self.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 256;
        }
        self.wave_pos = 0;
        self.freq_timer = self.period();
    }

    /// Advance the wave timer by `cycles` CPU cycles.
    pub fn advance_timer(&mut self, cycles: u32) {
        if !self.enabled || !self.dac_on {
            return;
        }
        let mut rem = cycles;
        loop {
            if rem < self.freq_timer {
                self.freq_timer -= rem;
                break;
            }
            rem -= self.freq_timer;
            self.wave_pos = (self.wave_pos + 1) % self.total_samples();
            self.freq_timer = self.period();
        }
    }

    pub fn clock_length(&mut self) {
        if self.length_enable && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled = false;
            }
        }
    }

    /// Read the current 4-bit sample from Wave RAM.
    fn current_nibble(&self) -> u8 {
        let pos = self.wave_pos as usize;
        let bank = if self.two_banks {
            // In 2-bank mode the wave_pos runs 0-63; the first 32 are in bank_select,
            // the second 32 are in the other bank.
            if pos < 32 { self.bank_select as usize } else { (self.bank_select ^ 1) as usize }
        } else {
            self.bank_select as usize
        };
        let local_pos = pos % 32;
        let byte = self.wave_ram[bank][local_pos / 2];
        // MSB of each byte is the first sample (even positions).
        if local_pos & 1 == 0 { (byte >> 4) & 0xF } else { byte & 0xF }
    }

    /// Current output sample in [-1.0, 1.0].
    pub fn sample(&self) -> f32 {
        if !self.enabled || !self.dac_on {
            return 0.0;
        }
        let nibble = self.current_nibble() as f32; // 0..15
        let shifted = if self.force_volume {
            // Force 75%: output at 75% of full amplitude (same as shift-by-1 after no attenuation)
            nibble * 0.75
        } else {
            match self.volume_shift {
                0 => 0.0,          // mute
                1 => nibble,       // 100%
                2 => nibble / 2.0, // 50%
                3 => nibble / 4.0, // 25%
                _ => 0.0,
            }
        };
        // Normalize to [-1, 1]: nibble range is 0..15 -> center at 7.5
        (shifted / 7.5 - 1.0).clamp(-1.0, 1.0)
    }

    // Wave RAM access (user sees the bank NOT currently playing)

    pub fn read_wave_byte(&self, offset: usize) -> u8 {
        let bank = (self.bank_select ^ 1) as usize;
        self.wave_ram[bank][offset & 0xF]
    }

    pub fn write_wave_byte(&mut self, offset: usize, value: u8) {
        let bank = (self.bank_select ^ 1) as usize;
        self.wave_ram[bank][offset & 0xF] = value;
    }
}
