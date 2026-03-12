mod disasm;
mod exec;

use super::{alu::*, *};
use checks::*;
use exec::handlers::*;

pub use disasm::disasm_arm;

/// Size of the ARM instruction handler table (2^12 for 12-bit opcode space).
pub const ARM_TABLE_SIZE: usize = 4096;

/// ARM instruction handler: receives mutable CPU and the raw 32-bit opcode.
/// Returns the number of cycles consumed.
pub type ArmHandler = fn(&mut Cpu, u32) -> u32;

/// Generates the ARM instruction handler table.
pub fn generate_arm_table() -> [ArmHandler; ARM_TABLE_SIZE] {
    let mut table = [arm_unknown as ArmHandler; ARM_TABLE_SIZE];

    for i in 0..ARM_TABLE_SIZE {
        // Dispatch table index encodes opcode bits [27:20] in index[11:4]
        // and opcode bits [7:4] in index[3:0]. Reconstruct those opcode bits
        // so the format matchers can run against normal ARM masks.
        let key = i as u32;
        let opcode_sig = (((key & 0xFF0) << 16) | ((key & 0x00F) << 4)) as usize;

        if is_branch_or_branch_and_exchange(opcode_sig) {
            table[i] = arm_branch_or_branch_and_exchange;
        } else if is_block_data_transfer(opcode_sig) {
            table[i] = arm_block_data_transfer;
        } else if is_branch_or_branch_with_link(opcode_sig) {
            table[i] = arm_branch_or_branch_with_link;
        } else if is_software_interrupt(opcode_sig) {
            table[i] = arm_software_interrupt;
        } else if is_undefined(opcode_sig) {
            table[i] = arm_undefined;
        } else if is_single_data_transfer(opcode_sig) {
            table[i] = arm_single_data_transfer;
        } else if is_single_data_swap(opcode_sig) {
            table[i] = arm_single_data_swap;
        } else if is_multiply(opcode_sig) {
            table[i] = arm_multiply;
        } else if is_halfword_data_transfer_register(opcode_sig) {
            table[i] = arm_halfword_data_transfer_register;
        } else if is_halfword_data_transfer_immediate(opcode_sig) {
            table[i] = arm_halfword_data_transfer_immediate;
        } else if is_psr_transfer_mrs(opcode_sig) {
            table[i] = arm_psr_transfer_mrs;
        } else if is_psr_transfer_msr(opcode_sig) {
            table[i] = arm_psr_transfer_msr;
        } else if is_data_processing(opcode_sig) {
            table[i] = arm_data_processing;
        }
    }

    table
}

pub mod checks {
    pub fn is_branch_or_branch_and_exchange(opcode: usize) -> bool {
        // opcode_sig only carries bits [27:20] and [7:4]; bits [19:8] are always
        // zero in the reconstructed signature even though the real BX encoding
        // requires them to be 0xFFF. Key 0x121 (bits[27:20]=0x12, bits[7:4]=0x1)
        // is exclusively BX, so we only check those two fields.
        let format = 0b0000_0001_0010_0000_0000_0000_0001_0000;
        let mask = 0b0000_1111_1111_0000_0000_0000_1111_0000;
        (opcode & mask) == format
    }

    pub fn is_block_data_transfer(opcode: usize) -> bool {
        let format = 0b0000_1000_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1110_0000_0000_0000_0000_0000_0000;
        (opcode & mask) == format
    }

    pub fn is_branch_or_branch_with_link(opcode: usize) -> bool {
        let format = 0b0000_1010_0000_0000_0000_0000_0000_0000;
        let format_link = 0b0000_1011_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1111_0000_0000_0000_0000_0000_0000;
        let extracted = opcode & mask;
        extracted == format || extracted == format_link
    }

    pub fn is_software_interrupt(opcode: usize) -> bool {
        let format = 0b0000_1111_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1111_0000_0000_0000_0000_0000_0000;
        (opcode & mask) == format
    }

    pub fn is_undefined(opcode: usize) -> bool {
        let format = 0b0000_0110_0000_0000_0000_0000_0001_0000;
        let mask = 0b0000_1110_0000_0000_0000_0000_0001_0000;
        (opcode & mask) == format
    }

    pub fn is_single_data_transfer(opcode: usize) -> bool {
        let format = 0b0000_0100_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1100_0000_0000_0000_0000_0000_0000;
        (opcode & mask) == format
    }

    pub fn is_single_data_swap(opcode: usize) -> bool {
        let format = 0b0000_0001_0000_0000_0000_0000_1001_0000;
        let mask = 0b0000_1111_1000_0000_0000_1111_1111_0000;
        (opcode & mask) == format
    }

    pub fn is_multiply(opcode: usize) -> bool {
        let format = 0b0000_0000_0000_0000_0000_0000_1001_0000;
        let format_long = 0b0000_0000_1000_0000_0000_0000_1001_0000;
        let mask = 0b0000_1111_1000_0000_0000_0000_1111_0000;
        let extracted = opcode & mask;
        extracted == format || extracted == format_long
    }

    pub fn is_halfword_data_transfer_register(opcode: usize) -> bool {
        let format = 0b0000_0000_0000_0000_0000_0000_1001_0000;
        let mask = 0b0000_1110_0100_0000_0000_1111_1001_0000;
        (opcode & mask) == format
    }

    pub fn is_halfword_data_transfer_immediate(opcode: usize) -> bool {
        let format = 0b0000_0000_0100_0000_0000_0000_1001_0000;
        let mask = 0b0000_1110_0100_0000_0000_0000_1001_0000;
        (opcode & mask) == format
    }

    pub fn is_psr_transfer_mrs(opcode: usize) -> bool {
        let format = 0b0000_0001_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1111_1011_0000_0000_0000_1111_0000;
        (opcode & mask) == format
    }

    pub fn is_psr_transfer_msr(opcode: usize) -> bool {
        // MSR immediate: 00110?10 ........ ........
        let format_imm = 0b0000_0011_0010_0000_0000_0000_0000_0000;
        let mask_imm = 0b0000_1111_1011_0000_0000_0000_0000_0000;

        // MSR register: 00010?10 ........ ....0000
        let format_reg = 0b0000_0001_0010_0000_0000_0000_0000_0000;
        let mask_reg = 0b0000_1111_1011_0000_0000_0000_1111_0000;

        (opcode & mask_imm) == format_imm || (opcode & mask_reg) == format_reg
    }

    pub fn is_data_processing(opcode: usize) -> bool {
        let format = 0b0000_0000_0000_0000_0000_0000_0000_0000;
        let mask = 0b0000_1100_0000_0000_0000_0000_0000_0000;
        (opcode & mask) == format
    }
}
