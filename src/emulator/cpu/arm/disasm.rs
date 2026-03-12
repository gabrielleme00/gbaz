use super::{checks::*, REG_PC};

/// Human-readable suffix for each ARM condition code (AL produces an empty string).
const COND: [&str; 16] = [
    "EQ", "NE", "CS", "CC", "MI", "PL", "VS", "VC", "HI", "LS", "GE", "LT", "GT", "LE", "", "NV",
];

/// Disassembles one ARM instruction into a human-readable string.
///
/// `addr` is the address at which the instruction was fetched.
/// `opcode` is the raw 32-bit instruction word.
pub fn disasm_arm(addr: u32, opcode: u32) -> String {
    let cond = COND[(opcode >> 28) as usize];
    let mnem = decode_arm(addr, opcode, cond);
    format!("{addr:08X}: {opcode:08X}  {mnem}")
}

/// Decodes the ARM instruction and returns a human-readable mnemonic with operands.
///
/// `addr` is the address at which the instruction was fetched.
/// `opcode` is the raw 32-bit instruction word.
/// `c` is the condition code suffix to append to the mnemonic.
fn decode_arm(addr: u32, opcode: u32, c: &str) -> String {
    let key = opcode as usize;

    if is_branch_or_branch_and_exchange(key) {
        format_branch_or_branch_and_exchange(opcode, c)
    } else if is_block_data_transfer(key) {
        format_block_data_transfer(opcode, c)
    } else if is_branch_or_branch_with_link(key) {
        format_branch_or_branch_with_link(opcode, addr, c)
    } else if is_software_interrupt(key) {
        format!("UNK{c}  ; {opcode:#010X}")
    } else if is_undefined(key) {
        format!("UNDF ; {opcode:#010X}")
    } else if is_single_data_transfer(key) {
        format_single_data_transfer(opcode, c)
    } else if is_single_data_swap(key) {
        format_single_data_swap(opcode, c)
    } else if is_multiply(key) {
        format_multiply(opcode, c)
    } else if is_halfword_data_transfer_register(key) {
        format_halfword_data_transfer_register(opcode, c)
    } else if is_halfword_data_transfer_immediate(key) {
        format_halfword_data_transfer_immediate(opcode, c)
    } else if is_psr_transfer_mrs(key) {
        format_psr_transfer_mrs(opcode, c)
    } else if is_psr_transfer_msr(key) {
        format_psr_transfer_msr(opcode, c)
    } else if is_data_processing(key) {
        format_data_processing(opcode, c)
    } else {
        format!("UNK{c}  ; {opcode:#010X}")
    }
}

fn format_operand2(opcode: u32) -> String {
    let is_immediate = ((opcode >> 25) & 1) != 0;
    if is_immediate {
        let imm8 = opcode & 0xFF;
        let rot = ((opcode >> 8) & 0xF) * 2;
        let value = imm8.rotate_right(rot);
        return format!("#0x{value:X}");
    }

    let rm = (opcode & 0xF) as usize;
    let shift_type = (opcode >> 5) & 0x3;
    let shift_by_register = ((opcode >> 4) & 1) != 0;
    let shift_mnem = match shift_type {
        0 => "LSL",
        1 => "LSR",
        2 => "ASR",
        _ => "ROR",
    };

    if shift_by_register {
        let rs = ((opcode >> 8) & 0xF) as usize;
        format!("R{rm}, {shift_mnem} R{rs}")
    } else {
        let imm5 = (opcode >> 7) & 0x1F;
        match shift_type {
            0 if imm5 == 0 => format!("R{rm}"),
            1 if imm5 == 0 => format!("R{rm}, LSR #32"),
            2 if imm5 == 0 => format!("R{rm}, ASR #32"),
            3 if imm5 == 0 => format!("R{rm}, RRX"),
            _ => format!("R{rm}, {shift_mnem} #{imm5}"),
        }
    }
}

fn format_branch_or_branch_and_exchange(opcode: u32, c: &str) -> String {
    let link = (opcode >> 24) & 1 != 0;
    let exchange = (opcode >> 4) & 1 != 0;
    if exchange {
        format!("BX{c} R{}", opcode & 0xF)
    } else {
        let mnem = if link { "BLX" } else { "BX" };
        format!("{mnem}{c} R{}", opcode & 0xF)
    }
}

fn format_block_data_transfer(opcode: u32, c: &str) -> String {
    let p_bit = (opcode >> 24) & 1 != 0;
    let u_bit = (opcode >> 23) & 1 != 0;
    let s_bit = (opcode >> 22) & 1 != 0;
    let w_bit = (opcode >> 21) & 1 != 0;
    let l_bit = (opcode >> 20) & 1 != 0;
    let rn = (opcode >> 16) & 0xF;
    let reg_list = opcode & 0xFFFF;

    let (mnem, stack_alias) = match (l_bit, p_bit, u_bit) {
        (true, false, true) => ("LDMIA", "LDMFD"), // Pop
        (true, true, true) => ("LDMIB", "LDMED"),
        (true, false, false) => ("LDMDA", "LDMFA"),
        (true, true, false) => ("LDMDB", "LDMEA"),
        (false, false, true) => ("STMIA", "STMEA"),
        (false, true, true) => ("STMIB", "STMFA"),
        (false, false, false) => ("STMDA", "STMED"),
        (false, true, false) => ("STMDB", "STMFD"), // Push
    };

    // Usually, debuggers prioritize the Stack Alias if Rn is R13 (SP)
    let final_mnem = if rn == 13 { stack_alias } else { mnem };

    let wb = if w_bit { "!" } else { "" };
    let hat = if s_bit { "^" } else { "" };

    // Register list formatting (with range optimization)
    let regs = format_reg_list(reg_list);

    format!("{final_mnem}{c} R{rn}{wb}, {{{regs}}}{hat}")
}

fn format_reg_list(reg_list: u32) -> String {
    if reg_list == 0 {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut i = 0;
    while i < 16 {
        if (reg_list >> i) & 1 != 0 {
            let start = i;
            while i < 15 && (reg_list >> (i + 1)) & 1 != 0 {
                i += 1;
            }
            if i == start {
                parts.push(format!("R{i}"));
            } else {
                parts.push(format!("R{start}-R{i}"));
            }
        }
        i += 1;
    }
    parts.join(", ")
}

fn format_branch_or_branch_with_link(opcode: u32, addr: u32, c: &str) -> String {
    let link = (opcode >> 24) & 1 != 0;
    let imm24 = opcode & 0x00FF_FFFF;
    // Reconstruct offset: shift up to get sign bit at 31, then arithmetic shift back.
    let offset = (((imm24 << 2) as i32) << 6) >> 6;
    let target = (addr as i64 + 8 + offset as i64) as u32;
    let mnem = if link { "BL" } else { "B" };
    format!("{mnem}{c}  #0x{target:08X}")
}

fn format_single_data_transfer(opcode: u32, c: &str) -> String {
    let i_bit = (opcode >> 25) & 1 != 0;
    let p_bit = (opcode >> 24) & 1 != 0;
    let u_bit = (opcode >> 23) & 1 != 0;
    let b_bit = (opcode >> 22) & 1 != 0;
    let w_bit = (opcode >> 21) & 1 != 0;
    let l_bit = (opcode >> 20) & 1 != 0;

    let rn = (opcode >> 16) & 0xF;
    let rd = (opcode >> 12) & 0xF;

    // Handle the PLD exception (Special case for ARMv5TE, but good for completeness)
    if (opcode >> 28) == 0xF {
        return format!("PLD [R{rn}, {}]", format_offset(opcode, i_bit, u_bit));
    }

    let mnem = if l_bit { "LDR" } else { "STR" };
    let b = if b_bit { "B" } else { "" };
    let t = if !p_bit && w_bit { "T" } else { "" }; // T bit only exists in post-indexing

    let offset_str = format_offset(opcode, i_bit, u_bit);

    if p_bit {
        // Pre-indexed: [Rn, offset] or [Rn, offset]!
        let writeback = if w_bit { "!" } else { "" };
        format!("{mnem}{c}{b}{t} R{rd}, [R{rn}, {offset_str}]{writeback}")
    } else {
        // Post-indexed: [Rn], offset (Write-back is always implicit)
        format!("{mnem}{c}{b}{t} R{rd}, [R{rn}], {offset_str}")
    }
}

/// Helper to format the offset part (Immediate or Shifted Register)
fn format_offset(opcode: u32, i_bit: bool, u_bit: bool) -> String {
    let sign = if u_bit { "" } else { "-" };

    if !i_bit {
        // Immediate Offset
        let imm = opcode & 0xFFF;
        format!("#{}{}", sign, imm)
    } else {
        // Register Offset
        let rm = opcode & 0xF;
        let shift_imm = (opcode >> 7) & 0x1F;
        let shift_type = (opcode >> 5) & 0x3;

        let shift_str = match (shift_type, shift_imm) {
            (0, 0) => String::new(), // LSL #0 is just the register
            (0, _) => format!(", LSL #{}", shift_imm),
            (1, 0) => ", LSR #32".to_string(),
            (1, _) => format!(", LSR #{}", shift_imm),
            (2, 0) => ", ASR #32".to_string(),
            (2, _) => format!(", ASR #{}", shift_imm),
            (3, 0) => ", RRX".to_string(),
            (3, _) => format!(", ROR #{}", shift_imm),
            _ => unreachable!(),
        };
        format!("{}R{}{}", sign, rm, shift_str)
    }
}

fn format_single_data_swap(opcode: u32, c: &str) -> String {
    let b_bit = (opcode >> 22) & 1 != 0;
    let rn = (opcode >> 16) & 0xF;
    let rd = (opcode >> 12) & 0xF;
    let rm = opcode & 0xF;

    let mnem = if b_bit { "SWPB" } else { "SWP" };
    format!("{mnem}{c} R{rd}, R{rm}, [R{rn}]")
}

fn format_multiply(opcode: u32, c: &str) -> String {
    let op_bits = (opcode >> 21) & 0xF;
    let s_bit = ((opcode >> 20) & 1) != 0;
    let s = if s_bit { "S" } else { "" };

    let rd = (opcode >> 16) & 0xF;
    let rn = (opcode >> 12) & 0xF;
    let rs = (opcode >> 8) & 0xF;
    let rm = opcode & 0xF;

    // Bit 7 and 4 are the "Halfword" discriminators
    let bit7 = (opcode >> 7) & 1;
    let bit4 = (opcode >> 4) & 1;
    let is_halfword = bit7 == 1 && bit4 == 0;

    match op_bits {
        // Simple Multiplies
        0b0000 => format!("MUL{c}{s} R{rd}, R{rm}, R{rs}"),
        0b0001 => format!("MLA{c}{s} R{rd}, R{rm}, R{rs}, R{rn}"),

        // Long Multiplies (64-bit results)
        0b0010 => format!("UMAAL{c} R{rn}, R{rd}, R{rm}, R{rs}"),
        0b0100 => format!("UMULL{c}{s} R{rn}, R{rd}, R{rm}, R{rs}"),
        0b0101 => format!("UMLAL{c}{s} R{rn}, R{rd}, R{rm}, R{rs}"),
        0b0110 => format!("SMULL{c}{s} R{rn}, R{rd}, R{rm}, R{rs}"),
        0b0111 => format!("SMLAL{c}{s} R{rn}, R{rd}, R{rm}, R{rs}"),

        // Halfword Multiplies (ARMv5TE / GBA internal)
        0b1000..=0b1011 if is_halfword => {
            let x = if (opcode >> 5) & 1 == 1 { "T" } else { "B" };
            let y = if (opcode >> 6) & 1 == 1 { "T" } else { "B" };

            match op_bits {
                0b1000 => format!("SMLA{x}{y}{c} R{rd}, R{rm}, R{rs}, R{rn}"),
                0b1001 => {
                    if (opcode >> 5) & 1 == 1 {
                        format!("SMULW{y}{c} R{rd}, R{rm}, R{rs}")
                    } else {
                        format!("SMLAW{y}{c} R{rd}, R{rm}, R{rs}, R{rn}")
                    }
                }
                0b1010 => format!("SMLAL{x}{y}{c} R{rn}, R{rd}, R{rm}, R{rs}"),
                0b1011 => format!("SMUL{x}{y}{c} R{rd}, R{rm}, R{rs}"),
                _ => format!("MUL_UNK{c} {opcode:08X}"),
            }
        }

        _ => format!("MUL_UNK{c} {opcode:08X}"),
    }
}

fn format_halfword_data_transfer_register(opcode: u32, c: &str) -> String {
    let p = ((opcode >> 24) & 1) != 0;
    let w = ((opcode >> 21) & 1) != 0;
    let l = ((opcode >> 20) & 1) != 0;
    let rn = ((opcode >> 16) & 0xF) as usize;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let op = (opcode >> 5) & 0x3;
    let rm = (opcode & 0xF) as usize;

    let mnem = match (l, op) {
        (false, 0b01) => "STRH",
        (true, 0b01) => "LDRH",
        (true, 0b10) => "LDRSB",
        (true, 0b11) => "LDRSH",
        _ => return format!("UNK{c}  ; {opcode:#010X}"),
    };

    let addr = if p {
        // Pre-indexed: [Rn, +/-Rm]{!}
        if rm == REG_PC {
            format!("[R{rn}, R{rm}]") // PC can't be used with write-back in pre-indexed mode
        } else if w {
            format!("[R{rn}, R{rm}]!")
        } else {
            format!("[R{rn}, R{rm}]")
        }
    } else if rm == REG_PC {
        format!("[R{rn}], #0") // PC can't be used as offset in post-indexed mode
    } else {
        format!("[R{rn}], R{rm}")
    };

    format!("{mnem}{c}  R{rd}, {addr}")
}

fn format_halfword_data_transfer_immediate(opcode: u32, c: &str) -> String {
    let p = ((opcode >> 24) & 1) != 0;
    let u = ((opcode >> 23) & 1) != 0;
    let w = ((opcode >> 21) & 1) != 0;
    let l = ((opcode >> 20) & 1) != 0;
    let rn = ((opcode >> 16) & 0xF) as usize;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let op = (opcode >> 5) & 0x3;
    let offset = (((opcode >> 8) & 0xF) << 4) | (opcode & 0xF);

    let mnem = match (l, op) {
        (false, 0b01) => "STRH",
        (true, 0b01) => "LDRH",
        (true, 0b10) => "LDRSB",
        (true, 0b11) => "LDRSH",
        _ => return format!("UNK{c}  ; {opcode:#010X}"),
    };

    let addr = if p {
        // Pre-indexed: [Rn, +/-#imm]{!}
        if offset == 0 {
            if w {
                format!("[R{rn}]!")
            } else {
                format!("[R{rn}]")
            }
        } else {
            let sign = if u { "" } else { "-" };
            if w {
                format!("[R{rn}, {sign}#0x{offset:X}]!")
            } else {
                format!("[R{rn}, {sign}#0x{offset:X}]")
            }
        }
    } else if offset == 0 {
        // Post-indexed: [Rn], +/-#imm
        format!("[R{rn}], #0")
    } else {
        let sign = if u { "" } else { "-" };
        format!("[R{rn}], {sign}#0x{offset:X}")
    };

    format!("{mnem}{c}  R{rd}, {addr}")
}

fn format_psr_transfer_mrs(opcode: u32, c: &str) -> String {
    let rd = ((opcode >> 12) & 0xF) as usize;
    let use_spsr = ((opcode >> 22) & 1) != 0;
    let psr = if use_spsr { "SPSR" } else { "CPSR" };
    format!("MRS{c}  R{rd}, {psr}")
}

fn format_psr_transfer_msr(opcode: u32, c: &str) -> String {
    let is_immediate = ((opcode >> 25) & 1) != 0;
    let use_spsr = ((opcode >> 22) & 1) != 0;
    let field_mask = (opcode >> 16) & 0xF;

    let mut fields = String::new();
    if (field_mask & 0x1) != 0 {
        fields.push('c');
    }
    if (field_mask & 0x2) != 0 {
        fields.push('x');
    }
    if (field_mask & 0x4) != 0 {
        fields.push('s');
    }
    if (field_mask & 0x8) != 0 {
        fields.push('f');
    }

    if fields.is_empty() {
        return format!("UNK{c}  ; {opcode:#010X}");
    }

    let psr = if use_spsr { "SPSR" } else { "CPSR" };
    let operand = if is_immediate {
        let imm8 = opcode & 0xFF;
        let rot = ((opcode >> 8) & 0xF) * 2;
        let value = imm8.rotate_right(rot);
        format!("#0x{value:X}")
    } else {
        let rm = (opcode & 0xF) as usize;
        format!("R{rm}")
    };

    format!("MSR{c}  {psr}_{fields}, {operand}")
}

fn format_data_processing(opcode: u32, c: &str) -> String {
    const ALU_OPS: [&str; 16] = [
        "AND", "EOR", "SUB", "RSB", "ADD", "ADC", "SBC", "RSC", "TST", "TEQ", "CMP", "CMN", "ORR",
        "MOV", "BIC", "MVN",
    ];

    let op = ((opcode >> 21) & 0xF) as usize;
    let s_bit = ((opcode >> 20) & 1) != 0;
    let rn = ((opcode >> 16) & 0xF) as usize;
    let rd = ((opcode >> 12) & 0xF) as usize;
    let operand2 = format_operand2(opcode);

    let test_op = matches!(op, 8..=11);
    let show_s = s_bit && !test_op;
    let sfx = if show_s { "S" } else { "" };
    let op_name = ALU_OPS[op];

    if test_op {
        format!("{op_name}{c}  R{rn}, {operand2}")
    } else if op == 13 || op == 15 {
        // MOV / MVN
        format!("{op_name}{c}{sfx}  R{rd}, {operand2}")
    } else {
        format!("{op_name}{c}{sfx}  R{rd}, R{rn}, {operand2}")
    }
}
