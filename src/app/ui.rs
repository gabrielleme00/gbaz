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

pub struct GbazApp {
    state: EmulatorState,
    config: EmulatorConfig,
    screen_image: egui::ColorImage,
    screen_texture: Option<egui::TextureHandle>,
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

        Self {
            state: EmulatorState::new(rom_data, config.bios_path.clone()),
            config,
            screen_image: egui::ColorImage::new(
                [SCREEN_WIDTH, SCREEN_HEIGHT],
                vec![egui::Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT],
            ),
            screen_texture: None,
        }
    }

    fn load_rom_from_path(&mut self, path: std::path::PathBuf) {
        match std::fs::read(&path) {
            Ok(data) => self.state.load_rom(data),
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
                });

                ui.menu_button("Settings", |ui| {
                    ui.set_min_size(egui::vec2(100.0, 0.0));

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

    fn show_audio_debug(&self, ctx: &egui::Context) {
        let diag: Option<AudioDiag> = self.state.audio_diag();
        egui::Window::new("Audio Debug")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 32.0))
            .show(ctx, |ui| {
                match diag {
                    None => { ui.label("Audio disabled"); }
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
                        ui.colored_label(over_color,  format!("Overflows:  {}", d.overflows));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(
                            if d.underflows > 0 {
                                "⚠ Underflows → emulator too slow / buffer starved"
                            } else if d.overflows > 0 {
                                "⚠ Overflows → emulator too fast / buffer full"
                            } else {
                                "✓ No drops detected"
                            }
                        ).small());
                    }
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
        ctx.request_repaint();
    }
}
