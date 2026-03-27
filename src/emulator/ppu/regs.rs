use super::*;

pub const SCREEN_BLOCK_SIZE: u32 = 0x800;

bitfield!(
    #[derive(Clone)]
    pub struct DisplayControl(u16);
    impl Debug;
    pub mode, set_mode: 2, 0;
    pub cgb_mode, set_cgb_mode: 3;
    pub frame_select, set_frame_select: 4;
    pub hblank_interval_free, set_hblank_interval_free: 5;
    pub obj_character_mapping, set_obj_character_mapping: 6;
    pub force_blank, set_force_blank: 7;
    pub bg0_enable, set_bg0_enable: 8;
    pub bg1_enable, set_bg1_enable: 9;
    pub bg2_enable, set_bg2_enable: 10;
    pub bg3_enable, set_bg3_enable: 11;
    pub obj_enable, set_obj_enable: 12;
    pub win0_enable, set_win0_enable: 13;
    pub win1_enable, set_win1_enable: 14;
    pub obj_win_enable, set_obj_win_enable: 15;
);

impl DisplayControl {
    #[inline]
    pub fn is_using_windows(&self) -> bool {
        self.win0_enable() || self.win1_enable() || self.obj_win_enable()
    }
}

bitfield!(
    #[derive(Clone)]
    pub struct DisplayStatus(u16);
    impl Debug;
    pub vblank_flag, set_vblank_flag: 0;
    pub hblank_flag, set_hblank_flag: 1;
    pub vcount_flag, set_vcount_flag: 2;
    pub vblank_irq_enable, set_vblank_irq_enable: 3;
    pub hblank_irq_enable, set_hblank_irq_enable: 4;
    pub vcount_irq_enable, set_vcount_irq_enable: 5;
    pub vcount_setting, set_vcount_setting: 15, 8;
);

bitfield!(
    #[derive(Clone, Copy)]
    pub struct BgControl(u16);
    impl Debug;
    pub priority, set_priority: 1, 0;
    pub char_block_offset, set_char_block_offset: 3, 2;
    pub mosaic, set_mosaic: 6;
    pub color_mode, set_color_mode: 7;
    pub screen_block_offset, set_screen_block_offset: 12, 8;
    pub wraparound, set_wraparound: 13;
    pub screen_size, set_screen_size: 15, 14;
);

impl BgControl {
    #[inline]
    pub fn char_block(&self) -> u32 {
        (self.char_block_offset() as u32) * 0x4000
    }
    #[inline]
    pub fn screen_block(&self) -> u32 {
        (self.screen_block_offset() as u32) * SCREEN_BLOCK_SIZE
    }
    #[inline]
    pub fn size_regular(&self) -> (u32, u32) {
        match self.screen_size() {
            0b00 => (256, 256),
            0b01 => (512, 256),
            0b10 => (256, 512),
            0b11 => (512, 512),
            _ => unreachable!(),
        }
    }
    #[inline]
    pub fn tile_format(&self) -> (u32, PixelFormat) {
        if self.color_mode() {
            (2 * TILE_SIZE, PixelFormat::BPP8)
        } else {
            (TILE_SIZE, PixelFormat::BPP4)
        }
    }
}


#[derive(Clone, Copy, Default)]
pub struct BgAffine {
    pub pa: i16, // dx
    pub pb: i16, // dmx
    pub pc: i16, // dy
    pub pd: i16, // dmy
    pub x: i32,
    pub y: i32,
    pub internal_x: i32,
    pub internal_y: i32,
}

impl BgAffine {
    pub fn increment(&mut self) {
        self.internal_x += self.pb as i32;
        self.internal_y += self.pd as i32;
    }

    pub fn latch(&mut self) {
        self.internal_x = self.x;
        self.internal_y = self.y;
    }
}

const BG_WIN_FLAG: [WindowFlags; 4] = [
    WindowFlags::BG0,
    WindowFlags::BG1,
    WindowFlags::BG2,
    WindowFlags::BG3,
];

bitflags! {
    #[derive(Default, Clone, Copy, Debug)]
    pub struct WindowFlags: u16 {
        const BG0 = 0b00000001;
        const BG1 = 0b00000010;
        const BG2 = 0b00000100;
        const BG3 = 0b00001000;
        const OBJ = 0b00010000;
        const SFX = 0b00100000;
    }
}

impl From<u16> for WindowFlags {
    fn from(v: u16) -> WindowFlags {
        WindowFlags::from_bits_truncate(v)
    }
}

impl WindowFlags {
    pub fn sfx_enabled(&self) -> bool {
        self.contains(WindowFlags::SFX)
    }
    pub fn bg_enabled(&self, bg: usize) -> bool {
        self.contains(BG_WIN_FLAG[bg])
    }
    pub fn obj_enabled(&self) -> bool {
        self.contains(WindowFlags::OBJ)
    }
}

bitfield! {
    #[derive(Default, Copy, Clone)]
    pub struct RegMosaic(u16);
    impl Debug;
    u32;
    pub bg_hsize, _: 3, 0;
    pub bg_vsize, _: 7, 4;
    pub obj_hsize, _ : 11, 8;
    pub obj_vsize, _ : 15, 12;
}

bitflags! {
    #[derive(Default, Clone, Copy, Debug)]
    pub struct BlendFlags: u16 {
        const BG0 = 0b00000001;
        const BG1 = 0b00000010;
        const BG2 = 0b00000100;
        const BG3 = 0b00001000;
        const OBJ = 0b00010000;
        const BACKDROP  = 0b00100000; // BACKDROP
    }
}

impl BlendFlags {
    const BG_LAYER_FLAG: [BlendFlags; 4] = [
        BlendFlags::BG0,
        BlendFlags::BG1,
        BlendFlags::BG2,
        BlendFlags::BG3,
    ];
    #[inline]
    pub fn from_bg(bg: usize) -> BlendFlags {
        Self::BG_LAYER_FLAG[bg]
    }
    #[inline]
    pub fn obj_enabled(&self) -> bool {
        self.contains(BlendFlags::OBJ)
    }
    #[inline]
    pub fn contains_render_layer(&self, layer: &RenderLayer) -> bool {
        let layer_flags = BlendFlags::from_bits_truncate(layer.kind as u16);
        self.contains(layer_flags)
    }
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum BlendMode {
    #[default]
    BldNone = 0b00,
    BldAlpha = 0b01,
    BldWhite = 0b10,
    BldBlack = 0b11,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct BlendControl {
    pub target1: BlendFlags,
    pub target2: BlendFlags,
    pub mode: BlendMode,
}

impl BlendMode {
    #[inline]
    pub fn from_u16(value: u16) -> Option<BlendMode> {
        match value {
            0b00 => Some(BlendMode::BldNone),
            0b01 => Some(BlendMode::BldAlpha),
            0b10 => Some(BlendMode::BldWhite),
            0b11 => Some(BlendMode::BldBlack),
            _ => None,
        }
    }
}

impl BlendControl {
    #[inline]
    pub fn write(&mut self, value: u16) {
        self.target1 = BlendFlags::from_bits_truncate(value & 0x3f);
        self.target2 = BlendFlags::from_bits_truncate((value >> 8) & 0x3f);
        self.mode = BlendMode::from_u16((value >> 6) & 0b11).unwrap_or_else(|| unreachable!());
    }

    #[inline]
    pub fn read(&self) -> u16 {
        self.target1.bits() | (self.mode as u16) << 6 | (self.target2.bits() << 8)
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub struct BlendAlpha {
    pub eva: u16,
    pub evb: u16,
}

impl BlendAlpha {
    #[inline]
    pub fn write(&mut self, value: u16) {
        self.eva = value & 0x1f;
        self.evb = (value >> 8) & 0x1f;
    }

    #[inline]
    pub fn read(&self) -> u16 {
        self.eva | self.evb << 8
    }
}
