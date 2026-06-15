//! Kitty graphics protocol: encode a FrameBuffer as an APC image command for
//! terminals that support it (Ghostty, kitty, WezTerm). Pure encoders here —
//! no terminal I/O — so the wire format is unit-testable. Each frame reuses a
//! fixed image id with `a=T` so the terminal replaces the previous frame in
//! place; raw RGB (`f=24`) keeps it dependency-free.

use termboy_core::{FrameBuffer, Rgb};

use crate::screen::{glyph, GLYPH_H, GLYPH_W};

/// Fixed image id reused every frame so the terminal replaces the prior frame.
const IMAGE_ID: u32 = 1;

/// Standard base64 alphabet.
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Kitty caps each APC payload chunk at 4096 bytes (multiple of 4 except last).
const CHUNK: usize = 4096;

/// Append the standard padded base64 of `data` to `out`.
fn base64_encode(data: &[u8], out: &mut String) {
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        out.push(B64[(b[0] >> 2) as usize] as char);
        out.push(B64[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { B64[(b[2] & 0x3f) as usize] as char } else { '=' });
    }
}

/// Append the kitty APC command(s) that transmit-and-display `fb` as image
/// `id`, sized to `cols`x`rows` terminal cells (the terminal scales the native
/// frame into that rectangle). Chunked per the protocol's 4096-byte limit.
pub fn encode_frame(fb: &FrameBuffer, id: u32, cols: u16, rows: u16, out: &mut String) {
    use std::fmt::Write;
    let mut rgb = Vec::with_capacity(fb.pixels.len() * 3);
    for p in &fb.pixels {
        rgb.extend_from_slice(&[p.0, p.1, p.2]);
    }
    let mut b64 = String::with_capacity(rgb.len().div_ceil(3) * 4);
    base64_encode(&rgb, &mut b64);

    let bytes = b64.as_bytes();
    let mut i = 0;
    let mut first = true;
    // Always emit at least one command (an empty frame would still announce it).
    loop {
        let end = (i + CHUNK).min(bytes.len());
        let more = end < bytes.len();
        let m = u8::from(more);
        if first {
            write!(
                out,
                "\x1b_Ga=T,f=24,s={},v={},i={id},p=1,c={cols},r={rows},C=1,q=2,m={m};",
                fb.width, fb.height
            )
            .unwrap();
            first = false;
        } else {
            write!(out, "\x1b_Gm={m};").unwrap();
        }
        out.push_str(&b64[i..end]);
        out.push_str("\x1b\\");
        i = end;
        if !more {
            break;
        }
    }
}

/// Heuristic graphics-support detection from environment variables, covering
/// the terminals that implement the protocol (kitty, Ghostty, WezTerm). `env`
/// is the lookup (e.g. `|k| std::env::var(k).ok()`) so it's testable. The
/// `--graphics kitty` override exists for anything this misses.
pub fn detect_kitty(env: impl Fn(&str) -> Option<String>) -> bool {
    if env("KITTY_WINDOW_ID").is_some()
        || env("GHOSTTY_RESOURCES_DIR").is_some()
        || env("WEZTERM_PANE").is_some()
    {
        return true;
    }
    if matches!(env("TERM_PROGRAM").as_deref(), Some("ghostty") | Some("WezTerm")) {
        return true;
    }
    matches!(env("TERM").as_deref(), Some("xterm-kitty") | Some("xterm-ghostty"))
}

/// Largest centered cell rectangle that fills `term_cols`x`term_rows` while
/// preserving the frame's aspect (terminal cells are ~1 wide x 2 tall, so the
/// target column:row ratio is `2*src_w : src_h`). Returns `(cols, rows, off_c,
/// off_r)`.
pub fn fit_cells(
    src_w: usize,
    src_h: usize,
    term_cols: usize,
    term_rows: usize,
) -> (usize, usize, usize, usize) {
    let ratio = 2.0 * src_w as f64 / src_h as f64; // cols:rows for a ~1x2 cell
    let mut c = term_cols;
    let mut r = (c as f64 / ratio).round() as usize;
    if r > term_rows {
        r = term_rows;
        c = (r as f64 * ratio).round() as usize;
    }
    c = c.clamp(1, term_cols.max(1));
    r = r.clamp(1, term_rows.max(1));
    let off_c = term_cols.saturating_sub(c) / 2;
    let off_r = term_rows.saturating_sub(r) / 2;
    (c, r, off_c, off_r)
}

/// Graphics-mode renderer: emits one kitty image per frame, scaled by the
/// terminal to fill a centered cell rectangle. Mirrors `screen::Screen`'s
/// interface so the frontend can hold either behind one `Display` enum. Unlike
/// the half-block renderer it doesn't diff — each frame is a full image that
/// replaces the last (fixed image id), which the protocol does without flicker.
pub struct KittyScreen {
    src_w: usize,
    src_h: usize,
    cols: u16,
    rows: u16,
    off_c: usize,
    off_r: usize,
    overlay: Option<String>,
}

impl KittyScreen {
    pub fn new(width: usize, height: usize) -> Self {
        Self { src_w: width, src_h: height, cols: 1, rows: 1, off_c: 0, off_r: 0, overlay: None }
    }

    /// Graphics scales to any terminal size; this is just a sane lower bound.
    pub fn required_size(&self) -> (usize, usize) {
        (16, 8)
    }

    pub fn set_viewport(&mut self, term_cols: usize, term_rows: usize) {
        let (c, r, oc, or) = fit_cells(self.src_w, self.src_h, term_cols, term_rows);
        self.cols = c as u16;
        self.rows = r as u16;
        self.off_c = oc;
        self.off_r = or;
    }

    /// No-op: every render re-emits the full image, so there's nothing to forget.
    pub fn invalidate(&mut self) {}

    pub fn set_overlay(&mut self, msg: &str) {
        self.overlay = Some(msg.to_string());
    }

    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    pub fn render(&mut self, fb: &FrameBuffer, out: &mut String) {
        use std::fmt::Write;
        // Place the image at its centered top-left; C=1 keeps the cursor here.
        write!(out, "\x1b[{};{}H", self.off_r + 1, self.off_c + 1).unwrap();
        match &self.overlay {
            Some(msg) => {
                let mut framed = FrameBuffer::new(fb.width, fb.height);
                framed.pixels.copy_from_slice(&fb.pixels);
                draw_badge(&mut framed, msg);
                encode_frame(&framed, IMAGE_ID, self.cols, self.rows, out);
            }
            None => encode_frame(fb, IMAGE_ID, self.cols, self.rows, out),
        }
    }
}

/// Burn the overlay badge (white 3x5 glyphs on a dark box) into the RGB frame,
/// top-left, at a size that scales with the frame so it stays legible.
fn draw_badge(fb: &mut FrameBuffer, msg: &str) {
    let chars: Vec<char> = msg.chars().collect();
    if chars.is_empty() {
        return;
    }
    let scale = (fb.height / 72).max(1); // frame pixels are square, so 1 axis
    let pad = scale;
    let cell = GLYPH_W + 1; // glyph columns + a 1px inter-glyph gap
    let box_w = (chars.len() * cell * scale + 2 * pad).min(fb.width);
    let box_h = (GLYPH_H * scale + 2 * pad).min(fb.height);
    for by in 0..box_h {
        for bx in 0..box_w {
            let (px, py) = (bx as isize - pad as isize, by as isize - pad as isize);
            let lit = px >= 0
                && py >= 0
                && {
                    let (fcol, frow) = (px as usize / scale, py as usize / scale);
                    let ci = fcol / cell;
                    let col_in = fcol % cell;
                    ci < chars.len()
                        && col_in < GLYPH_W
                        && frow < GLYPH_H
                        && glyph(chars[ci])[frow].as_bytes()[col_in] == b'#'
                };
            fb.pixels[by * fb.width + bx] = if lit { Rgb(255, 255, 255) } else { Rgb(0, 0, 0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        let mut o = String::new();
        base64_encode(s.as_bytes(), &mut o);
        o
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(b64(""), "");
        assert_eq!(b64("f"), "Zg==");
        assert_eq!(b64("fo"), "Zm8=");
        assert_eq!(b64("foo"), "Zm9v");
        assert_eq!(b64("foob"), "Zm9vYg==");
        assert_eq!(b64("fooba"), "Zm9vYmE=");
        assert_eq!(b64("foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encode_frame_single_chunk_has_keys_and_rgb_payload() {
        let mut fb = FrameBuffer::new(2, 1);
        fb.pixels = vec![Rgb(1, 2, 3), Rgb(4, 5, 6)];
        let mut out = String::new();
        encode_frame(&fb, 7, 20, 10, &mut out);
        let header = "\x1b_Ga=T,f=24,s=2,v=1,i=7,p=1,c=20,r=10,C=1,q=2,m=0;";
        assert!(out.starts_with(header), "got: {out:?}");
        let payload = out.strip_prefix(header).unwrap().strip_suffix("\x1b\\").unwrap();
        assert_eq!(payload, "AQIDBAUG"); // base64 of [1,2,3,4,5,6]
    }

    #[test]
    fn encode_frame_chunks_large_payload() {
        // 100x50 -> 15000 RGB bytes -> 20000 base64 chars -> 5 chunks of <=4096.
        let fb = FrameBuffer::new(100, 50);
        let mut out = String::new();
        encode_frame(&fb, 1, 50, 25, &mut out);
        assert!(out.starts_with("\x1b_Ga=T,f=24,s=100,v=50,i=1,p=1,c=50,r=25,C=1,q=2,m=1;"));
        assert!(out.contains("\x1b_Gm=1;")); // short continuation form
        assert_eq!(out.matches("m=0;").count(), 1); // exactly one final chunk
        assert_eq!(out.matches("\x1b\\").count(), 5); // one terminator per chunk
        // No single chunk's payload exceeds the protocol limit.
        for seg in out.split("\x1b\\").filter(|s| !s.is_empty()) {
            let payload = seg.rsplit(';').next().unwrap();
            assert!(payload.len() <= CHUNK, "chunk too big: {}", payload.len());
        }
    }

    #[test]
    fn detect_kitty_recognizes_known_terminals() {
        let set = |key: &'static str, val: &'static str| {
            move |k: &str| (k == key).then(|| val.to_string())
        };
        assert!(!detect_kitty(|_| None));
        assert!(detect_kitty(set("KITTY_WINDOW_ID", "1")));
        assert!(detect_kitty(set("WEZTERM_PANE", "0")));
        assert!(detect_kitty(set("GHOSTTY_RESOURCES_DIR", "/x")));
        assert!(detect_kitty(set("TERM_PROGRAM", "ghostty")));
        assert!(detect_kitty(set("TERM_PROGRAM", "WezTerm")));
        assert!(detect_kitty(set("TERM", "xterm-kitty")));
        assert!(!detect_kitty(set("TERM", "xterm-256color")));
    }

    #[test]
    fn fit_cells_fills_and_preserves_aspect() {
        // GBA 240x160 -> target col:row ratio 2*240/160 = 3.0
        let (c, r, oc, or) = fit_cells(240, 160, 120, 40);
        assert!((c as f64 / r as f64 - 3.0).abs() < 0.2, "ratio off: {c}x{r}");
        assert!(c <= 120 && r <= 40);
        assert_eq!(oc, (120 - c) / 2);
        assert_eq!(or, (40 - r) / 2);
    }

    #[test]
    fn fit_cells_row_limited_centers_horizontally() {
        // Wide-but-short terminal: rows bind, leaving horizontal padding.
        let (c, r, oc, _or) = fit_cells(240, 160, 200, 40);
        assert_eq!(r, 40);
        assert!((c as f64 / r as f64 - 3.0).abs() < 0.2);
        assert!(oc > 0);
    }

    #[test]
    fn render_emits_image_at_centered_offset() {
        let mut k = KittyScreen::new(4, 4);
        k.set_viewport(20, 20); // fit_cells(4,4,20,20) -> 20x10 cells, off (0,5)
        let mut out = String::new();
        k.render(&FrameBuffer::new(4, 4), &mut out);
        assert!(out.starts_with("\x1b[6;1H"), "cursor not at centered offset: {out:?}");
        assert!(out.contains("\x1b_Ga=T,f=24,s=4,v=4,"));
        assert!(out.ends_with("\x1b\\"));
    }

    #[test]
    fn badge_burns_white_glyph_pixels_into_frame() {
        let mut fb = FrameBuffer::new(64, 32);
        draw_badge(&mut fb, "8"); // '8' top row is "###"
        assert!(fb.pixels.iter().any(|p| *p == Rgb(255, 255, 255)));
        assert_eq!(fb.pixels[0], Rgb(0, 0, 0)); // corner is padding, not lit
    }
}
