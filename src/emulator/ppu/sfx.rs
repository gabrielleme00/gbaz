use super::layer::*;
use super::regs::*;
use super::*;
use std::cmp;

impl Rgb15 {
    fn blend_with(self, other: Rgb15, my_weight: u16, other_weight: u16) -> Rgb15 {
        let r = cmp::min(31, (self.r() * my_weight + other.r() * other_weight) >> 4);
        let g = cmp::min(31, (self.g() * my_weight + other.g() * other_weight) >> 4);
        let b = cmp::min(31, (self.b() * my_weight + other.b() * other_weight) >> 4);
        Rgb15::from_rgb(r, g, b)
    }
}

/// Filters a background indexes array by whether they're active
fn filter_window_backgrounds(backgrounds: &[usize], window_flags: WindowFlags) -> Vec<usize> {
    backgrounds
        .iter()
        .copied()
        .filter(|bg| window_flags.bg_enabled(*bg))
        .collect()
}

impl Ppu {
    #[allow(unused)]
    fn layer_to_pixel(&mut self, x: usize, y: usize, layer: &RenderLayer) -> Rgb15 {
        match layer.kind {
            RenderLayerKind::Background0 => self.bg_line[0][x],
            RenderLayerKind::Background1 => self.bg_line[1][x],
            RenderLayerKind::Background2 => self.bg_line[2][x],
            RenderLayerKind::Background3 => self.bg_line[3][x],
            RenderLayerKind::Objects => self.obj_buffer_get(x, y).color,
            RenderLayerKind::Backdrop => Rgb15(self.pram_read_16(0)),
        }
    }

    /// Composes the render layers into a final scanline while applying needed special effects,
    /// and render it to the frame buffer.
    pub fn finalize_scanline(&mut self, bg_start: usize, bg_end: usize) {
        let backdrop_color = Rgb15(self.pram_read_16(0));

        if self.vcount == 60 {
            // probe moved to cpu.rs brightness call
        }

        // Filter out disabled backgrounds and sort by priority
        // The backgrounds are sorted once for the entire scanline
        // In bitmap modes (3/4/5) BG2 is always active regardless of the bg2_enable bit.
        let bitmap_mode = matches!(self.dispcnt.mode(), 3 | 4 | 5);
        let mut sorted_backgrounds: Vec<usize> = (bg_start..=bg_end)
            .filter(|bg| bitmap_mode || self.is_bg_enabled(*bg))
            .collect();
        sorted_backgrounds.sort_by_key(|bg| (self.bgcnt[*bg].priority(), *bg));

        let y = self.vcount as usize;

        let output = unsafe {
            let ptr = self.frame_buffer[y * SCREEN_WIDTH..].as_mut_ptr();
            std::slice::from_raw_parts_mut(ptr, SCREEN_WIDTH)
        };

        if !self.dispcnt.is_using_windows() {
            for x in 0..SCREEN_WIDTH {
                let win = WindowInfo::new(WindowType::WinNone, WindowFlags::all());
                output[x] = self
                    .finalize_pixel(x, y, &win, &sorted_backgrounds, backdrop_color)
                    .to_rgb24();
            }
        } else {
            let mut occupied = [false; SCREEN_WIDTH];
            let mut occupied_count = 0;
            if self.dispcnt.win0_enable() && self.win0.contains_y(y) {
                let win = WindowInfo::new(WindowType::Win0, self.win0.flags);
                let backgrounds = filter_window_backgrounds(&sorted_backgrounds, win.flags);
                for (x, is_occupied) in occupied
                    .iter_mut()
                    .enumerate()
                    .take(self.win0.right())
                    .skip(self.win0.left())
                {
                    output[x] = self
                        .finalize_pixel(x, y, &win, &backgrounds, backdrop_color)
                        .to_rgb24();
                    *is_occupied = true;
                    occupied_count += 1;
                }
            }
            if occupied_count == SCREEN_WIDTH {
                return;
            }
            if self.dispcnt.win1_enable() && self.win1.contains_y(y) {
                let win = WindowInfo::new(WindowType::Win1, self.win1.flags);
                let backgrounds = filter_window_backgrounds(&sorted_backgrounds, win.flags);
                for (x, is_occupied) in occupied
                    .iter_mut()
                    .enumerate()
                    .take(self.win1.right() as usize)
                    .skip(self.win1.left() as usize)
                {
                    if *is_occupied {
                        continue;
                    }
                    output[x] = self
                        .finalize_pixel(x, y, &win, &backgrounds, backdrop_color)
                        .to_rgb24();
                    *is_occupied = true;
                    occupied_count += 1;
                }
            }
            if occupied_count == SCREEN_WIDTH {
                return;
            }
            let win_out = WindowInfo::new(WindowType::WinOut, self.winout_flags);
            let win_out_backgrounds = filter_window_backgrounds(&sorted_backgrounds, win_out.flags);
            if self.dispcnt.obj_win_enable() {
                let win_obj = WindowInfo::new(WindowType::WinObj, self.winobj_flags);
                let win_obj_backgrounds =
                    filter_window_backgrounds(&sorted_backgrounds, win_obj.flags);
                for (x, is_occupied) in occupied.iter().enumerate().take(SCREEN_WIDTH) {
                    if *is_occupied {
                        continue;
                    }
                    let obj_entry = self.obj_buffer_get(x, y);
                    if obj_entry.window {
                        // WinObj
                        output[x] = self
                            .finalize_pixel(x, y, &win_obj, &win_obj_backgrounds, backdrop_color)
                            .to_rgb24();
                    } else {
                        // WinOut
                        output[x] = self
                            .finalize_pixel(x, y, &win_out, &win_out_backgrounds, backdrop_color)
                            .to_rgb24();
                    }
                }
            } else {
                for (x, is_occupied) in occupied.iter().enumerate() {
                    if *is_occupied {
                        continue;
                    }
                    output[x] = self
                        .finalize_pixel(x, y, &win_out, &win_out_backgrounds, backdrop_color)
                        .to_rgb24();
                }
            }
        }
    }

    #[must_use]
    fn finalize_pixel(
        &mut self,
        x: usize,
        y: usize,
        win: &WindowInfo,
        backgrounds: &[usize],
        backdrop_color: Rgb15,
    ) -> Rgb15 {
        // The backdrop layer is the default
        let backdrop_layer = RenderLayer::backdrop(backdrop_color);

        // Backgrounds are already sorted
        // lets start by taking the first 2 backgrounds that have an opaque pixel at x
        let mut it = backgrounds
            .iter()
            .filter(|i| self.bg_line[**i][x].is_opaque())
            .take(2);

        let mut top_layer = it.next().map_or(backdrop_layer, |bg| {
            RenderLayer::background(*bg, self.bg_line[*bg][x], self.bgcnt[*bg].priority())
        });

        let mut bot_layer = it.next().map_or(backdrop_layer, |bg| {
            RenderLayer::background(*bg, self.bg_line[*bg][x], self.bgcnt[*bg].priority())
        });

        drop(it);

        // Now that backgrounds are taken care of, we need to check if there is an object pixel that takes priority of one of the layers
        let obj_entry = self.obj_buffer_get(x, y);
        if win.flags.obj_enabled() && self.dispcnt.obj_enable() && obj_entry.color.is_opaque() {
            let obj_layer = RenderLayer::objects(obj_entry.color, obj_entry.priority);
            if obj_layer.priority <= top_layer.priority {
                bot_layer = top_layer;
                top_layer = obj_layer;
            } else if obj_layer.priority <= bot_layer.priority {
                bot_layer = obj_layer;
            }
        }

        let (top_flags, bot_flags) = (self.bldcnt.target1, self.bldcnt.target2);

        let sfx_enabled =
            self.bldcnt.mode != BlendMode::BldNone && top_flags.contains_render_layer(&top_layer);

        if top_layer.is_object() && obj_entry.alpha && bot_flags.contains_render_layer(&bot_layer) {
            self.do_alpha(top_layer.pixel, bot_layer.pixel)
        } else if win.flags.sfx_enabled() && sfx_enabled {
            let (top_layer, bot_layer) = (top_layer, bot_layer);
            match self.bldcnt.mode {
                BlendMode::BldAlpha => {
                    if bot_flags.contains_render_layer(&bot_layer) {
                        self.do_alpha(top_layer.pixel, bot_layer.pixel)
                    } else {
                        // alpha blending must have a 2nd target
                        top_layer.pixel
                    }
                }
                BlendMode::BldWhite => self.do_brighten(top_layer.pixel),
                BlendMode::BldBlack => self.do_darken(top_layer.pixel),
                BlendMode::BldNone => top_layer.pixel,
            }
        } else {
            // no blending, just use the top pixel
            top_layer.pixel
        }
    }

    #[inline]
    fn do_alpha(&self, upper: Rgb15, lower: Rgb15) -> Rgb15 {
        let eva = self.bldalpha.eva;
        let evb = self.bldalpha.evb;
        upper.blend_with(lower, eva, evb)
    }

    #[inline]
    fn do_brighten(&self, c: Rgb15) -> Rgb15 {
        let evy = self.bldy.min(16);
        c.blend_with(Rgb15::WHITE, 16 - evy, evy)
    }

    #[inline]
    fn do_darken(&self, c: Rgb15) -> Rgb15 {
        let evy = self.bldy.min(16);
        c.blend_with(Rgb15::BLACK, 16 - evy, evy)
    }
}
