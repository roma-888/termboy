//! Half-block diff renderer. Pure: consumes a FrameBuffer, produces an ANSI
//! string. No terminal I/O here — that keeps it unit-testable.

use termboy_core::{FrameBuffer, Rgb};

pub struct Screen {
    src_w: usize,
    src_h: usize,
    cols: usize,
    rows: usize,
    off_c: usize, // centering offsets within the terminal
    off_r: usize,
    /// (top, bottom) color of every cell as last drawn; None = never drawn.
    prev: Vec<Option<(Rgb, Rgb)>>,
    /// Transient status text drawn over the top-left of the frame (save states).
    overlay: Option<String>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let cols = width;
        let rows = height.div_ceil(2);
        Self {
            src_w: width,
            src_h: height,
            cols,
            rows,
            off_c: 0,
            off_r: 0,
            prev: vec![None; cols * rows],
            overlay: None,
        }
    }

    /// Show transient status text over the frame (e.g. "saved slot 3").
    pub fn set_overlay(&mut self, msg: &str) {
        self.overlay = Some(msg.to_string());
    }

    /// Remove the overlay and force a full repaint so the covered cells redraw.
    pub fn clear_overlay(&mut self) {
        if self.overlay.is_some() {
            self.overlay = None;
            self.invalidate();
        }
    }

    /// Terminal size needed for pixel-perfect output: (cols, rows).
    pub fn required_size(&self) -> (usize, usize) {
        (self.src_w, self.src_h.div_ceil(2))
    }

    /// Fit the output to the terminal: pixel-perfect (centered) when it fits,
    /// box-filter downscaled to the largest size that does when it doesn't.
    pub fn set_viewport(&mut self, term_cols: usize, term_rows: usize) {
        let (native_c, native_r) = self.required_size();
        let (cols, rows) = if term_cols >= native_c && term_rows >= native_r {
            (native_c, native_r)
        } else {
            // scale preserving aspect: terminal cells are 1x2 pixels
            let s = (term_cols as f64 / self.src_w as f64)
                .min(term_rows as f64 * 2.0 / self.src_h as f64);
            let cols = ((self.src_w as f64 * s) as usize).max(2).min(term_cols.max(2));
            let rows = ((self.src_h as f64 * s / 2.0) as usize).max(1).min(term_rows.max(1));
            (cols, rows)
        };
        let off_c = (term_cols.saturating_sub(cols)) / 2;
        let off_r = (term_rows.saturating_sub(rows)) / 2;
        if (cols, rows, off_c, off_r) != (self.cols, self.rows, self.off_c, self.off_r) {
            self.cols = cols;
            self.rows = rows;
            self.off_c = off_c;
            self.off_r = off_r;
            self.prev = vec![None; cols * rows];
        }
    }

    /// Forget the previous frame (e.g. after a terminal resize/clear).
    pub fn invalidate(&mut self) {
        self.prev.fill(None);
    }

    /// Average the source pixels in the box [x0,x1) x [y0,y1).
    fn avg_box(fb: &FrameBuffer, x0: usize, x1: usize, y0: usize, y1: usize) -> Rgb {
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = fb.pixels[y * fb.width + x];
                r += p.0 as u32;
                g += p.1 as u32;
                b += p.2 as u32;
                n += 1;
            }
        }
        Rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
    }

    /// Output pixel (ox, oy) on an out_w x out_h grid, box-filtered from source.
    fn scaled_pixel(&self, fb: &FrameBuffer, ox: usize, oy: usize) -> Rgb {
        let out_h = self.rows * 2;
        let x0 = ox * self.src_w / self.cols;
        let x1 = (((ox + 1) * self.src_w).div_ceil(self.cols)).max(x0 + 1).min(self.src_w);
        let y0 = oy * self.src_h / out_h;
        let y1 = (((oy + 1) * self.src_h).div_ceil(out_h)).max(y0 + 1).min(self.src_h);
        Self::avg_box(fb, x0, x1, y0, y1)
    }

    /// Append ANSI for every changed cell to `out`.
    pub fn render(&mut self, fb: &FrameBuffer, out: &mut String) {
        use std::fmt::Write;
        let exact = self.cols == self.src_w;
        let mut fg: Option<Rgb> = None;
        let mut bg: Option<Rgb> = None;
        for row in 0..self.rows {
            let mut cursor_set = false;
            for col in 0..self.cols {
                let (top, bottom) = if exact {
                    let top = fb.pixels[(row * 2) * fb.width + col];
                    let bottom = if row * 2 + 1 < fb.height {
                        fb.pixels[(row * 2 + 1) * fb.width + col]
                    } else {
                        top
                    };
                    (top, bottom)
                } else {
                    (
                        self.scaled_pixel(fb, col, row * 2),
                        self.scaled_pixel(fb, col, row * 2 + 1),
                    )
                };
                let cell = (top, bottom);
                if self.prev[row * self.cols + col] == Some(cell) {
                    cursor_set = false; // gap: next change must reposition
                    continue;
                }
                self.prev[row * self.cols + col] = Some(cell);
                if !cursor_set {
                    write!(out, "\x1b[{};{}H", self.off_r + row + 1, self.off_c + col + 1).unwrap();
                    cursor_set = true;
                }
                if fg != Some(top) {
                    write!(out, "\x1b[38;2;{};{};{}m", top.0, top.1, top.2).unwrap();
                    fg = Some(top);
                }
                if bg != Some(bottom) {
                    write!(out, "\x1b[48;2;{};{};{}m", bottom.0, bottom.1, bottom.2).unwrap();
                    bg = Some(bottom);
                }
                out.push('▀');
            }
        }
        // Draw the overlay last so it sits on top of the frame. It writes at a
        // fixed position without touching `prev`; clear_overlay() invalidates
        // to repaint the covered cells once it expires.
        if let Some(msg) = &self.overlay {
            write!(out, "\x1b[1;1H\x1b[97;44m {msg} \x1b[0m").unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb_2x2(c: [Rgb; 4]) -> FrameBuffer {
        let mut fb = FrameBuffer::new(2, 2);
        fb.pixels = c.to_vec();
        fb
    }

    #[test]
    fn first_render_emits_every_cell() {
        let mut s = Screen::new(2, 2);
        let mut out = String::new();
        s.render(
            &fb_2x2([Rgb(1, 1, 1), Rgb(2, 2, 2), Rgb(3, 3, 3), Rgb(4, 4, 4)]),
            &mut out,
        );
        assert_eq!(out.matches('▀').count(), 2); // 2x2 px = 1 row of 2 cells
        assert!(out.starts_with("\x1b[1;1H"));
        assert!(out.contains("\x1b[38;2;1;1;1m"));
        assert!(out.contains("\x1b[48;2;3;3;3m"));
    }

    #[test]
    fn identical_frame_emits_nothing() {
        let mut s = Screen::new(2, 2);
        let fb = fb_2x2([Rgb(1, 1, 1); 4]);
        let mut out = String::new();
        s.render(&fb, &mut out);
        out.clear();
        s.render(&fb, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn single_pixel_change_emits_one_cell() {
        let mut s = Screen::new(2, 2);
        let mut out = String::new();
        s.render(&fb_2x2([Rgb(1, 1, 1); 4]), &mut out);
        out.clear();
        s.render(
            &fb_2x2([Rgb(9, 9, 9), Rgb(1, 1, 1), Rgb(1, 1, 1), Rgb(1, 1, 1)]),
            &mut out,
        );
        assert_eq!(out.matches('▀').count(), 1);
        assert!(out.contains("\x1b[1;1H"));
    }

    #[test]
    fn overlay_draws_text_over_the_frame() {
        let mut s = Screen::new(8, 4);
        let mut out = String::new();
        s.set_overlay("hi");
        s.render(&FrameBuffer::new(8, 4), &mut out);
        assert!(out.contains("hi"));
    }

    #[test]
    fn invalidate_forces_full_redraw() {
        let mut s = Screen::new(2, 2);
        let fb = fb_2x2([Rgb(1, 1, 1); 4]);
        let mut out = String::new();
        s.render(&fb, &mut out);
        s.invalidate();
        out.clear();
        s.render(&fb, &mut out);
        assert_eq!(out.matches('▀').count(), 2);
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn small_viewport_downscales_with_averaging() {
        // 4x4 source, quadrants of distinct colors
        let mut fb = FrameBuffer::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let v = match (x < 2, y < 2) {
                    (true, true) => 10,
                    (false, true) => 20,
                    (true, false) => 30,
                    (false, false) => 40,
                };
                fb.pixels[y * 4 + x] = Rgb(v, v, v);
            }
        }
        let mut s = Screen::new(4, 4);
        s.set_viewport(2, 1); // halve: 2x1 cells = 2x2 output pixels
        let mut out = String::new();
        s.render(&fb, &mut out);
        assert_eq!(out.matches('▀').count(), 2);
        // cell (0,0): top = avg of top-left quadrant (10), bottom = bottom-left (30)
        assert!(out.contains("\x1b[38;2;10;10;10m"));
        assert!(out.contains("\x1b[48;2;30;30;30m"));
        assert!(out.contains("\x1b[38;2;20;20;20m"));
        assert!(out.contains("\x1b[48;2;40;40;40m"));
    }

    #[test]
    fn large_viewport_centers_pixel_perfect_output() {
        let mut s = Screen::new(4, 4); // native: 4 cols x 2 rows
        s.set_viewport(10, 10);
        let fb = FrameBuffer::new(4, 4);
        let mut out = String::new();
        s.render(&fb, &mut out);
        assert_eq!(out.matches('▀').count(), 8); // pixel-perfect: 4x2 cells
        assert!(out.contains("\x1b[5;4H")); // centered: row off 4, col off 3
    }

    #[test]
    fn viewport_change_invalidates() {
        let mut s = Screen::new(4, 4);
        let fb = FrameBuffer::new(4, 4);
        let mut out = String::new();
        s.set_viewport(4, 2);
        s.render(&fb, &mut out);
        out.clear();
        s.set_viewport(4, 2); // unchanged: no redraw
        s.render(&fb, &mut out);
        assert!(out.is_empty());
        s.set_viewport(2, 1); // changed: full redraw at new size
        s.render(&fb, &mut out);
        assert_eq!(out.matches('▀').count(), 2);
    }
}
