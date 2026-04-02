/// Flash chip variants found in GBA cartridges.
///
/// The 16-bit identification code has MSB=device type, LSB=manufacturer.
/// Address 0xE000000 returns the manufacturer byte, 0xE000001 the device byte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlashChipType {
    /// SST 64K (ID 0xD4BF)
    #[allow(dead_code)] Sst64K,
    /// Macronix 64K (ID 0x1CC2)
    Macronix64K,
    /// Panasonic 64K (ID 0x1B32)
    #[allow(dead_code)] Panasonic64K,
    /// Atmel 64K (ID 0x3D1F)
    #[allow(dead_code)] Atmel64K,
    /// Sanyo 128K (ID 0x1362)
    #[allow(dead_code)] Sanyo128K,
    /// Macronix 128K (ID 0x09C2)
    Macronix128K,
}

impl FlashChipType {
    /// Total size of the flash chip in bytes (64 or 128 KiB).
    pub fn size_bytes(self) -> usize {
        match self {
            Self::Sanyo128K | Self::Macronix128K => 128 * 1024,
            _ => 64 * 1024,
        }
    }

    /// Byte at address offset 0 (manufacturer ID = LSB of 16-bit chip code).
    pub fn manufacturer_id(self) -> u8 {
        let (_, lsb) = self.id_pair();
        lsb
    }

    /// Byte at address offset 1 (device type ID = MSB of 16-bit chip code).
    pub fn device_id(self) -> u8 {
        let (msb, _) = self.id_pair();
        msb
    }

    /// Returns (MSB = device, LSB = manufacturer).
    fn id_pair(self) -> (u8, u8) {
        match self {
            Self::Sst64K => (0xD4, 0xBF),
            Self::Macronix64K => (0x1C, 0xC2),
            Self::Panasonic64K => (0x1B, 0x32),
            Self::Atmel64K => (0x3D, 0x1F),
            Self::Sanyo128K => (0x13, 0x62),
            Self::Macronix128K => (0x09, 0xC2),
        }
    }
}

// Flash ROM state machine

#[derive(Clone, Copy, Debug, PartialEq)]
enum FlashMode {
    /// Normal read-array mode.
    ReadArray,
    /// Chip-identification mode — address 0/1 return manufacturer/device IDs.
    ReadId,
    /// Next write is the data byte to program.
    Writing,
    /// Next write to address 0 selects the 64K bank (128K chips only).
    BankSwitchPending,
    /// Received 0x80 erase-setup command; waiting for the second 3-byte preamble.
    EraseSetup,
}

/// Flash ROM save-backup emulation.
///
/// Implements the command state machine described in GBATek for
/// SST / Macronix / Panasonic / Atmel / Sanyo chips.
#[derive(Clone)]
pub struct FlashRom {
    /// The full contents of the flash chip, including all banks. Unused bytes read as 0xFF.
    pub data: Vec<u8>,
    /// Specific flash chip variant being emulated, which determines size and ID response.
    chip: FlashChipType,
    /// Current mode of the flash chip's command state machine.
    mode: FlashMode,
    /// Position in the current command sequence (0–5).
    seq: u8,
    /// Active 64K bank (0 or 1; only relevant for 128K devices).
    bank: usize,
}

impl FlashRom {
    pub fn new(chip: FlashChipType) -> Self {
        Self {
            data: vec![0xFF; chip.size_bytes()],
            chip,
            mode: FlashMode::ReadArray,
            seq: 0,
            bank: 0,
        }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let offset = (addr & 0xFFFF) as usize;
        if self.mode == FlashMode::ReadId {
            return match offset {
                0 => self.chip.manufacturer_id(),
                1 => self.chip.device_id(),
                _ => 0xFF,
            };
        }
        let idx = self.bank * 0x10000 + offset;
        self.data.get(idx).copied().unwrap_or(0xFF)
    }

    pub fn write_8(&mut self, addr: u32, val: u8) {
        let offset = addr & 0xFFFF;

        // Writing mode: the very next byte programmed goes directly to flash data.
        if self.mode == FlashMode::Writing {
            let idx = self.bank * 0x10000 + offset as usize;
            if let Some(b) = self.data.get_mut(idx) {
                *b &= val; // Flash can only clear bits; erasing sets them back to 1.
            }
            self.mode = FlashMode::ReadArray;
            self.seq = 0;
            return;
        }

        // BankSwitch mode: the next byte written to address 0 selects the 64K bank.
        if self.mode == FlashMode::BankSwitchPending {
            if offset == 0x0000 {
                self.bank = (val & 0x01) as usize;
            }
            self.mode = FlashMode::ReadArray;
            self.seq = 0;
            return;
        }

        // F0h written to 0x5555 terminates any in-progress command (all device types;
        // officially only required for Macronix, but harmless applied universally).
        if offset == 0x5555 && val == 0xF0 {
            self.mode = FlashMode::ReadArray;
            self.seq = 0;
            return;
        }

        self.advance_seq(offset, val);
    }

    /// Advance the command-sequence state machine on a generic write.
    fn advance_seq(&mut self, offset: u32, val: u8) {
        match self.seq {
            // First three-byte preamble (shared by all commands)
            0 => {
                if offset == 0x5555 && val == 0xAA {
                    self.seq = 1;
                }
            }
            1 => {
                if offset == 0x2AAA && val == 0x55 {
                    self.seq = 2;
                } else {
                    self.seq = 0;
                }
            }
            2 => {
                // Command byte must be at 0x5555 for all commands except
                // sector-erase (the address encodes the sector there).
                if offset == 0x5555 {
                    match val {
                        0x90 => {
                            self.mode = FlashMode::ReadId;
                            self.seq = 0;
                        }
                        0xF0 => {
                            self.mode = FlashMode::ReadArray;
                            self.seq = 0;
                        }
                        0x80 => {
                            self.mode = FlashMode::EraseSetup;
                            self.seq = 3;
                        }
                        0xA0 => {
                            self.mode = FlashMode::Writing;
                            self.seq = 0;
                        }
                        0xB0 if self.chip.size_bytes() > 64 * 1024 => {
                            self.mode = FlashMode::BankSwitchPending;
                            self.seq = 0;
                        }
                        _ => {
                            self.seq = 0;
                        }
                    }
                } else {
                    self.seq = 0;
                }
            }
            // Second three-byte preamble (erase sub-command)
            3 => {
                if offset == 0x5555 && val == 0xAA {
                    self.seq = 4;
                } else {
                    self.mode = FlashMode::ReadArray;
                    self.seq = 0;
                }
            }
            4 => {
                if offset == 0x2AAA && val == 0x55 {
                    self.seq = 5;
                } else {
                    self.mode = FlashMode::ReadArray;
                    self.seq = 0;
                }
            }
            5 => {
                match val {
                    0x10 if offset == 0x5555 => {
                        // Erase entire chip — fills all data with 0xFF.
                        self.data.fill(0xFF);
                    }
                    0x30 => {
                        // Erase 4 KiB sector; the sector address is encoded in bits [15:12].
                        let sector_base = (offset & 0xF000) as usize + self.bank * 0x10000;
                        let end = (sector_base + 0x1000).min(self.data.len());
                        if sector_base < self.data.len() {
                            self.data[sector_base..end].fill(0xFF);
                        }
                    }
                    _ => {}
                }
                self.mode = FlashMode::ReadArray;
                self.seq = 0;
            }
            _ => {
                self.mode = FlashMode::ReadArray;
                self.seq = 0;
            }
        }
    }
}
