#[derive(Debug)]
pub enum MemoryRegion {
    Bios,
    Wram,
    Iwram,
    Io,
    Palette,
    Vram,
    Oam,
    Cartridge,
}

impl MemoryRegion {
    pub fn from_addr(addr: u32) -> Option<Self> {
        match addr {
            // General Internal Memory
            0x0000_0000..=0x0000_3FFF => Some(Self::Bios),
            0x0200_0000..=0x0203_FFFF => Some(Self::Wram),
            0x0300_0000..=0x0300_7FFF => Some(Self::Iwram),
            0x0400_0000..=0x04FF_FFFF => Some(Self::Io),
            // Internal Display Memory
            0x0500_0000..=0x0500_03FF => Some(Self::Palette),
            0x0600_0000..=0x0601_7FFF => Some(Self::Vram),
            0x0700_0000..=0x0700_03FF => Some(Self::Oam),
            // External Memory (Game Pak)
            0x0800_0000..=0x09FF_FFFF => Some(Self::Cartridge),
            // TODO: Handle wait states for different regions
            _ => None,
        }
    }
}

pub enum IoRegisterRegion {
    Lcd,
    Sound,
    Dma,
    Timer,
    Keypad,
    Serial,
    Interrupt,
}

impl IoRegisterRegion {
    pub fn from_addr(addr: u32) -> Option<Self> {
        match addr {
            0x0400_0000..=0x4000_0056 => Some(Self::Lcd),
            0x0400_0060..=0x4000_00A8 => Some(Self::Sound),
            0x0400_00B0..=0x4000_00E0 => Some(Self::Dma),
            0x0400_0100..=0x4000_0110 => Some(Self::Timer),
            0x0400_0130..=0x4000_0132 => Some(Self::Keypad),
            0x0400_0120..=0x4000_015A => Some(Self::Serial),
            0x0400_0200..=0x4700_0000 => Some(Self::Interrupt),
            _ => None,
        }
    }
}