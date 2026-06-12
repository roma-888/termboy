//! Object (sprite) line producer. Regular sprites in Task 6, affine in
//! Task 7. Among overlapping sprites the lowest OAM index wins the pixel.

use crate::bus::Bus;

impl Bus {
    pub(crate) fn render_objects(&mut self, line: usize, dispcnt: u16) {
        let _ = (line, dispcnt); // Tasks 6-7
    }
}
