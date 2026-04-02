mod consts {
    // Total serial-bit counts for each valid complete EEPROM command:
    // read-request:  2 cmd + N addr + 1 stop
    // write-request: 2 cmd + N addr + 64 data + 1 stop

    pub const EEPROM_READ_6BIT: usize = 9; // 512 B (6-bit addr)
    pub const EEPROM_READ_14BIT: usize = 17; // 8 KB  (14-bit addr)
    pub const EEPROM_WRITE_6BIT: usize = 73; // 512 B
    pub const EEPROM_WRITE_14BIT: usize = 81; // 8 KB

    pub const EEPROM_512B_SIZE: usize = 512;
    pub const EEPROM_8KB_SIZE: usize = 8 * 1024;
}

use consts::*;

#[derive(Clone, Debug)]
enum EepromState {
    /// Idle, waiting for the start of a command (first bit = 1).
    Idle,
    /// 4 dummy bits followed by 64 data bits (one bit per element, MSB first).
    Reading { response: [u8; 68], pos: u8 },
    /// Write completed; return 1 (ready) on subsequent reads.
    WriteComplete,
}

/// EEPROM backup-storage emulation (512 B or 8 KB).
///
/// Communication is serial: the GBA DMA3 transfers 16-bit halfwords to/from the
/// EEPROM address range, but only bit 0 of each halfword carries data.
/// The chip size (6-bit vs 14-bit address) is auto-detected from the first
/// complete DMA command stream.
#[derive(Clone)]
pub struct EepromStorage {
    pub data: Vec<u8>,
    /// 6 (512 B) or 14 (8 KB). 0 = not yet detected.
    addr_width: u8,
    state: EepromState,
    /// Incoming serial bits buffered during a DMA write sequence.
    rx_bits: Vec<u8>,
}

impl EepromStorage {
    /// Create a new EEPROM storage with the default size (8 KB) and idle state.
    pub fn auto() -> Self {
        Self {
            data: vec![0xFF; EEPROM_8KB_SIZE],
            addr_width: 0,
            state: EepromState::Idle,
            rx_bits: Vec::with_capacity(81),
        }
    }

    /// Called for each 16-bit DMA write to the EEPROM address range.
    /// Only bit 0 of `val` carries serial data.
    pub fn write_half(&mut self, val: u16) {
        self.rx_bits.push((val & 1) as u8);
        self.try_process_command();
    }

    /// Called for each 16-bit DMA read from the EEPROM address range.
    /// Returns the next serial response bit in bit 0.
    pub fn read_half(&mut self) -> u16 {
        match &mut self.state {
            EepromState::Reading { response, pos } => {
                let bit = if (*pos as usize) < 68 {
                    response[*pos as usize] as u16
                } else {
                    0
                };
                *pos += 1;
                if *pos >= 68 {
                    self.state = EepromState::Idle;
                }
                bit
            }
            // 1 = ready (idle or write complete).
            _ => 1,
        }
    }

    /// Process the accumulated bit stream if it forms a complete valid command.
    fn try_process_command(&mut self) {
        let len = self.rx_bits.len();

        let (is_read, addr_w): (bool, u8) = match len {
            EEPROM_READ_6BIT => (true, 6),
            EEPROM_READ_14BIT => (true, 14),
            EEPROM_WRITE_6BIT => (false, 6),
            EEPROM_WRITE_14BIT => (false, 14),
            _ => return,
        };

        let bits = &self.rx_bits;

        // Validate command bits (bits[0]=1 always; bits[1]=1 for read, 0 for write)
        // and stop bit (must be 0).
        let cmd1 = bits[0];
        let cmd2 = bits[1];
        let stop = bits[len - 1];

        let valid = cmd1 == 1 && stop == 0 && (if is_read { cmd2 == 1 } else { cmd2 == 0 });

        if !valid {
            self.rx_bits.clear();
            return;
        }

        // Auto-detect chip size from the first successful command.
        if self.addr_width == 0 {
            self.addr_width = addr_w;
            let target = if addr_w == 6 {
                EEPROM_512B_SIZE
            } else {
                EEPROM_8KB_SIZE
            };
            if self.data.len() < target {
                self.data.resize(target, 0xFF);
            }
        }

        // Decode address from bits [2 .. 2+addr_w), MSB first.
        let mut addr: usize = 0;
        for i in 0..addr_w as usize {
            addr = (addr << 1) | bits[2 + i] as usize;
        }
        // For 14-bit bus, only the lower 10 bits address the 1024 pages.
        let addr = if addr_w == 14 {
            addr & 0x3FF
        } else {
            addr & 0x3F
        };
        let byte_base = addr * 8; // each address unit is one 64-bit (8-byte) page

        if is_read {
            // Build 68-bit response: 4 dummy zero bits then 64 data bits MSB-first.
            let mut response = [0u8; 68];
            for byte_i in 0..8 {
                let b = self.data.get(byte_base + byte_i).copied().unwrap_or(0xFF);
                for bit_i in 0..8 {
                    response[4 + byte_i * 8 + bit_i] = (b >> (7 - bit_i)) & 1;
                }
            }
            self.state = EepromState::Reading { response, pos: 0 };
        } else {
            // Data occupies bits [2 + addr_w .. 2 + addr_w + 64).
            let data_start = 2 + addr_w as usize;
            for byte_i in 0..8 {
                let mut byte = 0u8;
                for bit_i in 0..8 {
                    byte = (byte << 1) | bits[data_start + byte_i * 8 + bit_i];
                }
                if let Some(b) = self.data.get_mut(byte_base + byte_i) {
                    *b = byte;
                }
            }
            self.state = EepromState::WriteComplete;
        }

        self.rx_bits.clear();
    }
}
