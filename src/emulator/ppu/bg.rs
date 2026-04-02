use super::*;
use crate::emulator::bus::regions::consts::VRAM_ADDR;

pub type Point = (i32, i32);

fn transform_bg_point(ref_point: Point, screen_x: i32, pa: i32, pc: i32) -> Point {
    let (ref_x, ref_y) = ref_point;
    ((ref_x + screen_x * pa) >> 8, (ref_y + screen_x * pc) >> 8)
}

struct ViewPort {
    pub origin: Point,
    pub w: i32,
    pub h: i32,
}

impl ViewPort {
    pub fn new(w: i32, h: i32) -> Self {
        Self {
            origin: (0, 0),
            w,
            h,
        }
    }

    pub fn contains_point(&self, p: Point) -> bool {
        let (mut x, mut y) = p;

        x -= self.origin.0;
        y -= self.origin.1;

        x >= 0 && x < self.w && y >= 0 && y < self.h
    }
}

static SCREEN_VIEWPORT: ViewPort = ViewPort {
    origin: (0, 0),
    w: SCREEN_WIDTH as i32,
    h: SCREEN_HEIGHT as i32,
};

static MODE5_VIEWPORT: ViewPort = ViewPort {
    origin: (0, 0),
    w: 160,
    h: 128,
};

impl Ppu {
    pub fn render_reg_bg_if_enabled(&mut self, bg: usize) {
        if self.is_bg_enabled(bg) {
            self.render_reg_bg(bg);
        }
    }

    pub fn render_aff_bg_if_enabled(&mut self, bg: usize) {
        if self.is_bg_enabled(bg) {
            self.render_aff_bg(bg);
        }
    }

    /// Regular BG with per-line horizontal scrolling and optional vertical scrolling.
    pub fn render_reg_bg(&mut self, bg: usize) {
        let (h_ofs, v_ofs) = (self.bg_hofs[bg] as u32, self.bg_vofs[bg] as u32);
        let tileset_base = self.bgcnt[bg].char_block();
        let tilemap_base = self.bgcnt[bg].screen_block();
        let (tile_size, pixel_format) = self.bgcnt[bg].tile_format();

        let (bg_width, bg_height) = self.bgcnt[bg].size_regular();

        let screen_y = self.vcount as u32;
        let mut screen_x = 0;

        // Calculate the bg coords at the top-left corner, including wraparound
        let bg_x = (screen_x + h_ofs) % bg_width;
        let bg_y = (screen_y + v_ofs) % bg_height;

        // Calculate the initial screen entry index
        let mut sbb = match (bg_width, bg_height) {
            (256, 256) => 0,
            (512, 256) => bg_x / 256,
            (256, 512) => bg_y / 256,
            (512, 512) => index2d!(u32, bg_x / 256, bg_y / 256, 2),
            _ => unreachable!(),
        } as u32;

        let mut se_row = (bg_x / 8) % 32;
        let se_column = (bg_y / 8) % 32;

        // This will be non-zero if the h-scroll lands in a middle of a tile
        let mut start_tile_x = bg_x % 8;
        let tile_py = bg_y % 8;

        macro_rules! render_loop {
            ($read_pixel_index:ident) => {
                loop {
                    let mut map_addr = tilemap_base
                        + SCREEN_BLOCK_SIZE * sbb
                        + 2 * index2d!(u32, se_row, se_column, 32);
                    for _ in se_row..32 {
                        let entry = TileMapEntry(self.vram_read_16(map_addr as usize));
                        let tile_addr = tileset_base + entry.tile_index() * tile_size;

                        for tile_px in start_tile_x..8 {
                            let index = self.$read_pixel_index(
                                tile_addr,
                                if entry.x_flip() { 7 - tile_px } else { tile_px },
                                if entry.y_flip() { 7 - tile_py } else { tile_py },
                            );
                            let palette_bank = match pixel_format {
                                PixelFormat::BPP4 => entry.palette_bank() as u32,
                                PixelFormat::BPP8 => 0u32,
                            };
                            let color = self.get_palette_color(index as u32, palette_bank, 0);
                            self.bg_line[bg][screen_x as usize] = color;
                            screen_x += 1;
                            if (SCREEN_WIDTH as u32) == screen_x {
                                return;
                            }
                        }
                        start_tile_x = 0;
                        map_addr += 2;
                    }
                    se_row = 0;
                    if bg_width == 512 {
                        sbb ^= 1;
                    }
                }
            };
        }

        match pixel_format {
            PixelFormat::BPP4 => render_loop!(read_pixel_index_bpp4),
            PixelFormat::BPP8 => render_loop!(read_pixel_index_bpp8),
        }
    }

    /// Affine BG (BG2 or BG3) with optional wrapping.
    pub fn render_aff_bg(&mut self, bg: usize) {
        assert!(bg == 2 || bg == 3);

        let texture_size = 128 << self.bgcnt[bg].screen_size();
        let viewport = ViewPort::new(texture_size, texture_size);

        let ref_point = self.get_ref_point(bg);
        let pa = self.bg_aff[bg - 2].pa as i32;
        let pc = self.bg_aff[bg - 2].pc as i32;

        let screen_block = self.bgcnt[bg].screen_block();
        let char_block = self.bgcnt[bg].char_block();

        let wraparound = self.bgcnt[bg].wraparound();

        for screen_x in 0..(SCREEN_WIDTH as i32) {
            let mut t = transform_bg_point(ref_point, screen_x, pa, pc);

            if !viewport.contains_point(t) {
                if wraparound {
                    t.0 = t.0.rem_euclid(texture_size);
                    t.1 = t.1.rem_euclid(texture_size);
                } else {
                    self.bg_line[bg][screen_x as usize] = Rgb15::TRANSPARENT;
                    continue;
                }
            }
            let map_addr = screen_block + index2d!(u32, t.0 / 8, t.1 / 8, texture_size / 8);
            let tile_index = self.vram_read_8(map_addr as usize) as u32;
            let tile_addr = char_block + tile_index * 0x40;

            let pixel_index = self.read_pixel_index(
                tile_addr,
                (t.0 % 8) as u32,
                (t.1 % 8) as u32,
                PixelFormat::BPP8,
            ) as u32;
            let color = self.get_palette_color(pixel_index, 0, 0);
            self.bg_line[bg][screen_x as usize] = color;
        }
    }

    /// Mode 3: 240×160 15-bit direct-colour bitmap.
    /// Each pixel is a 16-bit little-endian BGR555 value starting at VRAM offset 0.
    pub fn render_mode3(&mut self) {
        let bg = 2; // BG2 is the bitmap layer in Mode 3
        let _y = self.vcount;

        let pa = self.bg_aff[bg - 2].pa as i32;
        let pc = self.bg_aff[bg - 2].pc as i32;
        let ref_point = self.get_ref_point(bg);

        let wraparound = self.bgcnt[bg].wraparound();

        for x in 0..SCREEN_WIDTH {
            let mut t = transform_bg_point(ref_point, x as i32, pa, pc);
            if !SCREEN_VIEWPORT.contains_point(t) {
                if wraparound {
                    t.0 = t.0.rem_euclid(SCREEN_VIEWPORT.w);
                    t.1 = t.1.rem_euclid(SCREEN_VIEWPORT.h);
                } else {
                    self.bg_line[bg][x] = Rgb15::TRANSPARENT;
                    continue;
                }
            }
            let pixel_index = index2d!(u32, t.0, t.1, SCREEN_WIDTH);
            let pixel_ofs = 2 * pixel_index;
            let color = Rgb15(self.vram_read_16(pixel_ofs as usize));
            self.bg_line[bg][x] = color;
        }
    }

    /// Mode 4: 240×160 8-bit indexed-color bitmap with 256-entry palette.
    /// Each pixel is a single byte index into the palette, starting at VRAM offset 0.
    pub fn render_mode4(&mut self) {
        let bg = 2; // BG2 is the bitmap layer in Mode 4
        let page_ofs = self.get_page_offset();

        let _y = self.vcount;

        let pa = self.bg_aff[bg - 2].pa as i32;
        let pc = self.bg_aff[bg - 2].pc as i32;
        let ref_point = self.get_ref_point(bg);

        let wraparound = self.bgcnt[bg].wraparound();

        for x in 0..SCREEN_WIDTH {
            let mut t = transform_bg_point(ref_point, x as i32, pa, pc);
            if !SCREEN_VIEWPORT.contains_point(t) {
                if wraparound {
                    t.0 = t.0.rem_euclid(SCREEN_VIEWPORT.w);
                    t.1 = t.1.rem_euclid(SCREEN_VIEWPORT.h);
                } else {
                    self.bg_line[bg][x] = Rgb15::TRANSPARENT;
                    continue;
                }
            }
            let bitmap_index = index2d!(u32, t.0, t.1, SCREEN_WIDTH);
            let bitmap_ofs = page_ofs + (bitmap_index as u32);
            let index = self.vram_read_8(bitmap_ofs as usize) as u32;
            let color = self.get_palette_color(index, 0, 0);
            self.bg_line[bg][x] = color;
        }
    }

    pub fn render_mode5(&mut self) {
        let bg = 2; // BG2 is the bitmap layer in Mode 5
        let page_ofs = self.get_page_offset();
        let _y = self.vcount;

        let pa = self.bg_aff[bg - 2].pa as i32;
        let pc = self.bg_aff[bg - 2].pc as i32;
        let ref_point = self.get_ref_point(bg);

        let wraparound = self.bgcnt[bg].wraparound();

        for x in 0..SCREEN_WIDTH {
            let mut t = transform_bg_point(ref_point, x as i32, pa, pc);
            if !MODE5_VIEWPORT.contains_point(t) {
                if wraparound {
                    t.0 = t.0.rem_euclid(MODE5_VIEWPORT.w);
                    t.1 = t.1.rem_euclid(MODE5_VIEWPORT.h);
                } else {
                    self.bg_line[bg][x] = Rgb15::TRANSPARENT;
                    continue;
                }
            }
            let pixel_ofs = page_ofs + 2 * index2d!(u32, t.0, t.1, MODE5_VIEWPORT.w);
            let color = Rgb15(self.vram_read_16(pixel_ofs as usize));
            self.bg_line[bg][x] = color;
        }
    }

    fn get_page_offset(&self) -> u32 {
        (match self.dispcnt.frame_select() {
            false => 0x0600_0000,
            true => 0x0600_a000,
        }) - VRAM_ADDR
    }
}