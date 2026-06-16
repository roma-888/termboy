//! Save-state thumbnail sidecar: a hand-rolled raw-RGB format (`<rom>.ssN.thumb`)
//! holding a box-downscaled copy of the frame saved into a slot. No dependency
//! and no decoder elsewhere — we only ever read back our own format. Pure +
//! unit-testable.

use termboy_core::{FrameBuffer, Rgb};

const MAGIC: &[u8; 4] = b"TBTH";

/// Max thumbnail width before downscaling kicks in. Set above the GBA's 240 so
/// GB/GBA frames are stored at full native resolution (sharp source for the
/// crisp save/load preview); only larger frames (none today) would shrink.
pub const TARGET_W: usize = 240;

/// A decoded thumbnail. `pixels` is row-major `width`x`height` RGB.
#[derive(Clone)]
pub struct Thumb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
}

/// Box-downscale `fb` to ~`TARGET_W` wide (preserving aspect) and serialize:
/// `MAGIC` + `width` (u16 LE) + `height` (u16 LE) + `width*height*3` RGB bytes.
pub fn encode(fb: &FrameBuffer) -> Vec<u8> {
    let den = fb.width.div_ceil(TARGET_W).max(1);
    let w = (fb.width / den).max(1);
    let h = (fb.height / den).max(1);
    let mut out = Vec::with_capacity(8 + w * h * 3);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(w as u16).to_le_bytes());
    out.extend_from_slice(&(h as u16).to_le_bytes());
    for ty in 0..h {
        for tx in 0..w {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for dy in 0..den {
                for dx in 0..den {
                    let p = fb.pixels[(ty * den + dy) * fb.width + (tx * den + dx)];
                    r += p.0 as u32;
                    g += p.1 as u32;
                    b += p.2 as u32;
                    n += 1;
                }
            }
            out.push((r / n) as u8);
            out.push((g / n) as u8);
            out.push((b / n) as u8);
        }
    }
    out
}

/// Parse a `.thumb` file; `None` on bad magic or a truncated/oversized body.
pub fn decode(bytes: &[u8]) -> Option<Thumb> {
    if bytes.len() < 8 || &bytes[0..4] != MAGIC {
        return None;
    }
    let w = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    let h = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if bytes.len() != 8 + w * h * 3 {
        return None;
    }
    let pixels = (0..w * h)
        .map(|i| {
            let o = 8 + i * 3;
            Rgb(bytes[o], bytes[o + 1], bytes[o + 2])
        })
        .collect();
    Some(Thumb { width: w, height: h, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_small_frame() {
        let mut fb = FrameBuffer::new(4, 2);
        fb.pixels = vec![
            Rgb(10, 0, 0), Rgb(20, 0, 0), Rgb(30, 0, 0), Rgb(40, 0, 0),
            Rgb(50, 0, 0), Rgb(60, 0, 0), Rgb(70, 0, 0), Rgb(80, 0, 0),
        ];
        let bytes = encode(&fb); // den=1 (4 < 96), so no downscale
        let t = decode(&bytes).unwrap();
        assert_eq!((t.width, t.height), (4, 2));
        assert_eq!(t.pixels[0], Rgb(10, 0, 0));
        assert_eq!(t.pixels[7], Rgb(80, 0, 0));
    }

    #[test]
    fn stores_native_frames_full_res() {
        // GB and GBA frames are <= TARGET_W, so they're stored unscaled.
        assert_eq!((decode(&encode(&FrameBuffer::new(160, 144))).unwrap().width), 160);
        assert_eq!((decode(&encode(&FrameBuffer::new(240, 160))).unwrap().width), 240);
    }

    #[test]
    fn downscales_only_above_target_width() {
        let fb = FrameBuffer::new(720, 480); // den = ceil(720/240) = 3 -> 240x160
        let t = decode(&encode(&fb)).unwrap();
        assert_eq!((t.width, t.height), (240, 160));
    }

    #[test]
    fn decode_rejects_bad_magic_and_truncation() {
        assert!(decode(b"").is_none());
        assert!(decode(b"XXXX\x01\x00\x01\x00\x00\x00\x00").is_none()); // bad magic
        let mut good = encode(&FrameBuffer::new(8, 8));
        good.truncate(good.len() - 1); // one byte short
        assert!(decode(&good).is_none());
    }
}
