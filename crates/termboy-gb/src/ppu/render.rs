use super::Ppu;

impl Ppu {
    pub(super) fn render_line(&mut self) {
        let y = self.ly as usize;
        self.frame[y * 160..(y + 1) * 160].fill(0);
    }
}
