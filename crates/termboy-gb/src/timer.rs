//! DIV/TIMA/TMA/TAC timer with the real falling-edge detector behavior.

#[derive(Default)]
pub struct Timer {
    counter: u16, // internal divider; DIV is the high byte
    pub tima: u8,
    pub tma: u8,
    tac: u8,
    prev_bit: bool,
    overflow_delay: u8, // T-cycles until TIMA reloads from TMA
}

impl Timer {
    /// Advance one T-cycle. Returns true when the timer interrupt fires.
    pub fn tick(&mut self) -> bool {
        let mut irq = false;
        if self.overflow_delay > 0 {
            self.overflow_delay -= 1;
            if self.overflow_delay == 0 {
                self.tima = self.tma;
                irq = true;
            }
        }
        self.counter = self.counter.wrapping_add(1);
        self.update_edge();
        irq
    }

    fn update_edge(&mut self) {
        let bit = match self.tac & 0x03 {
            0 => 9, // 4096 Hz
            1 => 3, // 262144 Hz
            2 => 5, // 65536 Hz
            _ => 7, // 16384 Hz
        };
        let cur = (self.tac & 0x04 != 0) && (self.counter >> bit) & 1 == 1;
        if self.prev_bit && !cur {
            let (next, overflowed) = self.tima.overflowing_add(1);
            self.tima = next;
            if overflowed {
                self.overflow_delay = 4;
            }
        }
        self.prev_bit = cur;
    }

    pub fn read_div(&self) -> u8 { (self.counter >> 8) as u8 }
    /// Writing any value to DIV resets the whole internal counter.
    /// This can itself produce a falling edge — update_edge handles it.
    pub fn write_div(&mut self) {
        self.counter = 0;
        self.update_edge();
    }
    pub fn read_tac(&self) -> u8 { self.tac | 0xF8 }
    pub fn write_tac(&mut self, value: u8) {
        self.tac = value & 0x07;
        self.update_edge();
    }
    pub fn write_tima(&mut self, value: u8) {
        self.tima = value;
        self.overflow_delay = 0; // a write during the delay cancels the reload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_n(t: &mut Timer, n: u32) -> bool {
        let mut irq = false;
        for _ in 0..n {
            irq |= t.tick();
        }
        irq
    }

    #[test]
    fn div_increments_every_256_tcycles() {
        let mut t = Timer::default();
        tick_n(&mut t, 255);
        assert_eq!(t.read_div(), 0);
        tick_n(&mut t, 1);
        assert_eq!(t.read_div(), 1);
    }

    #[test]
    fn div_write_resets_counter() {
        let mut t = Timer::default();
        tick_n(&mut t, 300);
        t.write_div();
        assert_eq!(t.read_div(), 0);
    }

    #[test]
    fn tima_ticks_at_selected_rate() {
        let mut t = Timer::default();
        t.write_tac(0b101); // enabled, bit 3 -> period 16 T-cycles
        tick_n(&mut t, 16);
        assert_eq!(t.tima, 1);
        tick_n(&mut t, 16 * 9);
        assert_eq!(t.tima, 10);
    }

    #[test]
    fn tima_overflow_reloads_from_tma_after_4_cycles_and_raises_irq() {
        let mut t = Timer::default();
        t.write_tac(0b101);
        t.tma = 0xAB;
        t.tima = 0xFF;
        tick_n(&mut t, 16); // overflow happens here
        assert_eq!(t.tima, 0); // reads 0 during the delay
        let irq = tick_n(&mut t, 4);
        assert!(irq);
        assert_eq!(t.tima, 0xAB);
    }

    #[test]
    fn disabled_timer_does_not_tick_tima() {
        let mut t = Timer::default();
        t.write_tac(0b001); // bit-3 rate but NOT enabled
        tick_n(&mut t, 256);
        assert_eq!(t.tima, 0);
    }
}
