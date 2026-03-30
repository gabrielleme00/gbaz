mod ch3;
mod ch4;
mod dsp;
mod fifo;
mod square;

use dsp::CosineResampler;

use ch3::WaveChannel;
use ch4::NoiseChannel;
use fifo::DmaFifo;
use square::SquareChannel;

const CPU_CLOCK: f64 = 16_777_216.0;
/// Frame sequencer fires every 32768 CPU cycles (512 Hz with 8 steps -> 512 Hz).
const FRAME_SEQ_PERIOD: u64 = 32_768;

/// GBA APU - four PSG channels plus two DMA sound FIFOs.
pub struct Apu {
    // PSG Channels
    ch1: SquareChannel,
    ch2: SquareChannel,
    ch3: WaveChannel,
    ch4: NoiseChannel,

    // CH1 sweep state (not part of SquareChannel so CH2 shares the type).
    sweep_reg: u8,   // NR10: bits 6-4 sweep time, bit 3 direction, bits 2-0 shift
    sweep_timer: u8, // counts down per 128 Hz clock
    sweep_enabled: bool,
    shadow_freq: u16,

    // DMA Channels
    fifo_a: DmaFifo,
    fifo_b: DmaFifo,
    /// Bitmask: bit 0 = FIFO A requests DMA refill, bit 1 = FIFO B.
    fifo_dma_flags: u8,

    // Control Registers
    soundcnt_l: u16, // 0x4000080  NR50/NR51  master volume + per-channel L/R enables
    soundcnt_h: u16, // 0x4000082  DMA sound control / PSG volume
    soundcnt_x: u8,  // 0x4000084  NR52  master enable (bit 7) + channel status (bits 0-3)
    soundbias: u16,  // 0x4000088

    // Clocking
    frame_seq_acc: u64,
    frame_seq_step: u8,

    // Sample Generation
    resampler: CosineResampler,
    sample_acc: f64,
    sample_buffer: Vec<f32>,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            ch1: SquareChannel::new(),
            ch2: SquareChannel::new(),
            ch3: WaveChannel::new(),
            ch4: NoiseChannel::new(),
            sweep_reg: 0,
            sweep_timer: 0,
            sweep_enabled: false,
            shadow_freq: 0,
            fifo_a: DmaFifo::new(),
            fifo_b: DmaFifo::new(),
            fifo_dma_flags: 0,
            soundcnt_l: 0,
            soundcnt_h: 0,
            soundcnt_x: 0,
            soundbias: 0x200,
            frame_seq_acc: 0,
            frame_seq_step: 0,
            resampler: CosineResampler::new(32_768.0),
            sample_acc: 0.0,
            sample_buffer: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Set the output sample rate (must match the audio backend's sample rate).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.resampler.out_freq = rate as f32;
        self.sample_acc = 0.0;
    }

    // Main Clock

    /// Advance the APU by `delta` CPU cycles. Call once per bus tick.
    pub fn advance(&mut self, delta: u64) {
        if !self.master_enabled() {
            return;
        }

        // Frame sequencer (512 Hz)
        self.frame_seq_acc += delta;
        while self.frame_seq_acc >= FRAME_SEQ_PERIOD {
            self.frame_seq_acc -= FRAME_SEQ_PERIOD;
            self.clock_frame_sequencer();
        }

        // Sample generation: always run at GBA native 32768 Hz (512 CPU cycles per sample).
        const GBA_NATIVE_CYCLES: f64 = CPU_CLOCK / 32_768.0; // 512.0
        self.sample_acc += delta as f64;
        while self.sample_acc >= GBA_NATIVE_CYCLES {
            self.sample_acc -= GBA_NATIVE_CYCLES;
            self.generate_sample();
        }
    }

    /// Called by the timer block whenever timer 0 or 1 overflows.
    /// `timer_idx` is 0 or 1.
    pub fn on_timer_overflow(&mut self, timer_idx: usize) {
        if !self.master_enabled() {
            return;
        }
        let fifo_a_timer = (self.soundcnt_h >> 10) & 1;
        let fifo_b_timer = (self.soundcnt_h >> 14) & 1;

        if fifo_a_timer as usize == timer_idx {
            self.fifo_a.tick();
            if self.fifo_a.needs_refill() {
                self.fifo_dma_flags |= 1;
            }
        }
        if fifo_b_timer as usize == timer_idx {
            self.fifo_b.tick();
            if self.fifo_b.needs_refill() {
                self.fifo_dma_flags |= 2;
            }
        }
    }

    /// Returns and clears the DMA request flags (bit 0=FIFO A, bit 1=FIFO B).
    pub fn take_fifo_dma_flags(&mut self) -> u8 {
        let flags = self.fifo_dma_flags;
        self.fifo_dma_flags = 0;
        flags
    }

    /// Drain generated samples into `buf`.
    pub fn drain_samples(&mut self, buf: &mut Vec<f32>) {
        buf.append(&mut self.sample_buffer);
    }

    // Internal: frame sequencer

    fn clock_frame_sequencer(&mut self) {
        match self.frame_seq_step {
            0 | 4 => {
                self.clock_length_all();
            }
            2 | 6 => {
                self.clock_length_all();
                self.clock_sweep();
            }
            7 => {
                self.clock_envelope_all();
            }
            _ => {}
        }
        self.frame_seq_step = (self.frame_seq_step + 1) & 7;
    }

    fn clock_length_all(&mut self) {
        self.ch1.clock_length();
        self.ch2.clock_length();
        self.ch3.clock_length();
        self.ch4.clock_length();
    }

    fn clock_envelope_all(&mut self) {
        self.ch1.clock_envelope();
        self.ch2.clock_envelope();
        self.ch4.clock_envelope();
    }

    fn clock_sweep(&mut self) {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        let sweep_period = (self.sweep_reg >> 4) & 7;
        if self.sweep_timer == 0 {
            self.sweep_timer = if sweep_period == 0 { 8 } else { sweep_period };
            if self.sweep_enabled && sweep_period != 0 {
                let new_freq = self.sweep_calc();
                if new_freq <= 2047 && (self.sweep_reg & 7) != 0 {
                    self.shadow_freq = new_freq;
                    self.ch1.freq = new_freq;
                    // Overflow check after update
                    if self.sweep_calc() > 2047 {
                        self.ch1.enabled = false;
                    }
                }
            }
        }
    }

    fn sweep_calc(&mut self) -> u16 {
        let n = self.sweep_reg & 7;
        let delta = self.shadow_freq >> n;
        if self.sweep_reg & 8 != 0 {
            self.shadow_freq.saturating_sub(delta)
        } else {
            let result = self.shadow_freq + delta;
            if result > 2047 {
                self.ch1.enabled = false;
                2048 // overflow sentinel
            } else {
                result
            }
        }
    }

    // Internal: sample generation

    fn generate_sample(&mut self) {
        // Advance PSG timers by exactly one native period (512 CPU cycles = 32768 Hz).
        self.ch1.advance_timer(512);
        self.ch2.advance_timer(512);
        self.ch3.advance_timer(512);
        self.ch4.advance_timer(512);

        let [left, right] = self.mix();
        self.resampler.feed(left, right, &mut self.sample_buffer);
    }

    fn mix(&self) -> [f32; 2] {
        // PSG volume from SOUNDCNT_H bits 1-0.
        let psg_scale = match self.soundcnt_h & 3 {
            0 => 0.25_f32,
            1 => 0.5,
            2 => 1.0,
            _ => 0.25,
        };

        // SOUNDCNT_L bits 2-0: right master volume (0-7).
        // SOUNDCNT_L bits 6-4: left master volume (0-7).
        let psg_vol_r = (self.soundcnt_l & 7) as f32 / 7.0;
        let psg_vol_l = ((self.soundcnt_l >> 4) & 7) as f32 / 7.0;

        // SOUNDCNT_L bits 8-11: per-channel right enable.
        // SOUNDCNT_L bits 12-15: per-channel left enable.
        let ch1_r = (self.soundcnt_l >> 8) & 1 != 0;
        let ch2_r = (self.soundcnt_l >> 9) & 1 != 0;
        let ch3_r = (self.soundcnt_l >> 10) & 1 != 0;
        let ch4_r = (self.soundcnt_l >> 11) & 1 != 0;
        let ch1_l = (self.soundcnt_l >> 12) & 1 != 0;
        let ch2_l = (self.soundcnt_l >> 13) & 1 != 0;
        let ch3_l = (self.soundcnt_l >> 14) & 1 != 0;
        let ch4_l = (self.soundcnt_l >> 15) & 1 != 0;

        let s1 = self.ch1.sample();
        let s2 = self.ch2.sample();
        let s3 = self.ch3.sample();
        let s4 = self.ch4.sample();

        let psg_r = ((if ch1_r { s1 } else { 0.0 }) + (if ch2_r { s2 } else { 0.0 })
            + (if ch3_r { s3 } else { 0.0 }) + (if ch4_r { s4 } else { 0.0 }))
            * psg_scale * psg_vol_r * 0.25;
        let psg_l = ((if ch1_l { s1 } else { 0.0 }) + (if ch2_l { s2 } else { 0.0 })
            + (if ch3_l { s3 } else { 0.0 }) + (if ch4_l { s4 } else { 0.0 }))
            * psg_scale * psg_vol_l * 0.25;

        // DMA sound A/B volume: SOUNDCNT_H bit 2/3 (0=50%, 1=100%).
        let dma_a_vol: f32 = if (self.soundcnt_h >> 2) & 1 != 0 { 1.0 } else { 0.5 };
        let dma_b_vol: f32 = if (self.soundcnt_h >> 3) & 1 != 0 { 1.0 } else { 0.5 };

        // SOUNDCNT_H bit 8 = FIFO A right, bit 9 = FIFO A left.
        // SOUNDCNT_H bit 12 = FIFO B right, bit 13 = FIFO B left.
        let fifo_a_r = (self.soundcnt_h >> 8) & 1 != 0;
        let fifo_a_l = (self.soundcnt_h >> 9) & 1 != 0;
        let fifo_b_r = (self.soundcnt_h >> 12) & 1 != 0;
        let fifo_b_l = (self.soundcnt_h >> 13) & 1 != 0;

        let dma_a = self.fifo_a.sample() as f32 / 128.0 * dma_a_vol;
        let dma_b = self.fifo_b.sample() as f32 / 128.0 * dma_b_vol;

        let right = (psg_r
            + if fifo_a_r { dma_a } else { 0.0 }
            + if fifo_b_r { dma_b } else { 0.0 })
            .clamp(-1.0, 1.0);
        let left = (psg_l
            + if fifo_a_l { dma_a } else { 0.0 }
            + if fifo_b_l { dma_b } else { 0.0 })
            .clamp(-1.0, 1.0);

        [left, right]
    }

    fn master_enabled(&self) -> bool {
        self.soundcnt_x & 0x80 != 0
    }

    // Register I/O

    pub fn read_8(&self, addr: u32) -> u8 {
        let half = self.read_16(addr & !1);
        if addr & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        match addr {
            // CH1
            0x0400_0060 => self.sweep_reg as u16,
            0x0400_0062 => {
                ((self.ch1.duty as u16) << 6)
                    | ((self.ch1.env_initial as u16) << 12)
                    | (if self.ch1.env_increase { 1 << 11 } else { 0 })
                    | ((self.ch1.env_period as u16) << 8)
            }
            0x0400_0064 => {
                if self.ch1.length_enable {
                    1 << 14
                } else {
                    0
                }
            }
            // CH2
            0x0400_0068 => {
                ((self.ch2.duty as u16) << 6)
                    | ((self.ch2.env_initial as u16) << 12)
                    | (if self.ch2.env_increase { 1 << 11 } else { 0 })
                    | ((self.ch2.env_period as u16) << 8)
            }
            0x0400_006C => {
                if self.ch2.length_enable {
                    1 << 14
                } else {
                    0
                }
            }
            // CH3
            0x0400_0070 => {
                ((self.ch3.two_banks as u16) << 5)
                    | ((self.ch3.bank_select as u16) << 6)
                    | ((self.ch3.dac_on as u16) << 7)
            }
            0x0400_0072 => {
                ((self.ch3.volume_shift as u16) << 13) | ((self.ch3.force_volume as u16) << 15)
            }
            0x0400_0074 => {
                if self.ch3.length_enable {
                    1 << 14
                } else {
                    0
                }
            }
            // CH4
            0x0400_0078 => {
                ((self.ch4.env_initial as u16) << 12)
                    | (if self.ch4.env_increase { 1 << 11 } else { 0 })
                    | ((self.ch4.env_period as u16) << 8)
            }
            0x0400_007C => {
                (self.ch4.div_ratio as u16)
                    | ((self.ch4.short_mode as u16) << 3)
                    | ((self.ch4.shift_freq as u16) << 4)
                    | (if self.ch4.length_enable { 1 << 14 } else { 0 })
            }
            // Control
            0x0400_0080 => self.soundcnt_l,
            0x0400_0082 => self.soundcnt_h,
            0x0400_0084 => {
                let ch_flags = (self.ch1.enabled as u8)
                    | ((self.ch2.enabled as u8) << 1)
                    | ((self.ch3.enabled as u8) << 2)
                    | ((self.ch4.enabled as u8) << 3);
                (self.soundcnt_x & 0x80) as u16 | ch_flags as u16
            }
            0x0400_0088 => self.soundbias,
            // Wave RAM (user accesses the bank NOT currently playing)
            0x0400_0090..=0x0400_009E => {
                let off = (addr - 0x0400_0090) as usize;
                self.ch3.read_wave_byte(off) as u16
                    | ((self.ch3.read_wave_byte(off + 1) as u16) << 8)
            }
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
        // While master enable is off, PSG registers are writable but have no effect
        // (except SOUNDCNT_H and SOUNDBIAS always work).
        match addr {
            // CH1 Sweep (NR10)
            0x0400_0060 => {
                self.sweep_reg = (value & 0x7F) as u8;
            }
            // CH1 Duty/Length/Envelope (NR11/NR12)
            0x0400_0062 => {
                self.ch1.length_load = (value & 0x3F) as u8;
                self.ch1.duty = ((value >> 6) & 3) as u8;
                self.ch1.env_period = ((value >> 8) & 7) as u8;
                self.ch1.env_increase = (value >> 11) & 1 != 0;
                self.ch1.env_initial = ((value >> 12) & 0xF) as u8;
                if !self.ch1.is_dac_on() {
                    self.ch1.enabled = false;
                }
            }
            // CH1 Freq/Control (NR13/NR14)
            0x0400_0064 => {
                self.ch1.freq = (self.ch1.freq & 0x700) | (value & 0xFF);
                self.ch1.freq = (self.ch1.freq & 0x0FF) | ((value & 0x700) & !0);
                // Re-form freq from both bytes
                self.ch1.freq = (value & 0x7FF) as u16;
                self.ch1.length_enable = (value >> 14) & 1 != 0;
                if (value >> 15) & 1 != 0 {
                    self.trigger_ch1();
                }
            }
            // CH2 Duty/Length/Envelope (NR21/NR22)
            0x0400_0068 => {
                self.ch2.length_load = (value & 0x3F) as u8;
                self.ch2.duty = ((value >> 6) & 3) as u8;
                self.ch2.env_period = ((value >> 8) & 7) as u8;
                self.ch2.env_increase = (value >> 11) & 1 != 0;
                self.ch2.env_initial = ((value >> 12) & 0xF) as u8;
                if !self.ch2.is_dac_on() {
                    self.ch2.enabled = false;
                }
            }
            // CH2 Freq/Control (NR23/NR24)
            0x0400_006C => {
                self.ch2.freq = (value & 0x7FF) as u16;
                self.ch2.length_enable = (value >> 14) & 1 != 0;
                if (value >> 15) & 1 != 0 {
                    self.ch2.trigger();
                }
            }
            // CH3 Stop/Wave RAM select (NR30)
            0x0400_0070 => {
                self.ch3.two_banks = (value >> 5) & 1 != 0;
                self.ch3.bank_select = ((value >> 6) & 1) as u8;
                self.ch3.dac_on = (value >> 7) & 1 != 0;
                if !self.ch3.dac_on {
                    self.ch3.enabled = false;
                }
            }
            // CH3 Length/Volume (NR31/NR32)
            0x0400_0072 => {
                self.ch3.length_load = (value & 0xFF) as u16;
                self.ch3.volume_shift = ((value >> 13) & 3) as u8;
                self.ch3.force_volume = (value >> 15) & 1 != 0;
            }
            // CH3 Freq/Control (NR33/NR34)
            0x0400_0074 => {
                self.ch3.freq = (value & 0x7FF) as u16;
                self.ch3.length_enable = (value >> 14) & 1 != 0;
                if (value >> 15) & 1 != 0 {
                    self.ch3.trigger();
                }
            }
            // CH4 Length/Envelope (NR41/NR42)
            0x0400_0078 => {
                self.ch4.length_load = (value & 0x3F) as u8;
                self.ch4.env_period = ((value >> 8) & 7) as u8;
                self.ch4.env_increase = (value >> 11) & 1 != 0;
                self.ch4.env_initial = ((value >> 12) & 0xF) as u8;
                if !self.ch4.is_dac_on() {
                    self.ch4.enabled = false;
                }
            }
            // CH4 Freq/Control (NR43/NR44)
            0x0400_007C => {
                self.ch4.div_ratio = (value & 7) as u8;
                self.ch4.short_mode = (value >> 3) & 1 != 0;
                self.ch4.shift_freq = ((value >> 4) & 0xF) as u8;
                self.ch4.length_enable = (value >> 14) & 1 != 0;
                if (value >> 15) & 1 != 0 {
                    self.ch4.trigger();
                }
            }
            // Control
            0x0400_0080 => self.soundcnt_l = value,
            0x0400_0082 => {
                let old = self.soundcnt_h;
                self.soundcnt_h = value;
                // Reset FIFO A if bit 11 set
                if value & (1 << 11) != 0 {
                    self.fifo_a.reset();
                    self.soundcnt_h &= !(1 << 11);
                }
                // Reset FIFO B if bit 15 set
                if value & (1 << 15) != 0 {
                    self.fifo_b.reset();
                    self.soundcnt_h &= !(1 << 15);
                }
                let _ = old;
            }
            0x0400_0084 => {
                let was_on = self.master_enabled();
                self.soundcnt_x = (value & 0x80) as u8;
                if was_on && !self.master_enabled() {
                    // Power off: reset all PSG registers
                    self.ch1 = SquareChannel::new();
                    self.ch2 = SquareChannel::new();
                    self.ch3 = WaveChannel::new();
                    self.ch4 = NoiseChannel::new();
                    self.soundcnt_l = 0;
                }
            }
            0x0400_0088 => self.soundbias = value,
            // Wave RAM
            0x0400_0090..=0x0400_009E => {
                let off = (addr - 0x0400_0090) as usize;
                self.ch3.write_wave_byte(off, value as u8);
                self.ch3.write_wave_byte(off + 1, (value >> 8) as u8);
            }
            // DMA FIFOs (write-only, 32-bit recommended but handle 16-bit too)
            0x0400_00A0 | 0x0400_00A2 => {
                self.fifo_a.write_halfword(value);
            }
            0x0400_00A4 | 0x0400_00A6 => {
                self.fifo_b.write_halfword(value);
            }
            _ => {}
        }
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        match addr {
            0x0400_00A0 => self.fifo_a.write_word(value),
            0x0400_00A4 => self.fifo_b.write_word(value),
            _ => {
                self.write_16(addr, (value & 0xFFFF) as u16);
                self.write_16(addr + 2, (value >> 16) as u16);
            }
        }
    }

    // CH1 trigger (includes sweep init)

    fn trigger_ch1(&mut self) {
        self.ch1.trigger();
        // Sweep initialisation
        self.shadow_freq = self.ch1.freq;
        let sweep_period = (self.sweep_reg >> 4) & 7;
        let sweep_shift = self.sweep_reg & 7;
        self.sweep_timer = if sweep_period == 0 { 8 } else { sweep_period };
        self.sweep_enabled = sweep_period != 0 || sweep_shift != 0;
        // Perform one overflow check
        if sweep_shift != 0 && self.sweep_calc() > 2047 {
            self.ch1.enabled = false;
        }
    }
}
