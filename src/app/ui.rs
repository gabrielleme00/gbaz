use eframe::egui;

use crate::config::EmulatorConfig;
use crate::emulator::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::emulator::{Button, InputState};

use super::state::{AudioDiag, EmulatorState};

const KEY_MAP: &[(egui::Key, Button)] = &[
    (egui::Key::X, Button::A),
    (egui::Key::Z, Button::B),
    (egui::Key::Backspace, Button::Select),
    (egui::Key::Enter, Button::Start),
    (egui::Key::ArrowRight, Button::Right),
    (egui::Key::ArrowLeft, Button::Left),
    (egui::Key::ArrowUp, Button::Up),
    (egui::Key::ArrowDown, Button::Down),
    (egui::Key::S, Button::R),
    (egui::Key::A, Button::L),
];

const BYTES_PER_ROW: usize = 16;
/// Maximum bytes displayed per page (keeps pre-read within ~256 KB).
const PAGE_SIZE: u32 = 256 * 1024;
/// (label, base_addr, region_size)
const MEM_REGIONS: &[(&str, u32, u32)] = &[
    ("BIOS",  0x0000_0000, 0x0000_4000),
    ("EWRAM", 0x0200_0000, 0x0004_0000),
    ("IWRAM", 0x0300_0000, 0x0000_8000),
    ("IO",    0x0400_0000, 0x0000_0400),
    ("PRAM",  0x0500_0000, 0x0000_0400),
    ("VRAM",  0x0600_0000, 0x0001_8000),
    ("OAM",   0x0700_0000, 0x0000_0400),
    ("ROM",   0x0800_0000, 0x0200_0000),
];

struct MemViewerState {
    addr_input: String,
    /// Base address of the currently selected region.
    base_addr: u32,
    /// Total byte size of the region.
    region_size: u32,
    /// Byte offset within the region for the top of the displayed page.
    page_offset: u32,
    /// If set, scroll the view to this row on the next frame.
    scroll_to_row: Option<usize>,
}

impl Default for MemViewerState {
    fn default() -> Self {
        Self {
            addr_input: "02000000".to_owned(),
            base_addr: 0x0200_0000,
            region_size: 0x0004_0000,
            page_offset: 0,
            scroll_to_row: None,
        }
    }
}

pub struct GbazApp {
    state: EmulatorState,
    config: EmulatorConfig,
    screen_image: egui::ColorImage,
    screen_texture: Option<egui::TextureHandle>,
    mem_viewer: MemViewerState,
}

impl GbazApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        rom_path: Option<&str>,
        bios_path_override: Option<&str>,
    ) -> Self {
        let mut config = EmulatorConfig::load();

        // CLI arg takes precedence over saved config
        if let Some(p) = bios_path_override {
            config.bios_path = Some(std::path::PathBuf::from(p));
        }

        let rom_data = rom_path.and_then(|p| {
            std::fs::read(p)
                .map_err(|e| eprintln!("Failed to read ROM '{p}': {e}"))
                .ok()
        });

        let mut state = EmulatorState::new(rom_data, config.bios_path.clone());
        if let Some(p) = rom_path {
            state.set_rom_path(std::path::PathBuf::from(p));
            state.load_save();
        }

        Self {
            state,
            config,
            screen_image: egui::ColorImage::new(
                [SCREEN_WIDTH, SCREEN_HEIGHT],
                vec![egui::Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT],
            ),
            screen_texture: None,
            mem_viewer: MemViewerState::default(),
        }
    }

    fn load_rom_from_path(&mut self, path: std::path::PathBuf) {
        match std::fs::read(&path) {
            Ok(data) => {
                self.state.set_rom_path(path);
                self.state.load_rom(data);
            }
            Err(e) => {
                self.state.error_message = Some(format!("Failed to read ROM: {e}"));
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let mut input = InputState::default();
        ctx.input(|i| {
            for &(key, button) in KEY_MAP {
                input.set_pressed(button, i.key_down(key));
            }
        });
        self.state.set_input(input);
    }

    fn update_screen_texture(&mut self, ctx: &egui::Context) {
        let fb = self
            .state
            .emulator
            .as_ref()
            .map(|emu| emu.framebuffer().to_vec());

        if let Some(fb) = fb {
            for (pixel, &raw) in self.screen_image.pixels.iter_mut().zip(fb.iter()) {
                let r = ((raw >> 16) & 0xff) as u8;
                let g = ((raw >> 8) & 0xff) as u8;
                let b = (raw & 0xff) as u8;
                *pixel = egui::Color32::from_rgb(r, g, b);
            }
        }

        match &mut self.screen_texture {
            Some(tex) => tex.set(self.screen_image.clone(), egui::TextureOptions::NEAREST),
            None => {
                self.screen_texture = Some(ctx.load_texture(
                    "gba_screen",
                    self.screen_image.clone(),
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
    }

    fn show_screen(&self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                let available = ui.available_size();
                let scale_x = (available.x / SCREEN_WIDTH as f32).floor();
                let scale_y = (available.y / SCREEN_HEIGHT as f32).floor();
                let scale = scale_x.min(scale_y).max(1.0);
                let display_size =
                    egui::vec2(SCREEN_WIDTH as f32 * scale, SCREEN_HEIGHT as f32 * scale);
                let texture = self.screen_texture.as_ref().unwrap();
                ui.image((texture.id(), display_size));
            });
        });
    }

    fn show_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.set_min_size(egui::vec2(100.0, 0.0));

                    if ui.button("Open ROM...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("GBA ROM", &["gba"])
                            .set_title("Open GBA ROM")
                            .pick_file()
                        {
                            self.load_rom_from_path(path);
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Emulation", |ui| {
                    ui.set_min_size(egui::vec2(100.0, 0.0));

                    let label = if self.state.running {
                        "Pause"
                    } else {
                        "Resume"
                    };
                    if ui.button(label).clicked() {
                        self.state.toggle_pause();
                        ui.close();
                    }

                    ui.separator();
                    ui.checkbox(&mut self.state.audio_debug, "Audio debug");
                    ui.checkbox(&mut self.state.cpu_debug, "CPU debug");
                    ui.checkbox(&mut self.state.mem_debug, "Memory viewer");
                });

                ui.menu_button("Settings", |ui| {
                    ui.set_min_size(egui::vec2(180.0, 0.0));

                    // --- Audio ---
                    ui.label(egui::RichText::new("Audio").strong());

                    let mut muted = self.state.muted();
                    if ui.checkbox(&mut muted, "Mute").changed() {
                        self.state.set_muted(muted);
                    }

                    ui.add_enabled_ui(!muted, |ui| {
                        let mut volume = self.state.volume();
                        let slider = egui::Slider::new(&mut volume, 0.0..=2.0)
                            .text("Volume")
                            .step_by(0.05);
                        if ui.add(slider).changed() {
                            self.state.set_volume(volume);
                        }
                    });

                    ui.separator();

                    let bios_label = self
                        .config
                        .bios_path
                        .as_deref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "None".to_string());

                    if ui.button(format!("BIOS: {bios_label}")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("GBA BIOS", &["bin", "bios"])
                            .set_title("Select GBA BIOS")
                            .pick_file()
                        {
                            self.config.bios_path = Some(path.clone());
                            self.state.bios_path = Some(path);
                            if let Err(e) = self.config.save() {
                                self.state.error_message = Some(e);
                            }
                        }
                        ui.close();
                    }
                });
            });
        });
    }

    fn show_error_popup(&mut self, ctx: &egui::Context) {
        if let Some(err) = self.state.error_message.clone() {
            egui::Window::new("Error")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(&err);
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() {
                        self.state.error_message = None;
                    }
                });
        }
    }

    fn show_cpu_debug(&mut self, ctx: &egui::Context) {
        const DISASM_ROWS: usize = 24;
        const REG_NAMES: [&str; 16] = [
            "R0", "R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "R9", "R10", "R11", "R12", "SP",
            "LR", "PC",
        ];
        const MODE_NAMES: [(u32, &str); 7] = [
            (0x10, "USR"),
            (0x11, "FIQ"),
            (0x12, "IRQ"),
            (0x13, "SVC"),
            (0x17, "ABT"),
            (0x1B, "UND"),
            (0x1F, "SYS"),
        ];

        let Some(emu) = &self.state.emulator else {
            return;
        };

        let pc = emu.execute_addr();
        let cpsr = emu.cpsr();
        let thumb = emu.is_thumb_mode();
        let regs: Vec<u32> = (0..16).map(|i| emu.reg(i)).collect();
        let disasm = emu.disassemble(pc, DISASM_ROWS);

        let running = self.state.running;

        let flag = |bit: u32, ch: char| -> egui::RichText {
            let set = cpsr & bit != 0;
            let s = if set {
                ch.to_uppercase().next().unwrap()
            } else {
                ch
            };
            let color = if set {
                egui::Color32::from_rgb(100, 220, 100)
            } else {
                egui::Color32::GRAY
            };
            egui::RichText::new(s.to_string()).color(color).monospace()
        };

        let mode_str = MODE_NAMES
            .iter()
            .find(|(v, _)| *v == cpsr & 0x1F)
            .map(|(_, n)| *n)
            .unwrap_or("???");

        egui::Window::new("CPU Debug")
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                // Control buttons
                ui.horizontal(|ui| {
                    let run_label = if running { "⏸ Break" } else { "▶ Run" };
                    if ui.button(run_label).clicked() {
                        self.state.toggle_pause();
                    }
                    ui.add_enabled_ui(!running, |ui| {
                        if ui.button("⏭ Step").clicked() {
                            self.state.step_instruction();
                        }
                    });
                    ui.separator();
                    ui.label(
                        egui::RichText::new(if running { "RUNNING" } else { "PAUSED" })
                            .color(if running {
                                egui::Color32::from_rgb(100, 220, 100)
                            } else {
                                egui::Color32::YELLOW
                            })
                            .strong(),
                    );
                });

                ui.separator();

                ui.horizontal_top(|ui| {
                    // Fixed-width left panel: Registers + Flags
                    ui.allocate_ui(egui::vec2(140.0, 0.0), |ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new("Registers").strong());
                            egui::Grid::new("regs_grid")
                                .num_columns(2)
                                .spacing([8.0, 2.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for (i, &v) in regs.iter().enumerate() {
                                        ui.label(
                                            egui::RichText::new(REG_NAMES[i])
                                                .monospace()
                                                .color(egui::Color32::from_rgb(150, 180, 255)),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!("{v:08X}")).monospace(),
                                        );
                                        ui.end_row();
                                    }
                                });

                            ui.add_space(6.0);

                            ui.label(egui::RichText::new("Flags").strong());
                            ui.columns_const(|[col_1, col_2]| {
                                col_1.horizontal(|ui| {
                                    ui.label(flag(1 << 31, 'n'));
                                    ui.label(flag(1 << 30, 'z'));
                                    ui.label(flag(1 << 29, 'c'));
                                    ui.label(flag(1 << 28, 'v'));
                                });
                                col_1.label(
                                    egui::RichText::new(format!("{:08X}", cpsr)).monospace(),
                                );

                                col_2.label(
                                    egui::RichText::new(if thumb { "THUMB" } else { "ARM" })
                                        .monospace()
                                        .color(egui::Color32::from_rgb(255, 200, 80)),
                                );
                                col_2.label(egui::RichText::new(mode_str).monospace());
                            });
                        });
                    });

                    ui.separator();

                    // Disassembly takes all remaining space
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Disassembly").strong());
                        egui::ScrollArea::vertical()
                            .id_salt("disasm_scroll")
                            .show(ui, |ui| {
                                ui.take_available_width();
                                for (addr, _raw, text) in &disasm {
                                    let is_pc = *addr == pc;
                                    let mnem = text
                                        .splitn(3, ' ')
                                        .nth(2)
                                        .map(str::trim_start)
                                        .unwrap_or(text.as_str());
                                    let row = egui::RichText::new(format!("{addr:08X}  {mnem}"))
                                        .monospace();
                                    let row = if is_pc {
                                        row.color(egui::Color32::from_rgb(255, 230, 80)).strong()
                                    } else {
                                        row.color(egui::Color32::LIGHT_GRAY)
                                    };
                                    ui.label(row);
                                }
                            });
                    });
                });
            });
    }

    fn show_memory_viewer(&mut self, ctx: &egui::Context) {
        // Snapshot mutable state we need before any borrows
        let scroll_to_row = self.mem_viewer.scroll_to_row.take();
        let base_addr = self.mem_viewer.base_addr;
        let region_size = self.mem_viewer.region_size;
        let page_offset = self.mem_viewer.page_offset;

        // Pre-read the page bytes so we hold no emulator borrow inside the closure.
        let page_start = base_addr.saturating_add(page_offset);
        let page_byte_count = (region_size.saturating_sub(page_offset)).min(PAGE_SIZE) as usize;
        let region_data: Vec<u8> = match &self.state.emulator {
            Some(emu) => (0..page_byte_count as u32)
                .map(|i| emu.read_byte(page_start.wrapping_add(i)))
                .collect(),
            None => return,
        };
        // emu borrow ends here — &mut self is now freely available inside the closure.

        let num_rows = page_byte_count.div_ceil(BYTES_PER_ROW);
        let total_pages = region_size.div_ceil(PAGE_SIZE);
        let current_page = page_offset / PAGE_SIZE;

        egui::Window::new("Memory Viewer")
            .resizable(true)
            .default_width(680.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                // Region quick-select
                ui.horizontal_wrapped(|ui| {
                    for &(name, rbase, rsize) in MEM_REGIONS {
                        let active = self.mem_viewer.base_addr == rbase;
                        if ui.selectable_label(active, name).clicked() {
                            self.mem_viewer.base_addr = rbase;
                            self.mem_viewer.region_size = rsize;
                            self.mem_viewer.page_offset = 0;
                            self.mem_viewer.addr_input = format!("{:08X}", rbase);
                        }
                    }
                });

                // Address jump + page navigation
                ui.horizontal(|ui| {
                    ui.label("Jump to:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.mem_viewer.addr_input)
                            .desired_width(82.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    let go = ui.button("Go").clicked()
                        || (resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go {
                        let s = self
                            .mem_viewer
                            .addr_input
                            .trim_start_matches("0x")
                            .trim_start_matches("0X");
                        if let Ok(target) = u32::from_str_radix(s, 16) {
                            for &(_, rbase, rsize) in MEM_REGIONS {
                                if target >= rbase && target < rbase + rsize {
                                    self.mem_viewer.base_addr = rbase;
                                    self.mem_viewer.region_size = rsize;
                                    let off = target - rbase;
                                    self.mem_viewer.page_offset =
                                        (off / PAGE_SIZE * PAGE_SIZE).min(rsize.saturating_sub(1));
                                    let row_in_page =
                                        (off as usize % PAGE_SIZE as usize) / BYTES_PER_ROW;
                                    self.mem_viewer.scroll_to_row = Some(row_in_page);
                                    break;
                                }
                            }
                        }
                    }

                    if total_pages > 1 {
                        ui.separator();
                        ui.label(format!("Page {}/{}", current_page + 1, total_pages));
                        ui.add_enabled_ui(page_offset > 0, |ui| {
                            if ui.button("◀").clicked() {
                                self.mem_viewer.page_offset =
                                    page_offset.saturating_sub(PAGE_SIZE);
                            }
                        });
                        ui.add_enabled_ui(
                            page_offset + PAGE_SIZE < region_size,
                            |ui| {
                                if ui.button("▶").clicked() {
                                    self.mem_viewer.page_offset =
                                        (page_offset + PAGE_SIZE).min(region_size - 1);
                                }
                            },
                        );
                    }
                });

                ui.separator();

                // Column header
                ui.label(
                    egui::RichText::new(
                        "          +0 +1 +2 +3 +4 +5 +6 +7  +8 +9 +A +B +C +D +E +F  ASCII",
                    )
                    .monospace()
                    .color(egui::Color32::from_rgb(160, 160, 90))
                    .size(12.0),
                );

                // Hex dump (virtual scroll)
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 1.0;
                let mut scroll_area = egui::ScrollArea::vertical()
                    .id_salt("mem_viewer_scroll")
                    .auto_shrink([false; 2]);
                if let Some(row) = scroll_to_row {
                    scroll_area =
                        scroll_area.vertical_scroll_offset(row as f32 * row_height);
                }
                scroll_area.show_rows(ui, row_height, num_rows, |ui, row_range| {
                    for row in row_range {
                        let row_addr =
                            page_start.wrapping_add((row * BYTES_PER_ROW) as u32);
                        let base_off = row * BYTES_PER_ROW;

                        let mut hex_part = String::with_capacity(50);
                        let mut ascii_part = String::with_capacity(BYTES_PER_ROW);
                        for col in 0..BYTES_PER_ROW {
                            if col == 8 {
                                hex_part.push(' ');
                            }
                            let byte = region_data
                                .get(base_off + col)
                                .copied()
                                .unwrap_or(0xFF);
                            hex_part.push_str(&format!("{byte:02X} "));
                            ascii_part.push(
                                if (0x20u8..0x7F).contains(&byte) {
                                    byte as char
                                } else {
                                    '.'
                                },
                            );
                        }

                        let row_text = format!(
                            "{row_addr:08X}  {}  {ascii_part}",
                            hex_part.trim_end()
                        );
                        ui.label(
                            egui::RichText::new(row_text)
                                .monospace()
                                .size(12.0)
                                .color(egui::Color32::LIGHT_GRAY),
                        );
                    }
                });
            });
    }

    fn show_audio_debug(&self, ctx: &egui::Context) {
        let diag: Option<AudioDiag> = self.state.audio_diag();
        egui::Window::new("Audio Debug")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 32.0))
            .show(ctx, |ui| match diag {
                None => {
                    ui.label("Audio disabled");
                }
                Some(d) => {
                    let fill = d.buf_frames as f32 / d.buf_cap as f32;
                    ui.label(format!(
                        "Buffer: {:.1} ms  ({}/{})",
                        d.buf_ms, d.buf_frames, d.buf_cap
                    ));
                    ui.add(
                        egui::ProgressBar::new(fill)
                            .desired_width(200.0)
                            .text(format!("{:.0}%", fill * 100.0)),
                    );
                    ui.add_space(4.0);
                    let under_color = if d.underflows > 0 {
                        egui::Color32::from_rgb(255, 80, 80)
                    } else {
                        egui::Color32::GREEN
                    };
                    let over_color = if d.overflows > 0 {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::GREEN
                    };
                    ui.colored_label(under_color, format!("Underflows: {}", d.underflows));
                    ui.colored_label(over_color, format!("Overflows:  {}", d.overflows));
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(if d.underflows > 0 {
                            "⚠ Underflows → emulator too slow / buffer starved"
                        } else if d.overflows > 0 {
                            "⚠ Overflows → emulator too fast / buffer full"
                        } else {
                            "✓ No drops detected"
                        })
                        .small(),
                    );
                }
            });
    }
}

impl eframe::App for GbazApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);
        self.state.step_frame();
        self.update_screen_texture(ctx);
        self.show_menu(ctx);
        self.show_screen(ctx);
        self.show_error_popup(ctx);
        if self.state.audio_debug {
            self.show_audio_debug(ctx);
        }
        if self.state.cpu_debug {
            self.show_cpu_debug(ctx);
        }
        if self.state.mem_debug {
            self.show_memory_viewer(ctx);
        }
        ctx.request_repaint();
    }
}
