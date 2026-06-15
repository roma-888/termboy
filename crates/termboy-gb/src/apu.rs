//! APU: 2 pulse channels (ch1 with sweep), wave, noise. A 512 Hz frame
//! sequencer clocks lengths/envelopes/sweep; output is mixed to stereo f32
//! at the host sample rate. Documented simplifications: no zombie-mode
//! envelope writes, no length-clock trigger edge cases.

use termboy_core::state::{Reader, StateError, Writer};

const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];
const NOISE_DIVISORS: [u32; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

#[derive(Default)]
struct Pulse {
    has_sweep: bool,
    enabled: bool,
    nrx0: u8,
    nrx1: u8,
    nrx2: u8,
    freq: u16, // 11 bits from NRx3/NRx4
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

    /// Returns the next swept frequency; disables the channel on overflow.
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
                    self.sweep_calc(); // second overflow check
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

    fn serialize(&self, w: &mut Writer) {
        w.put_bool(self.has_sweep);
        w.put_bool(self.enabled);
        w.put_u8(self.nrx0);
        w.put_u8(self.nrx1);
        w.put_u8(self.nrx2);
        w.put_u16(self.freq);
        w.put_bool(self.length_enable);
        w.put_u8(self.duty_pos);
        w.put_u32(self.timer as u32);
        w.put_u16(self.length);
        w.put_u8(self.env_vol);
        w.put_u8(self.env_timer);
        w.put_u8(self.sweep_timer);
        w.put_u16(self.sweep_shadow);
        w.put_bool(self.sweep_enabled);
    }

    fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.has_sweep = r.get_bool()?;
        self.enabled = r.get_bool()?;
        self.nrx0 = r.get_u8()?;
        self.nrx1 = r.get_u8()?;
        self.nrx2 = r.get_u8()?;
        self.freq = r.get_u16()?;
        self.length_enable = r.get_bool()?;
        self.duty_pos = r.get_u8()?;
        self.timer = r.get_u32()? as i32;
        self.length = r.get_u16()?;
        self.env_vol = r.get_u8()?;
        self.env_timer = r.get_u8()?;
        self.sweep_timer = r.get_u8()?;
        self.sweep_shadow = r.get_u16()?;
        self.sweep_enabled = r.get_bool()?;
        Ok(())
    }
}

#[derive(Default)]
struct WaveCh {
    enabled: bool,
    dac: bool, // NR30 bit 7
    nr32: u8,
    freq: u16,
    length_enable: bool,
    length: u16, // 0..=256
    timer: i32,
    pos: u8, // 0..32
}

impl WaveCh {
    fn tick(&mut self) {
        self.timer -= 1;
        if self.timer <= 0 {
            self.timer += (2048 - self.freq as i32) * 2;
            self.pos = (self.pos + 1) & 31;
        }
    }

    fn output(&self, wave_ram: &[u8; 16]) -> u8 {
        if !self.enabled || !self.dac {
            return 0;
        }
        let byte = wave_ram[(self.pos / 2) as usize];
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

    fn serialize(&self, w: &mut Writer) {
        w.put_bool(self.enabled);
        w.put_bool(self.dac);
        w.put_u8(self.nr32);
        w.put_u16(self.freq);
        w.put_bool(self.length_enable);
        w.put_u16(self.length);
        w.put_u32(self.timer as u32);
        w.put_u8(self.pos);
    }

    fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.enabled = r.get_bool()?;
        self.dac = r.get_bool()?;
        self.nr32 = r.get_u8()?;
        self.freq = r.get_u16()?;
        self.length_enable = r.get_bool()?;
        self.length = r.get_u16()?;
        self.timer = r.get_u32()? as i32;
        self.pos = r.get_u8()?;
        Ok(())
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

    fn serialize(&self, w: &mut Writer) {
        w.put_bool(self.enabled);
        w.put_u8(self.nr42);
        w.put_u8(self.nr43);
        w.put_bool(self.length_enable);
        w.put_u16(self.length);
        w.put_u32(self.timer as u32);
        w.put_u16(self.lfsr);
        w.put_u8(self.env_vol);
        w.put_u8(self.env_timer);
    }

    fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.enabled = r.get_bool()?;
        self.nr42 = r.get_u8()?;
        self.nr43 = r.get_u8()?;
        self.length_enable = r.get_bool()?;
        self.length = r.get_u16()?;
        self.timer = r.get_u32()? as i32;
        self.lfsr = r.get_u16()?;
        self.env_vol = r.get_u8()?;
        self.env_timer = r.get_u8()?;
        Ok(())
    }
}

pub struct Apu {
    on: bool,
    seq_timer: u32,
    seq_step: u8,
    ch1: Pulse,
    ch2: Pulse,
    ch3: WaveCh,
    ch4: Noise,
    nr50: u8,
    nr51: u8,
    pub wave_ram: [u8; 16],
    sample_timer: f64,
    sample_period: f64, // T-cycles per host sample
    pub samples: Vec<(f32, f32)>,
}

impl Apu {
    pub fn new() -> Self {
        let mut apu = Self {
            on: true,
            seq_timer: 0,
            seq_step: 0,
            ch1: Pulse { has_sweep: true, ..Pulse::default() },
            ch2: Pulse::default(),
            ch3: WaveCh::default(),
            ch4: Noise::default(),
            nr50: 0x77,
            nr51: 0xF3,
            wave_ram: [0; 16],
            sample_timer: 0.0,
            sample_period: 4_194_304.0 / 48_000.0,
            samples: Vec::new(),
        };
        // post-boot ch1 state (the boot chime leaves it configured)
        apu.ch1.nrx1 = 0xBF;
        apu.ch1.nrx2 = 0xF3;
        apu
    }

    pub fn set_sample_rate(&mut self, hz: u32) {
        self.sample_period = 4_194_304.0 / hz.max(1) as f64;
    }

    pub fn tick(&mut self, tcycles: u32) {
        for _ in 0..tcycles {
            if self.on {
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
            self.sample_timer += 1.0;
            if self.sample_timer >= self.sample_period {
                self.sample_timer -= self.sample_period;
                let s = self.mix();
                self.samples.push(s);
            }
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
        // DAC maps 0-15 to +1..-1; an enabled-but-silent channel sits at +1
        // bias on hardware, but centering at 0 avoids pops and sounds the same.
        let analog = |v: u8, dac: bool| if dac { v as f32 / 7.5 - 1.0 } else { 0.0 };
        let outs = [
            analog(self.ch1.output(), self.ch1.dac_on() && self.ch1.enabled),
            analog(self.ch2.output(), self.ch2.dac_on() && self.ch2.enabled),
            analog(self.ch3.output(&self.wave_ram), self.ch3.dac && self.ch3.enabled),
            analog(self.ch4.output(), self.ch4.dac_on() && self.ch4.enabled),
        ];
        let mut l = 0.0;
        let mut r = 0.0;
        for (i, s) in outs.iter().enumerate() {
            if self.nr51 >> (4 + i) & 1 == 1 {
                l += s;
            }
            if self.nr51 >> i & 1 == 1 {
                r += s;
            }
        }
        l *= (1 + (self.nr50 >> 4 & 7)) as f32 / 8.0;
        r *= (1 + (self.nr50 & 7)) as f32 / 8.0;
        (l * 0.25, r * 0.25)
    }

    /// True while the channel counts toward the NR52 status bits.
    fn status(&self) -> u8 {
        (self.ch1.enabled as u8)
            | (self.ch2.enabled as u8) << 1
            | (self.ch3.enabled as u8) << 2
            | (self.ch4.enabled as u8) << 3
    }

    pub fn read(&self, addr: u16) -> u8 {
        const MASKS: [u8; 23] = [
            0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-14
            0xFF, 0x3F, 0x00, 0xFF, 0xBF, // NR20-24
            0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-34
            0xFF, 0xFF, 0x00, 0x00, 0xBF, // NR40-44
            0x00, 0x00, 0x70, // NR50-52
        ];
        let raw = match addr {
            0xFF10 => self.ch1.nrx0,
            0xFF11 => self.ch1.nrx1,
            0xFF12 => self.ch1.nrx2,
            0xFF14 => (self.ch1.length_enable as u8) << 6,
            0xFF16 => self.ch2.nrx1,
            0xFF17 => self.ch2.nrx2,
            0xFF19 => (self.ch2.length_enable as u8) << 6,
            0xFF1A => (self.ch3.dac as u8) << 7,
            0xFF1C => self.ch3.nr32,
            0xFF1E => (self.ch3.length_enable as u8) << 6,
            0xFF21 => self.ch4.nr42,
            0xFF22 => self.ch4.nr43,
            0xFF23 => (self.ch4.length_enable as u8) << 6,
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => ((self.on as u8) << 7) | self.status(),
            0xFF30..=0xFF3F => return self.wave_ram[(addr - 0xFF30) as usize],
            _ => 0,
        };
        raw | MASKS[(addr - 0xFF10) as usize]
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        if let 0xFF30..=0xFF3F = addr {
            self.wave_ram[(addr - 0xFF30) as usize] = v;
            return;
        }
        if !self.on && addr != 0xFF26 {
            return; // powered off: registers are read-only
        }
        match addr {
            0xFF10 => self.ch1.nrx0 = v,
            0xFF11 => {
                self.ch1.nrx1 = v;
                self.ch1.length = 64 - (v & 0x3F) as u16;
            }
            0xFF12 => {
                self.ch1.nrx2 = v;
                if !self.ch1.dac_on() {
                    self.ch1.enabled = false;
                }
            }
            0xFF13 => self.ch1.freq = (self.ch1.freq & 0x700) | v as u16,
            0xFF14 => {
                self.ch1.freq = (self.ch1.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch1.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch1.trigger();
                }
            }
            0xFF16 => {
                self.ch2.nrx1 = v;
                self.ch2.length = 64 - (v & 0x3F) as u16;
            }
            0xFF17 => {
                self.ch2.nrx2 = v;
                if !self.ch2.dac_on() {
                    self.ch2.enabled = false;
                }
            }
            0xFF18 => self.ch2.freq = (self.ch2.freq & 0x700) | v as u16,
            0xFF19 => {
                self.ch2.freq = (self.ch2.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch2.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch2.trigger();
                }
            }
            0xFF1A => {
                self.ch3.dac = v & 0x80 != 0;
                if !self.ch3.dac {
                    self.ch3.enabled = false;
                }
            }
            0xFF1B => self.ch3.length = 256 - v as u16,
            0xFF1C => self.ch3.nr32 = v,
            0xFF1D => self.ch3.freq = (self.ch3.freq & 0x700) | v as u16,
            0xFF1E => {
                self.ch3.freq = (self.ch3.freq & 0xFF) | ((v as u16 & 7) << 8);
                self.ch3.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch3.trigger();
                }
            }
            0xFF20 => self.ch4.length = 64 - (v & 0x3F) as u16,
            0xFF21 => {
                self.ch4.nr42 = v;
                if !self.ch4.dac_on() {
                    self.ch4.enabled = false;
                }
            }
            0xFF22 => self.ch4.nr43 = v,
            0xFF23 => {
                self.ch4.length_enable = v & 0x40 != 0;
                if v & 0x80 != 0 {
                    self.ch4.trigger();
                }
            }
            0xFF24 => self.nr50 = v,
            0xFF25 => self.nr51 = v,
            0xFF26 => {
                let was_on = self.on;
                self.on = v & 0x80 != 0;
                if was_on && !self.on {
                    // power off clears every register (wave RAM survives)
                    let wave = self.wave_ram;
                    *self = Apu { samples: std::mem::take(&mut self.samples), ..Apu::new() };
                    self.wave_ram = wave;
                    self.on = false;
                    self.ch1 = Pulse { has_sweep: true, ..Pulse::default() };
                    self.nr50 = 0;
                    self.nr51 = 0;
                }
                if !was_on && self.on {
                    self.seq_step = 0;
                    self.seq_timer = 0;
                }
            }
            _ => {}
        }
    }

    pub(crate) fn serialize(&self, w: &mut Writer) {
        w.put_bool(self.on);
        w.put_u32(self.seq_timer);
        w.put_u8(self.seq_step);
        self.ch1.serialize(w);
        self.ch2.serialize(w);
        self.ch3.serialize(w);
        self.ch4.serialize(w);
        w.put_u8(self.nr50);
        w.put_u8(self.nr51);
        w.put_bytes(&self.wave_ram);
    }

    pub(crate) fn deserialize(&mut self, r: &mut Reader) -> Result<(), StateError> {
        self.on = r.get_bool()?;
        self.seq_timer = r.get_u32()?;
        self.seq_step = r.get_u8()?;
        self.ch1.deserialize(r)?;
        self.ch2.deserialize(r)?;
        self.ch3.deserialize(r)?;
        self.ch4.deserialize(r)?;
        self.nr50 = r.get_u8()?;
        self.nr51 = r.get_u8()?;
        self.wave_ram.copy_from_slice(r.get_bytes(16)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEQ: u32 = 8192; // one frame-sequencer step

    fn apu() -> Apu {
        let mut a = Apu::new();
        a.set_sample_rate(48_000);
        a
    }

    fn trigger_ch2(a: &mut Apu, length: u8, length_enable: bool) {
        a.write(0xFF16, length & 0x3F); // duty 0, length
        a.write(0xFF17, 0xF0); // full volume, no envelope sweep
        a.write(0xFF18, 0x00);
        let nr24 = 0x80 | if length_enable { 0x40 } else { 0 };
        a.write(0xFF19, nr24);
    }

    #[test]
    fn length_counter_expires_channel() {
        let mut a = apu();
        trigger_ch2(&mut a, 0x3E, true); // length = 2
        assert_eq!(a.read(0xFF26) & 0x02, 0x02); // ch2 on
        a.tick(SEQ * 4); // length clocks on steps 0,2 (2 clocks within 4 steps)
        assert_eq!(a.read(0xFF26) & 0x02, 0x00); // expired
    }

    #[test]
    fn length_disabled_keeps_playing() {
        let mut a = apu();
        trigger_ch2(&mut a, 0x3E, false);
        a.tick(SEQ * 8);
        assert_eq!(a.read(0xFF26) & 0x02, 0x02);
    }

    #[test]
    fn envelope_decays_volume() {
        let mut a = apu();
        a.write(0xFF16, 0x00);
        a.write(0xFF17, 0xF1); // vol 15, decrease, period 1
        a.write(0xFF19, 0x80); // trigger
        a.tick(SEQ * 8); // one full sequence: envelope clocks once (step 7)
        assert_eq!(a.ch2.env_vol, 14);
        a.tick(SEQ * 8 * 14);
        assert_eq!(a.ch2.env_vol, 0);
    }

    #[test]
    fn sweep_overflow_disables_ch1() {
        let mut a = apu();
        a.write(0xFF10, 0x11); // period 1, add, shift 1
        a.write(0xFF12, 0xF0);
        a.write(0xFF13, 0xFF);
        a.write(0xFF14, 0x87); // freq 0x7FF, trigger -> immediate overflow check
        assert_eq!(a.read(0xFF26) & 0x01, 0x00); // 0x7FF + 0x3FF > 2047: off
    }

    #[test]
    fn dac_off_silences_and_disables() {
        let mut a = apu();
        trigger_ch2(&mut a, 0, false);
        a.write(0xFF17, 0x00); // DAC off
        assert_eq!(a.read(0xFF26) & 0x02, 0x00);
    }

    #[test]
    fn noise_lfsr_runs_and_produces_output() {
        let mut a = apu();
        a.write(0xFF21, 0xF0); // vol 15
        a.write(0xFF22, 0x00); // fastest: divisor 8, shift 0
        a.write(0xFF23, 0x80); // trigger
        // after trigger lfsr = 0x7FFF (all ones): output 0. Tick a while.
        a.tick(8 * 40);
        // LFSR has shifted: some zeros have entered bit 0 at some point
        assert_eq!(a.ch4.lfsr & 0x8000, 0); // 15-bit register
        assert_ne!(a.ch4.lfsr, 0x7FFF);
    }

    #[test]
    fn wave_plays_ram_nibbles() {
        let mut a = apu();
        a.wave_ram[0] = 0xAB;
        a.write(0xFF1A, 0x80); // DAC on
        a.write(0xFF1C, 0x20); // volume 100%
        a.write(0xFF1D, 0x00);
        a.write(0xFF1E, 0x80); // trigger, freq 0 -> period (2048)*2
        assert_eq!(a.ch3.output(&a.wave_ram), 0x0A); // first nibble, high
        a.tick(2048 * 2);
        assert_eq!(a.ch3.output(&a.wave_ram), 0x0B);
    }

    #[test]
    fn sample_pacing_matches_rate() {
        let mut a = apu();
        a.tick(4_194_304 / 60);
        let n = a.samples.len();
        assert!((795..=805).contains(&n), "got {n} samples"); // ~800 at 48kHz/60fps
    }

    #[test]
    fn power_off_clears_registers_and_blocks_writes() {
        let mut a = apu();
        a.write(0xFF25, 0xAA);
        a.write(0xFF26, 0x00); // power off
        assert_eq!(a.read(0xFF25), 0x00);
        a.write(0xFF25, 0x55); // ignored while off
        assert_eq!(a.read(0xFF25), 0x00);
        assert_eq!(a.read(0xFF26) & 0x80, 0x00);
        a.write(0xFF26, 0x80); // power on
        a.write(0xFF25, 0x55);
        assert_eq!(a.read(0xFF25), 0x55);
    }

    #[test]
    fn mix_pans_via_nr51() {
        let mut a = apu();
        a.write(0xFF24, 0x77);
        a.write(0xFF25, 0x02); // ch2 right only
        trigger_ch2(&mut a, 0, false);
        // find a sample where the duty is high: tick a bit and inspect mix
        a.tick(8192);
        let (l, r) = a.samples.iter().fold((0.0f32, 0.0f32), |(al, ar), (sl, sr)| {
            (al.max(sl.abs()), ar.max(sr.abs()))
        });
        assert_eq!(l, 0.0);
        assert!(r > 0.0);
    }
}
