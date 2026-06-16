//! Optional GBA color correction. The GBA LCD has a dark, washed-out gamut;
//! games were authored for it, so raw RGB555 looks oversaturated on a modern
//! sRGB display. This maps each 15-bit GBA color through the canonical
//! Talarubi/byuu curve (bsnes.org "Color Emulation"): per-channel input gamma,
//! a 3x3 channel-mixing matrix, output gamma, and the panel's ~9% darkening.
//!
//! GBA color is only 15 bits, so a 32768-entry table makes the per-pixel cost a
//! lookup; the float math runs once when correction is enabled.

use termboy_core::Rgb;

const LCD_GAMMA: f64 = 4.0;
const OUT_GAMMA: f64 = 2.2;
/// 8-bit output scale; the 255/280 factor is the GBA panel's slight darkening.
const SCALE: f64 = 255.0 * 255.0 / 280.0;

/// Talarubi/byuu GBA color correction for one BGR555 color.
pub fn correct(c: u16) -> Rgb {
    let r = (c & 31) as f64 / 31.0;
    let g = ((c >> 5) & 31) as f64 / 31.0;
    let b = ((c >> 10) & 31) as f64 / 31.0;
    let lr = r.powf(LCD_GAMMA);
    let lg = g.powf(LCD_GAMMA);
    let lb = b.powf(LCD_GAMMA);
    let ch = |mixed: f64| -> u8 {
        let v = (mixed / 255.0).powf(1.0 / OUT_GAMMA) * SCALE;
        v.round().clamp(0.0, 255.0) as u8
    };
    Rgb(
        ch(0.0 * lb + 50.0 * lg + 255.0 * lr),
        ch(30.0 * lb + 230.0 * lg + 10.0 * lr),
        ch(220.0 * lb + 10.0 * lg + 50.0 * lr),
    )
}

/// Precompute the correction for all 32768 GBA colors.
pub fn build_lut() -> Box<[Rgb; 32768]> {
    let v: Vec<Rgb> = (0..32768u32).map(|i| correct(i as u16)).collect();
    v.into_boxed_slice().try_into().expect("32768 entries")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_pins_canonical_values() {
        assert_eq!(correct(0x0000), Rgb(0, 0, 0));
        assert_eq!(correct(0x7FFF), Rgb(252, 238, 242)); // white: warm, dimmed
        assert_eq!(correct(0x001F), Rgb(232, 53, 111)); // pure red -> desaturated
        assert_eq!(correct(0x03E0), Rgb(111, 222, 53)); // pure green
        assert_eq!(correct(0x7C00), Rgb(0, 88, 217)); // pure blue
    }

    #[test]
    fn lut_is_full_size_and_matches_correct() {
        let lut = build_lut();
        assert_eq!(lut.len(), 32768);
        assert_eq!(lut[0x001F], correct(0x001F));
        assert_eq!(lut[0x7FFF], correct(0x7FFF));
    }
}
