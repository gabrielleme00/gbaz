/// Raw cartridge ROM container and metadata hooks.
#[derive(Clone)]
pub struct Cartridge {
    rom: Vec<u8>,
}

impl Cartridge {
    pub fn from_rom(rom: Vec<u8>) -> Self {
        Self { rom }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        self.rom.get(Self::index(addr)).copied().unwrap_or(0xFF)
    }

    pub fn write_8(&mut self, addr: u32, value: u8) {
        eprintln!(
            "Warning: Attempt to write {:02X} to cartridge ROM at address {:08X}",
            value, addr
        );
    }
    pub fn read16(&self, addr: u32) -> u16 {
        let i = Self::index(addr);
        let lo = self.rom.get(i).copied().unwrap_or(0xFF) as u16;
        let hi = self.rom.get(i + 1).copied().unwrap_or(0xFF) as u16;
        lo | (hi << 8)
    }

    pub fn write16(&mut self, _addr: u32, _value: u16) {}

    pub fn read32(&self, addr: u32) -> u32 {
        let i = Self::index(addr);
        let b = |j| self.rom.get(i + j).copied().unwrap_or(0xFF) as u32;
        b(0) | (b(1) << 8) | (b(2) << 16) | (b(3) << 24)
    }

    pub fn write32(&mut self, _addr: u32, _value: u32) {}
    pub fn size(&self) -> usize {
        self.rom.len()
    }

    fn index(addr: u32) -> usize {
        // The same ROM data is mirrored across three wait-state windows (WS0/WS1/WS2).
        // Masking off the upper bits gives the offset within the 32 MB ROM space.
        (addr & 0x01FF_FFFF) as usize
    }
}
