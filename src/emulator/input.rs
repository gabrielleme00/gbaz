#[derive(Clone, Copy)]
pub enum Button {
    A,
    B,
    Select,
    Start,
    Right,
    Left,
    Up,
    Down,
    R,
    L,
}

#[derive(Default, Clone, Copy)]
pub struct InputState {
    buttons: u16,
}

impl InputState {
    pub fn set_pressed(&mut self, button: Button, pressed: bool) {
        let bit = match button {
            Button::A => 0,
            Button::B => 1,
            Button::Select => 2,
            Button::Start => 3,
            Button::Right => 4,
            Button::Left => 5,
            Button::Up => 6,
            Button::Down => 7,
            Button::R => 8,
            Button::L => 9,
        };

        if pressed {
            self.buttons |= 1 << bit;
        } else {
            self.buttons &= !(1 << bit);
        }
    }

    pub fn read_8(&self, addr: u32) -> u8 {
        let keyinput = self.read_16();
        if addr & 1 == 0 { keyinput as u8 } else { (keyinput >> 8) as u8 }
    }

    pub fn read_16(&self) -> u16 {
        (!self.buttons & 0x03FF) | 0xFC00
    }

    pub fn read_32(&self) -> u32 {
        self.read_16() as u32
    }

    pub fn clear(&mut self) {
        self.buttons = 0;
    }
}
