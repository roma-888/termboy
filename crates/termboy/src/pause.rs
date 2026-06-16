//! In-game pause menu: a pure state machine over the menu screens plus a
//! render-ready `MenuView`. No I/O here (the pixel compositor `compose_menu`
//! lives alongside and is also pure), so the whole module is unit-testable
//! like `playback.rs` / `rewind.rs`.

/// Number of save-state slots (digits 0-9).
pub const SLOTS: usize = 10;

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

    pub fn screen(&self) -> Screen {
        self.screen
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
        assert_eq!(m.screen(), Screen::LoadBrowser);
    }

    #[test]
    fn resume_from_top_and_back_from_browser() {
        let mut m = menu();
        assert_eq!(m.select(), Action::Resume); // Resume is index 0
        let mut m = menu();
        m.down(); // Save
        assert_eq!(m.select(), Action::None);
        assert_eq!(m.screen(), Screen::SaveBrowser);
        assert_eq!(m.back(), Action::None);
        assert_eq!(m.screen(), Screen::Main);
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
