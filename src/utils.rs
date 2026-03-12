pub fn get_lo(value: u16) -> u8 {
    (value & 0xFF) as u8
}

pub fn get_hi(value: u16) -> u8 {
    (value >> 8) as u8
}

pub fn set_lo(original: &mut u16, low: u8) {
    *original = (*original & 0xFF00) | (low as u16);
}

pub fn set_hi(original: &mut u16, high: u8) {
    *original = (*original & 0x00FF) | ((high as u16) << 8);
}