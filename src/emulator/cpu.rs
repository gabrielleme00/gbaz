mod alu;
mod arm;
mod thumb;

use std::{cell::RefCell, rc::Rc};

use super::bus::{AccessWidth, Bus};

use arm::{ARM_TABLE_SIZE, ArmHandler, disasm_arm, generate_arm_table};
use thumb::{THUMB_TABLE_SIZE, ThumbHandler, disasm_thumb, generate_thumb_table};

// CPSR flag bit positions
const FLAG_N: u32 = 1 << 31;
const FLAG_Z: u32 = 1 << 30;
const FLAG_C: u32 = 1 << 29;
const FLAG_V: u32 = 1 << 28;

// Register indices
const GPREG_COUNT: usize = 16;
const REG_SP: usize = 13;
const REG_LR: usize = 14;
const REG_PC: usize = 15;

// CPSR mode bits
const CPSR_MODE_MASK: u32 = 0x1F;
const MODE_USR: u32 = 0x10;
const MODE_FIQ: u32 = 0x11;
const MODE_IRQ: u32 = 0x12;
const MODE_SVC: u32 = 0x13;
const MODE_ABT: u32 = 0x17;
const MODE_UND: u32 = 0x1B;
const MODE_SYS: u32 = 0x1F;

// Post-BIOS entry point values for SP, LR, PC, and CPSR.
const POST_BIOS_SP: u32 = 0x0300_7F00;
const POST_BIOS_LR: u32 = 0x0800_0000;
const POST_BIOS_PC: u32 = 0x0800_0000;
const POST_BIOS_CPSR: u32 = 0xD3;

const THUMB_BIT: u32 = 1 << 5;
const IRQ_DISABLE_BIT: u32 = 1 << 7; // I bit: when set, maskable IRQs are disabled
const IRQ_VECTOR: u32 = 0x0000_0018;
const SVC_VECTOR: u32 = 0x0000_0008;

/// A single stage of the instruction pipeline.
#[derive(Debug, Clone, Copy)]
struct PipeWord {
    /// Address at which this word was fetched.
    addr: u32,
    /// Raw instruction bits.
    /// In ARM mode this stores a 32-bit opcode.
    /// In Thumb mode only the low 16 bits are used.
    raw: u32,
}

impl PipeWord {
    const EMPTY: Self = Self { addr: 0, raw: 0 };
}

/// Banked registers for privileged modes. Each field is used for the appropriate mode(s).
#[derive(Default)]
struct Bank<T> {
    pub fiq: T,
    pub irq: T,
    pub svc: T,
    pub abt: T,
    pub und: T,
}

/// ARM7TDMI CPU with a 3-stage (fetch / decode / execute) pipeline model.
pub struct Cpu {
    /// Shared reference to the system bus for memory access.
    bus: Rc<RefCell<Bus>>,
    /// General-purpose registers R0-R15.  R15 holds the PC-visible value
    /// (exec_addr + 8) while a handler is running.
    regs: [u32; GPREG_COUNT],
    /// Current Program Status Register.
    cpsr: u32,
    /// Saved Program Status Registers for privileged modes.
    spsr: Bank<u32>,
    /// User/System bank for R8-R12 (shared by all non-FIQ modes).
    bank_usr_r8_12: [u32; 5],
    /// FIQ bank for R8-R12.
    bank_fiq_r8_12: [u32; 5],
    /// User/System bank for R13-R14.
    bank_usr_r13_14: [u32; 2],
    /// Exception-mode banks for R13-R14.
    bank_fiq_r13_14: [u32; 2],
    bank_irq_r13_14: [u32; 2],
    bank_svc_r13_14: [u32; 2],
    bank_abt_r13_14: [u32; 2],
    bank_und_r13_14: [u32; 2],
    /// Next address to fetch from.
    /// In ARM mode this stays 8 bytes ahead of execute.
    /// In Thumb mode this stays 4 bytes ahead of execute.
    fetch_pc: u32,
    /// Pipeline latches: [0] = execute stage, [1] = decode stage.
    arm_pipe: [PipeWord; 2],
    /// Set to `true` by `refill_pipeline` when a branch or PC-write
    /// flushes and refills the pipeline mid-step.
    pipeline_flushed: bool,
    /// Cycle cost of the last `refill_pipeline` call (1N + 1S fetches).
    /// Read by `step()` when `pipeline_flushed` is true.
    pending_fetch_cycles: u32,
    /// Decode-table of ARM instruction handlers (indexed by 12-bit key).
    arm_table: [ArmHandler; ARM_TABLE_SIZE],
    /// Decode-table of Thumb instruction handlers (indexed by 10-bit key).
    thumb_table: [ThumbHandler; THUMB_TABLE_SIZE],
    /// When `true`, each executed instruction is disassembled and printed to stderr.
    disasm_enabled: bool,
}

impl Cpu {
    /// Constructs a zeroed CPU.
    ///
    /// The pipeline is not valid until `reset` is called with a live bus.
    pub fn new(bus: Rc<RefCell<Bus>>) -> Self {
        Self {
            bus,
            regs: [0; GPREG_COUNT],
            cpsr: POST_BIOS_CPSR,
            spsr: Bank::default(),
            bank_usr_r8_12: [0; 5],
            bank_fiq_r8_12: [0; 5],
            bank_usr_r13_14: [0; 2],
            bank_fiq_r13_14: [0; 2],
            bank_irq_r13_14: [0; 2],
            bank_svc_r13_14: [0; 2],
            bank_abt_r13_14: [0; 2],
            bank_und_r13_14: [0; 2],
            fetch_pc: 0,
            arm_pipe: [PipeWord::EMPTY; 2],
            pipeline_flushed: false,
            pending_fetch_cycles: 0,
            arm_table: generate_arm_table(),
            thumb_table: generate_thumb_table(),
            disasm_enabled: false,
        }
    }

    /// Enables or disables per-instruction disassembly output on stderr.
    pub fn set_disasm_enabled(&mut self, enabled: bool) {
        self.disasm_enabled = enabled;
    }

    /// Resets all CPU state and fills the pipeline to a pure boot state
    /// point. Must be called after the bus is ready.
    pub fn reset(&mut self) {
        self.regs = [0; GPREG_COUNT];
        self.cpsr = POST_BIOS_CPSR;
        self.spsr = Bank::default();
        self.bank_usr_r8_12 = [0; 5];
        self.bank_fiq_r8_12 = [0; 5];
        self.bank_usr_r13_14 = [0; 2];
        self.bank_fiq_r13_14 = [0; 2];
        self.bank_irq_r13_14 = [0; 2];
        self.bank_svc_r13_14 = [0; 2];
        self.bank_abt_r13_14 = [0; 2];
        self.bank_und_r13_14 = [0; 2];
        self.regs[REG_SP] = 0;
        self.regs[REG_LR] = 0;
        self.bank_svc_r13_14 = [0, 0];
        self.pipeline_flushed = false;
        self.refill_pipeline(0);
    }

    /// Skips BIOS execution and fills the pipeline from the post-BIOS entry point.
    /// Must be called after the bus is ready to bypass the BIOS.
    pub fn skip_bios(&mut self) {
        self.regs[REG_SP] = POST_BIOS_SP;
        self.regs[REG_LR] = POST_BIOS_LR;
        self.bank_svc_r13_14 = [POST_BIOS_SP, POST_BIOS_LR];
        self.pipeline_flushed = false;
        self.refill_pipeline(POST_BIOS_PC);
    }

    /// Fills both pipeline stages from `addr` and updates `fetch_pc`.
    /// ARM mode fetches 32-bit words from word-aligned addresses.
    /// Thumb mode fetches 16-bit halfwords from halfword-aligned addresses.
    ///
    /// Charges the bus for two fetches (1N at the target, 1S at target+width)
    /// and stores the total in `pending_fetch_cycles` for `step()` to collect.
    /// Marks the pipeline as flushed so that
    /// `step` skips the normal advance after the current handler returns.
    fn refill_pipeline(&mut self, addr: u32) {
        if self.is_thumb_mode() {
            let a = addr & !1;
            let c0 = self.bus.borrow().access_cycles(a, AccessWidth::Half);
            self.arm_pipe[0] = PipeWord {
                addr: a,
                raw: self.bus.borrow().read_16(a) as u32,
            };
            let a1 = a + 2;
            let c1 = self.bus.borrow().access_cycles(a1, AccessWidth::Half);
            self.arm_pipe[1] = PipeWord {
                addr: a1,
                raw: self.bus.borrow().read_16(a1) as u32,
            };
            self.fetch_pc = a + 4;
            self.pending_fetch_cycles = c0 + c1;
        } else {
            let a = addr & !3;
            let c0 = self.bus.borrow().access_cycles(a, AccessWidth::Word);
            self.arm_pipe[0] = PipeWord {
                addr: a,
                raw: self.bus.borrow().read_32(a),
            };
            let a1 = a + 4;
            let c1 = self.bus.borrow().access_cycles(a1, AccessWidth::Word);
            self.arm_pipe[1] = PipeWord {
                addr: a1,
                raw: self.bus.borrow().read_32(a1),
            };
            self.fetch_pc = a + 8;
            self.pending_fetch_cycles = c0 + c1;
        }
        self.pipeline_flushed = true;
    }

    /// Shifts the pipeline forward by one stage, fetches the next opcode,
    /// and returns the S-cycle cost of that fetch.
    fn advance_pipeline(&mut self) -> u32 {
        self.arm_pipe[0] = self.arm_pipe[1];
        if self.is_thumb_mode() {
            let addr = self.fetch_pc;
            let cycles = self.bus.borrow().access_cycles(addr, AccessWidth::Half);
            self.arm_pipe[1] = PipeWord {
                addr,
                raw: self.bus.borrow().read_16(addr) as u32,
            };
            self.fetch_pc = self.fetch_pc.wrapping_add(2);
            cycles
        } else {
            let addr = self.fetch_pc;
            let cycles = self.bus.borrow().access_cycles(addr, AccessWidth::Word);
            self.arm_pipe[1] = PipeWord {
                addr,
                raw: self.bus.borrow().read_32(addr),
            };
            self.fetch_pc = self.fetch_pc.wrapping_add(4);
            cycles
        }
    }

    /// Redirects execution to `target`.
    ///
    /// Call this from any handler that modifies the PC
    /// (branches, `Rd = 15` data-processing results, etc.).
    pub fn branch_to(&mut self, target: u32) {
        self.refill_pipeline(target);
    }

    /// Enters SVC (supervisor) exception mode and vectors to `SVC_VECTOR` (0x08).
    ///
    /// Used by the SWI instruction. LR_svc is set to the instruction after the SWI
    /// so the BIOS handler can return with `MOVS PC, LR`.
    pub fn enter_svc(&mut self) -> u32 {
        let thumb = self.is_thumb_mode();
        let return_addr = self.arm_pipe[0].addr + if thumb { 2 } else { 4 };
        let saved_cpsr = self.cpsr;
        let new_cpsr = (saved_cpsr & !(CPSR_MODE_MASK | THUMB_BIT)) | MODE_SVC | IRQ_DISABLE_BIT;
        self.set_cpsr(new_cpsr);
        self.spsr.svc = saved_cpsr;
        self.regs[REG_LR] = return_addr;
        self.branch_to(SVC_VECTOR);
        4
    }

    /// Enters IRQ exception mode and vectors to `IRQ_VECTOR`.
    ///
    /// On ARM7TDMI the return linkage is `LR_irq = next_pc + 4`, so the
    /// standard handler epilogue `SUBS PC, LR, #4` resumes the interrupted
    /// instruction in both ARM and Thumb code.
    ///
    /// Returns the cycle cost of the exception entry (~4 cycles).
    fn enter_irq(&mut self) -> u32 {
        // The instruction sitting in the execute stage is the one that would
        // have run next; we want to resume it after the handler returns.
        let return_addr = self.arm_pipe[0].addr + 4;
        let saved_cpsr = self.cpsr;

        // Switch to IRQ mode, clear Thumb, disable further IRQs.
        let new_cpsr = (saved_cpsr & !(CPSR_MODE_MASK | THUMB_BIT)) | MODE_IRQ | IRQ_DISABLE_BIT;
        self.set_cpsr(new_cpsr); // saves/restores banked registers
        self.spsr.irq = saved_cpsr;
        self.regs[REG_LR] = return_addr;
        self.branch_to(IRQ_VECTOR);
        4
    }

    /// Executes one instruction and returns the number of cycles consumed.
    ///
    /// Pipeline notes:
    /// - In ARM mode, R15 is set to `exec_addr + 8` before the handler runs.
    /// - In Thumb mode, R15 is set to `exec_addr + 4` before the handler runs.
    /// - If the handler called `branch_to` the pipeline was already refilled;
    ///   otherwise we advance it normally.
    pub fn step(&mut self) -> u32 {
        // HALT mode: stall until IE & IF != 0 (independent of IME and I bit).
        if self.bus.borrow().io.borrow().halted {
            if self.bus.borrow().io.borrow().interrupt.irq_asserted() {
                self.bus.borrow_mut().io.borrow_mut().halted = false;
                // Fall through: the IRQ check below will dispatch if I bit allows.
            } else {
                return 1;
            }
        }

        // Sample IRQ between instructions (I bit clear = IRQs enabled).
        if self.cpsr & IRQ_DISABLE_BIT == 0 && self.bus.borrow().io.borrow().interrupt.irq_pending()
        {
            return self.enter_irq();
        }

        let exec = self.arm_pipe[0];
        let thumb_mode = self.is_thumb_mode();

        if self.disasm_enabled {
            let instr = if thumb_mode {
                format!("{}", disasm_thumb(exec.addr, exec.raw as u16))
            } else {
                format!("{}", disasm_arm(exec.addr, exec.raw))
            };
            println!("{:08X?} | {} | {}", self.regs, self.flags_to_string(), instr);
        }

        self.regs[REG_PC] = if thumb_mode {
            exec.addr.wrapping_add(4)
        } else {
            exec.addr.wrapping_add(8)
        };
        self.pipeline_flushed = false;

        let handler_cycles = if thumb_mode {
            // Thumb
            let opcode = exec.raw as u16;
            let index = Self::thumb_index(opcode);
            let handler = self.thumb_table[index];
            handler(self, opcode)
        } else {
            // ARM
            if self.check_condition(exec.raw >> 28) {
                let index = Self::arm_index(exec.raw);
                let handler = self.arm_table[index];
                handler(self, exec.raw)
            } else {
                0 // Condition failed: only the pipeline S-fetch counts
            }
        };

        let fetch_cycles = if self.pipeline_flushed {
            self.pending_fetch_cycles
        } else {
            self.advance_pipeline()
        };

        handler_cycles + fetch_cycles
    }

    // Helpers

    /// Evaluates the 4-bit condition field of an ARM instruction against CPSR.
    /// Returns `true` if the instruction should execute.
    fn check_condition(&self, cond: u32) -> bool {
        let n = (self.cpsr >> 31) & 1 != 0;
        let z = (self.cpsr >> 30) & 1 != 0;
        let c = (self.cpsr >> 29) & 1 != 0;
        let v = (self.cpsr >> 28) & 1 != 0;

        match cond {
            0x0 => z,              // EQ
            0x1 => !z,             // NE
            0x2 => c,              // CS / HS
            0x3 => !c,             // CC / LO
            0x4 => n,              // MI
            0x5 => !n,             // PL
            0x6 => v,              // VS
            0x7 => !v,             // VC
            0x8 => c && !z,        // HI
            0x9 => !c || z,        // LS
            0xA => n == v,         // GE
            0xB => n != v,         // LT
            0xC => !z && (n == v), // GT
            0xD => z || (n != v),  // LE
            0xE => true,           // AL (always)
            _ => false,            // NV (never / reserved)
        }
    }

    /// Extracts bits [27:20] and [7:4] to form the 12-bit ARM dispatch index.
    fn arm_index(raw: u32) -> usize {
        let hi = (raw >> 16) & 0xFF0; // bits 27-20 -> positions 11-4
        let lo = (raw >> 4) & 0x00F; //  bits  7-4  -> positions  3-0
        (hi | lo) as usize
    }

    /// Extracts bits [15:6] to form the 10-bit Thumb dispatch index.
    fn thumb_index(raw: u16) -> usize {
        (raw >> 6) as usize
    }

    // Register accessors (used by handlers)
    pub fn reg(&self, idx: usize) -> u32 {
        self.regs[idx]
    }

    /// Returns the address of the instruction currently in execute stage.
    pub fn execute_addr(&self) -> u32 {
        self.arm_pipe[0].addr
    }

    /// Writes to R15 (PC) must go through `branch_to` to ensure the pipeline is refilled.
    pub fn set_reg(&mut self, idx: usize, value: u32) {
        self.regs[idx] = value;
    }

    /// Returns the current value of CPSR.
    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    /// Writes to CPSR, handling mode switches and banked register saves/restores as needed.
    pub fn set_cpsr(&mut self, value: u32) {
        let old_mode = self.cpsr & CPSR_MODE_MASK;
        let new_mode = value & CPSR_MODE_MASK;

        if old_mode != new_mode {
            self.save_banked_registers(old_mode);
            self.restore_banked_registers(new_mode);
        }

        self.cpsr = value;
    }

    /// Returns SPSR of the current mode if one exists.
    pub fn spsr(&self) -> u32 {
        match self.cpsr & CPSR_MODE_MASK {
            MODE_FIQ => self.spsr.fiq,
            MODE_IRQ => self.spsr.irq,
            MODE_SVC => self.spsr.svc,
            MODE_ABT => self.spsr.abt,
            MODE_UND => self.spsr.und,
            _ => self.cpsr,
        }
    }

    /// Writes SPSR of the current mode if one exists.
    pub fn set_spsr(&mut self, value: u32) {
        match self.cpsr & CPSR_MODE_MASK {
            MODE_FIQ => self.spsr.fiq = value,
            MODE_IRQ => self.spsr.irq = value,
            MODE_SVC => self.spsr.svc = value,
            MODE_ABT => self.spsr.abt = value,
            MODE_UND => self.spsr.und = value,
            _ => {}
        }
    }

    /// Reads the user-mode view of a register (used by LDM/STM with S=1).
    /// For R0-R7 and R15, this is identical to `reg()`.
    pub fn reg_usr(&self, idx: usize) -> u32 {
        match idx {
            0..=7 | 15 => self.regs[idx],
            8..=12 => {
                if (self.cpsr & CPSR_MODE_MASK) == MODE_FIQ {
                    self.bank_usr_r8_12[idx - 8]
                } else {
                    self.regs[idx]
                }
            }
            13 => match self.cpsr & CPSR_MODE_MASK {
                MODE_USR | MODE_SYS => self.regs[13],
                _ => self.bank_usr_r13_14[0],
            },
            14 => match self.cpsr & CPSR_MODE_MASK {
                MODE_USR | MODE_SYS => self.regs[14],
                _ => self.bank_usr_r13_14[1],
            },
            _ => unreachable!(),
        }
    }

    /// Writes the user-mode view of a register (used by LDM/STM with S=1).
    pub fn set_reg_usr(&mut self, idx: usize, value: u32) {
        match idx {
            0..=7 | 15 => self.regs[idx] = value,
            8..=12 => {
                if (self.cpsr & CPSR_MODE_MASK) == MODE_FIQ {
                    self.bank_usr_r8_12[idx - 8] = value;
                } else {
                    self.regs[idx] = value;
                }
            }
            13 => match self.cpsr & CPSR_MODE_MASK {
                MODE_USR | MODE_SYS => self.regs[13] = value,
                _ => self.bank_usr_r13_14[0] = value,
            },
            14 => match self.cpsr & CPSR_MODE_MASK {
                MODE_USR | MODE_SYS => self.regs[14] = value,
                _ => self.bank_usr_r13_14[1] = value,
            },
            _ => unreachable!(),
        }
    }

    /// Returns `true` if the T bit in CPSR is set, indicating Thumb mode.
    pub fn is_thumb_mode(&self) -> bool {
        (self.cpsr & THUMB_BIT) != 0
    }

    /// Sets or clears the T bit in CPSR to switch between ARM and Thumb modes.
    pub fn set_thumb_mode(&mut self, set: bool) {
        if set {
            self.cpsr |= THUMB_BIT;
        } else {
            self.cpsr &= !THUMB_BIT;
        }
    }

    /// Saves banked registers for the old mode before a mode switch.
    fn save_banked_registers(&mut self, mode: u32) {
        let sp_lr = [self.regs[REG_SP], self.regs[REG_LR]];

        match mode {
            MODE_FIQ => {
                self.bank_fiq_r8_12.copy_from_slice(&self.regs[8..13]);
                self.bank_fiq_r13_14 = sp_lr;
            }
            MODE_IRQ => {
                self.bank_irq_r13_14 = sp_lr;
            }
            MODE_SVC => {
                self.bank_svc_r13_14 = sp_lr;
            }
            MODE_ABT => {
                self.bank_abt_r13_14 = sp_lr;
            }
            MODE_UND => {
                self.bank_und_r13_14 = sp_lr;
            }
            MODE_USR | MODE_SYS => {
                self.bank_usr_r13_14 = sp_lr;
            }
            _ => {}
        }

        if mode != MODE_FIQ {
            self.bank_usr_r8_12.copy_from_slice(&self.regs[8..13]);
        }
    }

    /// Restores banked registers for the new mode after a mode switch.
    fn restore_banked_registers(&mut self, mode: u32) {
        if mode == MODE_FIQ {
            self.regs[8..13].copy_from_slice(&self.bank_fiq_r8_12);
        } else {
            self.regs[8..13].copy_from_slice(&self.bank_usr_r8_12);
        }

        let sp_lr = match mode {
            MODE_FIQ => self.bank_fiq_r13_14,
            MODE_IRQ => self.bank_irq_r13_14,
            MODE_SVC => self.bank_svc_r13_14,
            MODE_ABT => self.bank_abt_r13_14,
            MODE_UND => self.bank_und_r13_14,
            MODE_USR | MODE_SYS => self.bank_usr_r13_14,
            _ => self.bank_usr_r13_14,
        };

        self.regs[REG_SP] = sp_lr[0];
        self.regs[REG_LR] = sp_lr[1];
    }

    /// Gets the current value of the C flag from CPSR.
    fn get_c(&self) -> bool {
        (self.cpsr & FLAG_C) != 0
    }

    /// Sets or clears the C flag in CPSR according to the provided boolean value.
    fn set_c(&mut self, value: bool) {
        if value {
            self.cpsr |= FLAG_C;
        } else {
            self.cpsr &= !FLAG_C;
        }
    }

    /// Gets the current value of the V flag from CPSR.
    fn get_v(&self) -> bool {
        (self.cpsr & FLAG_V) != 0
    }

    fn flags_to_string(&self) -> String {
        let n = if (self.cpsr & FLAG_N) != 0 { 'N' } else { 'n' };
        let z = if (self.cpsr & FLAG_Z) != 0 { 'Z' } else { 'z' };
        let c = if (self.cpsr & FLAG_C) != 0 { 'C' } else { 'c' };
        let v = if (self.cpsr & FLAG_V) != 0 { 'V' } else { 'v' };
        format!("{n}{z}{c}{v}")
    }

    /// Sets the N, Z, C, V flags in CPSR according to the provided boolean values.
    pub fn set_nzcv(&mut self, n: bool, z: bool, c: bool, v: bool) {
        let mut cpsr = self.cpsr;
        cpsr &= !0xF000_0000;

        if n {
            cpsr |= FLAG_N;
        } else {
            cpsr &= !FLAG_N;
        }

        if z {
            cpsr |= FLAG_Z;
        } else {
            cpsr &= !FLAG_Z;
        }

        if c {
            cpsr |= FLAG_C;
        } else {
            cpsr &= !FLAG_C;
        }

        if v {
            cpsr |= FLAG_V;
        } else {
            cpsr &= !FLAG_V;
        }

        self.cpsr = cpsr;
    }

    /// Sets the N, Z, C, V flags in CPSR according to the provided u32 result and carry/overflow values.
    pub fn set_nzcv_from_u32(&mut self, result: u32, carry: bool, overflow: bool) {
        self.set_nzcv((result >> 31) != 0, result == 0, carry, overflow);
    }

    /// Sets the N, Z flags in CPSR according to the provided boolean values.
    pub fn set_nz(&mut self, n: bool, z: bool) {
        self.set_nzcv(n, z, self.get_c(), self.get_v());
    }

    /// Sets the N, Z flags in CPSR according to the provided u32 value.
    pub fn set_nz_from_u32(&mut self, value: u32) {
        self.set_nz((value >> 31) != 0, value == 0);
    }

    /// Sets the N, Z flags in CPSR according to the provided u64 value.
    pub fn set_nz_from_u64(&mut self, value: u64) {
        self.set_nz((value >> 63) != 0, value == 0);
    }
}
