use minifb::{Key, Window, WindowOptions, Scale, ScaleMode};
pub use crate::emulator::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

pub fn run_app<F: FnMut(&mut Window, &mut [u32])>(mut update: F) {
    let mut buffer = vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT];
    let options = WindowOptions {
        resize: true,
        scale: Scale::X1,
        scale_mode: ScaleMode::AspectRatioStretch,
        ..WindowOptions::default()
    };
    let mut window = Window::new(
        "GBAZ",
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        options,
    )
    .unwrap_or_else(|e| {
        panic!("Unable to create window: {}", e);
    });

    while window.is_open() && !window.is_key_down(Key::Escape) {
        update(&mut window, &mut buffer);
        window.update_with_buffer(&buffer, SCREEN_WIDTH, SCREEN_HEIGHT).unwrap();
    }
}