#[macro_use]
extern crate bitfield;
#[macro_use]
extern crate bitflags;

pub mod app;
pub mod emulator;
pub mod utils;

#[macro_export]
macro_rules! index2d {
    ($x:expr, $y:expr, $w:expr) => {
        $w * $y + $x
    };
    ($t:ty, $x:expr, $y:expr, $w:expr) => {
        (($w as $t) * ($y as $t) + ($x as $t)) as $t
    };
}
