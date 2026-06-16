//! In-game pause menu: a pure state machine over the menu screens plus a
//! render-ready `MenuView`. No I/O here (the pixel compositor `compose_menu`
//! lives alongside and is also pure), so the whole module is unit-testable
//! like `playback.rs` / `rewind.rs`.

use termboy_core::{FrameBuffer, Rgb};

use crate::screen::{glyph, GLYPH_H, GLYPH_W};

/// Number of save-state slots (digits 0-9).
pub const SLOTS: usize = 10;

/// Compact "time ago" for a slot's save file, from seconds elapsed.
pub fn age_label(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Which menu screen is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Main,
    SaveBrowser,
    LoadBrowser,
}

/// What the frontend should do in response to a keypress in the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Resume,
    Save(usize),
    Load(usize),
    SpeedFaster,
    SpeedSlower,
    ToggleMute,
    Library,
    Quit,
}

/// Selectable rows on the main screen (filtered by `has_library`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MainItem {
    Resume,
    Save,
    Load,
    Speed,
    Mute,
    Library,
    Quit,
}

pub struct Menu {
    screen: Screen,
    items: Vec<MainItem>,
    main_sel: usize,
    slot_sel: usize,
    slots: [bool; SLOTS],
    speed_label: String,
    muted: bool,
}

impl Menu {
    /// `has_library` is false on a direct-ROM launch (no picker to return to),
    /// which hides the "Back to library" row. `slots[i]` marks an occupied slot.
    pub fn new(has_library: bool, speed_label: String, muted: bool, slots: [bool; SLOTS]) -> Self {
        let mut items = vec![
            MainItem::Resume,
            MainItem::Save,
            MainItem::Load,
            MainItem::Speed,
            MainItem::Mute,
        ];
        if has_library {
            items.push(MainItem::Library);
        }
        items.push(MainItem::Quit);
        Self {
            screen: Screen::Main,
            items,
            main_sel: 0,
            slot_sel: 0,
            slots,
            speed_label,
            muted,
        }
    }

    pub fn up(&mut self) {
        match self.screen {
            Screen::Main => self.main_sel = self.main_sel.saturating_sub(1),
            _ => self.slot_sel = self.slot_sel.saturating_sub(1),
        }
    }

    pub fn down(&mut self) {
        match self.screen {
            Screen::Main => self.main_sel = (self.main_sel + 1).min(self.items.len() - 1),
            _ => self.slot_sel = (self.slot_sel + 1).min(SLOTS - 1),
        }
    }

    /// `<-`: slow down on the Speed row; elsewhere a no-op.
    pub fn left(&mut self) -> Action {
        if self.screen == Screen::Main && self.items[self.main_sel] == MainItem::Speed {
            Action::SpeedSlower
        } else {
            Action::None
        }
    }

    /// `->`: speed up on the Speed row; open the browser on Save/Load; else no-op.
    pub fn right(&mut self) -> Action {
        if self.screen == Screen::Main {
            match self.items[self.main_sel] {
                MainItem::Speed => return Action::SpeedFaster,
                MainItem::Save => {
                    self.enter(Screen::SaveBrowser);
                }
                MainItem::Load => {
                    self.enter(Screen::LoadBrowser);
                }
                _ => {}
            }
        }
        Action::None
    }

    /// `Enter`.
    pub fn select(&mut self) -> Action {
        match self.screen {
            Screen::Main => match self.items[self.main_sel] {
                MainItem::Resume => Action::Resume,
                MainItem::Save => self.enter(Screen::SaveBrowser),
                MainItem::Load => self.enter(Screen::LoadBrowser),
                MainItem::Speed => Action::None,
                MainItem::Mute => Action::ToggleMute,
                MainItem::Library => Action::Library,
                MainItem::Quit => Action::Quit,
            },
            Screen::SaveBrowser => Action::Save(self.slot_sel),
            Screen::LoadBrowser => Action::Load(self.slot_sel),
        }
    }

    /// `Esc`: back out one screen, or Resume from the main screen.
    pub fn back(&mut self) -> Action {
        match self.screen {
            Screen::Main => Action::Resume,
            _ => {
                self.screen = Screen::Main;
                Action::None
            }
        }
    }

    fn enter(&mut self, screen: Screen) -> Action {
        self.screen = screen;
        self.slot_sel = 0;
        Action::None
    }

    /// Refresh the displayed speed after the frontend applies a speed change.
    pub fn set_speed_label(&mut self, label: String) {
        self.speed_label = label;
    }

    /// Refresh the displayed mute state after the frontend toggles it.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// A render-ready snapshot for `compose_menu`.
    pub fn view(&self) -> MenuView {
        match self.screen {
            Screen::Main => {
                let rows = self
                    .items
                    .iter()
                    .map(|it| match it {
                        MainItem::Resume => Row::label("RESUME"),
                        MainItem::Save => Row::label("SAVE"),
                        MainItem::Load => Row::label("LOAD"),
                        MainItem::Speed => Row::kv("SPEED", &self.speed_label.to_ascii_uppercase()),
                        MainItem::Mute => Row::kv("MUTE", if self.muted { "ON" } else { "OFF" }),
                        MainItem::Library => Row::label("LIBRARY"),
                        MainItem::Quit => Row::label("QUIT"),
                    })
                    .collect();
                MenuView {
                    title: "PAUSED".into(),
                    rows,
                    selected: self.main_sel,
                    hint: "UP DOWN MOVE  ENTER OK  ESC BACK".into(),
                }
            }
            Screen::SaveBrowser | Screen::LoadBrowser => {
                let rows = (0..SLOTS)
                    .map(|i| Row::kv(&format!("SLOT {i}"), if self.slots[i] { "SAVED" } else { "EMPTY" }))
                    .collect();
                let title = if self.screen == Screen::SaveBrowser { "SAVE STATE" } else { "LOAD STATE" };
                MenuView {
                    title: title.into(),
                    rows,
                    selected: self.slot_sel,
                    hint: "ENTER OK  ESC BACK".into(),
                }
            }
        }
    }
}

/// A render-ready snapshot of the current screen (built by `Menu::view`).
pub struct MenuView {
    pub title: String,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub hint: String,
}

pub struct Row {
    pub label: String,
    pub value: String,
}

impl Row {
    fn label(s: &str) -> Self {
        Self { label: s.into(), value: String::new() }
    }
    fn kv(k: &str, v: &str) -> Self {
        Self { label: k.into(), value: v.into() }
    }
}

// Panel palette.
const DIM_NUM: u16 = 90; // /256 ~= 0.35 background dim
const BG: Rgb = Rgb(18, 20, 30);
const BORDER: Rgb = Rgb(120, 124, 150);
const TITLE: Rgb = Rgb(180, 200, 255);
const TEXT: Rgb = Rgb(214, 216, 224);
const SEL_BG: Rgb = Rgb(70, 84, 140);
const SEL_TEXT: Rgb = Rgb(255, 255, 255);
const HINT: Rgb = Rgb(120, 122, 134);

fn dim_px(p: Rgb) -> Rgb {
    Rgb(
        ((p.0 as u16 * DIM_NUM) >> 8) as u8,
        ((p.1 as u16 * DIM_NUM) >> 8) as u8,
        ((p.2 as u16 * DIM_NUM) >> 8) as u8,
    )
}

fn put(fb: &mut FrameBuffer, x: usize, y: usize, c: Rgb) {
    if x < fb.width && y < fb.height {
        fb.pixels[y * fb.width + x] = c;
    }
}

fn fill_rect(fb: &mut FrameBuffer, x: usize, y: usize, w: usize, h: usize, c: Rgb) {
    for dy in 0..h {
        for dx in 0..w {
            put(fb, x + dx, y + dy, c);
        }
    }
}

/// Width in pixels `text` occupies at `scale` (3x5 glyphs, 1px inter-glyph gap).
fn text_width(text: &str, scale: usize) -> usize {
    let n = text.chars().count();
    if n == 0 {
        0
    } else {
        n * (GLYPH_W + 1) * scale - scale
    }
}

/// Widest rendered line (title, hint, or a label+value row) at `scale`.
fn widest(view: &MenuView, scale: usize) -> usize {
    let mut w = text_width(&view.title, scale).max(text_width(&view.hint, scale));
    for r in &view.rows {
        let rw = text_width(&r.label, scale)
            + if r.value.is_empty() { 0 } else { 4 * scale + text_width(&r.value, scale) };
        w = w.max(rw);
    }
    w
}

/// Draw uppercase `text` with the 3x5 glyph font, each font pixel `scale`x`scale`,
/// top-left at (x, y).
fn draw_text(fb: &mut FrameBuffer, x: usize, y: usize, scale: usize, text: &str, c: Rgb) {
    let mut cx = x;
    for ch in text.chars() {
        let g = glyph(ch);
        for (row, line) in g.iter().enumerate() {
            for (col, byte) in line.as_bytes().iter().enumerate() {
                if *byte == b'#' {
                    fill_rect(fb, cx + col * scale, y + row * scale, scale, scale, c);
                }
            }
        }
        cx += (GLYPH_W + 1) * scale;
    }
}

/// Dim a copy of `fb` and draw the menu `view` as a centered floating panel.
/// Rows are windowed around the selection so any count fits any frame size.
pub fn compose_menu(fb: &FrameBuffer, view: &MenuView) -> FrameBuffer {
    let mut out = FrameBuffer::new(fb.width, fb.height);
    for (i, p) in fb.pixels.iter().enumerate() {
        out.pixels[i] = dim_px(*p);
    }

    // The panel floats with margins (~85% of the frame at most) and its text is
    // crisp at the kitty path's display resolution. Pick the largest glyph scale
    // — capped to a sane size — whose panel (widest line, up to 8 rows + chrome)
    // fits those margins; smaller frames or longer hints just shrink the text.
    let max_w = fb.width * 85 / 100;
    let max_h = fb.height * 85 / 100;
    let mut scale = (fb.height / 110).clamp(1, 14);
    while scale > 1 {
        let pad = 3 * scale;
        let line_h = GLYPH_H * scale + scale;
        let want_rows = view.rows.len().min(8);
        let panel_w = widest(view, scale) + 2 * pad;
        let panel_h = 2 * pad + 2 * line_h + 2 * scale + want_rows * line_h;
        if panel_w <= max_w && panel_h <= max_h {
            break;
        }
        scale -= 1;
    }
    let line_h = GLYPH_H * scale + scale; // glyph height + 1px gap, scaled
    let pad = 3 * scale;
    let gap = scale; // space around the title / hint

    // Window the rows to what fits the height budget, around the selection.
    let avail = max_h.saturating_sub(2 * pad + 2 * line_h + 2 * gap);
    let max_rows = (avail / line_h).max(1);
    let visible = view.rows.len().min(max_rows);
    let top = if view.selected >= visible { view.selected + 1 - visible } else { 0 };

    let panel_w = (widest(view, scale) + 2 * pad).min(max_w);
    let panel_h = (pad + line_h + gap + visible * line_h + gap + line_h + pad).min(max_h);
    let px = (fb.width - panel_w) / 2;
    let py = (fb.height - panel_h) / 2;

    // Background + 1px (scaled) border.
    fill_rect(&mut out, px, py, panel_w, panel_h, BG);
    fill_rect(&mut out, px, py, panel_w, scale, BORDER);
    fill_rect(&mut out, px, py + panel_h - scale, panel_w, scale, BORDER);
    fill_rect(&mut out, px, py, scale, panel_h, BORDER);
    fill_rect(&mut out, px + panel_w - scale, py, scale, panel_h, BORDER);

    let mut y = py + pad;

    // Title, centered.
    let tw = text_width(&view.title, scale);
    draw_text(&mut out, px + panel_w.saturating_sub(tw) / 2, y, scale, &view.title, TITLE);
    y += line_h + gap;

    // Rows (windowed), label left, value right, selected row highlighted.
    for idx in top..top + visible {
        let r = &view.rows[idx];
        let color = if idx == view.selected {
            fill_rect(&mut out, px + scale, y, panel_w.saturating_sub(2 * scale), line_h, SEL_BG);
            SEL_TEXT
        } else {
            TEXT
        };
        draw_text(&mut out, px + pad, y, scale, &r.label, color);
        if !r.value.is_empty() {
            let vw = text_width(&r.value, scale);
            draw_text(&mut out, px + panel_w - pad - vw, y, scale, &r.value, color);
        }
        y += line_h;
    }
    y += gap;

    // Hint, centered and dim.
    let hw = text_width(&view.hint, scale);
    draw_text(&mut out, px + panel_w.saturating_sub(hw) / 2, y, scale, &view.hint, HINT);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        Menu::new(true, "1x".into(), false, [false; SLOTS])
    }

    #[test]
    fn down_then_up_moves_main_selection() {
        let mut m = menu();
        m.down();
        m.down();
        // 0 Resume, 1 Save, 2 Load
        assert_eq!(m.select(), Action::None); // Load -> opens LoadBrowser
        assert_eq!(m.view().title, "LOAD STATE");
    }

    #[test]
    fn resume_from_top_and_back_from_browser() {
        let mut m = menu();
        assert_eq!(m.select(), Action::Resume); // Resume is index 0
        let mut m = menu();
        m.down(); // Save
        assert_eq!(m.select(), Action::None);
        assert_eq!(m.view().title, "SAVE STATE");
        assert_eq!(m.back(), Action::None);
        assert_eq!(m.view().title, "PAUSED");
        assert_eq!(m.back(), Action::Resume); // Esc on Main resumes
    }

    #[test]
    fn save_and_load_browsers_return_the_selected_slot() {
        let mut m = menu();
        m.down(); // Save
        m.select(); // -> SaveBrowser
        m.down();
        m.down(); // slot 2
        assert_eq!(m.select(), Action::Save(2));

        let mut m = menu();
        m.down();
        m.down(); // Load
        m.select(); // -> LoadBrowser
        m.down(); // slot 1
        assert_eq!(m.select(), Action::Load(1));
    }

    #[test]
    fn speed_row_left_right_and_mute_enter() {
        let mut m = menu();
        m.down();
        m.down();
        m.down(); // Speed (index 3)
        assert_eq!(m.right(), Action::SpeedFaster);
        assert_eq!(m.left(), Action::SpeedSlower);
        m.down(); // Mute (index 4)
        assert_eq!(m.select(), Action::ToggleMute);
    }

    #[test]
    fn library_hidden_without_a_library() {
        // has_library=false: items are Resume,Save,Load,Speed,Mute,Quit (6).
        let mut m = Menu::new(false, "1x".into(), false, [false; SLOTS]);
        for _ in 0..10 {
            m.down(); // clamps at the last item (Quit)
        }
        assert_eq!(m.select(), Action::Quit);
    }

    #[test]
    fn library_present_with_a_library() {
        let mut m = menu(); // has_library=true
        for _ in 0..10 {
            m.down(); // clamps at Quit (last)
        }
        assert_eq!(m.select(), Action::Quit);
        // The second-to-last item is Library.
        let mut m = menu();
        for _ in 0..5 {
            m.down();
        }
        assert_eq!(m.select(), Action::Library);
    }

    #[test]
    fn age_label_picks_a_compact_unit() {
        assert_eq!(age_label(5), "5s");
        assert_eq!(age_label(90), "1m");
        assert_eq!(age_label(3700), "1h");
        assert_eq!(age_label(90_000), "1d");
    }

    #[test]
    fn compose_menu_dims_background_and_draws_panel() {
        let mut fb = FrameBuffer::new(160, 144);
        for p in fb.pixels.iter_mut() {
            *p = Rgb(200, 200, 200);
        }
        let view = MenuView {
            title: "PAUSED".into(),
            rows: vec![Row::label("RESUME"), Row::kv("SPEED", "1X")],
            selected: 0,
            hint: "ESC BACK".into(),
        };
        let out = compose_menu(&fb, &view);
        assert_eq!((out.width, out.height), (160, 144));
        // The top-left corner is background, untouched by the centered panel -> dimmed.
        assert_eq!(out.pixels[0], dim_px(Rgb(200, 200, 200)));
        // The panel drew title-colored pixels and a selected-row highlight.
        assert!(out.pixels.iter().any(|p| *p == TITLE));
        assert!(out.pixels.iter().any(|p| *p == SEL_BG));
    }

    #[test]
    fn compose_menu_leaves_edge_margins_at_display_resolution() {
        // At a large (upscaled) buffer the panel must float with margins, not
        // fill the screen and overflow the hint off both edges.
        let mut fb = FrameBuffer::new(1600, 900);
        for p in fb.pixels.iter_mut() {
            *p = Rgb(200, 200, 200);
        }
        let view = Menu::new(true, "1x".into(), false, [false; SLOTS]).view();
        let out = compose_menu(&fb, &view);
        let dim = dim_px(Rgb(200, 200, 200));
        // Edge columns at mid-height stay dimmed background (panel within margins).
        assert_eq!(out.pixels[450 * 1600], dim);
        assert_eq!(out.pixels[450 * 1600 + 1599], dim);
        // The panel still exists toward the middle.
        assert!(out.pixels.iter().any(|p| *p == BG || *p == BORDER));
    }

    #[test]
    fn compose_menu_windows_a_long_list_without_panicking() {
        let fb = FrameBuffer::new(160, 144);
        let rows: Vec<Row> = (0..SLOTS).map(|i| Row::kv(&format!("SLOT {i}"), "EMPTY")).collect();
        let view = MenuView { title: "LOAD STATE".into(), rows, selected: 9, hint: "ESC BACK".into() };
        let out = compose_menu(&fb, &view); // selected near the end -> windowed
        assert_eq!((out.width, out.height), (160, 144));
    }

    #[test]
    fn view_reflects_screen_and_values() {
        let mut m = Menu::new(true, "2x".into(), true, {
            let mut s = [false; SLOTS];
            s[3] = true;
            s
        });
        let v = m.view();
        assert_eq!(v.title, "PAUSED");
        assert_eq!(v.selected, 0);
        // Speed row shows "2X", Mute row shows "ON".
        assert!(v.rows.iter().any(|r| r.label == "SPEED" && r.value == "2X"));
        assert!(v.rows.iter().any(|r| r.label == "MUTE" && r.value == "ON"));

        m.down(); // Save
        m.select(); // SaveBrowser
        let v = m.view();
        assert_eq!(v.title, "SAVE STATE");
        assert_eq!(v.rows.len(), SLOTS);
        assert_eq!(v.rows[3].value, "SAVED");
        assert_eq!(v.rows[0].value, "EMPTY");
    }
}
