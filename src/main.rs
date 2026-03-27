use std::env;
use std::fs;
use std::process::ExitCode;

use gbaz::emulator::{Button, Emulator, InputState};
use gbaz::app::run_app;
use minifb::Key;

fn main() -> ExitCode {
    let mut args = env::args();
    let program = args.next().unwrap_or_else(|| "gbaz".to_string());

    let mut rom_path: Option<String> = None;
    let mut bios_path: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-b" | "--bios" => match args.next() {
                Some(path) => bios_path = Some(path),
                None => {
                    eprintln!("Error: {arg} requires a path argument");
                    return ExitCode::from(2);
                }
            },
            _ if !arg.starts_with('-') => rom_path = Some(arg),
            _ => {
                eprintln!("Unknown flag: {arg}");
                eprintln!("Usage: {program} [-b <bios.bin>] <rom.gba>");
                return ExitCode::from(2);
            }
        }
    }

    let Some(rom_path) = rom_path else {
        eprintln!("Usage: {program} [-b <bios.bin>] <rom.gba>");
        return ExitCode::from(2);
    };

    let rom = match fs::read(&rom_path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Failed to read ROM '{rom_path}': {err}");
            return ExitCode::from(1);
        }
    };

    let bios = match bios_path {
        Some(ref path) => match fs::read(path) {
            Ok(data) => Some(data),
            Err(err) => {
                eprintln!("Failed to read BIOS '{path}': {err}");
                return ExitCode::from(1);
            }
        },
        None => None,
    };

    let mut emulator = Emulator::new(rom, bios);

    run_app(|window, buffer| {
        let mut input = InputState::default();
        let key_map: &[(Key, Button)] = &[
            (Key::X,         Button::A),
            (Key::Z,         Button::B),
            (Key::Backspace, Button::Select),
            (Key::Enter,     Button::Start),
            (Key::Right,     Button::Right),
            (Key::Left,      Button::Left),
            (Key::Up,        Button::Up),
            (Key::Down,      Button::Down),
            (Key::S,         Button::R),
            (Key::A,         Button::L),
        ];
        for &(key, button) in key_map {
            input.set_pressed(button, window.is_key_down(key));
        }
        emulator.set_input(input);
        emulator.run_frame();

        for (dst, &pixel) in buffer.iter_mut().zip(emulator.framebuffer().iter()) {
            *dst = pixel;
        }
    });

    ExitCode::SUCCESS
}
