pub mod consts {
    pub const EWRAM_ADDR: u32 = 0x0200_0000;
    pub const IWRAM_ADDR: u32 = 0x0300_0000;
    pub const PRAM_ADDR: u32 = 0x0500_0000;
    pub const VRAM_ADDR: u32 = 0x0600_0000;
}

#[derive(Debug)]
pub enum MemoryRegion {
    Bios,
    Ewram,
    Iwram,
    Io,
    Pram,
    Vram,
    Oam,
    /// ROM wait-state window 0 (0x08000000–0x09FFFFFF)
    CartWs0,
    /// ROM wait-state window 1 (0x0A000000–0x0BFFFFFF)
    CartWs1,
    /// ROM wait-state window 2 (0x0C000000–0x0DFFFFFF)
    CartWs2,
    /// SRAM / Flash save storage (0x0E000000–0x0E00FFFF, 8-bit bus)
    CartSram,
}

impl MemoryRegion {
    pub fn from_addr(addr: u32) -> Option<Self> {
        match addr {
            // General Internal Memory
            0x0000_0000..=0x0000_3FFF => Some(Self::Bios),
            0x0200_0000..=0x0203_FFFF => Some(Self::Ewram),
            0x0300_0000..=0x03FF_FFFF => Some(Self::Iwram),
            0x0400_0000..=0x04FF_FFFF => Some(Self::Io),
            // Internal Display Memory
            0x0500_0000..=0x0500_03FF => Some(Self::Pram),
            0x0600_0000..=0x0601_FFFF => Some(Self::Vram),
            0x0700_0000..=0x0700_03FF => Some(Self::Oam),
            // External Memory (Game Pak) - three wait-state ROM windows plus SRAM
            0x0800_0000..=0x09FF_FFFF => Some(Self::CartWs0),
            0x0A00_0000..=0x0BFF_FFFF => Some(Self::CartWs1),
            0x0C00_0000..=0x0DFF_FFFF => Some(Self::CartWs2),
            0x0E00_0000..=0x0E00_FFFF => Some(Self::CartSram),
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
            0x0400_0000..=0x0400_0056 => Some(Self::Lcd),
            0x0400_0060..=0x0400_00A8 => Some(Self::Sound),
            0x0400_00B0..=0x0400_00E0 => Some(Self::Dma),
            0x0400_0100..=0x0400_0110 => Some(Self::Timer),
            0x0400_0130..=0x0400_0132 => Some(Self::Keypad),
            0x0400_0120..=0x0400_015A => Some(Self::Serial),
            0x0400_0200..=0x0470_0000 => Some(Self::Interrupt),
            _ => None,
        }
    }
}