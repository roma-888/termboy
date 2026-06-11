//! Half-block diff renderer. Pure: consumes a FrameBuffer, produces an ANSI
//! string. No terminal I/O here — that keeps it unit-testable.

use termboy_core::{FrameBuffer, Rgb};

pub struct Screen {
    cols: usize,
    rows: usize,
    /// (top, bottom) color of every cell as last drawn; None = never drawn.
    prev: Vec<Option<(Rgb, Rgb)>>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let cols = width;
        let rows = height.div_ceil(2);
        Self { cols, rows, prev: vec![None; cols * rows] }
    }

    /// Terminal size needed: (cols, rows).
    pub fn required_size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Forget the previous frame (e.g. after a terminal resize/clear).
    pub fn invalidate(&mut self) {
        self.prev.fill(None);
    }

    /// Append ANSI for every changed cell to `out`.
    pub fn render(&mut self, fb: &FrameBuffer, out: &mut String) {
        use std::fmt::Write;
        let mut fg: Option<Rgb> = None;
        let mut bg: Option<Rgb> = None;
        for row in 0..self.rows {
            let mut cursor_set = false;
            for col in 0..self.cols {
                let top = fb.pixels[(row * 2) * fb.width + col];
                let bottom = if row * 2 + 1 < fb.height {
                    fb.pixels[(row * 2 + 1) * fb.width + col]
                } else {
                    top
                };
                let cell = (top, bottom);
                if self.prev[row * self.cols + col] == Some(cell) {
                    cursor_set = false; // gap: next change must reposition
                    continue;
                }
                self.prev[row * self.cols + col] = Some(cell);
                if !cursor_set {
                    write!(out, "\x1b[{};{}H", row + 1, col + 1).unwrap();
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
