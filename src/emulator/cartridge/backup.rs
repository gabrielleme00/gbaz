use super::{EepromStorage, FlashChipType, FlashRom};
use std::cell::RefCell;

const SRAM_SIZE: usize = 32 * 1024;

/// Type of save-backup storage embedded in a cartridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackupType {
    None,
    Sram,
    Flash64K,
    Flash128K,
    Eeprom,
}

/// Scan the ROM binary for the Nintendo-standard backup-type identifier strings.
pub(super) fn detect_backup_type(rom: &[u8]) -> BackupType {
    use BackupType::*;

    // Longer patterns must be checked before shorter ones that are prefixes.
    let checklist_map: [(&[u8], BackupType); 5] = [
        (b"FLASH1M_V", Flash128K),
        (b"FLASH512_V", Flash64K),
        (b"FLASH_V", Flash64K),
        (b"SRAM_V", Sram),
        (b"EEPROM_V", Eeprom),
    ];

    // Checks if the given pattern appears anywhere in the ROM.
    let check = |pattern: &[u8]| {
        for w in rom.windows(pattern.len()) {
            if w.starts_with(pattern) {
                return true;
            }
        }
        false
    };

    checklist_map
        .iter()
        .find_map(|(pattern, bt)| check(pattern).then_some(*bt))
        .unwrap_or(None)
}

/// Backup storage for a cartridge, which may be one of several types or none at all.
#[derive(Clone)]
pub(super) enum BackupStorage {
    None,
    /// SRAM is a simple byte array that can be mutated through `&mut self`.
    Sram(Vec<u8>),
    /// Flash ROMs have internal state but are not mutated through `&self`, so `FlashRom` can be used directly.
    Flash(FlashRom),
    /// Interior mutability allows `read_half` to advance state through `&self`.
    Eeprom(RefCell<EepromStorage>),
}

impl BackupStorage {
    pub(super) fn from_type(ty: BackupType) -> Self {
        use BackupType::*;
        use FlashChipType::*;

        match ty {
            Sram => Self::Sram(vec![0xFF; SRAM_SIZE]),
            Flash64K => Self::Flash(FlashRom::new(Macronix64K)),
            Flash128K => Self::Flash(FlashRom::new(Macronix128K)),
            Eeprom => Self::Eeprom(RefCell::new(EepromStorage::auto())),
            None => Self::None,
        }
    }

    pub(super) fn is_eeprom(&self) -> bool {
        matches!(self, Self::Eeprom(_))
    }

    /// 16-bit serial read from EEPROM (bit 0 = next response bit).
    pub(super) fn read_eeprom_half(&self) -> u16 {
        if let Self::Eeprom(e) = self {
            e.borrow_mut().read_half()
        } else {
            1
        }
    }

    /// 16-bit serial write to EEPROM (bit 0 = next request bit).
    pub(super) fn write_eeprom_half(&self, val: u16) {
        if let Self::Eeprom(e) = self {
            e.borrow_mut().write_half(val);
        }
    }

    pub(super) fn read_8(&self, addr: u32) -> u8 {
        let offset = (addr & 0x7FFF) as usize; // SRAM/Flash: 32 KB window
        match self {
            Self::None => 0xFF,
            Self::Sram(data) => data.get(offset).copied().unwrap_or(0xFF),
            Self::Flash(f) => f.read_8(addr),
            Self::Eeprom(_) => 0xFF, // EEPROM uses halfword bus only
        }
    }

    pub(super) fn write_8(&mut self, addr: u32, val: u8) {
        let offset = (addr & 0x7FFF) as usize;
        match self {
            Self::None | Self::Eeprom(_) => {}
            Self::Sram(data) => {
                if let Some(b) = data.get_mut(offset) {
                    *b = val;
                }
            }
            Self::Flash(f) => f.write_8(addr, val),
        }
    }

    /// Returns the raw save bytes for persistence.
    pub(super) fn save_data(&self) -> Option<Vec<u8>> {
        match self {
            Self::None => None,
            Self::Sram(data) => Some(data.clone()),
            Self::Flash(f) => Some(f.data.clone()),
            Self::Eeprom(e) => Some(e.borrow().data.clone()),
        }
    }

    /// Restores backup storage from previously-persisted save bytes.
    pub(super) fn load_save_data(&mut self, data: &[u8]) {
        match self {
            Self::None => {}
            Self::Sram(d) => {
                let len = d.len().min(data.len());
                d[..len].copy_from_slice(&data[..len]);
            }
            Self::Flash(f) => {
                let len = f.data.len().min(data.len());
                f.data[..len].copy_from_slice(&data[..len]);
            }
            Self::Eeprom(e) => {
                let mut e = e.borrow_mut();
                let len = e.data.len().min(data.len());
                e.data[..len].copy_from_slice(&data[..len]);
            }
        }
    }
}
