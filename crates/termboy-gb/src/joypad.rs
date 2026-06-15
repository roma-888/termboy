//! P1 (0xFF00) button matrix. Select lines are active-low; pressed = 0.

use termboy_core::state::{Reader, StateError, Writer};
use termboy_core::Buttons;

pub struct Joypad {
    select: u8, // bits 4-5 as last written
    buttons: Buttons,
}

impl Default for Joypad {
    fn default() -> Self {
        Self { select: 0x30, buttons: Buttons::default() } // nothing selected
    }
}

impl Joypad {
    pub fn read(&self) -> u8 {
        let mut low = 0x0F;
        if self.select & 0x10 == 0 {
            low &= !(self.buttons.0 as u8 & 0x0F) & 0x0F; // directions
        }
        if self.select & 0x20 == 0 {
            low &= !((self.buttons.0 >> 4) as u8 & 0x0F) & 0x0F; // actions
        }
        0xC0 | self.select | low
    }

    pub fn write(&mut self, value: u8) {
        self.select = value & 0x30;
    }

    /// Update state from the frontend. Returns true when any button became
    /// newly pressed (joypad interrupt condition). Simplification: fires
    /// regardless of which matrix line is selected.
    pub fn set_buttons(&mut self, buttons: Buttons) -> bool {
        let newly_pressed = buttons.0 & !self.buttons.0;
        self.buttons = buttons;
        newly_pressed != 0
    }

    pub(crate) fn serialize(&self, w: &mut Writer) {
        w.put_u8(self.select);
    }

    pub(crate) fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.select = r.get_u8()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_selected_reads_all_ones() {
        let mut j = Joypad::default();
        j.set_buttons(Buttons::A.with(Buttons::DOWN));
        j.write(0x30);
        assert_eq!(j.read(), 0xFF);
    }

    #[test]
    fn direction_row_active_low() {
        let mut j = Joypad::default();
        j.set_buttons(Buttons::RIGHT);
        j.write(0x20); // bit 4 low -> directions selected
        assert_eq!(j.read(), 0xEE); // 0xC0 | 0x20 | (0xF & !0x01)
    }

    #[test]
    fn action_row_active_low() {
        let mut j = Joypad::default();
        j.set_buttons(Buttons::A.with(Buttons::START));
        j.write(0x10); // bit 5 low -> actions selected
        assert_eq!(j.read(), 0xD6); // A=bit0, Start=bit3 pressed -> 0110
    }

    #[test]
    fn both_rows_combine() {
        let mut j = Joypad::default();
        j.set_buttons(Buttons::RIGHT.with(Buttons::A));
        j.write(0x00);
        assert_eq!(j.read() & 0x0F, 0x0E); // bit 0 low from both rows
    }

    #[test]
    fn irq_only_on_new_press() {
        let mut j = Joypad::default();
        assert!(j.set_buttons(Buttons::A));
        assert!(!j.set_buttons(Buttons::A)); // still held: no new edge
        assert!(!j.set_buttons(Buttons::default())); // release: no irq
        assert!(j.set_buttons(Buttons::A)); // pressed again
    }
}
