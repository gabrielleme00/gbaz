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
pub use cpu::{disasm_arm, disasm_thumb};
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
    cpu: Cpu,
    bus: Rc<RefCell<Bus>>,
    pub io_devs: Rc<RefCell<IoDevices>>,
}

impl Emulator {
    pub fn new(rom: Vec<u8>, bios: Option<Vec<u8>>) -> Self {
        let cartridge = Cartridge::from_rom(rom);
        let has_bios = bios.is_some();

        let interrupt_flags = Rc::new(RefCell::new(0));

        let intc = InterruptController::new(interrupt_flags.clone());
        let ppu = Ppu::new(interrupt_flags.clone());
        let apu = Apu::new();
        let dma = Dma::new(interrupt_flags.clone());
        let timer = Timer::new(interrupt_flags.clone());

        let io_devs = Rc::new(RefCell::new(IoDevices::new(intc, ppu, apu, dma, timer)));
        let bus = Rc::new(RefCell::new(Bus::new(cartridge, io_devs.clone(), bios)));
        let cpu = Cpu::new(bus.clone());

        let mut emu = Self { cpu, bus, io_devs };

        emu.reset();
        if !has_bios {
            emu.skip_bios();
        }
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

            // Consume all edge-triggered flags
            let (hblank, vblank, fifo_flags, vcount) = {
                let mut io = self.io_devs.borrow_mut();
                let hblank = io.ppu.take_hblank_dma_trigger();
                let vblank = io.ppu.take_vblank_dma_trigger();
                let fifo_flags = io.take_sound_dma_flags();
                let vcount = io.ppu.vcount();
                (hblank, vblank, fifo_flags, vcount)
            };

            if hblank {
                self.run_dma_event(DmaEvent::HBlank);

                // Video capture (DMA3 Special): fires each scanline for VCOUNT 2..=161,
                // then auto-disables at VCOUNT 162.
                if vcount == 162 {
                    self.io_devs.borrow_mut().dma.stop_video_capture();
                } else if vcount >= 2
                    && vcount < 162
                    && self.io_devs.borrow().dma.channel_wants_run(3, DmaEvent::Special)
                {
                    self.run_dma_channel(3);
                }
            }

            if vblank {
                self.run_dma_event(DmaEvent::VBlank);
            }

            // Sound FIFO DMA: channels 1 and 2 carry FIFO A and B respectively.
            if fifo_flags != 0 {
                let (wants1, wants2) = {
                    let io = self.io_devs.borrow();
                    (
                        fifo_flags & 1 != 0 && io.dma.channel_wants_run(1, DmaEvent::Special),
                        fifo_flags & 2 != 0 && io.dma.channel_wants_run(2, DmaEvent::Special),
                    )
                };
                if wants1 { self.run_dma_channel(1); }
                if wants2 { self.run_dma_channel(2); }
            }
        }
    }

    /// Fire all enabled channels that match `event`.
    fn run_dma_event(&mut self, event: DmaEvent) {
        let wants = {
            let io = self.io_devs.borrow();
            [
                io.dma.channel_wants_run(0, event),
                io.dma.channel_wants_run(1, event),
                io.dma.channel_wants_run(2, event),
                io.dma.channel_wants_run(3, event),
            ]
        };
        for (ch, run) in wants.into_iter().enumerate() {
            if run {
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
        let mut bus = self.bus.borrow_mut();
        for _ in 0..params.count {
            if params.is_word {
                let val = bus.read_32(src);
                bus.write_32(dst, val);
            } else {
                let val = bus.read_16(src);
                bus.write_16(dst, val);
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

    /// Returns the current CPSR value.
    pub fn cpsr(&self) -> u32 {
        self.cpu.cpsr()
    }

    /// Returns true if the CPU is in Thumb mode.
    pub fn is_thumb_mode(&self) -> bool {
        self.cpu.is_thumb_mode()
    }

    /// Disassembles `count` instructions starting at `addr`, reading from the bus.
    /// Returns a Vec of (address, raw_bits, mnemonic_string).
    pub fn disassemble(&self, addr: u32, count: usize) -> Vec<(u32, u32, String)> {
        let bus = self.bus.borrow();
        let thumb = self.cpu.is_thumb_mode();
        let mut pc = addr;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if thumb {
                let raw = bus.read_16(pc) as u32;
                let s = disasm_thumb(pc, raw as u16);
                out.push((pc, raw, s));
                pc = pc.wrapping_add(2);
            } else {
                let raw = bus.read_32(pc);
                let s = disasm_arm(pc, raw);
                out.push((pc, raw, s));
                pc = pc.wrapping_add(4);
            }
        }
        out
    }

    /// Returns the completed framebuffer (BGR555, 240×160 row-major) from the last finished frame.
    pub fn framebuffer(&self) -> Ref<'_, [u32]> {
        Ref::map(self.io_devs.borrow(), |io| io.ppu.get_frame_buffer())
    }

    /// Updates the button state seen by KEYINPUT (0x0400_0130).
    pub fn set_input(&mut self, input: InputState) {
        self.io_devs.borrow_mut().input = input;
    }

    /// Returns the raw save-backup bytes for the loaded cartridge, if any.
    pub fn save_data(&self) -> Option<Vec<u8>> {
        self.bus.borrow().cartridge_save_data()
    }

    /// Restores backup storage from previously-persisted save bytes.
    pub fn load_save(&mut self, data: &[u8]) {
        self.bus.borrow_mut().load_cartridge_save(data);
    }

    pub fn is_save_dirty(&self) -> bool {
        self.bus.borrow().is_save_dirty()
    }

    pub fn clear_save_dirty(&mut self) {
        self.bus.borrow_mut().clear_save_dirty();
    }

    /// Configures the APU output sample rate to match the audio backend.
    pub fn set_audio_sample_rate(&mut self, rate: u32) {
        self.io_devs.borrow_mut().apu.set_sample_rate(rate);
    }

    /// Drains APU-generated samples into `buf`.
    pub fn drain_audio_samples(&mut self, buf: &mut Vec<f32>) {
        self.io_devs.borrow_mut().apu.drain_samples(buf);
    }

    /// Reads a single byte from the bus at `addr` without side-effects.
    pub fn read_byte(&self, addr: u32) -> u8 {
        self.bus.borrow().read_8(addr)
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
