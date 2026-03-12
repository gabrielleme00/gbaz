use std::env;
use std::fs;
use std::process::ExitCode;

use gbaz::emulator::Emulator;
use gbaz::app::run_app;

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

    run_app(|_window, buffer| {
        emulator.run_frame();
        // Convert BGR555 (GBA) → 0x00RRGGBB (minifb).
        // BGR555: bits [4:0]=R, [9:5]=G, [14:10]=B, each 5-bit → 8-bit by << 3.
        for (dst, &pixel) in buffer.iter_mut().zip(emulator.framebuffer().iter()) {
            let r = ((pixel & 0x1F) as u32) << 3;
            let g = (((pixel >> 5) & 0x1F) as u32) << 3;
            let b = (((pixel >> 10) & 0x1F) as u32) << 3;
            *dst = (r << 16) | (g << 8) | b;
        }
    });

    ExitCode::SUCCESS
}
