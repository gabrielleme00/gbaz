use super::WindowFlags;
use super::{SCREEN_HEIGHT, SCREEN_WIDTH};

#[derive(Clone, Debug, Default)]
pub struct Window {
    pub left: u8,
    pub right: u8,
    pub top: u8,
    pub bottom: u8,
    pub flags: WindowFlags,
}

impl Window {
    #[inline]
    pub fn left(&self) -> usize {
        self.left as usize
    }

    #[inline]
    pub fn right(&self) -> usize {
        let left = self.left as usize;
        let mut right = self.right as usize;
        if right > SCREEN_WIDTH || right < left {
            right = SCREEN_WIDTH;
        }
        right
    }

    #[inline]
    pub fn top(&self) -> usize {
        self.top as usize
    }

    #[inline]
    pub fn bottom(&self) -> usize {
        let top = self.top as usize;
        let mut bottom = self.bottom as usize;
        if bottom > SCREEN_HEIGHT || bottom < top {
            bottom = SCREEN_HEIGHT;
        }
        bottom
    }

    #[inline]
    pub fn contains_y(&self, y: usize) -> bool {
        let top = self.top();
        let bottom = self.bottom();
        y >= top && y < bottom
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum WindowType {
    Win0,
    Win1,
    WinObj,
    WinOut,
    WinNone,
}

#[derive(Debug)]
pub struct WindowInfo {
    pub _type: WindowType,
    pub flags: WindowFlags,
}

impl WindowInfo {
    pub fn new(typ: WindowType, flags: WindowFlags) -> WindowInfo {
        WindowInfo { _type: typ, flags }
    }
}
