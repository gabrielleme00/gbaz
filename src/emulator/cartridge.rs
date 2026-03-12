const CARTRIDGE_BASE_ADDR: u32 = 0x0800_0000;

/// Raw cartridge ROM container and metadata hooks.
#[derive(Clone)]
pub struct Cartridge {
    rom: Vec<u8>,
}

impl Cartridge {
    pub fn from_rom(rom: Vec<u8>) -> Self {
        Self { rom }
    }

    pub fn read8(&self, addr: u32) -> u8 {
        self.rom.get(Self::index(addr)).copied().unwrap_or(0xFF)
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        eprintln!(
            "Warning: Attempt to write {:02X} to cartridge ROM at address {:08X}",
            value, addr
        );
    }

    pub fn size(&self) -> usize {
        self.rom.len()
    }

    fn index(addr: u32) -> usize {
        (addr - CARTRIDGE_BASE_ADDR) as usize
    }
}
