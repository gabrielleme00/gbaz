mod disasm;
mod exec;

use super::{alu::*, *};
use checks::*;
use exec::handlers::*;

pub use disasm::disasm_thumb;

/// Size of the Thumb instruction handler table (2^10 for 10-bit opcode space).
pub const THUMB_TABLE_SIZE: usize = 1024;

/// Thumb instruction handler: receives mutable CPU, mutable bus, and the raw 16-bit opcode.
/// Returns the number of cycles consumed.
pub type ThumbHandler = fn(&mut Cpu, u16) -> u32;

/// Generates the Thumb instruction handler table filled with the unknown-opcode fallback.
pub fn generate_thumb_table() -> [ThumbHandler; THUMB_TABLE_SIZE] {
    let mut table = [thumb_unknown as ThumbHandler; THUMB_TABLE_SIZE];

    for i in 0..THUMB_TABLE_SIZE {
        let signature = (i as u16) << 6; // Shift index to align with opcode bits [15:6]

        if is_software_interrupt(signature) {
            table[i] = thumb_software_interrupt;
        } else if is_unconditional_branch(signature) {
            table[i] = thumb_unconditional_branch;
        } else if is_conditional_branch(signature) {
            table[i] = thumb_conditional_branch;
        } else if is_multiple_load_store(signature) {
            table[i] = thumb_multiple_load_store;
        } else if is_long_branch_with_link(signature) {
            table[i] = thumb_long_branch_with_link;
        } else if is_add_offset_to_stack_pointer(signature) {
            table[i] = thumb_add_offset_to_stack_pointer;
        } else if is_push_pop_registers(signature) {
            table[i] = thumb_push_pop_registers;
        } else if is_load_store_halfword(signature) {
            table[i] = thumb_load_store_halfword;
        } else if is_sp_relative_load_store(signature) {
            table[i] = thumb_sp_relative_load_store;
        } else if is_load_address(signature) {
            table[i] = thumb_load_address;
        } else if is_load_store_with_immediate_offset(signature) {
            table[i] = thumb_load_store_with_immediate_offset;
        } else if is_load_store_with_register_offset(signature) {
            table[i] = thumb_load_store_with_register_offset;
        } else if is_load_store_with_sign_extend_byte_halfword(signature) {
            table[i] = thumb_load_store_with_sign_extend_byte_halfword;
        } else if is_pc_relative_load(signature) {
            table[i] = thumb_pc_relative_load;
        } else if is_hi_register_operations_branch_exchange(signature) {
            table[i] = thumb_hi_register_operations_branch_exchange;
        } else if is_alu_operations(signature) {
            table[i] = thumb_alu_operations;
        } else if is_move_compare_add_subtract_immediate(signature) {
            table[i] = thumb_move_compare_add_subtract_immediate;
        } else if is_add_subtract(signature) {
            table[i] = thumb_add_subtract;
        } else if is_move_shifted_register(signature) {
            table[i] = thumb_move_shifted_register;
        }
    }

    table
}

mod checks {
    pub fn is_software_interrupt(signature: u16) -> bool {
        let format = 0b1101_1111_0000_0000;
        let mask = 0b1111_1111_0000_0000;
        (signature & mask) == format
    }

    pub fn is_unconditional_branch(signature: u16) -> bool {
        let format = 0b1110_0000_0000_0000;
        let mask = 0b1111_1000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_conditional_branch(signature: u16) -> bool {
        let format = 0b1101_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_multiple_load_store(signature: u16) -> bool {
        let format = 0b1100_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_long_branch_with_link(signature: u16) -> bool {
        let format = 0b1111_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_add_offset_to_stack_pointer(signature: u16) -> bool {
        let format = 0b1011_0000_0000_0000;
        let mask = 0b1111_1111_0000_0000;
        (signature & mask) == format
    }

    pub fn is_push_pop_registers(signature: u16) -> bool {
        let format = 0b1011_0100_0000_0000;
        let mask = 0b1111_0110_0000_0000;
        (signature & mask) == format
    }

    pub fn is_load_store_halfword(signature: u16) -> bool {
        let format = 0b1000_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_sp_relative_load_store(signature: u16) -> bool {
        let format = 0b1001_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_load_address(signature: u16) -> bool {
        let format = 0b1010_0000_0000_0000;
        let mask = 0b1111_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_load_store_with_immediate_offset(signature: u16) -> bool {
        let format = 0b0110_0000_0000_0000;
        let mask = 0b1110_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_load_store_with_register_offset(signature: u16) -> bool {
        let format = 0b0101_0000_0000_0000;
        let mask = 0b1111_0010_0000_0000;
        (signature & mask) == format
    }

    pub fn is_load_store_with_sign_extend_byte_halfword(signature: u16) -> bool {
        let format = 0b0101_0010_0000_0000;
        let mask = 0b1111_0010_0000_0000;
        (signature & mask) == format
    }

    pub fn is_pc_relative_load(signature: u16) -> bool {
        let format = 0b0100_1000_0000_0000;
        let mask = 0b1111_1000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_hi_register_operations_branch_exchange(signature: u16) -> bool {
        let format = 0b0100_0100_0000_0000;
        let mask = 0b1111_1100_0000_0000;
        (signature & mask) == format
    }

    pub fn is_alu_operations(signature: u16) -> bool {
        let format = 0b0100_0000_0000_0000;
        let mask = 0b1111_1100_0000_0000;
        (signature & mask) == format
    }

    pub fn is_move_compare_add_subtract_immediate(signature: u16) -> bool {
        let format = 0b0010_0000_0000_0000;
        let mask = 0b1110_0000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_add_subtract(signature: u16) -> bool {
        let format = 0b0001_1000_0000_0000;
        let mask = 0b1111_1000_0000_0000;
        (signature & mask) == format
    }

    pub fn is_move_shifted_register(signature: u16) -> bool {
        let format = 0b0000_0000_0000_0000;
        let mask = 0b1110_0000_0000_0000;
        (signature & mask) == format
    }
}
