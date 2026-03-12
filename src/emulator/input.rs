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

    pub fn raw(&self) -> u16 {
        self.buttons
    }

    pub fn clear(&mut self) {
        self.buttons = 0;
    }
}
