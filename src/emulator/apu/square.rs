// Duty wave tables (8 steps each, position 0 played first).
// Pattern shape: 12.5% / 25% / 50% / 75%
const DUTY_TABLE: [[bool; 8]; 4] = [
    [false, false, false, false, false, false, false, true],  // 12.5%
    [true,  false, false, false, false, false, false, true],  // 25%
    [true,  false, false, false, true,  true,  true,  true],  // 50%
    [false, true,  true,  true,  true,  true,  true,  false], // 75%
];

/// Shared square-wave channel for CH1 and CH2.
pub struct SquareChannel {
    // Latched register fields
    pub duty: u8,           // bits 7-6 of NRx1 (0-3)
    pub length_load: u8,    // bits 5-0 of NRx1 (reload for length counter = 64 - n)
    pub env_initial: u8,    // bits 15-12 of NRx2 (0-15)
    pub env_increase: bool, // bit 11 of NRx2
    pub env_period: u8,     // bits 10-8 of NRx2 (0 = no envelope)
    pub freq: u16,          // bits 10-0 of NRx3+NRx4 (11-bit)
    pub length_enable: bool,// bit 14 of NRx4

    // Runtime state
    pub enabled: bool,
    length_counter: u16,    // counts down; channel off when it reaches 0
    volume: u8,             // current envelope volume 0-15
    env_timer: u8,          // frame-seq ticks until next envelope step
    freq_timer: u32,        // CPU cycles remaining in current duty step
    duty_pos: u8,           // 0-7 duty step index
}

impl SquareChannel {
    pub fn new() -> Self {
        Self {
            duty: 0,
            length_load: 0,
            env_initial: 0,
            env_increase: false,
            env_period: 0,
            freq: 0,
            length_enable: false,
            enabled: false,
            length_counter: 0,
            volume: 0,
            env_timer: 0,
            freq_timer: 0,
            duty_pos: 0,
        }
    }

    /// Duty step timer period in CPU cycles. Period = (2048 - n) × 16.
    fn period(&self) -> u32 {
        (2048 - self.freq as u32) * 16
    }

    /// Trigger (restart) the channel. Called when bit 15 of NRx4 is set.
    pub fn trigger(&mut self) {
        self.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.freq_timer = self.period();
        self.volume = self.env_initial;
        self.env_timer = if self.env_period == 0 { 8 } else { self.env_period };
    }

    /// Advance the frequency (duty) timer by `cycles` CPU cycles.
    pub fn advance_timer(&mut self, cycles: u32) {
        if !self.enabled || self.volume == 0 && !self.env_increase {
            return;
        }
        let mut rem = cycles;
        loop {
            if rem < self.freq_timer {
                self.freq_timer -= rem;
                break;
            }
            rem -= self.freq_timer;
            self.duty_pos = (self.duty_pos + 1) & 7;
            self.freq_timer = self.period();
        }
    }

    /// Clock the length counter (called at 256 Hz = every 2nd frame-seq step).
    pub fn clock_length(&mut self) {
        if self.length_enable && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled = false;
            }
        }
    }

    /// Clock the volume envelope (called at 64 Hz = frame-seq step 7).
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
    pub fn sample(&self) -> f32 {
        if !self.enabled || !self.is_dac_on() {
            return 0.0;
        }
        let high = DUTY_TABLE[self.duty as usize][self.duty_pos as usize];
        let amp = self.volume as f32 / 15.0;
        if high { amp } else { -amp }
    }
}
