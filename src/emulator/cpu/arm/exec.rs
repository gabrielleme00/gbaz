pub mod handlers {
    use super::helpers::*;
    use super::super::*;

    #[derive(Clone, Copy)]
    enum AluFlagKind {
        Logical,
        Arithmetic,
    }

    /// Fallback handler for opcodes that have not been implemented yet.
    pub fn arm_unknown(_cpu: &mut Cpu, opcode: u32) -> u32 {
        panic!("Unknown ARM opcode: {opcode:#010X} | {opcode:#034b}");
    }

    /// Handler for BX and BLX instructions.
    pub fn arm_branch_or_branch_and_exchange(cpu: &mut Cpu, opcode: u32) -> u32 {
        let rn = (opcode & 0xF) as usize;
        let target = cpu.reg(rn);
        let thumb = (target & 1) != 0;
        let target_aligned = target & !1;

        if thumb {
            cpu.set_thumb_mode(true);
            cpu.branch_to(target_aligned);
        } else {
            cpu.branch_to(target_aligned);
        }

        0 // fetch cost comes from refill_pipeline via branch_to
    }

    /// Handler for block data transfer instructions (LDM, STM).
    pub fn arm_block_data_transfer(cpu: &mut Cpu, opcode: u32) -> u32 {
        let p_bit = (opcode >> 24) & 1 != 0; // pre-index
        let u_bit = (opcode >> 23) & 1 != 0; // up (add offset)
        let s_bit = (opcode >> 22) & 1 != 0; // S: user regs / exception return
        let w_bit = (opcode >> 21) & 1 != 0; // writeback
        let l_bit = (opcode >> 20) & 1 != 0; // load
        let rn = ((opcode >> 16) & 0xF) as usize; // base register index
        let mut reg_list = opcode & 0xFFFF; // register list bitmask

        let mut n = reg_list.count_ones(); // Number of registers in the list (0-16) for address calculation
        let base = cpu.reg(rn); // Base address for the transfer

        // Empty list quirk
        if reg_list == 0 {
            reg_list = 0x8000; // Treat as if R15 (PC) is in the list
            n = 16; // Address calculation uses 16 registers (0x40 bytes)
        }

        // All transfers are performed lowest-address-first (ascending register number).
        // Compute the lowest address involved.
        let start_addr = if u_bit {
            if p_bit { base.wrapping_add(4) } else { base }
        } else {
            if p_bit {
                base.wrapping_sub(n * 4)
            } else {
                base.wrapping_sub(n * 4).wrapping_add(4)
            }
        };

        let writeback_addr = if u_bit {
            base.wrapping_add(n * 4)
        } else {
            base.wrapping_sub(n * 4)
        };

        let pc_in_list = (reg_list >> 15) & 1 != 0;
        // S=1 with STM, or S=1 with LDM and PC not in list: transfer user registers.
        let use_user_regs = s_bit && (!l_bit || !pc_in_list);

        if l_bit {
            // LDM
            let mut addr = start_addr;
            let mut loaded_pc = false;
            for i in 0..16usize {
                if (reg_list >> i) & 1 == 0 {
                    continue;
                }
                let val = cpu.bus.borrow().read32(addr & !3);
                if use_user_regs {
                    cpu.set_reg_usr(i, val);
                } else {
                    cpu.set_reg(i, val);
                    if i == REG_PC {
                        loaded_pc = true;
                    }
                }
                addr = addr.wrapping_add(4);
            }

            // Writeback is suppressed when Rn is in the register list.
            let rn_in_list = (reg_list >> rn) & 1 != 0;
            if w_bit && !rn_in_list {
                cpu.set_reg(rn, writeback_addr);
            }

            // S=1 with PC in list: exception return (restore CPSR from SPSR).
            if s_bit && pc_in_list {
                let spsr = cpu.spsr();
                cpu.set_cpsr(spsr);
            }

            if loaded_pc {
                let new_pc = cpu.reg(REG_PC) & !3; // Ensure PC is word-aligned
                cpu.branch_to(new_pc);
            }
        } else {
            // STM
            let mut addr = start_addr;

            // Determine the value of Rn to be stored.
            // If Rn is the FIRST register in the list, store the original base.
            // Otherwise, store the writeback_addr.
            let first_reg = (reg_list & !(reg_list - 1)).trailing_zeros() as usize;

            for i in 0..16usize {
                if (reg_list >> i) & 1 == 0 {
                    continue; // Skip registers not in the list
                }

                let val = if use_user_regs {
                    // S=1 with STM: store user registers.
                    cpu.reg_usr(i)
                } else if i == REG_PC {
                    // STM with PC stores the address of the STM + 12.
                    cpu.reg(REG_PC).wrapping_add(4)
                } else if i == rn && w_bit && i != first_reg {
                    // Quirk: Store the updated writeback address if not the first reg
                    writeback_addr
                } else {
                    // Normal case: store the current value of the register.
                    cpu.reg(i)
                };
                cpu.bus.borrow_mut().write32(addr & !3, val); // Stores are word-aligned by the hardware (bits 0-1 ignored)
                addr = addr.wrapping_add(4); // Increment address for next transfer
            }

            if w_bit {
                cpu.set_reg(rn, writeback_addr);
            }
        }

        1
    }

    /// Handler for B and BL instructions.
    pub fn arm_branch_or_branch_with_link(cpu: &mut Cpu, opcode: u32) -> u32 {
        let offset = ((opcode & 0x00FF_FFFF) << 2) as i32; // Extract and shift the 24-bit signed offset
        let offset = (offset << 6) >> 6; // Sign-extend to 32 bits
        let link = (opcode >> 24) & 1 != 0; // Check if it's a BL (link) instruction

        let target = cpu.reg(REG_PC).wrapping_add(offset as u32);

        if link {
            // Save return address (PC + 4) in LR and branch to target.
            cpu.set_reg(REG_LR, cpu.regs[REG_PC].wrapping_sub(4));
            cpu.branch_to(target);
        } else {
            cpu.branch_to(target);
        }

        0 // fetch cost comes from refill_pipeline via branch_to
    }

    pub fn arm_software_interrupt(cpu: &mut Cpu, _opcode: u32) -> u32 {
        cpu.enter_svc()
    }

    pub fn arm_undefined(_cpu: &mut Cpu, opcode: u32) -> u32 {
        panic!("Undefined ARM opcode: {opcode:#010X} | {opcode:#034b}");
    }

    /// Handler for single data transfer instructions (LDR, STR) with various addressing modes.
    pub fn arm_single_data_transfer(cpu: &mut Cpu, opcode: u32) -> u32 {
        let i_bit = (opcode >> 25) & 1 != 0; // Immediate offset if 0, register offset if 1 (with optional shift)
        let p_bit = (opcode >> 24) & 1 != 0; // Pre-indexing if 1, post-indexing if 0
        let u_bit = (opcode >> 23) & 1 != 0; // Add offset to base if 1, subtract if 0
        let b_bit = (opcode >> 22) & 1 != 0; // Byte transfer if 1, word transfer if 0
        let w_bit = (opcode >> 21) & 1 != 0; // Write-back base register if 1 (ignored for pre-indexing), no write-back if 0
        let l_bit = (opcode >> 20) & 1 != 0; // Load if 1, store if 0

        let rn_idx = ((opcode >> 16) & 0xF) as usize;
        let rd_idx = ((opcode >> 12) & 0xF) as usize;

        // Calculate the Offset
        let offset = if !i_bit {
            // Immediate Offset
            opcode & 0xFFF
        } else {
            // Register Offset (optionally shifted)
            // Note: For memory transfers, Rm cannot be R15 as it would cause a UNPREDICTABLE state
            // on real hardware. We can ignore that case here since it's not architecturally valid.
            let rm = cpu.reg((opcode & 0xF) as usize);
            let shift_imm = (opcode >> 7) & 0x1F;
            let shift_type = (opcode >> 5) & 0x3;

            // We only care about the value, not the shifter carry out here
            match shift_type {
                // LSL
                0b00 => rm << shift_imm,
                // LSR
                0b01 => {
                    if shift_imm == 0 {
                        0
                    } else {
                        rm >> shift_imm
                    }
                }
                // ASR
                0b10 => {
                    if shift_imm == 0 {
                        if (rm & 0x8000_0000) != 0 {
                            0xFFFF_FFFF
                        } else {
                            0
                        }
                    } else {
                        (rm as i32 >> shift_imm) as u32
                    }
                }
                // ROR
                0b11 => {
                    if shift_imm == 0 {
                        let c = if cpu.get_c() { 1 } else { 0 };
                        (rm >> 1) | (c << 31)
                    } else {
                        rm.rotate_right(shift_imm)
                    }
                }
                _ => unreachable!(),
            }
        };

        // Determine base address and offset direction
        let base_address = cpu.reg(rn_idx);
        let offset_signed = if u_bit {
            offset as i32
        } else {
            -(offset as i32)
        };

        // Address Calculation Logic (Pre/Post)
        let (transfer_addr, final_base_addr) = if p_bit {
            let addr = base_address.wrapping_add(offset_signed as u32);
            (addr, addr) // Pre-indexed: use calculated address
        } else {
            (
                base_address,
                base_address.wrapping_add(offset_signed as u32),
            ) // Post-indexed: use original base
        };

        // Perform the Transfer
        if l_bit {
            // LOAD
            let data = if b_bit {
                cpu.bus.borrow().read8(transfer_addr) as u32
            } else {
                // Misaligned Word Load quirk: Rotate the result
                let raw_data = cpu.bus.borrow().read32(transfer_addr & !3);
                let rotation = (transfer_addr & 3) * 8;
                raw_data.rotate_right(rotation)
            };

            // If we load into PC, we must flush the pipeline by branching
            if rd_idx == 15 {
                cpu.branch_to(data);
            } else {
                cpu.set_reg(rd_idx, data);
            }
        } else {
            // STORE
            // In ARM state, using R15 as a store source writes PC+12.
            let val = if rd_idx == REG_PC {
                cpu.reg(rd_idx).wrapping_add(4)
            } else {
                cpu.reg(rd_idx)
            };
            if b_bit {
                cpu.bus.borrow_mut().write8(transfer_addr, val as u8);
            } else {
                // Stores are typically word-aligned by the hardware (bits 0-1 ignored)
                cpu.bus.borrow_mut().write32(transfer_addr & !3, val);
            }
        }

        // Write-back Logic
        let is_load_conflict = l_bit && (rd_idx == rn_idx);
        if !is_load_conflict {
            if !p_bit || w_bit {
                cpu.set_reg(rn_idx, final_base_addr);
            }
        }

        // Timing: Base internal cycles + memory access cycles (calculated by bus)
        1
    }

    /// Handler for SWP and SWPB instructions.
    pub fn arm_single_data_swap(cpu: &mut Cpu, opcode: u32) -> u32 {
        let b_bit = (opcode >> 22) & 1 != 0; // Byte swap if 1, word swap if 0
        let rn_idx = ((opcode >> 16) & 0xF) as usize; // Base address
        let rd_idx = ((opcode >> 12) & 0xF) as usize; // Destination
        let rm_idx = (opcode & 0xF) as usize; // Source value

        let addr = cpu.reg(rn_idx);
        // Must get source value before we potentially overwrite the register
        let val_rm = cpu.reg(rm_idx);

        if b_bit {
            // SWPB: Byte swap
            let mem_val = cpu.bus.borrow().read8(addr) as u32;
            cpu.bus.borrow_mut().write8(addr, val_rm as u8);
            cpu.set_reg(rd_idx, mem_val);
        } else {
            // SWP: Word swap
            // Read the word at the aligned address
            let raw_mem_val = cpu.bus.borrow().read32(addr & !3);

            // t452: Calculate rotation for misaligned addresses
            let rotation = (addr & 3) * 8;
            let rotated_mem_val = raw_mem_val.rotate_right(rotation);

            // Write the source value to the aligned address
            cpu.bus.borrow_mut().write32(addr & !3, val_rm);

            // Update destination register with the (rotated) memory value
            cpu.set_reg(rd_idx, rotated_mem_val);
        }

        // SWP is notoriously slow on ARM7TDMI.
        // It typically involves 1N + 2S + 1I cycles.
        1
    }

    /// Handler for multiply instructions (MUL, MLA, UMULL, UMLAL, SMULL, SMLAL).
    pub fn arm_multiply(cpu: &mut Cpu, opcode: u32) -> u32 {
        let op_bits = (opcode >> 21) & 0xF;
        let s_bit = ((opcode >> 20) & 1) != 0;

        let rd_hi_idx = ((opcode >> 16) & 0xF) as usize; // Rd for MUL/MLA
        let rd_lo_idx = ((opcode >> 12) & 0xF) as usize; // Rn for MLA
        let rs_idx = ((opcode >> 8) & 0xF) as usize;
        let rm_idx = (opcode & 0xF) as usize;

        let rm = cpu.reg(rm_idx);
        let rs = cpu.reg(rs_idx);

        match op_bits {
            // 0000: MUL - Rd = Rm * Rs
            0b0000 => {
                let result = rm.wrapping_mul(rs);
                cpu.set_reg(rd_hi_idx, result);
                if s_bit {
                    cpu.set_nz_from_u32(result)
                };
                get_mul_cycles(rs, false)
            }

            // 0001: MLA - Rd = (Rm * Rs) + Rn
            0b0001 => {
                let rn = cpu.reg(rd_lo_idx);
                let result = rm.wrapping_mul(rs).wrapping_add(rn);
                cpu.set_reg(rd_hi_idx, result);
                if s_bit {
                    cpu.set_nz_from_u32(result);
                }
                get_mul_cycles(rs, true)
            }

            // 0100: UMULL - RdHi:RdLo = Rm * Rs (Unsigned)
            0b0100 => {
                let result = (rm as u64).wrapping_mul(rs as u64);
                cpu.set_reg(rd_lo_idx, (result & 0xFFFFFFFF) as u32);
                cpu.set_reg(rd_hi_idx, (result >> 32) as u32);
                if s_bit {
                    cpu.set_nz_from_u64(result);
                }
                get_mull_cycles(rs, false, false)
            }

            // 0101: UMLAL - RdHi:RdLo = (Rm * Rs) + RdHi:RdLo (Unsigned)
            0b0101 => {
                let current_val = ((cpu.reg(rd_hi_idx) as u64) << 32) | (cpu.reg(rd_lo_idx) as u64);
                let result = (rm as u64)
                    .wrapping_mul(rs as u64)
                    .wrapping_add(current_val);
                cpu.set_reg(rd_lo_idx, (result & 0xFFFFFFFF) as u32);
                cpu.set_reg(rd_hi_idx, (result >> 32) as u32);
                if s_bit {
                    cpu.set_nz_from_u64(result);
                }
                get_mull_cycles(rs, true, false)
            }

            // 0110: SMULL - RdHi:RdLo = Rm * Rs (Signed)
            0b0110 => {
                let result = (rm as i32 as i64).wrapping_mul(rs as i32 as i64) as u64;
                cpu.set_reg(rd_lo_idx, (result & 0xFFFFFFFF) as u32);
                cpu.set_reg(rd_hi_idx, (result >> 32) as u32);
                if s_bit {
                    cpu.set_nz_from_u64(result);
                }
                get_mull_cycles(rs, false, true)
            }

            // 0111: SMLAL - RdHi:RdLo = (Rm * Rs) + RdHi:RdLo (Signed)
            0b0111 => {
                let current_val = ((cpu.reg(rd_hi_idx) as u64) << 32) | (cpu.reg(rd_lo_idx) as u64);
                let result = (rm as i32 as i64)
                    .wrapping_mul(rs as i32 as i64)
                    .wrapping_add(current_val as i64) as u64;
                cpu.set_reg(rd_lo_idx, (result & 0xFFFFFFFF) as u32);
                cpu.set_reg(rd_hi_idx, (result >> 32) as u32);
                if s_bit {
                    cpu.set_nz_from_u64(result);
                }
                get_mull_cycles(rs, true, true)
            }

            _ => panic!("Unsupported multiply opcode: {:b}", op_bits),
        }
    }

    /// Handler for halfword data transfer instructions with register offset
    /// (STRH, LDRH, LDRSB, LDRSH).
    pub fn arm_halfword_data_transfer_register(cpu: &mut Cpu, opcode: u32) -> u32 {
        let offset = cpu.reg((opcode & 0xF) as usize);
        halfword_data_transfer(cpu, opcode, offset)
    }

    /// Handler for halfword data transfer instructions with immediate offset
    /// (STRH, LDRH, LDRSB, LDRSH).
    pub fn arm_halfword_data_transfer_immediate(cpu: &mut Cpu, opcode: u32) -> u32 {
        let offset = (((opcode >> 8) & 0xF) << 4) | (opcode & 0xF);
        halfword_data_transfer(cpu, opcode, offset)
    }

    /// Handler for MRS instructions that read from CPSR or SPSR into a register.
    pub fn arm_psr_transfer_mrs(cpu: &mut Cpu, opcode: u32) -> u32 {
        let rd = ((opcode >> 12) & 0xF) as usize;
        let use_spsr = ((opcode >> 22) & 1) != 0;

        let value = if use_spsr { cpu.spsr() } else { cpu.cpsr() };

        cpu.set_reg(rd, value);
        0
    }

    /// Handler for MSR instructions that write to CPSR (and optionally SPSR).
    pub fn arm_psr_transfer_msr(cpu: &mut Cpu, opcode: u32) -> u32 {
        let is_immediate = ((opcode >> 25) & 1) != 0;
        let use_spsr = ((opcode >> 22) & 1) != 0;
        let r = (opcode & 0xF) as usize;
        let field_mask = (opcode >> 16) & 0xF;

        let value = if is_immediate {
            let imm8 = opcode & 0xFF;
            let rot = ((opcode >> 8) & 0xF) * 2;
            imm8.rotate_right(rot)
        } else {
            cpu.reg(r)
        };

        // MSR updates selected PSR fields.
        // Field mask bits: 1=control, 2=extension, 4=status, 8=flags.
        let mut write_mask = 0u32;
        if field_mask & 0x8 != 0 {
            write_mask |= 0xF000_0000; // N,Z,C,V
        }
        if field_mask & 0x1 != 0 {
            write_mask |= 0x0000_00FF; // M[4:0], T, F, I
        }

        // Keep only modeled bits from extension/status fields for now.
        if field_mask & 0x2 != 0 {
            write_mask |= 0x0000_FF00;
        }
        if field_mask & 0x4 != 0 {
            write_mask |= 0x00FF_0000;
        }

        if use_spsr {
            let current_spsr = cpu.spsr();
            let new_spsr = (current_spsr & !write_mask) | (value & write_mask);
            cpu.set_spsr(new_spsr);
        } else {
            let new_cpsr = (cpu.cpsr() & !write_mask) | (value & write_mask);
            cpu.set_cpsr(new_cpsr);
        }

        0
    }

    /// Handler for data processing instructions
    /// (AND, EOR, SUB, RSB, ADD, ADC, SBC, RSC, TST, TEQ, CMP, CMN, ORR, MOV, BIC, MVN).
    pub fn arm_data_processing(cpu: &mut Cpu, opcode: u32) -> u32 {
        use AluFlagKind::*;

        let op = (opcode >> 21) & 0xF;
        let s_bit = ((opcode >> 20) & 1) != 0;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;
        let by_reg_shift = ((opcode >> 25) & 1) == 0 && ((opcode >> 4) & 1) != 0;

        // In register-shifted forms Rn=PC sees PC+12 (extra +4 on top of the
        // usual +8 that is already stored in regs[PC]).
        let op1 = if by_reg_shift && rn == REG_PC {
            cpu.reg(rn).wrapping_add(4)
        } else {
            cpu.reg(rn)
        };
        let (op2, sh_c) = decode_operand2(cpu, opcode);
        let c_in = cpu.get_c();

        let (result, c_out, v_out, writeback, flag_kind) = match op {
            0x0 => (op1 & op2, sh_c, false, true, Logical), // AND
            0x1 => (op1 ^ op2, sh_c, false, true, Logical), // EOR
            0x2 => {
                let (r, c, v) = add_with_carry(op1, !op2, true); // SUB
                (r, c, v, true, Arithmetic)
            }
            0x3 => {
                let (r, c, v) = add_with_carry(op2, !op1, true); // RSB
                (r, c, v, true, Arithmetic)
            }
            0x4 => {
                let (r, c, v) = add_with_carry(op1, op2, false); // ADD
                (r, c, v, true, Arithmetic)
            }
            0x5 => {
                let (r, c, v) = add_with_carry(op1, op2, c_in); // ADC
                (r, c, v, true, Arithmetic)
            }
            0x6 => {
                let (r, c, v) = add_with_carry(op1, !op2, c_in); // SBC
                (r, c, v, true, Arithmetic)
            }
            0x7 => {
                let (r, c, v) = add_with_carry(op2, !op1, c_in); // RSC
                (r, c, v, true, Arithmetic)
            }
            0x8 => (op1 & op2, sh_c, false, false, Logical), // TST
            0x9 => (op1 ^ op2, sh_c, false, false, Logical), // TEQ
            0xA => {
                let (r, c, v) = add_with_carry(op1, !op2, true); // CMP
                (r, c, v, false, Arithmetic)
            }
            0xB => {
                let (r, c, v) = add_with_carry(op1, op2, false); // CMN
                (r, c, v, false, Arithmetic)
            }
            0xC => (op1 | op2, sh_c, false, true, Logical), // ORR
            0xD => (op2, sh_c, false, true, Logical),       // MOV
            0xE => (op1 & !op2, sh_c, false, true, Logical), // BIC
            0xF => (!op2, sh_c, false, true, Logical),      // MVN
            _ => unreachable!(),
        };

        // Write result back to Rd if needed
        if writeback {
            if rd == REG_PC {
                if s_bit {
                    let spsr = cpu.spsr();
                    // Data-processing with S=1 and Rd=PC performs exception return.
                    cpu.set_cpsr(spsr);
                }
                cpu.branch_to(result); // PC writes flush/refill pipeline.
                return 0; // fetch cost is in pending_fetch_cycles
            } else {
                cpu.set_reg(rd, result);
            }
        }

        if s_bit {
            let n = (result >> 31) != 0;
            let z = result == 0;

            match flag_kind {
                Logical => {
                    // Logical ops update N,Z,C and preserve V.
                    let v_old = ((cpu.cpsr >> 28) & 1) != 0;
                    cpu.set_nzcv(n, z, c_out, v_old);
                }
                Arithmetic => {
                    cpu.set_nzcv(n, z, c_out, v_out);
                }
            }
        }

        // Register-shifted data-processing costs 1 extra I-cycle (barrel-shifter stall).
        // All other forms are pure S-cycle (0 extra).
        by_reg_shift as u32
    }

}

mod helpers {
    use super::super::*;

    /// Shared core of all halfword data-transfer instructions.
    /// `offset` has already been resolved by the caller (register or immediate form).
    pub fn halfword_data_transfer(cpu: &mut Cpu, opcode: u32, offset: u32) -> u32 {
        let p_bit = (opcode >> 24) & 1 == 1;
        let u_bit = (opcode >> 23) & 1 == 1;
        let w_bit = (opcode >> 21) & 1 == 1;
        let l_bit = (opcode >> 20) & 1 == 1;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;
        let op = (opcode >> 5) & 0x3; // bits 6:5 (S, H)

        let base = cpu.reg(rn);
        let offset_val = if u_bit {
            offset
        } else {
            0_u32.wrapping_sub(offset)
        };
        let indexed = base.wrapping_add(offset_val);

        // Address to use for the actual memory access
        let addr = if p_bit { indexed } else { base };

        if l_bit {
            let val = match op {
                0b01 => {
                    // LDRH (Unsigned Halfword)
                    if (addr & 1) == 0 {
                        cpu.bus.borrow().read16(addr & !1) as u32
                    } else {
                        // t408: Misaligned LDRH quirk; ARM7TDMI bus-rotates the full 32-bit
                        // word and returns all 32 bits (no halfword mask).
                        let word = cpu.bus.borrow().read32(addr & !3);
                        let rotation = (addr & 3) * 8;
                        word.rotate_right(rotation)
                    }
                }
                0b10 => cpu.bus.borrow().read8(addr) as i8 as i32 as u32, // LDRSB (Signed Byte)
                0b11 => {
                    // LDRSH (Signed Halfword)
                    if (addr & 1) == 0 {
                        cpu.bus.borrow().read16(addr & !1) as i16 as i32 as u32
                    } else {
                        // LDRSH at odd address behaves like LDRSB
                        cpu.bus.borrow().read8(addr) as i8 as i32 as u32
                    }
                }
                _ => unreachable!(),
            };

            if rd == 15 {
                cpu.branch_to(val);
            } else {
                cpu.set_reg(rd, val);
            }
        } else {
            // STRH
            let val = if rd == 15 {
                cpu.reg(rd).wrapping_add(4)
            } else {
                cpu.reg(rd)
            };
            cpu.bus.borrow_mut().write16(addr & !1, val as u16);
        }

        // Write-back Logic
        // 1. Post-indexing (P=0) always writes back
        // 2. Pre-indexing (P=1) writes back if W=1
        // 3. Conflict: If Load and Rd == Rn, write-back is suppressed
        let is_writeback = !p_bit || w_bit;
        let has_conflict = l_bit && (rd == rn);

        if is_writeback && !has_conflict {
            cpu.set_reg(rn, indexed);
        }

        1
    }

    /// Gets the cycle count for a MUL instruction based on the value of Rs and whether it's an MLA.
    pub fn get_mul_cycles(rs: u32, accumulate: bool) -> u32 {
        let mut cycles = if (rs & 0xFFFFFF00) == 0 || (rs & 0xFFFFFF00) == 0xFFFFFF00 {
            1
        } else if (rs & 0xFFFF0000) == 0 || (rs & 0xFFFF0000) == 0xFFFF0000 {
            2
        } else if (rs & 0xFF000000) == 0 || (rs & 0xFF000000) == 0xFF000000 {
            3
        } else {
            4
        };
        if accumulate {
            cycles += 1;
        }
        cycles
    }

    /// Gets the cycle count for a long multiply (UMULL/UMLAL/SMULL/SMLAL).
    ///
    /// ARM7TDMI TRM: UMULL/SMULL cost 1S+(m+1)I; UMLAL/SMLAL cost 1S+(m+2)I,
    /// where m is determined by the significant-byte count of Rs (same rule as
    /// short multiply). Signed and unsigned variants have identical timing.
    pub fn get_mull_cycles(rs: u32, accumulate: bool, _signed: bool) -> u32 {
        // Same m-factor as short multiply
        let m = if (rs & 0xFFFF_FF00) == 0 || (rs & 0xFFFF_FF00) == 0xFFFF_FF00 { 1 }
                else if (rs & 0xFFFF_0000) == 0 || (rs & 0xFFFF_0000) == 0xFFFF_0000 { 2 }
                else if (rs & 0xFF00_0000) == 0 || (rs & 0xFF00_0000) == 0xFF00_0000 { 3 }
                else { 4 };

        // UMULL/SMULL: m+1 I-cycles; UMLAL/SMLAL: m+2 I-cycles.
        // Return the I-cycle count, consistent with get_mul_cycles convention.
        m + 1 + if accumulate { 1 } else { 0 }
    }

    /// Decodes ARM Operand2 and returns (value, shifter_carry_out).
    pub fn decode_operand2(cpu: &Cpu, opcode: u32) -> (u32, bool) {
        let is_immediate = ((opcode >> 25) & 1) != 0;

        if is_immediate {
            let imm8 = opcode & 0xFF;
            let rot = ((opcode >> 8) & 0xF) * 2;
            let value = imm8.rotate_right(rot);
            let carry = if rot == 0 {
                cpu.get_c()
            } else {
                (value >> 31) != 0
            };
            return (value, carry);
        }

        let shift_type = (opcode >> 5) & 0x3;
        let by_reg = ((opcode >> 4) & 1) != 0;
        let rm = (opcode & 0xF) as usize;
        // In register-shifted forms, reading PC as Rm/Rs observes PC+12.
        let rm_value = if by_reg && rm == REG_PC {
            cpu.reg(rm).wrapping_add(4)
        } else {
            cpu.reg(rm)
        };

        if by_reg {
            let rs = ((opcode >> 8) & 0xF) as usize;
            let rs_value = if rs == REG_PC {
                cpu.reg(rs).wrapping_add(4)
            } else {
                cpu.reg(rs)
            };
            let amount = rs_value & 0xFF;

            match shift_type {
                // LSL by register
                0b00 => {
                    if amount == 0 {
                        (rm_value, cpu.get_c())
                    } else if amount < 32 {
                        let value = rm_value << amount;
                        let carry = ((rm_value >> (32 - amount)) & 1) != 0;
                        (value, carry)
                    } else if amount == 32 {
                        (0, (rm_value & 1) != 0)
                    } else {
                        (0, false)
                    }
                }
                // LSR by register
                0b01 => {
                    if amount == 0 {
                        (rm_value, cpu.get_c())
                    } else if amount < 32 {
                        let value = rm_value >> amount;
                        let carry = ((rm_value >> (amount - 1)) & 1) != 0;
                        (value, carry)
                    } else if amount == 32 {
                        (0, ((rm_value >> 31) & 1) != 0)
                    } else {
                        (0, false)
                    }
                }
                // ASR by register
                0b10 => {
                    if amount == 0 {
                        (rm_value, cpu.get_c())
                    } else if amount < 32 {
                        let value = (rm_value as i32 >> amount) as u32;
                        let carry = ((rm_value >> (amount - 1)) & 1) != 0;
                        (value, carry)
                    } else {
                        let sign = ((rm_value >> 31) & 1) != 0;
                        let value = if sign { 0xFFFF_FFFF } else { 0 };
                        (value, sign)
                    }
                }
                // ROR by register
                _ => {
                    if amount == 0 {
                        (rm_value, cpu.get_c())
                    } else {
                        let rot = amount & 31;
                        if rot == 0 {
                            (rm_value, ((rm_value >> 31) & 1) != 0)
                        } else {
                            let value = rm_value.rotate_right(rot);
                            (value, ((value >> 31) & 1) != 0)
                        }
                    }
                }
            }
        } else {
            let imm5 = (opcode >> 7) & 0x1F;

            match shift_type {
                // LSL #imm
                0b00 => {
                    if imm5 == 0 {
                        (rm_value, cpu.get_c())
                    } else {
                        let value = rm_value << imm5;
                        let carry = ((rm_value >> (32 - imm5)) & 1) != 0;
                        (value, carry)
                    }
                }
                // LSR #imm, imm=0 means 32
                0b01 => {
                    if imm5 == 0 {
                        (0, ((rm_value >> 31) & 1) != 0)
                    } else {
                        let value = rm_value >> imm5;
                        let carry = ((rm_value >> (imm5 - 1)) & 1) != 0;
                        (value, carry)
                    }
                }
                // ASR #imm, imm=0 means 32
                0b10 => {
                    if imm5 == 0 {
                        let sign = ((rm_value >> 31) & 1) != 0;
                        let value = if sign { 0xFFFF_FFFF } else { 0 };
                        (value, sign)
                    } else {
                        let value = (rm_value as i32 >> imm5) as u32;
                        let carry = ((rm_value >> (imm5 - 1)) & 1) != 0;
                        (value, carry)
                    }
                }
                // ROR #imm, imm=0 means RRX
                _ => {
                    if imm5 == 0 {
                        let c_in = if cpu.get_c() { 1u32 } else { 0u32 };
                        let value = (c_in << 31) | (rm_value >> 1);
                        (value, (rm_value & 1) != 0)
                    } else {
                        let value = rm_value.rotate_right(imm5);
                        (value, ((value >> 31) & 1) != 0)
                    }
                }
            }
        }
    }
}

