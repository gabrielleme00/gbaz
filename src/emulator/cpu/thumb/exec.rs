pub mod handlers {
    use core::panic;

    use super::super::*;

    /// Fallback handler for opcodes that have not been implemented yet.
    pub fn thumb_unknown(_cpu: &mut Cpu, opcode: u16) -> u32 {
        panic!("Unknown Thumb opcode: {opcode:#06X} | {opcode:#018b}");
    }

    /// Handler for the SWI (Software Interrupt) instruction, which triggers a software interrupt.
    pub fn thumb_software_interrupt(cpu: &mut Cpu, _opcode: u16) -> u32 {
        cpu.enter_svc()
    }

    /// Handler for the B (Unconditional Branch) instruction, which performs an unconditional
    /// jump to a target address.
    pub fn thumb_unconditional_branch(cpu: &mut Cpu, opcode: u16) -> u32 {
        // Extract 11-bit immediate
        let mut imm11 = (opcode & 0x07FF) as i32;

        // Sign extend from bit 10 to 32 bits
        if (imm11 & 0x0400) != 0 {
            imm11 |= !0x7FF;
        }

        let offset = (imm11 << 1) as u32;

        // PC in Thumb state is Current Address + 4
        let current_pc = cpu.reg(15);
        let target = current_pc.wrapping_add(offset);

        // Perform the jump
        cpu.branch_to(target);

        0 // fetch cost comes from refill_pipeline via branch_to
    }

    /// Handler for the B<cond> (Conditional Branch) instruction, which conditionally
    /// branches based on the specified condition code.
    pub fn thumb_conditional_branch(cpu: &mut Cpu, opcode: u16) -> u32 {
        let cond = (opcode >> 8) & 0xF;

        if cpu.check_condition(cond.into()) {
            let imm8 = (opcode & 0xFF) as i8;
            let offset = (imm8 as i32) << 1;

            // Target = Current PC (PC+2) + 2 + offset = PC + 4 + offset
            let current_pc = cpu.reg(15);
            let target = current_pc.wrapping_add(offset as u32);

            cpu.branch_to(target);

            0 // fetch cost comes from refill_pipeline
        } else {
            0 // not taken: only the S fetch (from advance_pipeline) counts
        }
    }

    /// Handler for the LDMIA/STMIA (Multiple Load/Store) instruction, which loads or stores
    pub fn thumb_multiple_load_store(cpu: &mut Cpu, opcode: u16) -> u32 {
        let is_load = (opcode >> 11) & 1 == 1;
        let rb_idx = ((opcode >> 8) & 0x7) as usize;
        let r_list = opcode & 0xFF;

        let mut addr = cpu.reg(rb_idx);
        let mut num_registers = 0;

        // ARM7TDMI quirk: empty rlist loads/stores R15 and increments Rb by 0x40
        if r_list == 0 {
            if is_load {
                let val = cpu.bus.borrow().read32(addr & !3);
                cpu.set_reg(rb_idx, addr.wrapping_add(0x40));
                cpu.branch_to(val);
            } else {
                // Stores fetch PC (exec_addr + 6 = pc_reg + 2) (quirk)
                let val = cpu.reg(15).wrapping_add(2);
                cpu.bus.borrow_mut().write32(addr & !3, val);
                cpu.set_reg(rb_idx, addr.wrapping_add(0x40));
            }
            return 1;
        }

        if is_load {
            // LDMIA
            for i in 0..8 {
                if (r_list >> i) & 1 == 1 {
                    let val = cpu.bus.borrow().read32(addr & !3);
                    cpu.set_reg(i, val);
                    addr = addr.wrapping_add(4);
                    num_registers += 1;
                }
            }
            // Writeback: update base register
            cpu.set_reg(rb_idx, addr);
        } else {
            // STMIA
            // ARM7TDMI quirk: if Rb is in rlist but NOT the lowest register,
            // the writeback value (addr + count*4) is stored instead of the original.
            let rb_in_rlist = (r_list >> rb_idx) & 1 == 1;
            let lower_bits = if rb_idx == 0 {
                0
            } else {
                r_list & ((1 << rb_idx) - 1)
            };
            let rb_is_lowest_in_rlist = rb_in_rlist && lower_bits == 0;
            let writeback_addr = addr.wrapping_add(u32::from(r_list.count_ones()) * 4);

            for i in 0..8 {
                if (r_list >> i) & 1 == 1 {
                    // For the register being stored, if it's Rb and Rb is in the list
                    // but not the lowest, use the writeback address instead of the original value
                    let val = if i == rb_idx && rb_in_rlist && !rb_is_lowest_in_rlist {
                        writeback_addr
                    } else {
                        cpu.reg(i)
                    };
                    cpu.bus.borrow_mut().write32(addr & !3, val);
                    addr = addr.wrapping_add(4);
                    num_registers += 1;
                }
            }
            // Writeback: update base register
            cpu.set_reg(rb_idx, addr);
        }
        num_registers
    }

    /// Handler for the BL (Long Branch with Link) instruction, which performs a long branch
    pub fn thumb_long_branch_with_link(cpu: &mut Cpu, opcode: u16) -> u32 {
        let setup_bit = (opcode >> 11) & 1 == 0;
        let nn = (opcode & 0x7FF) as u32;

        if setup_bit {
            // First Instruction
            // Sign-extend the 11-bit immediate and shift it to the upper bits
            let mut offset = nn << 12;
            if (nn & 0x400) != 0 {
                offset |= 0xFF80_0000;
            }

            // LR = PC + Offset
            // Note: cpu.reg(15) is already Instruction Address + 4
            let res = cpu.reg(15).wrapping_add(offset);
            cpu.set_reg(14, res);

            1 // 1S cycle
        } else {
            // Second Instruction
            // Final Address = LR + (nn << 1)
            let offset = nn << 1;
            let lr = cpu.reg(14);
            let target = lr.wrapping_add(offset);

            // LR = Return Address (Address of this instruction + 2) | 1
            // Note: Current PC (cpu.reg(15)) is Address of this instruction + 2
            let return_addr = (cpu.reg(15) - 2) | 1;
            cpu.set_reg(14, return_addr);

            // Jump to target and flush pipeline
            cpu.branch_to(target);

            0 // fetch cost comes from refill_pipeline via branch_to
        }
    }

    /// Handler for the ADD/SUB SP, #imm instruction, which adds or subtracts an immediate
    /// value to/from the stack pointer (SP).
    pub fn thumb_add_offset_to_stack_pointer(cpu: &mut Cpu, opcode: u16) -> u32 {
        let sign_bit = (opcode >> 7) & 1;
        let imm7 = (opcode & 0x7F) as u32;
        let val = imm7 << 2; // Unsigned Offset (0-508, step 4)

        let current_sp = cpu.reg(13); // R13 is SP

        let new_sp = if sign_bit == 0 {
            current_sp.wrapping_add(val)
        } else {
            current_sp.wrapping_sub(val)
        };

        cpu.set_reg(13, new_sp);

        0 // pure ALU, no extra cycles beyond the S fetch
    }

    /// Handler for the PUSH/POP instruction, which pushes or pops multiple registers to/from the stack.
    pub fn thumb_push_pop_registers(cpu: &mut Cpu, opcode: u16) -> u32 {
        let is_pop = (opcode >> 11) & 1 == 1;
        let extra_bit = (opcode >> 8) & 1 == 1;
        let r_list = opcode & 0xFF;

        let mut sp = cpu.reg(13);
        let mut registers = Vec::new();

        // Collect registers in the list
        for i in 0..8 {
            if (r_list >> i) & 1 == 1 {
                registers.push(i);
            }
        }

        if is_pop {
            // POP (LDMIA style
            for &reg_idx in &registers {
                let val = cpu.bus.borrow().read32(sp);
                cpu.set_reg(reg_idx, val);
                sp = sp.wrapping_add(4);
            }

            if extra_bit {
                let mut target_pc = cpu.bus.borrow().read32(sp);
                // GBA/ARM7TDMI Quirk: POP {PC} forces bit 0 to 0 but stays in Thumb
                target_pc &= !1;
                cpu.branch_to(target_pc);
                sp = sp.wrapping_add(4);
            }
        } else {
            // PUSH (STMDB style)
            // PUSH stores the extra register (LR) first in the memory layout
            let total_regs = registers.len() + (if extra_bit { 1 } else { 0 });
            let mut start_addr = sp.wrapping_sub((total_regs * 4) as u32);
            let final_sp = start_addr;

            for &reg_idx in &registers {
                cpu.bus.borrow_mut().write32(start_addr, cpu.reg(reg_idx));
                start_addr = start_addr.wrapping_add(4);
            }

            if extra_bit {
                cpu.bus.borrow_mut().write32(start_addr, cpu.reg(14)); // Push LR
            }

            sp = final_sp;
        }

        cpu.set_reg(13, sp); // Update SP

        1 // Cycles vary based on register count
    }

    /// Handler for the Load/Store Halfword instructions (LDRH/STRH).
    /// These instructions support both aligned and misaligned accesses.
    pub fn thumb_load_store_halfword(cpu: &mut Cpu, opcode: u16) -> u32 {
        let l_bit = (opcode >> 11) & 1 == 1;
        let imm5 = (opcode >> 6) & 0x1F;
        let rb_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let offset = (imm5 << 1) as u32;
        let addr = cpu.reg(rb_idx).wrapping_add(offset);

        if l_bit {
            // LDRH
            let val = if (addr & 1) == 0 {
                // Aligned read
                cpu.bus.borrow().read16(addr) as u32
            } else {
                // Misaligned LDRH: ARM7TDMI rotates the full 32-bit word into Rd (no masking).
                let word = cpu.bus.borrow().read32(addr & !3);
                let rotation = (addr & 3) * 8;
                word.rotate_right(rotation)
            };
            cpu.set_reg(rd_idx, val);
        } else {
            // STRH
            let val = cpu.reg(rd_idx) as u16;
            // Hardware masks bit 0 for halfword stores
            cpu.bus.borrow_mut().write16(addr & !1, val);
        }

        1 // Base cycle count; Bus timing (N/S/I) handled by Bus/System
    }

    /// Handler for the SP-relative Load/Store instructions (LDR/STR with SP-relative addressing).
    pub fn thumb_sp_relative_load_store(cpu: &mut Cpu, opcode: u16) -> u32 {
        let l_bit = (opcode >> 11) & 1 == 1;
        let rd_idx = ((opcode >> 8) & 0x7) as usize;
        let imm8 = (opcode & 0xFF) as u32;
        let offset = imm8 << 2;

        let addr = cpu.reg(13).wrapping_add(offset); // R13 is SP

        if l_bit {
            // LDR (Word)
            let val = if (addr & 3) == 0 {
                cpu.bus.borrow().read32(addr)
            } else {
                // Misaligned LDR: Rotate Right quirk
                let raw = cpu.bus.borrow().read32(addr & !3);
                raw.rotate_right((addr & 3) * 8)
            };
            cpu.set_reg(rd_idx, val);
        } else {
            // STR (Word)
            let val = cpu.reg(rd_idx);
            cpu.bus.borrow_mut().write32(addr & !3, val);
        }

        1 // Base cycle count
    }

    /// Handler for the Load Address instruction, which computes an address based on PC or SP
    /// and an immediate offset, and stores it in a register.
    pub fn thumb_load_address(cpu: &mut Cpu, opcode: u16) -> u32 {
        let sp_bit = (opcode >> 11) & 1; // 0: PC, 1: SP
        let rd_idx = ((opcode >> 8) & 0x7) as usize;
        let imm8 = (opcode & 0xFF) as u32;
        let val = imm8 << 2; // nn: Unsigned Offset (step 4)

        let base = if sp_bit == 0 {
            cpu.reg(15) & !2 // PC with bit 1 cleared
        } else {
            cpu.reg(13) // SP
        };

        cpu.set_reg(rd_idx, base.wrapping_add(val));
        cpu.set_nz_from_u32(base.wrapping_add(val));
        // This instruction does not affect Carry or Overflow

        0 // pure ALU, no extra cycles
    }

    /// Handler for the Load/Store with Immediate Offset instructions (LDR/STR/LDRB/STRB with immediate offset).
    pub fn thumb_load_store_with_immediate_offset(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 11) & 0x3;
        let imm5 = ((opcode >> 6) & 0x1F) as u32;
        let rb_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let rb_val = cpu.reg(rb_idx);

        match op {
            0 => {
                // STR (Word) - Offset scaled by 4
                let addr = rb_val.wrapping_add(imm5 << 2);
                let val = cpu.reg(rd_idx);
                cpu.bus.borrow_mut().write32(addr & !3, val);
            }
            1 => {
                // LDR (Word) - Offset scaled by 4
                let addr = rb_val.wrapping_add(imm5 << 2);
                let val = if (addr & 3) == 0 {
                    cpu.bus.borrow().read32(addr)
                } else {
                    // Misaligned LDR: Rotate Right quirk
                    let raw = cpu.bus.borrow().read32(addr & !3);
                    raw.rotate_right((addr & 3) * 8)
                };
                cpu.set_reg(rd_idx, val);
            }
            2 => {
                // STRB (Byte) - Offset scaled by 1
                let addr = rb_val.wrapping_add(imm5);
                let val = cpu.reg(rd_idx) as u8;
                cpu.bus.borrow_mut().write8(addr, val);
            }
            3 => {
                // LDRB (Byte) - Offset scaled by 1
                let addr = rb_val.wrapping_add(imm5);
                let val = cpu.bus.borrow().read8(addr) as u32;
                cpu.set_reg(rd_idx, val);
            }
            _ => unreachable!(),
        }

        1 // Base cycle count
    }

    /// Handler for the Load/Store with Register Offset instructions (LDR/STR with register offset).
    pub fn thumb_load_store_with_register_offset(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 10) & 0x3;
        let ro_idx = ((opcode >> 6) & 0x7) as usize;
        let rb_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let addr = cpu.reg(rb_idx).wrapping_add(cpu.reg(ro_idx));

        match op {
            0 => {
                // STR (Word)
                let val = cpu.reg(rd_idx);
                cpu.bus.borrow_mut().write32(addr & !3, val);
            }
            1 => {
                // STRB (Byte)
                let val = cpu.reg(rd_idx) as u8;
                cpu.bus.borrow_mut().write8(addr, val);
            }
            2 => {
                // LDR (Word)
                let val = if (addr & 3) == 0 {
                    cpu.bus.borrow().read32(addr)
                } else {
                    // Misaligned LDR: Rotate Right quirk
                    let raw = cpu.bus.borrow().read32(addr & !3);
                    raw.rotate_right((addr & 3) * 8)
                };
                cpu.set_reg(rd_idx, val);
            }
            3 => {
                // LDRB (Byte)
                let val = cpu.bus.borrow().read8(addr) as u32;
                cpu.set_reg(rd_idx, val);
            }
            _ => unreachable!(),
        }

        // Timing varies: LDR is 1S+1N+1I, STR is 2N
        1
    }

    /// Handler for the Load/Store with Sign-Extend Byte/Halfword instructions (LDSB/LDSH/STRH).
    pub fn thumb_load_store_with_sign_extend_byte_halfword(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 10) & 0x3;
        let ro_idx = ((opcode >> 6) & 0x7) as usize;
        let rb_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let addr = cpu.reg(rb_idx).wrapping_add(cpu.reg(ro_idx));

        match op {
            0 => {
                // STRH (Store Halfword)
                let val = cpu.reg(rd_idx) as u16;
                cpu.bus.borrow_mut().write16(addr & !1, val);
            }
            1 => {
                // LDSB (Load Sign-extended Byte)
                let val = cpu.bus.borrow().read8(addr) as i8 as i32 as u32;
                cpu.set_reg(rd_idx, val);
            }
            2 => {
                // LDRH (Load Zero-extended Halfword)
                let val = if (addr & 1) == 0 {
                    cpu.bus.borrow().read16(addr) as u32
                } else {
                    // Misaligned LDRH: ARM7TDMI rotates the full 32-bit word into Rd (no masking).
                    let raw = cpu.bus.borrow().read32(addr & !3);
                    raw.rotate_right((addr & 3) * 8)
                };
                cpu.set_reg(rd_idx, val);
            }
            3 => {
                // LDSH (Load Sign-extended Halfword)
                if (addr & 1) == 0 {
                    let val = cpu.bus.borrow().read16(addr) as i16 as i32 as u32;
                    cpu.set_reg(rd_idx, val);
                } else {
                    // Misaligned LDSH behaves like LDSB on ARM7TDMI
                    let val = cpu.bus.borrow().read8(addr) as i8 as i32 as u32;
                    cpu.set_reg(rd_idx, val);
                }
            }
            _ => unreachable!(),
        }

        1 // Base cycle; LDR: 1S+1N+1I, STR: 2N
    }

    /// Handler for the PC-Relative Load instruction, which loads a 32-bit word from a literal pool
    pub fn thumb_pc_relative_load(cpu: &mut Cpu, opcode: u16) -> u32 {
        let rd_idx = ((opcode >> 8) & 0x7) as usize;
        let imm8 = (opcode & 0xFF) as u32;
        let offset = imm8 << 2;

        // The value of PC is interpreted as (($+4) AND NOT 2)
        // cpu.reg(15) already returns $+4 in Thumb state
        let pc_base = cpu.reg(15) & !2;
        let addr = pc_base.wrapping_add(offset);

        // Load 32-bit word from the literal pool
        let val = cpu.bus.borrow().read32(addr & !3);
        cpu.set_reg(rd_idx, val);

        // 1S + 1N + 1I cycles
        1
    }

    /// Handler for the Hi Register Operations and Branch Exchange instruction,
    /// which performs operations on high registers or exchanges the instruction set
    /// (ADD, CMP, MOV, NOP, BX, BLX).
    pub fn thumb_hi_register_operations_branch_exchange(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 8) & 0x3;
        let h1 = (opcode >> 7) & 1;
        let h2 = (opcode >> 6) & 1;
        let rs_idx = (((opcode >> 3) & 0x7) | (h2 << 3)) as usize;
        let rd_idx = ((opcode & 0x7) | (h1 << 3)) as usize;

        match op {
            0b00 => {
                // ADD
                let val_rs = cpu.reg(rs_idx);
                let val_rd = cpu.reg(rd_idx);
                let result = val_rd.wrapping_add(val_rs);

                // If Rd is R15, we must flush the pipeline
                if rd_idx == 15 {
                    cpu.branch_to(result);
                } else {
                    cpu.set_reg(rd_idx, result);
                }
            }
            0b01 => {
                // CMP
                let val_rs = cpu.reg(rs_idx);
                let val_rd = cpu.reg(rd_idx);
                let (result, overflow) = val_rd.overflowing_sub(val_rs);

                // CMP always updates flags based on the subtraction
                let z = result == 0;
                let n = (result >> 31) != 0;
                let c = val_rd >= val_rs;
                let v = overflow;
                cpu.set_nzcv(n, z, c, v);
            }
            0b10 => {
                // MOV
                let val_rs = cpu.reg(rs_idx);

                if rd_idx == 15 {
                    cpu.branch_to(val_rs);
                } else {
                    cpu.set_reg(rd_idx, val_rs);
                }
            }
            0b11 => {
                // BX
                let rs_value = cpu.reg(rs_idx);
                let switch_to_arm = rs_value & 1 == 0;
                if switch_to_arm {
                    cpu.set_thumb_mode(!switch_to_arm);
                    cpu.branch_to(rs_value & !2);
                } else {
                    cpu.branch_to(rs_value & !1);
                }
            }
            _ => unreachable!(),
        }

        // If branch_to was called (BX or R15 dest), cost is in pending_fetch_cycles.
        // For non-branching ops (CMP, ADD/MOV to Lo regs) there are also no extra cycles.
        0
    }

    pub fn thumb_alu_operations(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 6) & 0xF;
        let rs_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let val_rs = cpu.reg(rs_idx);
        let val_rd = cpu.reg(rd_idx);

        match op {
            0x0 => {
                // AND
                let res = val_rd & val_rs;
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
            }
            0x1 => {
                // EOR
                let res = val_rd ^ val_rs;
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
            }
            0x2 => {
                // LSL
                let shift = val_rs & 0xFF;
                let (res, carry) = if shift == 0 {
                    (val_rd, cpu.get_c())
                } else if shift < 32 {
                    (val_rd << shift, (val_rd >> (32 - shift)) & 1 != 0)
                } else if shift == 32 {
                    (0, val_rd & 1 != 0)
                } else {
                    (0, false)
                };
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
                if shift > 0 {
                    cpu.set_c(carry);
                }
            }
            0x3 => {
                // LSR
                let shift = val_rs & 0xFF;
                let (res, carry) = if shift == 0 {
                    (val_rd, cpu.get_c())
                } else if shift < 32 {
                    (val_rd >> shift, (val_rd >> (shift - 1)) & 1 != 0)
                } else if shift == 32 {
                    (0, (val_rd >> 31) & 1 != 0)
                } else {
                    (0, false)
                };
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
                if shift > 0 {
                    cpu.set_c(carry);
                }
            }
            0x4 => {
                // ASR
                let shift = val_rs & 0xFF;
                let (res, carry) = if shift == 0 {
                    (val_rd, cpu.get_c())
                } else if shift < 32 {
                    (
                        ((val_rd as i32) >> shift) as u32,
                        (val_rd >> (shift - 1)) & 1 != 0,
                    )
                } else {
                    // shift >= 32: saturate to sign bit
                    let sign = val_rd & 0x8000_0000 != 0;
                    (if sign { 0xFFFF_FFFF } else { 0 }, sign)
                };
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
                if shift > 0 {
                    cpu.set_c(carry);
                }
            }
            0x5 => {
                // ADC
                let (res, c, v) = add_with_carry(val_rd, val_rs, cpu.get_c());
                cpu.set_reg(rd_idx, res);
                cpu.set_nzcv_from_u32(res, c, v);
            }
            0x6 => {
                // SBC
                let (res, c, v) = sub_with_carry(val_rd, val_rs, cpu.get_c());
                cpu.set_reg(rd_idx, res);
                cpu.set_nzcv_from_u32(res, c, v);
            }
            0x7 => {
                // ROR
                let shift = val_rs & 0xFF;
                let (res, carry) = if shift == 0 {
                    (val_rd, cpu.get_c())
                } else {
                    let s = shift & 0x1F;
                    if s == 0 {
                        // Multiple of 32: value unchanged, carry = bit 31
                        (val_rd, (val_rd >> 31) & 1 != 0)
                    } else {
                        let result = val_rd.rotate_right(s);
                        (result, (result >> 31) & 1 != 0)
                    }
                };
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
                if shift > 0 {
                    cpu.set_c(carry);
                }
            }
            0x8 => {
                // TST
                cpu.set_nz_from_u32(val_rd & val_rs);
            }
            0x9 => {
                // NEG (RSBS Rd, Rs, #0)
                let res = 0u32.wrapping_sub(val_rs);
                cpu.set_reg(rd_idx, res);
                let (res, c, v) = sub_with_carry(0, val_rs, true);
                cpu.set_nzcv_from_u32(res, c, v);
            }
            0xA => {
                // CMP
                let (res, c, v) = sub_with_carry(val_rd, val_rs, true);
                cpu.set_nzcv_from_u32(res, c, v);
            }
            0xB => {
                // CMN
                let (res, c, v) = add_with_carry(val_rd, val_rs, false);
                cpu.set_nzcv_from_u32(res, c, v);
            }
            0xC => {
                // ORR
                let res = val_rd | val_rs;
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
            }
            0xD => {
                // MUL — charges mI extra cycles (same byte-group rule as ARM MUL)
                let res = val_rd.wrapping_mul(val_rs);
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
                cpu.set_c(false);
                // Return m I-cycles directly; the 1S fetch is from advance_pipeline.
                let m = if (val_rs & 0xFFFF_FF00) == 0 || (val_rs & 0xFFFF_FF00) == 0xFFFF_FF00 {
                    1
                } else if (val_rs & 0xFFFF_0000) == 0 || (val_rs & 0xFFFF_0000) == 0xFFFF_0000 {
                    2
                } else if (val_rs & 0xFF00_0000) == 0 || (val_rs & 0xFF00_0000) == 0xFF00_0000 {
                    3
                } else {
                    4
                };
                return m;
            }
            0xE => {
                // BIC
                let res = val_rd & !val_rs;
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
            }
            0xF => {
                // MVN
                let res = !val_rs;
                cpu.set_reg(rd_idx, res);
                cpu.set_nz_from_u32(res);
            }
            _ => unreachable!(),
        }
        0 // all non-MUL ALU ops: 0 extra cycles (only the S fetch from advance_pipeline)
    }

    /// which performs a move, compare, add, or subtract operation with an immediate value.
    pub fn thumb_move_compare_add_subtract_immediate(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 11) & 0x3;
        let rd_idx = ((opcode >> 8) & 0x7) as usize;
        let imm = (opcode & 0xFF) as u32;

        match op {
            0b00 => {
                // MOV Rd, #imm
                cpu.set_reg(rd_idx, imm);
                cpu.set_nz_from_u32(imm);
                // MOV immediate does not affect Carry or Overflow
            }
            0b01 => {
                // CMP Rd, #imm
                let val_rd = cpu.reg(rd_idx);
                let result = val_rd.wrapping_sub(imm);

                let c = val_rd >= imm;
                let v = (val_rd as i32).checked_sub(imm as i32).is_none();
                cpu.set_nzcv_from_u32(result, c, v);
            }
            0b10 => {
                // ADD Rd, #imm
                let val_rd = cpu.reg(rd_idx);
                let result = val_rd.wrapping_add(imm);

                let c = val_rd.checked_add(imm).is_none();
                let v = (val_rd as i32).checked_add(imm as i32).is_none();
                cpu.set_nzcv_from_u32(result, c, v);

                cpu.set_reg(rd_idx, result);
            }
            0b11 => {
                // SUB Rd, #imm
                let val_rd = cpu.reg(rd_idx);
                let result = val_rd.wrapping_sub(imm);

                let c = val_rd >= imm;
                let v = (val_rd as i32).checked_sub(imm as i32).is_none();
                cpu.set_nzcv_from_u32(result, c, v);

                cpu.set_reg(rd_idx, result);
            }
            _ => unreachable!(),
        }

        0 // pure ALU, no extra cycles
    }

    pub fn thumb_add_subtract(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 9) & 0x3;
        let rn_imm = ((opcode >> 6) & 0x7) as u32;
        let rs_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let val_rs = cpu.reg(rs_idx);

        // Determine operand 2: either a register value or a 3-bit immediate
        let val_op2 = if (op & 2) == 0 {
            cpu.reg(rn_imm as usize) // Opcode 0 or 1: Rn is a register
        } else {
            rn_imm // Opcode 2 or 3: nn is an immediate
        };

        match op {
            0 | 2 => {
                // ADD
                let result = val_rs.wrapping_add(val_op2);
                let carry = val_rs.checked_add(val_op2).is_none();
                let overflow = (val_rs as i32).checked_add(val_op2 as i32).is_none();
                cpu.set_nzcv_from_u32(result, carry, overflow);
                cpu.set_reg(rd_idx, result);
            }
            1 | 3 => {
                // SUB
                let result = val_rs.wrapping_sub(val_op2);
                let carry = val_rs >= val_op2;
                let overflow = (val_rs as i32).checked_sub(val_op2 as i32).is_none();
                cpu.set_nzcv_from_u32(result, carry, overflow);
                cpu.set_reg(rd_idx, result);
            }
            _ => unreachable!(),
        }

        0 // pure ALU, no extra cycles
    }

    pub fn thumb_move_shifted_register(cpu: &mut Cpu, opcode: u16) -> u32 {
        let op = (opcode >> 11) & 0x3;
        let offset = (opcode >> 6) & 0x1F;
        let rs_idx = ((opcode >> 3) & 0x7) as usize;
        let rd_idx = (opcode & 0x7) as usize;

        let val = cpu.reg(rs_idx);

        let (result, carry) = match op {
            0b00 => {
                // LSL
                if offset == 0 {
                    // LSL #0: No change to Carry flag
                    (val, cpu.get_c())
                } else {
                    (val << offset, (val >> (32 - offset)) & 1 != 0)
                }
            }
            0b01 => {
                // LSR
                if offset == 0 {
                    // LSR #0 is interpreted as LSR #32
                    (0, (val >> 31) & 1 != 0)
                } else {
                    (val >> offset, (val >> (offset - 1)) & 1 != 0)
                }
            }
            0b10 => {
                // ASR
                if offset == 0 {
                    // ASR #0 is interpreted as ASR #32
                    if (val as i32) < 0 {
                        (0xFFFF_FFFF, (val >> 31) & 1 != 0)
                    } else {
                        (0, (val >> 31) & 1 != 0)
                    }
                } else {
                    let res = ((val as i32) >> offset) as u32;
                    (res, (val >> (offset - 1)) & 1 != 0)
                }
            }
            _ => unreachable!("Handled by add/sub decoder"),
        };

        cpu.set_reg(rd_idx, result);
        cpu.set_nzcv_from_u32(result, carry, false);

        0 // pure ALU, no extra cycles
    }
}
