//! Buttons -> KEYINPUT (0x4000130). Active low: 0 = pressed. The bit order
//! is GBA-native and differs from the frontend Buttons layout — this table
//! is the only place that knows both.

use termboy_core::Buttons;

pub fn keyinput(b: Buttons) -> u16 {
    let pairs = [
        (Buttons::A, 0),
        (Buttons::B, 1),
        (Buttons::SELECT, 2),
        (Buttons::START, 3),
        (Buttons::RIGHT, 4),
        (Buttons::LEFT, 5),
        (Buttons::UP, 6),
        (Buttons::DOWN, 7),
        (Buttons::R, 8),
        (Buttons::L, 9),
    ];
    let mut down = 0u16;
    for (btn, bit) in pairs {
        if b.contains(btn) {
            down |= 1 << bit;
        }
    }
    !down & 0x03FF
}

#[cfg(test)]
mod tests {
    use super::*;
    use termboy_core::Buttons;

    #[test]
    fn released_is_all_ones() {
        assert_eq!(keyinput(Buttons::default()), 0x03FF);
    }

    #[test]
    fn buttons_map_to_gba_bit_order() {
        assert_eq!(keyinput(Buttons::A), 0x03FE); // bit 0 low
        assert_eq!(keyinput(Buttons::START), 0x03F7); // bit 3 low
        assert_eq!(keyinput(Buttons::DOWN), 0x037F); // bit 7 low
        assert_eq!(keyinput(Buttons::R), 0x02FF); // bit 8 low
        assert_eq!(keyinput(Buttons::L), 0x01FF); // bit 9 low
        assert_eq!(keyinput(Buttons::L.with(Buttons::R)), 0x00FF);
    }
}
