use super::{layer::*, regs::*, *};
use std::cmp;

impl Rgb15 {
    fn blend_with(self, other: Rgb15, my_weight: u16, other_weight: u16) -> Rgb15 {
        let r = cmp::min(31, (self.r() * my_weight + other.r() * other_weight) >> 4);
        let g = cmp::min(31, (self.g() * my_weight + other.g() * other_weight) >> 4);
        let b = cmp::min(31, (self.b() * my_weight + other.b() * other_weight) >> 4);
        Rgb15::from_rgb(r, g, b)
    }
}

/// Fills a stack buffer with the subset of `backgrounds` enabled by `window_flags`.
fn filter_window_backgrounds(
    backgrounds: &[usize],
    window_flags: WindowFlags,
) -> ([usize; 4], usize) {
    let mut buf = [0usize; 4];
    let mut len = 0;
    for &bg in backgrounds {
        if window_flags.bg_enabled(bg) {
            buf[len] = bg;
            len += 1;
        }
    }
    (buf, len)
}

impl Ppu {
    /// Composes the render layers into a final scanline while applying needed special effects,
    /// and render it to the frame buffer.
    pub fn finalize_scanline(&mut self, bg_start: usize, bg_end: usize) {
        let backdrop_color = Rgb15(self.pram_read_16(0));

        // Filter out disabled backgrounds and sort by priority
        // The backgrounds are sorted once for the entire scanline
        // In bitmap modes (3/4/5) BG2 is always active regardless of the bg2_enable bit.
        let bitmap_mode = matches!(self.dispcnt.mode(), 3 | 4 | 5);
        let mut sorted_bg_arr = [0usize; 4];
        let mut sorted_bg_len = 0;
        for bg in bg_start..=bg_end {
            if bitmap_mode || self.is_bg_enabled(bg) {
                sorted_bg_arr[sorted_bg_len] = bg;
                sorted_bg_len += 1;
            }
        }
        let sorted_bg = &mut sorted_bg_arr[..sorted_bg_len];
        sorted_bg.sort_by_key(|bg| (self.bgcnt[*bg].priority(), *bg));
        let sorted_backgrounds: &[usize] = sorted_bg;

        let y = self.vcount as usize;

        let output = unsafe {
            let ptr = self.frame_buffer[y * SCREEN_WIDTH..].as_mut_ptr();
            std::slice::from_raw_parts_mut(ptr, SCREEN_WIDTH)
        };

        if !self.dispcnt.is_using_windows() {
            for x in 0..SCREEN_WIDTH {
                let win = WindowInfo::new(WindowType::WinNone, WindowFlags::all());
                output[x] = self
                    .finalize_pixel(x, &win, sorted_backgrounds, backdrop_color)
                    .to_rgb24();
            }
        } else {
            let mut occupied = [false; SCREEN_WIDTH];
            let mut occupied_count = 0;
            if self.dispcnt.win0_enable() && self.win0.contains_y(y) {
                let win = WindowInfo::new(WindowType::Win0, self.win0.flags);
                let (bg_buf, bg_len) = filter_window_backgrounds(sorted_backgrounds, win.flags);
                let backgrounds = &bg_buf[..bg_len];
                for (x, is_occupied) in occupied
                    .iter_mut()
                    .enumerate()
                    .take(self.win0.right())
                    .skip(self.win0.left())
                {
                    output[x] = self
                        .finalize_pixel(x, &win, backgrounds, backdrop_color)
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
                let (bg_buf, bg_len) = filter_window_backgrounds(sorted_backgrounds, win.flags);
                let backgrounds = &bg_buf[..bg_len];
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
                        .finalize_pixel(x, &win, backgrounds, backdrop_color)
                        .to_rgb24();
                    *is_occupied = true;
                    occupied_count += 1;
                }
            }
            if occupied_count == SCREEN_WIDTH {
                return;
            }
            let win_out = WindowInfo::new(WindowType::WinOut, self.winout_flags);
            let (win_out_bg_buf, win_out_bg_len) =
                filter_window_backgrounds(sorted_backgrounds, win_out.flags);
            let win_out_backgrounds = &win_out_bg_buf[..win_out_bg_len];
            if self.dispcnt.obj_win_enable() {
                let win_obj = WindowInfo::new(WindowType::WinObj, self.winobj_flags);
                let (win_obj_bg_buf, win_obj_bg_len) =
                    filter_window_backgrounds(sorted_backgrounds, win_obj.flags);
                let win_obj_backgrounds = &win_obj_bg_buf[..win_obj_bg_len];
                for (x, is_occupied) in occupied.iter().enumerate().take(SCREEN_WIDTH) {
                    if *is_occupied {
                        continue;
                    }
                    let obj = self.obj_buffer_get(x);
                    if obj.window {
                        // WinObj
                        output[x] = self
                            .finalize_pixel(x, &win_obj, win_obj_backgrounds, backdrop_color)
                            .to_rgb24();
                    } else {
                        // WinOut
                        output[x] = self
                            .finalize_pixel(x, &win_out, win_out_backgrounds, backdrop_color)
                            .to_rgb24();
                    }
                }
            } else {
                for (x, is_occupied) in occupied.iter().enumerate() {
                    if *is_occupied {
                        continue;
                    }
                    output[x] = self
                        .finalize_pixel(x, &win_out, win_out_backgrounds, backdrop_color)
                        .to_rgb24();
                }
            }
        }
    }

    #[must_use]
    fn finalize_pixel(
        &mut self,
        x: usize,
        win: &WindowInfo,
        backgrounds: &[usize],
        backdrop_color: Rgb15,
    ) -> Rgb15 {
        use BlendMode::*;

        // The backdrop layer is the default
        let backdrop_layer = RenderLayer::backdrop(backdrop_color);

        // Backgrounds are already sorted, so we just need to take the first 2
        // that have an opaque pixel at x
        let is_opaque = |bg: usize| self.bg_line[bg][x].is_opaque();
        let mut it = backgrounds.iter().filter(|i| is_opaque(**i)).take(2);

        // If there are no opaque backgrounds, the top layer is the backdrop and
        // the bottom layer is also the backdrop (for blending purposes)
        let build_bg = |bg| self.layer_from_bg(bg, x);
        let mut top_layer = it.next().map_or(backdrop_layer, |bg| build_bg(*bg));
        let mut bot_layer = it.next().map_or(backdrop_layer, |bg| build_bg(*bg));

        drop(it);

        // Now that backgrounds are taken care of, we need to check if there is
        // an object pixel that takes priority of one of the layers
        let obj = self.obj_buffer_get(x);
        if win.flags.obj_enabled() && self.dispcnt.obj_enable() && obj.color.is_opaque() {
            let obj_layer = self.layer_from_obj(&obj);
            if obj_layer.priority <= top_layer.priority {
                bot_layer = top_layer;
                top_layer = obj_layer;
            } else if obj_layer.priority <= bot_layer.priority {
                bot_layer = obj_layer;
            }
        }

        // With the final top and bottom layers determined, we can now apply
        // special effects if needed
        let (top_flags, bot_flags) = (self.bldcnt.target1, self.bldcnt.target2);

        // SFX is applied if the top layer is in the target layers and the mode
        // is not BldNone
        let top_sfx = self.bldcnt.mode != BldNone && top_flags.contains_render_layer(&top_layer);

        if top_layer.is_object() && obj.alpha && bot_flags.contains_render_layer(&bot_layer) {
            // Object pixel with alpha enabled: alpha blend the top and bottom
            // layers regardless of mode.
            self.do_alpha(top_layer.pixel, bot_layer.pixel)
        } else if win.flags.sfx_enabled() && top_sfx {
            // The top layer is eligible for special effects, so apply the
            // selected effect if the bottom layer is also in the target layers.
            let (top_layer, bot_layer) = (top_layer, bot_layer);
            match self.bldcnt.mode {
                BldAlpha => {
                    if bot_flags.contains_render_layer(&bot_layer) {
                        // Both layers are in target layers, so apply alpha
                        // blending with the specified coefficients
                        self.do_alpha(top_layer.pixel, bot_layer.pixel)
                    } else {
                        // Bottom layer is not in target layers, so just return
                        // the top layer's pixel
                        top_layer.pixel
                    }
                }
                BldWhite => self.do_brighten(top_layer.pixel),
                BldBlack => self.do_darken(top_layer.pixel),
                BldNone => top_layer.pixel,
            }
        } else {
            // No special effects, so just return the top layer's pixel
            top_layer.pixel
        }
    }

    #[inline]
    fn layer_from_bg(&self, bg: usize, x: usize) -> RenderLayer {
        RenderLayer::background(bg, self.bg_line[bg][x], self.bgcnt[bg].priority())
    }

    #[inline]
    fn layer_from_obj(&self, obj: &ObjBufferEntry) -> RenderLayer {
        RenderLayer::objects(obj.color, obj.priority)
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
