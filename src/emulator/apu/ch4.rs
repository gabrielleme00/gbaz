/// Channel 4 - Noise output using a linear feedback shift register (LFSR).
pub struct NoiseChannel {
    // ── Latched register fields ──────────────────────────────────────────────
    pub length_load: u8,    // bits 5-0 of NR41 (length = 64 - n)
    pub env_initial: u8,    // bits 15-12 of NR42
    pub env_increase: bool, // bit 11 of NR42
    pub env_period: u8,     // bits 10-8 of NR42
    pub div_ratio: u8,      // bits 2-0 of NR43 (r; 0 -> use 0.5)
    pub short_mode: bool,   // bit 3 of NR43 (true = 7-bit LFSR)
    pub shift_freq: u8,     // bits 7-4 of NR43 (s; 0-15)
    pub length_enable: bool,// bit 14 of NR44

    // ── Runtime state ────────────────────────────────────────────────────────
    pub enabled: bool,
    length_counter: u16,
    volume: u8,
    env_timer: u8,
    noise_timer: u32,   // CPU cycles remaining until next LFSR clock
    lfsr: u16,          // 15-bit LFSR (or 7-bit in short_mode)
}

impl NoiseChannel {
    pub fn new() -> Self {
        Self {
            length_load: 0,
            env_initial: 0,
            env_increase: false,
            env_period: 0,
            div_ratio: 0,
            short_mode: false,
            shift_freq: 0,
            length_enable: false,
            enabled: false,
            length_counter: 0,
            volume: 0,
            env_timer: 0,
            noise_timer: 0,
            lfsr: 0x7FFF,
        }
    }

    fn period(&self) -> u32 {
        // f = 524288 / r / 2^(s+1) Hz  (r=0 -> use r=0.5)
        // period (CPU cycles) = CPU_CLOCK / f
        //  = 16777216 * r * 2^(s+1) / 524288  for r > 0
        //  = 16 * 2^(s+1)                      for r = 0
        let s = self.shift_freq as u32;
        if self.div_ratio == 0 {
            16u32 << (s + 1)
        } else {
            32u32 * self.div_ratio as u32 * (1u32 << (s + 1))
        }
    }

    pub fn trigger(&mut self) {
        self.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.volume = self.env_initial;
        self.env_timer = if self.env_period == 0 { 8 } else { self.env_period };
        self.lfsr = if self.short_mode { 0x7F } else { 0x7FFF };
        self.noise_timer = self.period();
    }

    /// Advance the LFSR timer by `cycles` CPU cycles.
    pub fn advance_timer(&mut self, cycles: u32) {
        if !self.enabled {
            return;
        }
        let mut rem = cycles;
        loop {
            if rem < self.noise_timer {
                self.noise_timer -= rem;
                break;
            }
            rem -= self.noise_timer;
            self.clock_lfsr();
            self.noise_timer = self.period();
        }
    }

    fn clock_lfsr(&mut self) {
        // X = X >> 1, carry = bit 0 of old X
        let carry = self.lfsr & 1;
        self.lfsr >>= 1;
        if carry != 0 {
            // XOR feedback into bit 14 (15-bit) or bit 6 (7-bit)
            self.lfsr ^= if self.short_mode { 0x60 } else { 0x6000 };
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

    pub fn clock_envelope(&mut self) {
        if self.env_period == 0 {
            return;
        }
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_increase {
                if self.volume < 15 {
                    self.volume += 1;
                }
            } else if self.volume > 0 {
                self.volume -= 1;
            }
        }
    }

    pub fn is_dac_on(&self) -> bool {
        self.env_initial > 0 || self.env_increase
    }

    /// Current output sample in [-1.0, 1.0].
    /// The LFSR's LSB is 0 = HIGH, 1 = LOW (inverted).
    pub fn sample(&self) -> f32 {
        if !self.enabled || !self.is_dac_on() {
            return 0.0;
        }
        let amp = self.volume as f32 / 15.0;
        if self.lfsr & 1 == 0 { amp } else { -amp }
    }
}
