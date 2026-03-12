use super::bus::Bus;

/// Hardware timer block scaffold.
pub struct Timer {
    cycles: u64,
}

impl Timer {
    pub fn new() -> Self {
        Self { cycles: 0 }
    }

    pub fn reset(&mut self) {
        self.cycles = 0;
    }

    pub fn tick(&mut self, cycles: u32, _bus: &mut Bus) {
        self.cycles = self.cycles.saturating_add(cycles as u64);
    }
}
