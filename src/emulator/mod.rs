pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod input;
pub mod interrupt;
pub mod iodev;
pub mod ppu;
pub mod timer;

use std::{cell::{Ref, RefCell}, rc::Rc};

pub use apu::Apu;
pub use bus::Bus;
pub use cartridge::Cartridge;
pub use cpu::Cpu;
pub use input::InputState;
pub use interrupt::InterruptController;
pub use iodev::IoDevices;
pub use ppu::Ppu;
pub use timer::Timer;

/// Coordinates all emulated hardware blocks and advances them in lockstep.
pub struct Emulator {
    cpu: Box<Cpu>,
    bus: Rc<RefCell<Bus>>,
    io_devs: Rc<RefCell<IoDevices>>,
}

impl Emulator {
    pub fn new(rom: Vec<u8>, bios: Option<Vec<u8>>) -> Self {
        let cartridge = Cartridge::from_rom(rom);
        let has_bios = bios.is_some();

        let interrupt_flags = Rc::new(RefCell::new(0));

        let intc = InterruptController::new(interrupt_flags.clone());
        let ppu = Box::new(Ppu::new(interrupt_flags.clone()));
        let apu = Apu::new();
        let timer = Timer::new();

        let io_devs = Rc::new(RefCell::new(IoDevices::new(intc, ppu, apu, timer)));
        let bus = Rc::new(RefCell::new(Bus::new(cartridge, io_devs.clone(), bios)));
        let cpu = Box::new(Cpu::new(bus.clone()));

        let mut emu = Self {
            cpu,
            bus,
            io_devs,
        };

        emu.reset();
        if !has_bios {
            emu.skip_bios();
        }
        emu.cpu.set_disasm_enabled(true);
        emu
    }

    fn skip_bios(&mut self) {
        self.cpu.skip_bios();
        self.bus.borrow_mut().io.borrow_mut().ppu.skip_bios();
    }

    /// Runs CPU+subsystems until one video frame is produced.
    pub fn run_frame(&mut self) {
        self.io_devs.borrow_mut().ppu.begin_frame();

        while !self.io_devs.borrow().ppu.frame_ready() {
            self.step();
        }
    }

    /// Executes one CPU instruction and advances dependent hardware by cycle count.
    pub fn step(&mut self) {
        let cycles = self.cpu.step();
        for _ in 0..cycles {
            self.bus.borrow_mut().tick();
        }
    }

    /// Returns the address of the instruction about to execute on next step.
    pub fn execute_addr(&self) -> u32 {
        self.cpu.execute_addr()
    }

    /// Reads a CPU general-purpose register by index.
    pub fn reg(&self, idx: usize) -> u32 {
        self.cpu.reg(idx)
    }

    /// Returns the completed framebuffer (BGR555, 240×160 row-major) from the last finished frame.
    pub fn framebuffer(&self) -> Ref<'_, [u16]> {
        Ref::map(self.io_devs.borrow(), |io| io.ppu.framebuffer())
    }

    /// Enables or disables CPU disassembly output.
    pub fn set_disasm_enabled(&mut self, enabled: bool) {
        self.cpu.set_disasm_enabled(enabled);
    }

    pub fn reset(&mut self) {
        // Bus must be reset before CPU so the pipeline fill reads valid memory
        // self.bus.reset();
        // self.apu.reset();
        // self.ppu.reset();
        // self.timer.reset();
        // self.cpu.reset(&mut self.bus);
    }
}
