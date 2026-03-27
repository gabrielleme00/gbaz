use eframe::egui;
use gbaz::app::GbazApp;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut rom_path: Option<String> = None;
    let mut bios_path: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-b" | "--bios" => {
                i += 1;
                if i < args.len() {
                    bios_path = Some(args[i].clone());
                }
            }
            arg if !arg.starts_with('-') => rom_path = Some(arg.to_string()),
            _ => {}
        }
        i += 1;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 680.0])
            .with_title("GBAZ - GBA Emulator"),
        ..Default::default()
    };

    eframe::run_native(
        "GBAZ",
        options,
        Box::new(move |cc| {
            Ok(Box::new(GbazApp::new(cc, rom_path.as_deref(), bios_path.as_deref())))
        }),
    )
}
