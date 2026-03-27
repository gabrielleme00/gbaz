use super::*;

#[derive(Debug, Ord, Eq, PartialOrd, PartialEq, Clone, Copy)]
pub enum RenderLayerKind {
    Backdrop = 0b00100000,
    Background3 = 0b00001000,
    Background2 = 0b00000100,
    Background1 = 0b00000010,
    Background0 = 0b00000001,
    Objects = 0b00010000,
}

impl RenderLayerKind {
    pub fn get_blend_flag(&self) -> BlendFlags {
        match self {
            RenderLayerKind::Background0 => BlendFlags::BG0,
            RenderLayerKind::Background1 => BlendFlags::BG1,
            RenderLayerKind::Background2 => BlendFlags::BG2,
            RenderLayerKind::Background3 => BlendFlags::BG3,
            RenderLayerKind::Objects => BlendFlags::OBJ,
            RenderLayerKind::Backdrop => BlendFlags::BACKDROP,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RenderLayer {
    pub kind: RenderLayerKind,
    pub priority: u16,
    pub pixel: Rgb15,
}

impl RenderLayer {
    pub fn background(bg: usize, pixel: Rgb15, priority: u16) -> RenderLayer {
        RenderLayer {
            kind: match bg {
                0 => RenderLayerKind::Background0,
                1 => RenderLayerKind::Background1,
                2 => RenderLayerKind::Background2,
                3 => RenderLayerKind::Background3,
                _ => unreachable!(),
            },
            pixel,
            priority,
        }
    }

    pub fn objects(pixel: Rgb15, priority: u16) -> RenderLayer {
        RenderLayer {
            kind: RenderLayerKind::Objects,
            pixel,
            priority,
        }
    }

    pub fn backdrop(pixel: Rgb15) -> RenderLayer {
        RenderLayer {
            kind: RenderLayerKind::Backdrop,
            pixel,
            priority: 4,
        }
    }

    pub(super) fn is_object(&self) -> bool {
        self.kind == RenderLayerKind::Objects
    }
}