pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod dma;
pub mod input;
pub mod interrupt;
pub mod iodev;
pub mod memory;
pub mod ppu;
pub mod timer;

pub use apu::Apu;
pub use bus::Bus;
pub use cartridge::Cartridge;
pub use cpu::Cpu;
pub use dma::{Dma, DmaEvent, DmaRunParams};
pub use input::{Button, InputState};
pub use interrupt::InterruptController;
pub use iodev::IoDevices;
pub use memory::MemoryInterface;
pub use ppu::Ppu;
pub use timer::Timer;

use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

/// Coordinates all emulated hardware blocks and advances them in lockstep.
pub struct Emulator {
    cpu: Box<Cpu>,
    bus: Rc<RefCell<Bus>>,
    pub io_devs: Rc<RefCell<IoDevices>>,
}

impl Emulator {
    pub fn new(rom: Vec<u8>, bios: Option<Vec<u8>>) -> Self {
        let cartridge = Cartridge::from_rom(rom);
        let has_bios = bios.is_some();

        let interrupt_flags = Rc::new(RefCell::new(0));

        let intc = InterruptController::new(interrupt_flags.clone());
        let ppu = Box::new(Ppu::new(interrupt_flags.clone()));
        let apu = Apu::new();
        let dma = Dma::new(interrupt_flags.clone());
        let timer = Timer::new();

        let io_devs = Rc::new(RefCell::new(IoDevices::new(intc, ppu, apu, dma, timer)));
        let bus = Rc::new(RefCell::new(Bus::new(cartridge, io_devs.clone(), bios)));
        let cpu = Box::new(Cpu::new(bus.clone()));

        let mut emu = Self { cpu, bus, io_devs };

        emu.reset();
        if !has_bios {
            emu.skip_bios();
        }
        emu.set_disasm_enabled(false);
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
        // Immediate DMA suspends the CPU until the transfer completes.
        self.run_dma_event(DmaEvent::Immediate);

        let cycles = self.cpu.step();
        for _ in 0..cycles {
            self.bus.borrow_mut().tick();

            // Consume edge-triggered DMA events from the PPU.
            let (hblank, vblank) = {
                let mut io = self.io_devs.borrow_mut();
                (
                    io.ppu.take_hblank_dma_trigger(),
                    io.ppu.take_vblank_dma_trigger(),
                )
            };

            if hblank {
                self.run_dma_event(DmaEvent::HBlank);

                // Video capture (DMA3 Special): fires each scanline for VCOUNT 2..=161,
                // then auto-disables at VCOUNT 162.
                let vcount = self.io_devs.borrow().ppu.vcount();
                if vcount == 162 {
                    self.io_devs.borrow_mut().dma.stop_video_capture();
                } else if vcount >= 2
                    && vcount < 162
                    && self
                        .io_devs
                        .borrow()
                        .dma
                        .channel_wants_run(3, DmaEvent::Special)
                {
                    self.run_dma_channel(3);
                }
            }

            if vblank {
                self.run_dma_event(DmaEvent::VBlank);
            }
        }
    }

    /// Fire all enabled channels that match `event`.
    fn run_dma_event(&mut self, event: DmaEvent) {
        for ch in 0..4_usize {
            if self.io_devs.borrow().dma.channel_wants_run(ch, event) {
                self.run_dma_channel(ch);
            }
        }
    }

    /// Execute one burst for channel `ch` using the bus, then update DMA state.
    fn run_dma_channel(&mut self, ch: usize) {
        let params = self.io_devs.borrow().dma.run_params(ch);
        let (new_src, new_dst) = self.do_dma_burst(&params);
        self.io_devs
            .borrow_mut()
            .dma
            .finish_channel(ch, new_src, new_dst);
    }

    /// Execute the raw memory transfers described by `params`.
    /// Returns the (src, dst) addresses after the last unit was transferred.
    fn do_dma_burst(&mut self, params: &DmaRunParams) -> (u32, u32) {
        let mut src = params.src;
        let mut dst = params.dst;
        for _ in 0..params.count {
            if params.is_word {
                let val = self.bus.borrow().read_32(src);
                self.bus.borrow_mut().write_32(dst, val);
            } else {
                let val = self.bus.borrow().read_16(src);
                self.bus.borrow_mut().write_16(dst, val);
            }
            src = src.wrapping_add_signed(params.src_step);
            dst = dst.wrapping_add_signed(params.dst_step);
        }
        (src, dst)
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
    pub fn framebuffer(&self) -> Ref<'_, [u32]> {
        Ref::map(self.io_devs.borrow(), |io| io.ppu.get_frame_buffer())
    }

    /// Updates the button state seen by KEYINPUT (0x0400_0130).
    pub fn set_input(&mut self, input: InputState) {
        self.io_devs.borrow_mut().input = input;
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
