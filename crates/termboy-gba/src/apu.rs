//! GBA APU: the four classic PSG channels (2 pulse, wave, noise) plus the two
//! Direct Sound DMA-FIFO PCM channels, mixed to stereo f32 at the host rate.
//!
//! The PSG channel logic is ported from `termboy-gb/src/apu.rs` — it is
//! T-cycle accurate at the GB's 4.19 MHz, so the APU runs it via a ÷4
//! prescaler off the GBA's 16.78 MHz cycle clock, leaving the ported code
//! unchanged. (A future refactor could share these channels via a crate.)
//! GBA-specific: registers at 0x60-0x84, wave RAM banking, SOUNDCNT volume,
//! and the FIFO channels driven by timer overflow + DMA refill.

use std::collections::VecDeque;

const GBA_CLOCK: f64 = 16_777_216.0;
const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];
const NOISE_DIVISORS: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

#[derive(Default)]
struct Pulse {
    has_sweep: bool,
    enabled: bool,
    nrx0: u8,
    nrx1: u8,
    nrx2: u8,
    freq: u16,
    length_enable: bool,
    duty_pos: u8,
    timer: i32,
    length: u16,
    env_vol: u8,
    env_timer: u8,
    sweep_timer: u8,
    sweep_shadow: u16,
    sweep_enabled: bool,
}

impl Pulse {
    fn dac_on(&self) -> bool {
        self.nrx2 & 0xF8 != 0
    }
    fn tick(&mut self) {
        self.timer -= 1;
        if self.timer <= 0 {
            self.timer += (2048 - self.freq as i32) * 4;
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }
    fn output(&self) -> u8 {
        if !self.enabled || !self.dac_on() {
            return 0;
        }
        let duty = DUTY[(self.nrx1 >> 6) as usize];
        if duty >> (7 - self.duty_pos) & 1 == 1 { self.env_vol } else { 0 }
    }
    fn tick_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }
    fn tick_envelope(&mut self) {
        let period = self.nrx2 & 7;
        if period == 0 {
            return;
        }
        self.env_timer = self.env_timer.saturating_sub(1);
        if self.env_timer == 0 {
            self.env_timer = period;
            if self.nrx2 & 8 != 0 {
                self.env_vol = (self.env_vol + 1).min(15);
            } else {
                self.env_vol = self.env_vol.saturating_sub(1);
            }
        }
    }
    fn sweep_calc(&mut self) -> u16 {
        let shift = self.nrx0 & 7;
        let delta = self.sweep_shadow >> shift;
        let next = if self.nrx0 & 8 != 0 {
            self.sweep_shadow.wrapping_sub(delta)
        } else {
            self.sweep_shadow + delta
        };
        if next > 2047 {
            self.enabled = false;
        }
        next
    }
    fn tick_sweep(&mut self) {
        if !self.sweep_enabled || !self.enabled {
            return;
        }
        self.sweep_timer = self.sweep_timer.saturating_sub(1);
        if self.sweep_timer == 0 {
            let period = (self.nrx0 >> 4) & 7;
            self.sweep_timer = if period == 0 { 8 } else { period };
            if period != 0 {
                let next = self.sweep_calc();
                if next <= 2047 && self.nrx0 & 7 != 0 {
                    self.sweep_shadow = next;
                    self.freq = next;
                    self.sweep_calc();
                }
            }
        }
    }
    fn trigger(&mut self) {
        self.enabled = self.dac_on();
        if self.length == 0 {
            self.length = 64;
        }
        self.timer = (2048 - self.freq as i32) * 4;
        self.env_vol = self.nrx2 >> 4;
        self.env_timer = if self.nrx2 & 7 == 0 { 8 } else { self.nrx2 & 7 };
        if self.has_sweep {
            self.sweep_shadow = self.freq;
            let period = (self.nrx0 >> 4) & 7;
            self.sweep_timer = if period == 0 { 8 } else { period };
            self.sweep_enabled = period != 0 || self.nrx0 & 7 != 0;
            if self.nrx0 & 7 != 0 {
                self.sweep_calc();
            }
        }
    }
}

#[derive(Default)]
struct WaveCh {
    enabled: bool,
    dac: bool,
    nr32: u8,
    freq: u16,
    length_enable: bool,
    length: u16,
    timer: i32,
    pos: u8,
    /// Which 16-byte bank plays; wave-RAM writes target the other bank.
    play_bank: usize,
}

impl WaveCh {
    fn tick(&mut self) {
        self.timer -= 1;
        if self.timer <= 0 {
            self.timer += (2048 - self.freq as i32) * 2;
            self.pos = (self.pos + 1) & 31;
        }
    }
    fn output(&self, wave_ram: &[u8; 32]) -> u8 {
        if !self.enabled || !self.dac {
            return 0;
        }
        let byte = wave_ram[self.play_bank * 16 + (self.pos / 2) as usize];
        let nibble = if self.pos & 1 == 0 { byte >> 4 } else { byte & 0x0F };
        match (self.nr32 >> 5) & 3 {
            0 => 0,
            1 => nibble,
            2 => nibble >> 1,
            _ => nibble >> 2,
        }
    }
    fn tick_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }
    fn trigger(&mut self) {
        self.enabled = self.dac;
        if self.length == 0 {
            self.length = 256;
        }
        self.timer = (2048 - self.freq as i32) * 2;
        self.pos = 0;
    }
}

#[derive(Default)]
struct Noise {
    enabled: bool,
    nr42: u8,
    nr43: u8,
    length_enable: bool,
    length: u16,
    timer: i32,
    lfsr: u16,
    env_vol: u8,
    env_timer: u8,
}

impl Noise {
    fn dac_on(&self) -> bool {
        self.nr42 & 0xF8 != 0
    }
    fn period(&self) -> i32 {
        (NOISE_DIVISORS[(self.nr43 & 7) as usize] << (self.nr43 >> 4)) as i32
    }
    fn tick(&mut self) {
        self.timer -= 1;
        if self.timer <= 0 {
            self.timer += self.period();
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.nr43 & 8 != 0 {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        }
    }
    fn output(&self) -> u8 {
        if !self.enabled || !self.dac_on() {
            return 0;
        }
        if self.lfsr & 1 == 0 { self.env_vol } else { 0 }
    }
    fn tick_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }
    fn tick_envelope(&mut self) {
        let period = self.nr42 & 7;
        if period == 0 {
            return;
        }
        self.env_timer = self.env_timer.saturating_sub(1);
        if self.env_timer == 0 {
            self.env_timer = period;
            if self.nr42 & 8 != 0 {
                self.env_vol = (self.env_vol + 1).min(15);
            } else {
                self.env_vol = self.env_vol.saturating_sub(1);
            }
        }
    }
    fn trigger(&mut self) {
        self.enabled = self.dac_on();
        if self.length == 0 {
            self.length = 64;
        }
        self.timer = self.period();
        self.lfsr = 0x7FFF;
        self.env_vol = self.nr42 >> 4;
        self.env_timer = if self.nr42 & 7 == 0 { 8 } else { self.nr42 & 7 };
    }
}

/// A 32-byte Direct Sound FIFO: one signed byte popped per selected-timer
/// overflow (sample-and-hold), refilled by DMA when it runs low.
#[derive(Default)]
struct Fifo {
    bytes: VecDeque<u8>,
    output: i8,
    needs_dma: bool,
}

impl Fifo {
    fn push(&mut self, b: u8) {
        if self.bytes.len() < 32 {
            self.bytes.push_back(b);
        }
    }
    fn step(&mut self) {
        if let Some(b) = self.bytes.pop_front() {
            self.output = b as i8;
        }
        self.needs_dma = self.bytes.len() <= 16;
    }
}

pub struct Apu {
    on: bool,
    seq_timer: u32,
    seq_step: u8,
    prescale: u32,
    ch1: Pulse,
    ch2: Pulse,
    ch3: WaveCh,
    ch4: Noise,
    soundcnt_l: u16,
    soundcnt_h: u16,
    pub wave_ram: [u8; 32],
    fifo: [Fifo; 2],
    sample_timer: f64,
    sample_period: f64,
    pub samples: Vec<(f32, f32)>,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            on: false,
            seq_timer: 0,
            seq_step: 0,
            prescale: 0,
            ch1: Pulse { has_sweep: true, ..Pulse::default() },
            ch2: Pulse::default(),
            ch3: WaveCh::default(),
            ch4: Noise::default(),
            soundcnt_l: 0,
            soundcnt_h: 0,
            wave_ram: [0; 32],
            fifo: Default::default(),
            sample_timer: 0.0,
            sample_period: GBA_CLOCK / 48_000.0,
            samples: Vec::new(),
        }
    }

    pub fn set_sample_rate(&mut self, hz: u32) {
        self.sample_period = GBA_CLOCK / hz.max(1) as f64;
    }

    /// Advance by `cycles` GBA cycles: PSG at GB rate (÷4), host-rate sampling.
    pub fn tick(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.prescale += 1;
            if self.prescale == 4 {
                self.prescale = 0;
                self.tick_psg();
            }
            self.sample_timer += 1.0;
            if self.sample_timer >= self.sample_period {
                self.sample_timer -= self.sample_period;
                let s = self.mix();
                self.samples.push(s);
            }
        }
    }

    /// One GB T-cycle of PSG (channel timers + the 512 Hz frame sequencer).
    fn tick_psg(&mut self) {
        if !self.on {
            return;
        }
        self.ch1.tick();
        self.ch2.tick();
        self.ch3.tick();
        self.ch4.tick();
        self.seq_timer += 1;
        if self.seq_timer == 8192 {
            self.seq_timer = 0;
            match self.seq_step {
                0 | 4 => self.tick_lengths(),
                2 | 6 => {
                    self.tick_lengths();
                    self.ch1.tick_sweep();
                }
                7 => {
                    self.ch1.tick_envelope();
                    self.ch2.tick_envelope();
                    self.ch4.tick_envelope();
                }
                _ => {}
            }
            self.seq_step = (self.seq_step + 1) & 7;
        }
    }

    fn tick_lengths(&mut self) {
        self.ch1.tick_length();
        self.ch2.tick_length();
        self.ch3.tick_length();
        self.ch4.tick_length();
    }

    fn mix(&self) -> (f32, f32) {
        if !self.on {
            return (0.0, 0.0);
        }
        let analog = |v: u8, dac: bool| if dac { v as f32 / 7.5 - 1.0 } else { 0.0 };
        let outs = [
            analog(self.ch1.output(), self.ch1.dac_on() && self.ch1.enabled),
            analog(self.ch2.output(), self.ch2.dac_on() && self.ch2.enabled),
            analog(self.ch3.output(&self.wave_ram), self.ch3.dac && self.ch3.enabled),
            analog(self.ch4.output(), self.ch4.dac_on() && self.ch4.enabled),
        ];
        // SOUNDCNT_L: low byte = volumes (NR50), high byte = enables (NR51).
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for (i, s) in outs.iter().enumerate() {
            if self.soundcnt_l >> (12 + i) & 1 == 1 {
                l += s;
            }
            if self.soundcnt_l >> (8 + i) & 1 == 1 {
                r += s;
            }
        }
        l *= (1 + (self.soundcnt_l >> 4 & 7)) as f32 / 8.0;
        r *= (1 + (self.soundcnt_l & 7)) as f32 / 8.0;
        // SOUNDCNT_H bits 0-1: PSG volume 0->25% 1->50% 2->100%.
        let psg_vol = match self.soundcnt_h & 3 {
            0 => 0.25,
            1 => 0.5,
            _ => 1.0,
        };
        l *= psg_vol;
        r *= psg_vol;
        // Direct Sound A/B: 8-bit signed; vol bit (2=A,3=B) 0->50% 1->100%;
        // enables bits 8/9 (A R/L) and 12/13 (B R/L).
        for w in 0..2 {
            let vol = if self.soundcnt_h & (1 << (2 + w)) != 0 { 1.0 } else { 0.5 };
            let s = self.fifo[w].output as f32 / 128.0 * vol;
            if self.soundcnt_h & (1 << (9 + w * 4)) != 0 {
                l += s;
            }
            if self.soundcnt_h & (1 << (8 + w * 4)) != 0 {
                r += s;
            }
        }
        (l * 0.25, r * 0.25)
    }

    fn status(&self) -> u8 {
        (self.ch1.enabled as u8)
            | (self.ch2.enabled as u8) << 1
            | (self.ch3.enabled as u8) << 2
            | (self.ch4.enabled as u8) << 3
    }

    // ---- Direct Sound hooks (driven by the bus/timers/DMA) ----

    /// FIFO data-port write (DMA target). `which`: 0 = A, 1 = B.
    pub fn fifo_push(&mut self, which: usize, byte: u8) {
        self.fifo[which].push(byte);
    }

    /// A timer overflowed `times`; advance whichever FIFOs it drives.
    pub fn on_timer_overflow(&mut self, timer: usize, times: u32) {
        for w in 0..2 {
            let sel = (self.soundcnt_h >> (10 + w * 4)) & 1;
            if sel as usize == timer {
                for _ in 0..times {
                    self.fifo[w].step();
                }
            }
        }
    }

    /// Take-and-clear the refill request for FIFO `which`.
    pub fn fifo_needs_dma(&mut self, which: usize) -> bool {
        std::mem::take(&mut self.fifo[which].needs_dma)
    }

    pub fn read(&self, reg: usize) -> u8 {
        match reg {
            0x60 => self.ch1.nrx0 | 0x80,
            0x62 => self.ch1.nrx1 | 0x3F,
            0x63 => self.ch1.nrx2,
            0x65 => ((self.ch1.length_enable as u8) << 6) | 0xBF,
            0x68 => self.ch2.nrx1 | 0x3F,
            0x69 => self.ch2.nrx2,
            0x6D => ((self.ch2.length_enable as u8) << 6) | 0xBF,
            0x70 => ((self.ch3.dac as u8) << 7) | 0x7F,
            0x72 => self.ch3.nr32 | 0x1F,
            0x75 => ((self.ch3.length_enable as u8) << 6) | 0xBF,
            0x79 => self.ch4.nr42,
            0x7C => self.ch4.nr43,
            0x7D => ((self.ch4.length_enable as u8) << 6) | 0xBF,
            0x80 => self.soundcnt_l as u8,
            0x81 => (self.soundcnt_l >> 8) as u8,
            0x82 => self.soundcnt_h as u8,
            0x83 => (self.soundcnt_h >> 8) as u8,
            0x84 => ((self.on as u8) << 7) | self.status() | 0x70,
            0x90..=0x9F => self.wave_ram[(1 - self.ch3.play_bank) * 16 + (reg - 0x90)],
            _ => 0,
        }
    }

    pub fn write(&mut self, reg: usize, v: u8) {
        // Wave RAM writes target the bank not currently playing.
        if let 0x90..=0x9F = reg {
            self.wave_ram[(1 - self.ch3.play_bank) * 16 + (reg - 0x90)] = v;
            return;
        }
        // SOUNDCNT_X master switch and FIFO control are always writable.
        match reg {
            0x84 => {
                self.on = v & 0x80 != 0;
                if !self.on {
                    let wave = self.wave_ram;
                    *self = Apu { samples: std::mem::take(&mut self.samples), ..Apu::new() };
                    self.wave_ram = wave;
                }
                return;
            }
            0x82 | 0x83 => {
                let shift = (reg - 0x82) * 8;
                self.soundcnt_h = (self.soundcnt_h & !(0xFF << shift)) | ((v as u16) << shift);
                if v & (1 << 3) != 0 && reg == 0x83 {
                    self.fifo[0] = Fifo::default(); // bit 11: reset FIFO A
                }
                if v & (1 << 7) != 0 && reg == 0x83 {
                    self.fifo[1] = Fifo::default(); // bit 15: reset FIFO B
                }
                return;
            }
            _ => {}
        }
        if !self.on {
            return; // PSG registers are read-only while master-disabled
        }
        match reg {
            0x60 => self.ch1.nrx0 = v,
            0x62 => {
                self.ch1.nrx1 = v;
                self.ch1.length = 64 - (v & 0x3F) as u16;
            }
            0x63 => {
                self.ch1.nrx2 = v;
                if !self.ch1.dac_on() {
                    self.ch1.enabled = false;
                }
            }
            0x64 => self.ch1.freq = (self.ch1.freq & 0x700) | v as u16,
            0x65 => {
                self.ch1.freq = (self.ch1.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch1.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch1.trigger();
                }
            }
            0x68 => {
                self.ch2.nrx1 = v;
                self.ch2.length = 64 - (v & 0x3F) as u16;
            }
            0x69 => {
                self.ch2.nrx2 = v;
                if !self.ch2.dac_on() {
                    self.ch2.enabled = false;
                }
            }
            0x6C => self.ch2.freq = (self.ch2.freq & 0x700) | v as u16,
            0x6D => {
                self.ch2.freq = (self.ch2.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch2.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch2.trigger();
                }
            }
            0x70 => {
                self.ch3.dac = v & 0x80 != 0;
                self.ch3.play_bank = ((v >> 6) & 1) as usize;
                if !self.ch3.dac {
                    self.ch3.enabled = false;
                }
            }
            0x72 => self.ch3.length = 256 - v as u16,
            0x73 => self.ch3.nr32 = v,
            0x74 => self.ch3.freq = (self.ch3.freq & 0x700) | v as u16,
            0x75 => {
                self.ch3.freq = (self.ch3.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch3.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch3.trigger();
                }
            }
            0x78 => self.ch4.length = 64 - (v & 0x3F) as u16,
            0x79 => {
                self.ch4.nr42 = v;
                if !self.ch4.dac_on() {
                    self.ch4.enabled = false;
                }
            }
            0x7C => self.ch4.nr43 = v,
            0x7D => {
                self.ch4.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch4.trigger();
                }
            }
            0x80 => self.soundcnt_l = (self.soundcnt_l & 0xFF00) | v as u16,
            0x81 => self.soundcnt_l = (self.soundcnt_l & 0xFF) | ((v as u16) << 8),
            _ => {}
        }
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apu() -> Apu {
        let mut a = Apu::new();
        a.write(0x84, 0x80); // master enable
        a.set_sample_rate(48_000);
        a
    }

    #[test]
    fn pulse_length_expires() {
        let mut a = apu();
        a.write(0x68, 0x3E); // ch2 length=2, duty 0
        a.write(0x69, 0xF0); // full vol
        a.write(0x6D, 0xC0); // trigger + length enable
        assert_eq!(a.read(0x84) & 0x02, 0x02);
        a.tick(4 * 8192 * 4); // 4 sequencer steps (x4 prescaler)
        assert_eq!(a.read(0x84) & 0x02, 0x00);
    }

    #[test]
    fn produces_samples_at_rate() {
        let mut a = apu();
        a.tick(16_777_216 / 60);
        let n = a.samples.len();
        assert!((795..=805).contains(&n), "got {n}");
    }

    #[test]
    fn master_off_silences() {
        let mut a = apu();
        a.write(0x68, 0x00);
        a.write(0x69, 0xF0);
        a.write(0x6D, 0x80); // trigger ch2
        a.write(0x84, 0x00); // master off
        a.tick(8192 * 4);
        assert!(a.samples.iter().all(|&(l, r)| l == 0.0 && r == 0.0));
    }

    #[test]
    fn fifo_outputs_on_selected_timer_overflow() {
        let mut a = apu();
        a.write(0x82, 0x02); // PSG 100% (keeps scaling sane)
        a.write(0x83, 0x0B); // A enabled L+R (bits 8,9), timer0 (bit10=0)
        a.fifo_push(0, 0x40); // +64
        a.on_timer_overflow(0, 1);
        a.tick(1000);
        assert!(a.samples.iter().any(|&(l, _)| l > 0.0));
    }

    #[test]
    fn fifo_ignores_the_other_timer() {
        let mut a = apu();
        a.write(0x83, 0x0B); // timer 0 drives A
        a.fifo_push(0, 0x40);
        a.on_timer_overflow(1, 5); // wrong timer
        assert_eq!(a.fifo[0].bytes.len(), 1); // not consumed
    }
}
