//! Background line producers: text (Task 3) and affine (Task 5).

use crate::bus::Bus;

impl Bus {
    pub(crate) fn render_text_bg(&mut self, bg: usize, line: usize) {
        let _ = (bg, line); // Task 3
    }

    /// `enabled` still matters when off: the internal reference point
    /// advances every scanline regardless (Task 5).
    pub(crate) fn render_affine_bg(&mut self, bg: usize, line: usize, enabled: bool) {
        let _ = (bg, line, enabled); // Task 5
    }
}
