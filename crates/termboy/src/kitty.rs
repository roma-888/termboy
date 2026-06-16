//! Kitty graphics protocol: encode a FrameBuffer as an APC image command for
//! terminals that support it (Ghostty, kitty, WezTerm). Pure encoders here —
//! no terminal I/O — so the wire format is unit-testable. Each frame reuses a
//! fixed image id with `a=T` so the terminal replaces the previous frame in
//! place. Frames are nearest-neighbor upscaled to the on-screen pixel size so
//! the terminal does no blurry scaling (crisp retro pixels), then sent as
//! zlib-compressed RGB (`f=24,o=z`) to keep that larger payload affordable.

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

/// Nearest-neighbor upscale `fb` to `dst_w`x`dst_h`, returning packed RGB. Done
/// in-process so the terminal displays our exact pixels with no blurry linear
/// upscaling; an identity copy when the destination equals the source size.
fn nearest_upscale(fb: &FrameBuffer, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let mut out = vec![0u8; dst_w * dst_h * 3];
    for dy in 0..dst_h {
        let row = (dy * fb.height / dst_h) * fb.width;
        for dx in 0..dst_w {
            let p = fb.pixels[row + dx * fb.width / dst_w];
            let o = (dy * dst_w + dx) * 3;
            out[o] = p.0;
            out[o + 1] = p.1;
            out[o + 2] = p.2;
        }
    }
    out
}

/// Emit `payload` (base64) as one or more APC chunks: the first carries
/// `control` plus data, the rest just `m=`. Splits at the protocol's 4096-byte
/// limit, marking every chunk but the last with `m=1`.
fn push_chunked(control: &str, payload: &str, out: &mut String) {
    use std::fmt::Write;
    let mut i = 0;
    let mut first = true;
    loop {
        let end = (i + CHUNK).min(payload.len());
        let more = end < payload.len();
        let m = u8::from(more);
        if first {
            write!(out, "\x1b_G{control},m={m};").unwrap();
            first = false;
        } else {
            write!(out, "\x1b_Gm={m};").unwrap();
        }
        out.push_str(&payload[i..end]);
        out.push_str("\x1b\\");
        i = end;
        if !more {
            break;
        }
    }
}

/// Append the kitty APC command(s) that transmit-and-display the `img_w`x`img_h`
/// packed-RGB buffer `rgb` as image `id`, filling `cols`x`rows` terminal cells.
/// The payload is zlib-compressed (`o=z`) — essential once frames are upscaled.
pub fn encode_image(
    rgb: &[u8],
    img_w: usize,
    img_h: usize,
    id: u32,
    cols: u16,
    rows: u16,
    out: &mut String,
) {
    // Level 1: upscaled frames are mostly repeated blocks, so even the fastest
    // setting compresses them to a fraction of their size.
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 1);
    let mut b64 = String::with_capacity(compressed.len().div_ceil(3) * 4);
    base64_encode(&compressed, &mut b64);
    let control =
        format!("a=T,f=24,o=z,s={img_w},v={img_h},i={id},p=1,c={cols},r={rows},C=1,q=2");
    push_chunked(&control, &b64, out);
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
    /// On-screen pixel size to upscale to (the cell rectangle's pixels), so the
    /// image maps 1:1 and stays crisp. `None` when the terminal didn't report a
    /// cell size — then we send the native frame and let it scale (softer).
    target: Option<(usize, usize)>,
    overlay: Option<String>,
}

impl KittyScreen {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            src_w: width,
            src_h: height,
            cols: 1,
            rows: 1,
            off_c: 0,
            off_r: 0,
            target: None,
            overlay: None,
        }
    }

    /// Graphics scales to any terminal size; this is just a sane lower bound.
    pub fn required_size(&self) -> (usize, usize) {
        (16, 8)
    }

    /// `cell_px` is the terminal's pixels-per-cell (width, height), used to size
    /// the upscale target; `None` (terminal didn't report it) falls back to native.
    pub fn set_viewport(
        &mut self,
        term_cols: usize,
        term_rows: usize,
        cell_px: Option<(usize, usize)>,
    ) {
        let (c, r, oc, or) = fit_cells(self.src_w, self.src_h, term_cols, term_rows);
        self.cols = c as u16;
        self.rows = r as u16;
        self.off_c = oc;
        self.off_r = or;
        self.target = cell_px.map(|(cw, ch)| (c * cw, r * ch));
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
        let (tw, th) = self.target.unwrap_or((self.src_w, self.src_h));
        // Composite the overlay at native res first, then upscale the whole frame
        // so the badge scales with the image (matching the half-block overlay).
        let rgb = match &self.overlay {
            Some(msg) => {
                let mut framed = FrameBuffer::new(fb.width, fb.height);
                framed.pixels.copy_from_slice(&fb.pixels);
                draw_badge(&mut framed, msg);
                nearest_upscale(&framed, tw, th)
            }
            None => nearest_upscale(fb, tw, th),
        };
        encode_image(&rgb, tw, th, IMAGE_ID, self.cols, self.rows, out);
    }

    /// Draw the pause menu: upscale the frozen game to the on-screen pixel size
    /// first, then compose the dimmed panel onto *that* buffer, so the panel text
    /// is rendered at full display resolution (crisp) instead of being drawn at
    /// the native frame size and blown up with it.
    pub fn render_menu(&mut self, fb: &FrameBuffer, view: &crate::pause::MenuView, out: &mut String) {
        use std::fmt::Write;
        write!(out, "\x1b[{};{}H", self.off_r + 1, self.off_c + 1).unwrap();
        let (tw, th) = self.target.unwrap_or((self.src_w, self.src_h));
        let big = upscale_fb(fb, tw, th);
        let composed = crate::pause::compose_menu(&big, view);
        let mut rgb = vec![0u8; tw * th * 3];
        for (i, p) in composed.pixels.iter().enumerate() {
            rgb[i * 3] = p.0;
            rgb[i * 3 + 1] = p.1;
            rgb[i * 3 + 2] = p.2;
        }
        encode_image(&rgb, tw, th, IMAGE_ID, self.cols, self.rows, out);
    }
}

/// Nearest-neighbor upscale `fb` to `dst_w`x`dst_h` as a new `FrameBuffer`. Used
/// only by the (rare) pause-menu path; the hot per-frame path uses the packed
/// `nearest_upscale` to stay allocation-lean.
fn upscale_fb(fb: &FrameBuffer, dst_w: usize, dst_h: usize) -> FrameBuffer {
    let mut out = FrameBuffer::new(dst_w, dst_h);
    for dy in 0..dst_h {
        let sy = dy * fb.height / dst_h;
        for dx in 0..dst_w {
            out.pixels[dy * dst_w + dx] = fb.pixels[sy * fb.width + dx * fb.width / dst_w];
        }
    }
    out
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
    fn nearest_upscale_replicates_pixels() {
        let mut fb = FrameBuffer::new(2, 1);
        fb.pixels = vec![Rgb(10, 0, 0), Rgb(20, 0, 0)];
        let up = nearest_upscale(&fb, 4, 2); // 2x each axis
        assert_eq!(&up[0..3], &[10, 0, 0]); // (0,0) <- src 0
        assert_eq!(&up[3..6], &[10, 0, 0]); // (1,0) <- src 0
        assert_eq!(&up[6..9], &[20, 0, 0]); // (2,0) <- src 1
        assert_eq!(&up[9..12], &[20, 0, 0]); // (3,0) <- src 1
        assert_eq!(&up[12..15], &[10, 0, 0]); // row 1 replicates row 0
    }

    #[test]
    fn encode_image_emits_compressed_payload() {
        let rgb = vec![1u8, 2, 3, 4, 5, 6]; // 2x1 RGB
        let mut out = String::new();
        encode_image(&rgb, 2, 1, 7, 20, 10, &mut out);
        assert!(out.contains("\x1b_Ga=T,f=24,o=z,s=2,v=1,i=7,p=1,c=20,r=10,C=1,q=2,m=0;"));
        assert!(out.ends_with("\x1b\\"));
        // The payload is base64(zlib(rgb)) — recover and compare to that exactly.
        let payload = out.rsplit_once(';').unwrap().1.strip_suffix("\x1b\\").unwrap();
        let mut expected = String::new();
        base64_encode(&miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 1), &mut expected);
        assert_eq!(payload, expected);
    }

    #[test]
    fn push_chunked_splits_at_4096() {
        let payload: String = std::iter::repeat_n('A', 10000).collect();
        let mut out = String::new();
        push_chunked("a=T,f=24", &payload, &mut out);
        assert!(out.starts_with("\x1b_Ga=T,f=24,m=1;"));
        assert!(out.contains("\x1b_Gm=1;")); // short continuation form
        assert_eq!(out.matches("m=0;").count(), 1); // exactly one final chunk
        assert_eq!(out.matches("\x1b\\").count(), 3); // 4096 + 4096 + 1808
        for seg in out.split("\x1b\\").filter(|s| !s.is_empty()) {
            assert!(seg.rsplit(';').next().unwrap().len() <= CHUNK);
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
        k.set_viewport(20, 20, None); // None cell_px -> native 4x4 (no upscale)
        let mut out = String::new();
        k.render(&FrameBuffer::new(4, 4), &mut out);
        assert!(out.starts_with("\x1b[6;1H"), "cursor not at centered offset: {out:?}");
        assert!(out.contains("\x1b_Ga=T,f=24,o=z,s=4,v=4,"));
        assert!(out.ends_with("\x1b\\"));
    }

    #[test]
    fn render_menu_emits_an_image_at_display_resolution() {
        let mut k = KittyScreen::new(160, 144);
        k.set_viewport(80, 40, Some((8, 16))); // reports cell px -> upscales
        let view = crate::pause::Menu::new(true, "1x".into(), false, [false; crate::pause::SLOTS]).view();
        let mut out = String::new();
        k.render_menu(&FrameBuffer::new(160, 144), &view, &mut out);
        assert!(out.contains("\x1b_Ga=T,f=24,o=z,"));
        assert!(out.ends_with("\x1b\\"));
        // The image is sent at the upscaled size, not the native 160x144.
        assert!(!out.contains("s=160,v=144,"));
    }

    #[test]
    fn upscale_fb_replicates_pixels() {
        let mut fb = FrameBuffer::new(2, 1);
        fb.pixels = vec![Rgb(10, 0, 0), Rgb(20, 0, 0)];
        let up = upscale_fb(&fb, 4, 2);
        assert_eq!(up.pixels[0], Rgb(10, 0, 0));
        assert_eq!(up.pixels[2], Rgb(20, 0, 0));
        assert_eq!(up.pixels[4], Rgb(10, 0, 0)); // row 1 replicates row 0
    }

    #[test]
    fn badge_burns_white_glyph_pixels_into_frame() {
        let mut fb = FrameBuffer::new(64, 32);
        draw_badge(&mut fb, "8"); // '8' top row is "###"
        assert!(fb.pixels.iter().any(|p| *p == Rgb(255, 255, 255)));
        assert_eq!(fb.pixels[0], Rgb(0, 0, 0)); // corner is padding, not lit
    }
}
