/// ARM AddWithCarry primitive used by ADD/ADC/SUB/SBC/RSB/RSC/CMP/CMN.
pub fn add_with_carry(x: u32, y: u32, carry_in: bool) -> (u32, bool, bool) {
    let carry = if carry_in { 1u64 } else { 0u64 };
    let sum64 = x as u64 + y as u64 + carry;
    let result = sum64 as u32;
    let carry_out = (sum64 >> 32) != 0;

    // Signed overflow: x and y same sign, result different sign.
    let sx = (x >> 31) & 1;
    let sy = (y >> 31) & 1;
    let sr = (result >> 31) & 1;
    let overflow = (sx == sy) && (sx != sr);

    (result, carry_out, overflow)
}

/// ARM SubWithCarry primitive used by ADD/ADC/SUB/SBC/RSB/RSC/CMP/CMN.
pub fn sub_with_carry(x: u32, y: u32, carry_in: bool) -> (u32, bool, bool) {
    let carry = if carry_in { 1u64 } else { 0u64 };
    let diff64 = x as i64 - y as i64 - (1 - carry) as i64;
    let result = diff64 as u32;
    let carry_out = diff64 >= 0;

    // Signed overflow: x and y different signs, result different sign from x.
    let sx = (x >> 31) & 1;
    let sy = (y >> 31) & 1;
    let sr = (result >> 31) & 1;
    let overflow = (sx != sy) && (sx != sr);

    (result, carry_out, overflow)
}