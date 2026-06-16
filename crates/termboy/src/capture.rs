//! Hand-rolled PNG (screenshot) and APNG (clip) encoding. Reuses the
//! `miniz_oxide` zlib already pulled in for the kitty renderer, so no new
//! dependency and full 24-bit color. GBA/GB frames are tiny, so we nearest-
//! neighbor upscale before encoding to make the output viewable.

use termboy_core::FrameBuffer;

const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// PNG/zlib CRC-32 (reflected, poly 0xEDB88320).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Append one PNG chunk: length, type, data, CRC(type+data).
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_in = Vec::with_capacity(4 + data.len());
    crc_in.extend_from_slice(kind);
    crc_in.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

fn ihdr(w: usize, h: usize) -> Vec<u8> {
    let mut d = Vec::with_capacity(13);
    d.extend_from_slice(&(w as u32).to_be_bytes());
    d.extend_from_slice(&(h as u32).to_be_bytes());
    d.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolor RGB, no interlace
    d
}

/// Nearest-neighbor upscale → (w, h, raw RGB bytes).
fn upscale(fb: &FrameBuffer, scale: usize) -> (usize, usize, Vec<u8>) {
    let (sw, sh) = (fb.width, fb.height);
    let (w, h) = (sw * scale, sh * scale);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let sy = y / scale;
        for x in 0..w {
            let p = fb.pixels[sy * sw + x / scale];
            rgb.extend_from_slice(&[p.0, p.1, p.2]);
        }
    }
    (w, h, rgb)
}

/// Prefix each scanline with filter byte 0 (none), then zlib-compress.
fn compress_image(w: usize, h: usize, rgb: &[u8], level: u8) -> Vec<u8> {
    let row = w * 3;
    let mut raw = Vec::with_capacity(h * (row + 1));
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * row..(y + 1) * row]);
    }
    miniz_oxide::deflate::compress_to_vec_zlib(&raw, level)
}

/// Encode one frame as a PNG, upscaled by `scale`.
pub fn encode_png(fb: &FrameBuffer, scale: usize) -> Vec<u8> {
    let (w, h, rgb) = upscale(fb, scale);
    let idat = compress_image(w, h, &rgb, 6);
    let mut out = Vec::new();
    out.extend_from_slice(&SIG);
    chunk(&mut out, b"IHDR", &ihdr(w, h));
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Accumulates frames as already-zlib-compressed image data (so memory holds
/// compressed bytes, not raw RGB), then assembles an APNG on `finish`.
pub struct Recorder {
    scale: usize,
    w: usize,
    h: usize,
    frames: Vec<Vec<u8>>,
}

impl Recorder {
    pub fn new(scale: usize) -> Self {
        Self { scale, w: 0, h: 0, frames: Vec::new() }
    }

    pub fn push(&mut self, fb: &FrameBuffer) {
        let (w, h, rgb) = upscale(fb, self.scale);
        self.w = w;
        self.h = h;
        self.frames.push(compress_image(w, h, &rgb, 1)); // level 1: fast, per-frame
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Assemble the APNG. ~30 fps (delay 1/30). Each frame is a full image.
    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&SIG);
        chunk(&mut out, b"IHDR", &ihdr(self.w, self.h));

        let mut actl = Vec::with_capacity(8);
        actl.extend_from_slice(&(self.frames.len() as u32).to_be_bytes());
        actl.extend_from_slice(&0u32.to_be_bytes()); // num_plays: 0 = loop forever
        chunk(&mut out, b"acTL", &actl);

        let mut seq: u32 = 0;
        for (i, data) in self.frames.iter().enumerate() {
            let mut fctl = Vec::with_capacity(26);
            fctl.extend_from_slice(&seq.to_be_bytes());
            seq += 1;
            fctl.extend_from_slice(&(self.w as u32).to_be_bytes());
            fctl.extend_from_slice(&(self.h as u32).to_be_bytes());
            fctl.extend_from_slice(&0u32.to_be_bytes()); // x_offset
            fctl.extend_from_slice(&0u32.to_be_bytes()); // y_offset
            fctl.extend_from_slice(&1u16.to_be_bytes()); // delay_num
            fctl.extend_from_slice(&30u16.to_be_bytes()); // delay_den
            fctl.push(0); // dispose_op: none
            fctl.push(0); // blend_op: source
            chunk(&mut out, b"fcTL", &fctl);

            if i == 0 {
                chunk(&mut out, b"IDAT", data);
            } else {
                let mut fdat = Vec::with_capacity(4 + data.len());
                fdat.extend_from_slice(&seq.to_be_bytes());
                seq += 1;
                fdat.extend_from_slice(data);
                chunk(&mut out, b"fdAT", &fdat);
            }
        }
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termboy_core::Rgb;

    fn solid(w: usize, h: usize, c: Rgb) -> FrameBuffer {
        FrameBuffer { width: w, height: h, pixels: vec![c; w * h] }
    }

    /// Walk chunks after the signature, verifying every CRC; return (type, len) list.
    fn chunks(png: &[u8]) -> Vec<([u8; 4], usize)> {
        assert_eq!(png[0..8], SIG);
        let mut out = Vec::new();
        let mut i = 8;
        while i + 12 <= png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
            let data = &png[i + 8..i + 8 + len];
            let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            let mut ci = kind.to_vec();
            ci.extend_from_slice(data);
            assert_eq!(crc, crc32(&ci), "bad CRC for {:?}", kind);
            out.push((kind, len));
            i += 12 + len;
        }
        out
    }

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn png_has_valid_structure_and_dimensions() {
        let png = encode_png(&solid(2, 2, Rgb(10, 20, 30)), 3);
        let cs = chunks(&png);
        assert_eq!(cs[0].0, *b"IHDR");
        // IHDR data is at offset 16; width/height are upscaled 2*3 = 6.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 6);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 6);
        assert!(cs.iter().any(|(k, _)| k == b"IDAT"));
        assert_eq!(cs.last().unwrap().0, *b"IEND");
    }

    #[test]
    fn apng_has_actl_and_per_frame_chunks() {
        let mut rec = Recorder::new(2);
        for _ in 0..3 {
            rec.push(&solid(2, 2, Rgb(1, 2, 3)));
        }
        assert_eq!(rec.len(), 3);
        let png = rec.finish();
        let cs = chunks(&png); // also verifies every CRC
        let count = |t: &[u8; 4]| cs.iter().filter(|(k, _)| k == t).count();
        assert_eq!(count(b"acTL"), 1);
        assert_eq!(count(b"fcTL"), 3); // one per frame
        assert_eq!(count(b"IDAT"), 1); // first frame
        assert_eq!(count(b"fdAT"), 2); // remaining frames
        assert_eq!(cs.last().unwrap().0, *b"IEND");
        // acTL num_frames: sig(8) + IHDR chunk(25) + acTL header(8) = 41.
        let actl_off = 8 + (12 + 13) + 8;
        assert_eq!(u32::from_be_bytes(png[actl_off..actl_off + 4].try_into().unwrap()), 3);
    }
}
