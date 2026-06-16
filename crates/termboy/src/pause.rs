//! In-game pause menu: a pure state machine over the menu screens plus a
//! render-ready `MenuView`. No I/O here (the pixel compositor `compose_menu`
//! lives alongside and is also pure), so the whole module is unit-testable
//! like `playback.rs` / `rewind.rs`.

use termboy_core::{FrameBuffer, Rgb};

use crate::screen::{glyph, GLYPH_H, GLYPH_W};
use crate::thumb::Thumb;

/// Number of save-state slots (digits 0-9).
pub const SLOTS: usize = 10;

/// Columns in the save/load thumbnail grid (10 slots -> 5x2).
pub const GRID_COLS: usize = 5;

/// Slot number shown at grid position `pos`, ordered like the keyboard number
/// row: positions 0..8 are slots 1..9 and the last position is slot 0 — so the
/// grid reads `1 2 3 4 5 / 6 7 8 9 0`, matching the `1`-`0` save keys.
fn slot_at(pos: usize) -> usize {
    if pos + 1 < SLOTS {
        pos + 1
    } else {
        0
    }
}

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
    Controls,
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
    Controls,
    Library,
    Quit,
}

/// Per-slot info the frontend loads when the menu opens (filled? + age string +
/// decoded thumbnail). Held by `Menu`, projected into a `Cell` for the view.
pub struct SlotInfo {
    pub filled: bool,
    pub age: String,
    pub thumb: Option<crate::thumb::Thumb>,
}

pub struct Menu {
    screen: Screen,
    items: Vec<MainItem>,
    main_sel: usize,
    slot_sel: usize,
    ctrl_sel: usize,
    slots: Vec<SlotInfo>,
    controls: Vec<(String, String)>,
    speed_label: String,
    muted: bool,
}

impl Menu {
    /// `has_library` is false on a direct-ROM launch (no picker to return to),
    /// which hides the "Back to library" row. `slots[i]` marks an occupied slot.
    pub fn new(has_library: bool, speed_label: String, muted: bool, slots: Vec<SlotInfo>) -> Self {
        let mut items = vec![
            MainItem::Resume,
            MainItem::Save,
            MainItem::Load,
            MainItem::Speed,
            MainItem::Mute,
            MainItem::Controls,
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
            ctrl_sel: 0,
            slots,
            controls: Vec::new(),
            speed_label,
            muted,
        }
    }

    /// Provide the controls reference rows (built by the frontend from the live
    /// keymap + fixed hotkeys) for the Controls screen.
    pub fn set_controls(&mut self, rows: Vec<(String, String)>) {
        self.controls = rows;
    }

    pub fn up(&mut self) {
        match self.screen {
            Screen::Main => self.main_sel = self.main_sel.saturating_sub(1),
            Screen::Controls => self.ctrl_sel = self.ctrl_sel.saturating_sub(1),
            _ => {
                if self.slot_sel >= GRID_COLS {
                    self.slot_sel -= GRID_COLS;
                }
            }
        }
    }

    pub fn down(&mut self) {
        match self.screen {
            Screen::Main => self.main_sel = (self.main_sel + 1).min(self.items.len() - 1),
            Screen::Controls => {
                self.ctrl_sel = (self.ctrl_sel + 1).min(self.controls.len().saturating_sub(1))
            }
            _ => {
                if self.slot_sel + GRID_COLS < SLOTS {
                    self.slot_sel += GRID_COLS;
                }
            }
        }
    }

    /// `<-`: slow down on the Speed row; move left in the browser grid.
    pub fn left(&mut self) -> Action {
        match self.screen {
            Screen::Main => {
                if self.items[self.main_sel] == MainItem::Speed {
                    return Action::SpeedSlower;
                }
                Action::None
            }
            Screen::Controls => Action::None,
            _ => {
                if self.slot_sel % GRID_COLS != 0 {
                    self.slot_sel -= 1;
                }
                Action::None
            }
        }
    }

    /// `->`: speed up / open a browser on the main screen; move right in the grid.
    pub fn right(&mut self) -> Action {
        match self.screen {
            Screen::Main => {
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
                Action::None
            }
            Screen::Controls => Action::None,
            Screen::SaveBrowser | Screen::LoadBrowser => {
                if self.slot_sel % GRID_COLS != GRID_COLS - 1 && self.slot_sel + 1 < SLOTS {
                    self.slot_sel += 1;
                }
                Action::None
            }
        }
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
                MainItem::Controls => self.enter(Screen::Controls),
                MainItem::Library => Action::Library,
                MainItem::Quit => Action::Quit,
            },
            Screen::SaveBrowser => Action::Save(slot_at(self.slot_sel)),
            Screen::LoadBrowser => Action::Load(slot_at(self.slot_sel)),
            Screen::Controls => Action::None,
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
        self.ctrl_sel = 0;
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
                        MainItem::Controls => Row::label("CONTROLS"),
                        MainItem::Library => Row::label("LIBRARY"),
                        MainItem::Quit => Row::label("QUIT"),
                    })
                    .collect();
                MenuView {
                    title: "PAUSED".into(),
                    selected: self.main_sel,
                    hint: "UP DOWN MOVE  ENTER OK  ESC BACK".into(),
                    selectable: true,
                    body: Body::List(rows),
                }
            }
            Screen::SaveBrowser | Screen::LoadBrowser => {
                let cells = (0..SLOTS)
                    .map(|pos| {
                        let s = slot_at(pos);
                        Cell {
                            num: format!("{s}"),
                            age: self.slots[s].age.clone(),
                            filled: self.slots[s].filled,
                            thumb: self.slots[s].thumb.clone(),
                        }
                    })
                    .collect();
                let title = if self.screen == Screen::SaveBrowser { "SAVE STATE" } else { "LOAD STATE" };
                MenuView {
                    title: title.into(),
                    selected: self.slot_sel,
                    hint: "MOVE  ENTER OK  ESC BACK".into(),
                    selectable: true,
                    body: Body::Grid(cells),
                }
            }
            Screen::Controls => {
                let rows = self
                    .controls
                    .iter()
                    .map(|(action, key)| Row::kv(action, key))
                    .collect();
                MenuView {
                    title: "CONTROLS".into(),
                    selected: self.ctrl_sel,
                    hint: "UP DOWN SCROLL  ESC BACK".into(),
                    selectable: false,
                    body: Body::List(rows),
                }
            }
        }
    }
}

/// A render-ready snapshot of the current screen (built by `Menu::view`).
pub struct MenuView {
    pub title: String,
    pub selected: usize,
    pub hint: String,
    /// Whether the selected row is drawn highlighted (false for reference screens
    /// like Controls, where the list just scrolls).
    pub selectable: bool,
    pub body: Body,
}

/// The body of a screen: a vertical list (main) or a thumbnail grid (browser).
pub enum Body {
    List(Vec<Row>),
    Grid(Vec<Cell>),
}

/// One save-slot cell in the browser grid.
pub struct Cell {
    pub num: String,
    pub age: String,
    pub filled: bool,
    pub thumb: Option<crate::thumb::Thumb>,
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
const CELL_BG: Rgb = Rgb(28, 30, 42);

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

/// Widest rendered line among the title, hint, and label+value rows at `scale`.
fn list_widest(title: &str, hint: &str, rows: &[Row], scale: usize) -> usize {
    let mut w = text_width(title, scale).max(text_width(hint, scale));
    for r in rows {
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

/// Draw a `t`-thick rectangle outline.
fn border(fb: &mut FrameBuffer, x: usize, y: usize, w: usize, h: usize, t: usize, c: Rgb) {
    fill_rect(fb, x, y, w, t, c);
    fill_rect(fb, x, y + h.saturating_sub(t), w, t, c);
    fill_rect(fb, x, y, t, h, c);
    fill_rect(fb, x + w.saturating_sub(t), y, t, h, c);
}

/// Nearest-scale `t` to fit (preserving aspect) centered in the `area_w`x`area_h`
/// box at (x, y).
fn blit_thumb(fb: &mut FrameBuffer, x: usize, y: usize, area_w: usize, area_h: usize, t: &Thumb) {
    if t.width == 0 || t.height == 0 || area_w == 0 || area_h == 0 {
        return;
    }
    let s = (area_w * 1000 / t.width).min(area_h * 1000 / t.height);
    let dw = (t.width * s / 1000).max(1);
    let dh = (t.height * s / 1000).max(1);
    let ox = x + area_w.saturating_sub(dw) / 2;
    let oy = y + area_h.saturating_sub(dh) / 2;
    for ddy in 0..dh {
        let sy = ddy * t.height / dh;
        for ddx in 0..dw {
            let sx = ddx * t.width / dw;
            put(fb, ox + ddx, oy + ddy, t.pixels[sy * t.width + sx]);
        }
    }
}

/// Dim a copy of `fb` and draw the menu `view` as a centered floating panel.
pub fn compose_menu(fb: &FrameBuffer, view: &MenuView) -> FrameBuffer {
    let mut out = FrameBuffer::new(fb.width, fb.height);
    for (i, p) in fb.pixels.iter().enumerate() {
        out.pixels[i] = dim_px(*p);
    }
    match &view.body {
        Body::List(rows) => draw_list_panel(&mut out, view, rows),
        Body::Grid(cells) => draw_grid_panel(&mut out, view, cells),
    }
    out
}

/// The main-screen list: a floating panel sized to fit within ~85% margins; rows
/// windowed around the selection so any count fits any frame size.
fn draw_list_panel(out: &mut FrameBuffer, view: &MenuView, rows: &[Row]) {
    let (w, h) = (out.width, out.height);
    let max_w = w * 85 / 100;
    let max_h = h * 85 / 100;
    let mut scale = (h / 110).clamp(1, 14);
    while scale > 1 {
        let pad = 3 * scale;
        let line_h = GLYPH_H * scale + scale;
        let want_rows = rows.len().min(8);
        let panel_w = list_widest(&view.title, &view.hint, rows, scale) + 2 * pad;
        let panel_h = 2 * pad + 2 * line_h + 2 * scale + want_rows * line_h;
        if panel_w <= max_w && panel_h <= max_h {
            break;
        }
        scale -= 1;
    }
    let line_h = GLYPH_H * scale + scale;
    let pad = 3 * scale;
    let gap = scale;

    let avail = max_h.saturating_sub(2 * pad + 2 * line_h + 2 * gap);
    let max_rows = (avail / line_h).max(1);
    let visible = rows.len().min(max_rows);
    let top = if view.selected >= visible { view.selected + 1 - visible } else { 0 };

    let panel_w = (list_widest(&view.title, &view.hint, rows, scale) + 2 * pad).min(max_w);
    let panel_h = (pad + line_h + gap + visible * line_h + gap + line_h + pad).min(max_h);
    let px = (w - panel_w) / 2;
    let py = (h - panel_h) / 2;

    fill_rect(out, px, py, panel_w, panel_h, BG);
    border(out, px, py, panel_w, panel_h, scale, BORDER);

    let mut y = py + pad;
    let tw = text_width(&view.title, scale);
    draw_text(out, px + panel_w.saturating_sub(tw) / 2, y, scale, &view.title, TITLE);
    y += line_h + gap;

    for idx in top..top + visible {
        let r = &rows[idx];
        let color = if view.selectable && idx == view.selected {
            fill_rect(out, px + scale, y, panel_w.saturating_sub(2 * scale), line_h, SEL_BG);
            SEL_TEXT
        } else {
            TEXT
        };
        draw_text(out, px + pad, y, scale, &r.label, color);
        if !r.value.is_empty() {
            let vw = text_width(&r.value, scale);
            draw_text(out, px + panel_w - pad - vw, y, scale, &r.value, color);
        }
        y += line_h;
    }
    y += gap;

    let hw = text_width(&view.hint, scale);
    draw_text(out, px + panel_w.saturating_sub(hw) / 2, y, scale, &view.hint, HINT);
}

/// The save/load browser: a grid of slot cells, each a thumbnail (or EMPTY) with
/// a "N age" label, selected cell outlined.
fn draw_grid_panel(out: &mut FrameBuffer, view: &MenuView, cells: &[Cell]) {
    let (w, h) = (out.width, out.height);
    let scale = (h / 240).clamp(1, 6); // label text scale
    let pad = 4 * scale;
    let line_h = GLYPH_H * scale + scale;

    let panel_w = w * 90 / 100;
    let panel_h = h * 85 / 100;
    let px = (w - panel_w) / 2;
    let py = (h - panel_h) / 2;
    fill_rect(out, px, py, panel_w, panel_h, BG);
    border(out, px, py, panel_w, panel_h, scale, BORDER);

    let tw = text_width(&view.title, scale);
    draw_text(out, px + panel_w.saturating_sub(tw) / 2, py + pad, scale, &view.title, TITLE);
    let hw = text_width(&view.hint, scale);
    draw_text(
        out,
        px + panel_w.saturating_sub(hw) / 2,
        py + panel_h - pad - GLYPH_H * scale,
        scale,
        &view.hint,
        HINT,
    );

    let grid_rows = cells.len().div_ceil(GRID_COLS).max(1);
    let gx = px + pad;
    let gy = py + pad + line_h + pad;
    let gw = panel_w.saturating_sub(2 * pad);
    let gh = panel_h.saturating_sub(2 * pad + 2 * line_h + 2 * pad);
    let cgap = 2 * scale;
    let cell_w = gw.saturating_sub((GRID_COLS - 1) * cgap) / GRID_COLS;
    let cell_h = gh.saturating_sub((grid_rows - 1) * cgap) / grid_rows;
    let label_h = line_h + scale;

    for (i, cell) in cells.iter().enumerate() {
        let (r, c) = (i / GRID_COLS, i % GRID_COLS);
        let cx = gx + c * (cell_w + cgap);
        let cy = gy + r * (cell_h + cgap);
        let selected = i == view.selected;
        fill_rect(out, cx, cy, cell_w, cell_h, if selected { SEL_BG } else { CELL_BG });

        let thumb_h = cell_h.saturating_sub(label_h);
        match &cell.thumb {
            Some(t) => blit_thumb(out, cx, cy, cell_w, thumb_h, t),
            None => {
                let ew = text_width("EMPTY", scale);
                draw_text(
                    out,
                    cx + cell_w.saturating_sub(ew) / 2,
                    cy + thumb_h.saturating_sub(GLYPH_H * scale) / 2,
                    scale,
                    "EMPTY",
                    TEXT,
                );
            }
        }

        let label = if cell.filled && !cell.age.is_empty() {
            format!("{} {}", cell.num, cell.age)
        } else {
            cell.num.clone()
        };
        let lw = text_width(&label, scale);
        let lcolor = if selected { SEL_TEXT } else { TEXT };
        draw_text(out, cx + cell_w.saturating_sub(lw) / 2, cy + cell_h - line_h, scale, &label, lcolor);

        if selected {
            border(out, cx, cy, cell_w, cell_h, scale, SEL_TEXT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> Menu {
        Menu::new(true, "1x".into(), false, empty_slots())
    }

    fn empty_slots() -> Vec<SlotInfo> {
        (0..SLOTS).map(|_| SlotInfo { filled: false, age: String::new(), thumb: None }).collect()
    }

    fn list_rows(v: &MenuView) -> &Vec<Row> {
        match &v.body {
            Body::List(r) => r,
            Body::Grid(_) => panic!("expected a list body"),
        }
    }

    fn grid_cells(v: &MenuView) -> &Vec<Cell> {
        match &v.body {
            Body::Grid(c) => c,
            Body::List(_) => panic!("expected a grid body"),
        }
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
        // Positions map to slots 1,2,3,4,5 / 6,7,8,9,0.
        let mut m = menu();
        m.down(); // Save
        m.select(); // -> SaveBrowser (pos 0 = slot 1)
        m.right();
        m.right(); // pos 2 = slot 3
        assert_eq!(m.select(), Action::Save(3));

        let mut m = menu();
        m.down();
        m.down(); // Load
        m.select(); // -> LoadBrowser (pos 0 = slot 1)
        m.right(); // pos 1 = slot 2
        assert_eq!(m.select(), Action::Load(2));
    }

    #[test]
    fn grid_nav_moves_in_two_dimensions() {
        let mut m = menu();
        m.down(); // Save
        m.select(); // -> SaveBrowser, pos 0 (slot 1)
        m.right();
        m.right(); // pos 2 (slot 3)
        m.down(); // pos 7 (slot 8)
        assert_eq!(m.select(), Action::Save(8));
        m.up(); // pos 2 (slot 3)
        m.left(); // pos 1 (slot 2)
        assert_eq!(m.select(), Action::Save(2));
    }

    #[test]
    fn grid_orders_slot_zero_last() {
        let mut m = menu();
        m.down(); // Save
        m.select(); // SaveBrowser
        let v = m.view();
        let cells = grid_cells(&v);
        assert_eq!(cells[0].num, "1"); // first cell is slot 1
        assert_eq!(cells[SLOTS - 1].num, "0"); // last cell is slot 0
        drop(v);
        // Navigating to the last cell saves slot 0.
        m.down(); // pos 5
        for _ in 0..4 {
            m.right(); // pos 9
        }
        assert_eq!(m.select(), Action::Save(0));
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
        let mut m = Menu::new(false, "1x".into(), false, empty_slots());
        for _ in 0..10 {
            m.down(); // clamps at the last item (Quit)
        }
        assert_eq!(m.select(), Action::Quit);
    }

    #[test]
    fn library_present_with_a_library() {
        // Resume Save Load Speed Mute Controls(5) Library(6) Quit(7)
        let mut m = menu();
        for _ in 0..20 {
            m.down(); // clamps at Quit (last)
        }
        assert_eq!(m.select(), Action::Quit);
        // Index 5 is Controls (opens the screen); index 6 is Library.
        let mut m = menu();
        for _ in 0..5 {
            m.down();
        }
        assert_eq!(m.select(), Action::None);
        assert_eq!(m.view().title, "CONTROLS");
        let mut m = menu();
        for _ in 0..6 {
            m.down();
        }
        assert_eq!(m.select(), Action::Library);
    }

    #[test]
    fn controls_screen_lists_provided_rows() {
        let mut m = menu();
        m.set_controls(vec![("A".into(), "X".into()), ("MENU".into(), "ESC".into())]);
        for _ in 0..5 {
            m.down(); // to Controls (index 5)
        }
        assert_eq!(m.select(), Action::None); // open Controls
        let v = m.view();
        assert_eq!(v.title, "CONTROLS");
        assert!(!v.selectable);
        let rows = list_rows(&v);
        assert!(rows.iter().any(|r| r.label == "A" && r.value == "X"));
        assert_eq!(m.back(), Action::None); // Esc -> Main
        assert_eq!(m.view().title, "PAUSED");
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
            selected: 0,
            hint: "ESC BACK".into(),
            selectable: true,
            body: Body::List(vec![Row::label("RESUME"), Row::kv("SPEED", "1X")]),
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
        let view = Menu::new(true, "1x".into(), false, empty_slots()).view();
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
        let view = MenuView { title: "LOAD STATE".into(), selected: 9, hint: "ESC BACK".into(), selectable: true, body: Body::List(rows) };
        let out = compose_menu(&fb, &view); // selected near the end -> windowed
        assert_eq!((out.width, out.height), (160, 144));
    }

    #[test]
    fn compose_menu_draws_the_browser_grid() {
        let fb = FrameBuffer::new(1600, 900);
        let mut slots = empty_slots();
        slots[0] = SlotInfo {
            filled: true,
            age: "2m".into(),
            thumb: Some(crate::thumb::decode(&crate::thumb::encode(&FrameBuffer::new(160, 144))).unwrap()),
        };
        let mut m = Menu::new(true, "1x".into(), false, slots);
        m.down();
        m.select(); // SaveBrowser
        let out = compose_menu(&fb, &m.view());
        assert_eq!((out.width, out.height), (1600, 900));
        assert!(out.pixels.iter().any(|p| *p == BG)); // panel exists
        assert!(out.pixels.iter().any(|p| *p == SEL_TEXT)); // selected cell border
    }

    #[test]
    fn view_reflects_screen_and_values() {
        let mut slots = empty_slots();
        slots[3] = SlotInfo { filled: true, age: "2m".into(), thumb: None };
        let mut m = Menu::new(true, "2x".into(), true, slots);
        let v = m.view();
        assert_eq!(v.title, "PAUSED");
        assert_eq!(v.selected, 0);
        let rows = list_rows(&v);
        assert!(rows.iter().any(|r| r.label == "SPEED" && r.value == "2X"));
        assert!(rows.iter().any(|r| r.label == "MUTE" && r.value == "ON"));

        m.down(); // Save
        m.select(); // SaveBrowser
        let v = m.view();
        assert_eq!(v.title, "SAVE STATE");
        let cells = grid_cells(&v);
        assert_eq!(cells.len(), SLOTS);
        // Slot 3 is filled; it renders at grid position 2 (slots 1,2,3,...).
        assert_eq!(cells[2].num, "3");
        assert!(cells[2].filled && cells[2].age == "2m");
        assert!(!cells[0].filled); // position 0 = slot 1, empty
    }
}
