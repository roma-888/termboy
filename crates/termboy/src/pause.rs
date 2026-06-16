//! In-game pause menu: a pure state machine over the menu screens plus a pure
//! `render` that turns a `MenuView` into a full-screen ANSI string (terminal
//! text, ROM-picker style). No terminal I/O here, so the whole module is
//! unit-testable like `playback.rs` / `rewind.rs`.

use termboy_core::Rgb;

use crate::thumb::Thumb;

/// Number of save-state slots (digits 0-9).
pub const SLOTS: usize = 10;

/// Slot number shown at list position `pos`, ordered like the keyboard number
/// row: positions 0..8 are slots 1..9 and the last position is slot 0 — so the
/// list reads `1 2 3 … 9 0`, matching the `1`-`0` save keys.
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
/// decoded thumbnail). Held by `Menu`, projected into the view.
pub struct SlotInfo {
    pub filled: bool,
    pub age: String,
    pub thumb: Option<Thumb>,
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
    /// which hides the "Back to library" row.
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
            _ => self.slot_sel = self.slot_sel.saturating_sub(1),
        }
    }

    pub fn down(&mut self) {
        match self.screen {
            Screen::Main => self.main_sel = (self.main_sel + 1).min(self.items.len() - 1),
            Screen::Controls => {
                self.ctrl_sel = (self.ctrl_sel + 1).min(self.controls.len().saturating_sub(1))
            }
            _ => self.slot_sel = (self.slot_sel + 1).min(SLOTS - 1),
        }
    }

    /// `<-`: slow down on the Speed row; a no-op elsewhere.
    pub fn left(&mut self) -> Action {
        if self.screen == Screen::Main && self.items[self.main_sel] == MainItem::Speed {
            Action::SpeedSlower
        } else {
            Action::None
        }
    }

    /// `->`: speed up / open a browser on the main screen; a no-op elsewhere.
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

    /// A render-ready snapshot for `render`.
    pub fn view(&self) -> MenuView {
        match self.screen {
            Screen::Main => {
                let rows = self
                    .items
                    .iter()
                    .map(|it| match it {
                        MainItem::Resume => Row::label("Resume"),
                        MainItem::Save => Row::label("Save state"),
                        MainItem::Load => Row::label("Load state"),
                        MainItem::Speed => Row::kv("Speed", &self.speed_label),
                        MainItem::Mute => Row::kv("Mute", if self.muted { "on" } else { "off" }),
                        MainItem::Controls => Row::label("Controls"),
                        MainItem::Library => Row::label("Back to library"),
                        MainItem::Quit => Row::label("Quit"),
                    })
                    .collect();
                MenuView {
                    title: "PAUSED".into(),
                    selected: self.main_sel,
                    hint: "↑/↓ move · Enter select · Esc resume".into(),
                    selectable: true,
                    rows,
                    preview: None,
                }
            }
            Screen::SaveBrowser | Screen::LoadBrowser => {
                let rows = (0..SLOTS)
                    .map(|pos| {
                        let s = slot_at(pos);
                        let value = if self.slots[s].filled {
                            if self.slots[s].age.is_empty() {
                                "saved".into()
                            } else {
                                self.slots[s].age.clone()
                            }
                        } else {
                            "empty".into()
                        };
                        Row::kv(&format!("Slot {s}"), &value)
                    })
                    .collect();
                let title = if self.screen == Screen::SaveBrowser { "SAVE STATE" } else { "LOAD STATE" };
                let preview = self.slots[slot_at(self.slot_sel)].thumb.clone();
                MenuView {
                    title: title.into(),
                    selected: self.slot_sel,
                    hint: "↑/↓ move · Enter ok · Esc back".into(),
                    selectable: true,
                    rows,
                    preview,
                }
            }
            Screen::Controls => {
                let rows = self.controls.iter().map(|(a, k)| Row::kv(a, k)).collect();
                MenuView {
                    title: "CONTROLS".into(),
                    selected: self.ctrl_sel,
                    hint: "↑/↓ scroll · Esc back".into(),
                    selectable: false,
                    rows,
                    preview: None,
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
    /// Whether the selected row is highlighted (false for the Controls reference).
    pub selectable: bool,
    pub rows: Vec<Row>,
    /// Selected slot's thumbnail on the Save/Load screens, for the preview pane.
    pub preview: Option<Thumb>,
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

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const REV: &str = "\x1b[7m";
const ACCENT: &str = "\x1b[38;2;150;180;255m";

/// Preview pane size (cells) on the Save/Load screens.
const PREVIEW_COLS: usize = 28;
const PREVIEW_ROWS: usize = 13;
const PREVIEW_GAP: usize = 4;

/// Render the menu as a full-screen ANSI string: a centered title, a windowed row
/// list (selected row reverse-video when `selectable`), a dim hint, and — on
/// Save/Load — a half-block preview of the selected slot to the right.
pub fn render(view: &MenuView, cols: usize, rows: usize) -> String {
    use std::fmt::Write as _;
    let cols = cols.max(24);
    let rows = rows.max(10);

    let row_w = |r: &Row| {
        r.label.chars().count() + if r.value.is_empty() { 0 } else { 2 + r.value.chars().count() }
    };
    let mut list_w = view.title.chars().count();
    for r in &view.rows {
        list_w = list_w.max(row_w(r));
    }
    list_w = list_w.max(8);

    let has_preview = view.preview.is_some();
    let block_w = list_w + if has_preview { PREVIEW_GAP + PREVIEW_COLS } else { 0 };

    // Vertical layout: title, blank, rows.., blank, hint (+ 1-row top/bottom margin).
    let chrome = 4;
    let avail = rows.saturating_sub(2 + chrome);
    let visible = view.rows.len().min(avail.max(1));
    let top = if view.selected >= visible { view.selected + 1 - visible } else { 0 };
    let list_h = chrome + visible;
    let content_h = list_h.max(if has_preview { PREVIEW_ROWS + 2 } else { 0 });

    let start_row = rows.saturating_sub(content_h) / 2 + 1;
    let left = cols.saturating_sub(block_w) / 2 + 1;

    let mut out = String::from("\x1b[0m\x1b[2J\x1b[H");
    let mut row = start_row;

    // Title, centered over the list column.
    let tcol = left + list_w.saturating_sub(view.title.chars().count()) / 2;
    let _ = write!(out, "\x1b[{row};{tcol}H{BOLD}{ACCENT}{}{RESET}", view.title);
    row += 2;

    // Rows.
    for idx in top..top + visible {
        let r = &view.rows[idx];
        let used = r.label.chars().count() + r.value.chars().count();
        let pad = " ".repeat(list_w.saturating_sub(used));
        let _ = write!(out, "\x1b[{row};{left}H");
        if view.selectable && idx == view.selected {
            let _ = write!(out, "{REV} {}{pad}{} {RESET}", r.label, r.value);
        } else {
            let _ = write!(out, " {}{pad}{DIM}{}{RESET} ", r.label, r.value);
        }
        row += 1;
    }
    row += 1;

    // Hint, centered on the screen.
    let hcol = cols.saturating_sub(view.hint.chars().count()) / 2 + 1;
    let _ = write!(out, "\x1b[{row};{hcol}H{DIM}{}{RESET}", view.hint);

    // Preview pane to the right of the list.
    if let Some(t) = &view.preview {
        let pcol = left + list_w + PREVIEW_GAP;
        let prow = start_row + 1;
        for (i, line) in thumb_cells(t, PREVIEW_COLS, PREVIEW_ROWS).iter().enumerate() {
            let _ = write!(out, "\x1b[{};{pcol}H{line}{RESET}", prow + i);
        }
    }

    out
}

/// Half-block cell lines (no cursor positioning) rendering `t` into a
/// `box_cols` x `box_rows` cell region (each cell = 1x2 px), aspect-preserved and
/// centered on a dark backdrop.
fn thumb_cells(t: &Thumb, box_cols: usize, box_rows: usize) -> Vec<String> {
    use std::fmt::Write as _;
    let bg = Rgb(16, 18, 26);
    if t.width == 0 || t.height == 0 {
        return vec![" ".repeat(box_cols); box_rows];
    }
    let (tw, th) = (box_cols, box_rows * 2);
    let s = (tw * 1000 / t.width).min(th * 1000 / t.height).max(1);
    let dw = (t.width * s / 1000).clamp(1, tw);
    let dh = (t.height * s / 1000).clamp(1, th);
    let (ox, oy) = ((tw - dw) / 2, (th - dh) / 2);
    let sample = |px: usize, py: usize| -> Rgb {
        if px < ox || px >= ox + dw || py < oy || py >= oy + dh {
            return bg;
        }
        let sx = (px - ox) * t.width / dw;
        let sy = (py - oy) * t.height / dh;
        t.pixels[sy * t.width + sx]
    };
    (0..box_rows)
        .map(|cy| {
            let mut line = String::new();
            for cx in 0..box_cols {
                let (a, b) = (sample(cx, cy * 2), sample(cx, cy * 2 + 1));
                let _ = write!(
                    line,
                    "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m\u{2580}",
                    a.0, a.1, a.2, b.0, b.1, b.2
                );
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use termboy_core::FrameBuffer;

    fn menu() -> Menu {
        Menu::new(true, "1x".into(), false, empty_slots())
    }

    fn empty_slots() -> Vec<SlotInfo> {
        (0..SLOTS).map(|_| SlotInfo { filled: false, age: String::new(), thumb: None }).collect()
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
    fn save_and_load_return_the_selected_slot() {
        // Positions map to slots 1,2,…,9,0.
        let mut m = menu();
        m.down(); // Save
        m.select(); // SaveBrowser (pos 0 = slot 1)
        m.down();
        m.down(); // pos 2 = slot 3
        assert_eq!(m.select(), Action::Save(3));

        let mut m = menu();
        m.down();
        m.down(); // Load
        m.select(); // LoadBrowser (pos 0 = slot 1)
        m.down(); // pos 1 = slot 2
        assert_eq!(m.select(), Action::Load(2));
    }

    #[test]
    fn slot_list_orders_zero_last() {
        let mut m = menu();
        m.down(); // Save
        m.select(); // SaveBrowser
        let v = m.view();
        assert_eq!(v.rows[0].label, "Slot 1");
        assert_eq!(v.rows[SLOTS - 1].label, "Slot 0");
        drop(v);
        for _ in 0..SLOTS {
            m.down(); // clamps at the last position (slot 0)
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
        let mut m = Menu::new(false, "1x".into(), false, empty_slots());
        for _ in 0..20 {
            m.down(); // clamps at the last item (Quit)
        }
        assert_eq!(m.select(), Action::Quit);
    }

    #[test]
    fn library_present_with_a_library() {
        // Resume Save Load Speed Mute Controls(5) Library(6) Quit(7)
        let mut m = menu();
        for _ in 0..20 {
            m.down();
        }
        assert_eq!(m.select(), Action::Quit);
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
            m.down();
        }
        assert_eq!(m.select(), Action::None); // open Controls
        let v = m.view();
        assert_eq!(v.title, "CONTROLS");
        assert!(!v.selectable);
        assert!(v.rows.iter().any(|r| r.label == "A" && r.value == "X"));
        assert_eq!(m.back(), Action::None); // Esc -> Main
        assert_eq!(m.view().title, "PAUSED");
    }

    #[test]
    fn view_reflects_screen_and_values() {
        let mut slots = empty_slots();
        slots[3] = SlotInfo { filled: true, age: "2m".into(), thumb: None };
        let mut m = Menu::new(true, "2x".into(), true, slots);
        let v = m.view();
        assert_eq!(v.title, "PAUSED");
        assert!(v.rows.iter().any(|r| r.label == "Speed" && r.value == "2x"));
        assert!(v.rows.iter().any(|r| r.label == "Mute" && r.value == "on"));

        m.down(); // Save
        m.select(); // SaveBrowser
        let v = m.view();
        assert_eq!(v.title, "SAVE STATE");
        assert_eq!(v.rows.len(), SLOTS);
        // Slot 3 is filled; it renders at list position 2 (slots 1,2,3,…).
        assert_eq!(v.rows[2].label, "Slot 3");
        assert_eq!(v.rows[2].value, "2m");
        assert_eq!(v.rows[0].value, "empty"); // position 0 = slot 1
    }

    #[test]
    fn age_label_picks_a_compact_unit() {
        assert_eq!(age_label(5), "5s");
        assert_eq!(age_label(90), "1m");
        assert_eq!(age_label(3700), "1h");
        assert_eq!(age_label(90_000), "1d");
    }

    #[test]
    fn render_draws_title_rows_and_selection() {
        let s = render(&menu().view(), 80, 24);
        assert!(s.starts_with("\x1b[0m\x1b[2J\x1b[H"));
        assert!(s.contains("PAUSED"));
        assert!(s.contains("Resume"));
        assert!(s.contains(REV)); // selectable -> selected row highlighted
    }

    #[test]
    fn render_controls_has_no_selection_highlight() {
        let mut m = menu();
        m.set_controls(vec![("A".into(), "X".into())]);
        for _ in 0..5 {
            m.down();
        }
        m.select(); // Controls
        let s = render(&m.view(), 80, 24);
        assert!(s.contains("CONTROLS"));
        assert!(!s.contains(REV)); // reference screen: no highlight
    }

    #[test]
    fn render_save_screen_includes_a_preview_when_slot_filled() {
        let mut slots = empty_slots();
        slots[1] = SlotInfo {
            filled: true,
            age: "2m".into(),
            thumb: Some(crate::thumb::decode(&crate::thumb::encode(&FrameBuffer::new(160, 144))).unwrap()),
        };
        let mut m = Menu::new(true, "1x".into(), false, slots);
        m.down();
        m.select(); // SaveBrowser, pos 0 = slot 1 (filled) -> preview Some
        let v = m.view();
        assert!(v.preview.is_some());
        let s = render(&v, 100, 30);
        assert!(s.contains("Slot 1"));
        assert!(s.contains('\u{2580}')); // half-block preview cell
    }
}
