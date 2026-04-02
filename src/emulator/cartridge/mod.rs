mod backup;
mod eeprom;
mod flash;

pub use {
    eeprom::EepromStorage,
    flash::{FlashChipType, FlashRom},
};

use backup::{BackupStorage, detect_backup_type};

/// Raw cartridge ROM container and save-backup storage.
#[derive(Clone)]
pub struct Cartridge {
    /// The full ROM binary, including the header and any padding.
    rom: Vec<u8>,
    /// The save-backup storage, if any.
    backup: BackupStorage,
    /// Indicates whether the save data has been modified since the last load/save.
    save_dirty: bool,
}

impl Cartridge {
    /// Creates a Cartridge by scanning the ROM data for backup-type identifiers
    /// and initializing the appropriate backup storage.
    pub fn from_rom(rom: Vec<u8>) -> Self {
        let backup_type = detect_backup_type(&rom);
        let backup = BackupStorage::from_type(backup_type);
        Self {
            rom,
            backup,
            save_dirty: false,
        }
    }

    /// Returns the size of the ROM in bytes.
    #[inline]
    pub fn size(&self) -> usize {
        self.rom.len()
    }

    /// Returns the raw save-backup bytes, or `None` if this cartridge has no backup storage.
    #[inline]
    pub fn save_data(&self) -> Option<Vec<u8>> {
        self.backup.save_data()
    }

    /// Overwrites the backup storage with previously-persisted save data.
    #[inline]
    pub fn load_save_data(&mut self, data: &[u8]) {
        self.backup.load_save_data(data);
        self.save_dirty = false;
    }

    /// Returns true if the cartridge has backup storage and it has been modified since the last load/save.
    #[inline]
    pub fn is_save_dirty(&self) -> bool {
        self.save_dirty
    }

    /// Marks the save data as clean (not dirty).
    #[inline]
    pub fn clear_save_dirty(&mut self) {
        self.save_dirty = false;
    }

    /// Returns true when `addr` maps to the EEPROM serial bus.
    ///
    /// For ROMs ≤16 MB the entire WS2 D-window (`0x0D00_0000–0x0DFF_FFFF`) is
    /// used; for larger ROMs only the top 256 bytes (`0x0DFF_FF00–0x0DFF_FFFF`).
    fn is_eeprom_addr(&self, addr: u32) -> bool {
        if !self.backup.is_eeprom() {
            return false;
        }
        if self.rom.len() <= 16 * 1024 * 1024 {
            (0x0D00_0000..=0x0DFF_FFFF).contains(&addr)
        } else {
            (0x0DFF_FF00..=0x0DFF_FFFF).contains(&addr)
        }
    }

    /// Returns true for the SRAM/Flash backup window (`0x0E000000–0x0E00FFFF`).
    #[inline]
    fn is_backup_addr(addr: u32) -> bool {
        (0x0E00_0000..=0x0E00_FFFF).contains(&addr)
    }

    /// Maps a CPU address in the ROM space to a byte index within the cartridge's ROM data.
    #[inline]
    fn rom_index(addr: u32) -> usize {
        // The same ROM data is mirrored across the three wait-state windows (WS0/WS1/WS2).
        // Masking off the upper bits gives the byte offset within the 32 MB ROM space.
        (addr & 0x01FF_FFFF) as usize
    }
}

impl Cartridge {
    pub fn read_8(&self, addr: u32) -> u8 {
        if Self::is_backup_addr(addr) {
            return self.backup.read_8(addr);
        }
        self.rom.get(Self::rom_index(addr)).copied().unwrap_or(0xFF)
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        if Self::is_backup_addr(addr) {
            self.backup.write_8(addr, value);
            self.save_dirty = true;
        }
    }

    pub fn read_16(&self, addr: u32) -> u16 {
        // EEPROM: serial halfword bus in the WS2 D-window.
        if self.is_eeprom_addr(addr) {
            return self.backup.read_eeprom_half();
        }
        // SRAM/Flash: 8-bit bus — widen byte to both halves.
        if Self::is_backup_addr(addr) {
            let b = self.backup.read_8(addr) as u16;
            return b | (b << 8);
        }
        let i = Self::rom_index(addr);
        let lo = self.rom.get(i).copied().unwrap_or(0xFF) as u16;
        let hi = self.rom.get(i + 1).copied().unwrap_or(0xFF) as u16;
        lo | (hi << 8)
    }

    pub fn write_16(&mut self, addr: u32, value: u16) {
        // EEPROM: interior-mutable, so &self suffices for the state update.
        if self.is_eeprom_addr(addr) {
            self.backup.write_eeprom_half(value);
            self.save_dirty = true;
            return;
        }
        if Self::is_backup_addr(addr) {
            self.backup.write_8(addr, value as u8);
            self.save_dirty = true;
        }
    }

    pub fn read_32(&self, addr: u32) -> u32 {
        if Self::is_backup_addr(addr) {
            let b = self.backup.read_8(addr) as u32;
            return b | (b << 8) | (b << 16) | (b << 24);
        }
        let i = Self::rom_index(addr);
        let b = |j| self.rom.get(i + j).copied().unwrap_or(0xFF) as u32;
        b(0) | (b(1) << 8) | (b(2) << 16) | (b(3) << 24)
    }

    pub fn write_32(&mut self, addr: u32, value: u32) {
        if Self::is_backup_addr(addr) {
            self.backup.write_8(addr, value as u8);
        }
    }
}
