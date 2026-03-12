use super::*;

/// Disassembles one Thumb instruction into a human-readable string.
///
/// `addr` is the address at which the instruction was fetched.
/// `opcode` is the raw 16-bit instruction halfword.
pub fn disasm_thumb(addr: u32, opcode: u16) -> String {
    let mnem = decode_thumb(addr, opcode);
    format!("{addr:08X}: {opcode:04X}      {mnem}")
}

fn decode_thumb(addr: u32, opcode: u16) -> String {
    if is_software_interrupt(opcode) {
        format!("THM  ; {opcode:#06X} (software_interrupt)") // TODO
    } else if is_unconditional_branch(opcode) {
        format_unconditional_branch(opcode, addr)
    } else if is_conditional_branch(opcode) {
        format_conditional_branch(opcode, addr)
    } else if is_multiple_load_store(opcode) {
        format_multiple_load_store(opcode)
    } else if is_long_branch_with_link(opcode) {
        format_long_branch_with_link(opcode)
    } else if is_add_offset_to_stack_pointer(opcode) {
        format_add_offset_to_stack_pointer(opcode)
    } else if is_push_pop_registers(opcode) {
        format_push_pop_registers(opcode)
    } else if is_load_store_halfword(opcode) {
        format_load_store_halfword(opcode)
    } else if is_sp_relative_load_store(opcode) {
        format_sp_relative_load_store(opcode)
    } else if is_load_address(opcode) {
        format_load_address(opcode)
    } else if is_load_store_with_immediate_offset(opcode) {
        format_load_store_with_immediate_offset(opcode)
    } else if is_load_store_with_register_offset(opcode) {
        format_load_store_with_register_offset(opcode)
    } else if is_load_store_with_sign_extend_byte_halfword(opcode) {
        format_load_store_with_sign_extend_byte_halfword(opcode)
    } else if is_pc_relative_load(opcode) {
        format_pc_relative_load(opcode, addr)
    } else if is_hi_register_operations_branch_exchange(opcode) {
        format_hi_register_operations_branch_exchange(opcode)
    } else if is_alu_operations(opcode) {
        format_alu_operations(opcode)
    } else if is_move_compare_add_subtract_immediate(opcode) {
        format_move_compare_add_subtract_immediate(opcode)
    } else if is_add_subtract(opcode) {
        format_add_subtract(opcode)
    } else if is_move_shifted_register(opcode) {
        format_move_shifted_register(opcode)
    } else {
        format!("THM  ; {opcode:#06X}")
    }
}

fn format_unconditional_branch(opcode: u16, addr: u32) -> String {
    // Extract 11 bits and sign-extend
    let mut imm11 = (opcode & 0x07FF) as i32;
    if (imm11 & 0x0400) != 0 {
        imm11 |= !0x7FF; // Sign extend
    }

    let offset = imm11 << 1;
    let target = addr.wrapping_add(4).wrapping_add(offset as u32);

    format!("B {:#010X}", target)
}

fn format_conditional_branch(opcode: u16, addr: u32) -> String {
    let cond = (opcode >> 8) & 0xF;
    let imm8 = (opcode & 0xFF) as i8; // Sign-extend 8-bit immediate
    let offset = (imm8 as i32) << 1;
    let target = addr.wrapping_add(4).wrapping_add(offset as u32);

    let cond_str = match cond {
        0x0 => "EQ",
        0x1 => "NE",
        0x2 => "CS",
        0x3 => "CC",
        0x4 => "MI",
        0x5 => "PL",
        0x6 => "VS",
        0x7 => "VC",
        0x8 => "HI",
        0x9 => "LS",
        0xA => "GE",
        0xB => "LT",
        0xC => "GT",
        0xD => "LE",
        0xE => "UND",
        0xF => "SWI",
        _ => unreachable!(),
    };

    if cond == 0xF {
        format!("SWI #{:#04X}", opcode & 0xFF)
    } else {
        format!("B{cond_str} {:#010X}", target)
    }
}

fn format_multiple_load_store(opcode: u16) -> String {
    let is_load = (opcode >> 11) & 1 == 1;
    let rb = (opcode >> 8) & 0x7;
    let r_list = opcode & 0xFF;

    let mut registers = Vec::new();
    for i in 0..8 {
        if (r_list >> i) & 1 == 1 {
            registers.push(format!("R{i}"));
        }
    }

    let mnem = if is_load { "LDMIA" } else { "STMIA" };
    format!("{mnem} R{rb}!, {{{}}}", registers.join(", "))
}

fn format_long_branch_with_link(opcode: u16) -> String {
    let setup_bit = (opcode >> 11) & 1 == 0; // 11110 = setup, 11111 = complete
    let nn = (opcode & 0x7FF) as u32;

    if setup_bit {
        // First Instruction: Offset bits [22:12]
        // Sign extend 11-bit value to 32 bits
        let mut offset = nn << 12;
        if (offset & 0x0040_0000) != 0 {
            offset |= 0xFF80_0000;
        }
        format!("BL (setup) LR, PC + {:#X}", offset as i32)
    } else {
        // Second Instruction: Offset bits [11:1]
        let offset = nn << 1;
        format!("BL (complete) PC, LR + {:#X}", offset)
    }
}

fn format_add_offset_to_stack_pointer(opcode: u16) -> String {
    let sign_bit = (opcode >> 7) & 1; // 0: Positive, 1: Negative
    let imm7 = (opcode & 0x7F) as u32;
    let val = imm7 << 2; // nn: Unsigned Offset (step 4)

    if sign_bit == 0 {
        format!("ADD SP, #{val}")
    } else {
        format!("ADD SP, #-{val}")
    }
}

fn format_push_pop_registers(opcode: u16) -> String {
    let is_pop = (opcode >> 11) & 1 == 1;
    let extra_bit = (opcode >> 8) & 1 == 1;
    let r_list = opcode & 0xFF;

    let mut registers = Vec::new();
    for i in 0..8 {
        if (r_list >> i) & 1 == 1 {
            registers.push(format!("R{i}"));
        }
    }

    if extra_bit {
        registers.push(if is_pop { "PC" } else { "LR" }.to_string());
    }

    let mnem = if is_pop { "POP" } else { "PUSH" };
    format!("{mnem} {{{}}}", registers.join(", "))
}

fn format_load_store_halfword(opcode: u16) -> String {
    let l_bit = (opcode >> 11) & 1; // 0: STRH, 1: LDRH
    let imm5 = (opcode >> 6) & 0x1F;
    let rb = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    let offset = imm5 << 1; // nn: Unsigned Offset (step 2)
    let mnem = if l_bit == 1 { "LDRH" } else { "STRH" };

    format!("{mnem} R{rd}, [R{rb}, #{offset}]")
}

fn format_sp_relative_load_store(opcode: u16) -> String {
    let l_bit = (opcode >> 11) & 1; // 0: STR, 1: LDR
    let rd = (opcode >> 8) & 0x7;
    let imm8 = (opcode & 0xFF) as u32;
    let offset = imm8 << 2;

    let mnem = if l_bit == 1 { "LDR" } else { "STR" };

    format!("{mnem} R{rd}, [SP, #{offset}]")
}

fn format_load_address(opcode: u16) -> String {
    let sp_bit = (opcode >> 11) & 1;
    let rd_idx = (opcode >> 8) & 0x7;
    let imm8 = (opcode & 0xFF) as u32;
    let val = imm8 << 2;

    let src = if sp_bit == 0 { "PC" } else { "SP" };

    format!("ADD R{rd_idx}, {src}, #{val}")
}

fn format_load_store_with_immediate_offset(opcode: u16) -> String {
    let op = (opcode >> 11) & 0x3;
    let imm5 = (opcode >> 6) & 0x1F;
    let rb = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    match op {
        0 => format!("STR  R{rd}, [R{rb}, #{}]", imm5 << 2),
        1 => format!("LDR  R{rd}, [R{rb}, #{}]", imm5 << 2),
        2 => format!("STRB R{rd}, [R{rb}, #{imm5}]"),
        3 => format!("LDRB R{rd}, [R{rb}, #{imm5}]"),
        _ => unreachable!(),
    }
}

fn format_load_store_with_register_offset(opcode: u16) -> String {
    let op = (opcode >> 10) & 0x3;
    let ro = (opcode >> 6) & 0x7;
    let rb = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    let mnem = match op {
        0 => "STR",
        1 => "STRB",
        2 => "LDR",
        3 => "LDRB",
        _ => unreachable!(),
    };

    format!("{mnem} R{rd}, [R{rb}, R{ro}]")
}

fn format_load_store_with_sign_extend_byte_halfword(opcode: u16) -> String {
    let op = (opcode >> 10) & 0x3;
    let ro = (opcode >> 6) & 0x7;
    let rb = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    let mnem = match op {
        0 => "STRH",
        1 => "LDSB",
        2 => "LDRH",
        3 => "LDSH",
        _ => unreachable!(),
    };

    format!("{mnem} R{rd}, [R{rb}, R{ro}]")
}

fn format_pc_relative_load(opcode: u16, addr: u32) -> String {
    let rd = (opcode >> 8) & 0x7;
    let imm8 = (opcode & 0xFF) as u32;
    let offset = imm8 << 2; // nn: Unsigned offset (step 4)

    // PC is (current_instruction + 4) AND NOT 2
    let pc_base = (addr.wrapping_add(4)) & !2;
    let target = pc_base.wrapping_add(offset);

    format!("LDR R{rd}, [PC, #{offset}] ; = {:#010X}", target)
}

fn format_hi_register_operations_branch_exchange(opcode: u16) -> String {
    let op = (opcode >> 8) & 3;
    let h1 = (opcode >> 7) & 1;
    let h2 = (opcode >> 6) & 1;
    let rs_idx = (((opcode >> 3) & 0x7) | (h2 << 3)) as usize;
    let rd_idx = ((opcode & 0x7) | (h1 << 3)) as usize;

    match op {
        0b00 => format!("ADD R{rd_idx}, R{rs_idx}"),
        0b01 => format!("CMP R{rd_idx}, R{rs_idx}"),
        0b10 => format!("MOV R{rd_idx}, R{rs_idx}"),
        0b11 => format!("BX R{rs_idx}"),
        _ => unreachable!(),
    }
}

fn format_alu_operations(opcode: u16) -> String {
    let op = (opcode >> 6) & 0xF;
    let rs = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    let mnem = match op {
        0x0 => "AND",
        0x1 => "EOR",
        0x2 => "LSL",
        0x3 => "LSR",
        0x4 => "ASR",
        0x5 => "ADC",
        0x6 => "SBC",
        0x7 => "ROR",
        0x8 => "TST",
        0x9 => "NEG",
        0xA => "CMP",
        0xB => "CMN",
        0xC => "ORR",
        0xD => "MUL",
        0xE => "BIC",
        0xF => "MVN",
        _ => unreachable!(),
    };

    format!("{mnem} R{rd}, R{rs}")
}

fn format_move_compare_add_subtract_immediate(opcode: u16) -> String {
    let op = (opcode >> 11) & 0x3;
    let rd_idx = ((opcode >> 8) & 0x7) as usize;
    let imm8 = (opcode & 0xFF) as u32;

    match op {
        0b00 => format!("MOV R{rd_idx}, #{imm8}"),
        0b01 => format!("CMP R{rd_idx}, #{imm8}"),
        0b10 => format!("ADD R{rd_idx}, #{imm8}"),
        0b11 => format!("SUB R{rd_idx}, #{imm8}"),
        _ => unreachable!("THM  ; {opcode:#06X}"),
    }
}

fn format_add_subtract(opcode: u16) -> String {
    let op = (opcode >> 9) & 0x3;
    let rn_imm = (opcode >> 6) & 0x7;
    let rs = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    match op {
        0 => format!("ADD R{rd}, R{rs}, R{rn_imm}"),
        1 => format!("SUB R{rd}, R{rs}, R{rn_imm}"),
        2 => {
            if rn_imm == 0 {
                format!("MOV R{rd}, R{rs}")
            } else {
                format!("ADD R{rd}, R{rs}, #{rn_imm}")
            }
        }
        3 => format!("SUB R{rd}, R{rs}, #{rn_imm}"),
        _ => unreachable!(),
    }
}

fn format_move_shifted_register(opcode: u16) -> String {
    let op = (opcode >> 11) & 0x3;
    let offset = (opcode >> 6) & 0x1F;
    let rs = (opcode >> 3) & 0x7;
    let rd = opcode & 0x7;

    let mnem = match op {
        0b00 => "LSL",
        0b01 => "LSR",
        0b10 => "ASR",
        _ => unreachable!("0b11 is Add/Sub format"),
    };

    format!("{mnem} R{rd}, R{rs}, #{offset}")
}
