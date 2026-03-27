pub trait MemoryInterface {
    /// Read a byte
    fn load_8(&mut self, addr: u32) -> u8;
    /// Read a halfword
    fn load_16(&mut self, addr: u32) -> u16;
    /// Read a word
    fn load_32(&mut self, addr: u32) -> u32;

    /// Write a byte
    fn store_8(&mut self, addr: u32, value: u8);
    /// Write a halfword
    fn store_16(&mut self, addr: u32, value: u16);
    /// Write a word
    fn store_32(&mut self, addr: u32, value: u32);
}