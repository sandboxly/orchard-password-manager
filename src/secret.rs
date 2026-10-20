//! A one-line field for secrets (the master password): drawn as "•", its
//! text wiped when it is cleared or dropped. The kit's `TextField` keeps a
//! plain `String` and forms clone it every frame, so secrets never go
//! through it.

use orchard_grid::{CellRect, Cursor, CursorShape, Style};
use orchard_kit::input::{Key, KeyEvent, Named};
use orchard_kit::widgets::{DrawCtx, put};
use zeroize::{Zeroize, Zeroizing};

#[derive(Default)]
pub struct SecretField {
    text: Zeroizing<String>,
    /// In chars.
    cursor: usize,
}

impl SecretField {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Wipe the text.
    pub fn clear(&mut self) {
        self.text.zeroize();
        self.cursor = 0;
    }

    /// Replace the text (the old one wiped); the cursor goes to its end.
    pub fn set(&mut self, text: &str) {
        self.clear();
        self.insert(text);
    }

    fn byte(&self, i: usize) -> usize {
        self.text.char_indices().nth(i).map_or(self.text.len(), |(b, _)| b)
    }

    fn insert(&mut self, s: &str) {
        // Grow by hand: a reallocation would leave a copy behind unwiped.
        if self.text.capacity() < self.text.len() + s.len() {
            let mut bigger = Zeroizing::new(String::with_capacity((self.text.len() + s.len()).max(64) * 2));
            bigger.push_str(&self.text);
            self.text = bigger;
        }
        let at = self.byte(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    /// Edit keys; returns whether the key was taken. ⌘V / Ctrl+V pastes.
    pub fn handle(&mut self, ev: &KeyEvent) -> bool {
        let n = self.text.chars().count();
        if ev.mods.primary() {
            if ev.key == Key::Char('v')
                && let Some(t) = orchard_kit::clipboard::text().map(Zeroizing::new)
            {
                let line: Zeroizing<String> = Zeroizing::new(t.trim_end_matches(['\r', '\n']).chars().filter(|c| !c.is_control()).collect());
                self.insert(&line);
                return true;
            }
            return false;
        }
        match &ev.key {
            Key::Named(Named::Left) => self.cursor = self.cursor.saturating_sub(1),
            Key::Named(Named::Right) => self.cursor = (self.cursor + 1).min(n),
            Key::Named(Named::Home) => self.cursor = 0,
            Key::Named(Named::End) => self.cursor = n,
            Key::Named(Named::Backspace) if self.cursor > 0 => {
                let (a, b) = (self.byte(self.cursor - 1), self.byte(self.cursor));
                self.text.replace_range(a..b, "");
                self.cursor -= 1;
            }
            Key::Named(Named::Delete) if self.cursor < n => {
                let (a, b) = (self.byte(self.cursor), self.byte(self.cursor + 1));
                self.text.replace_range(a..b, "");
            }
            Key::Named(Named::Backspace | Named::Delete) => {}
            _ => match ev.typed() {
                Some(t) => {
                    let t = Zeroizing::new(t.to_string());
                    self.insert(&t);
                }
                None => return false,
            },
        }
        true
    }

    /// One row, a dot per character (the text itself if `reveal`), on the
    /// window background like the kit's form fields; the cursor shows while
    /// `focused`.
    pub fn draw(&self, f: &mut orchard_grid::Frame, r: CellRect, ctx: &DrawCtx, focused: bool, reveal: bool) {
        let c = &ctx.theme.colors;
        let style = Style::new(c.foreground, c.background);
        f.fill(r, ' ', style);
        if r.cols < 4 {
            return;
        }
        let area = CellRect::new(r.col + 1, r.row, r.cols - 2, 1);
        let room = area.cols.saturating_sub(1) as usize;
        let start = self.cursor.saturating_sub(room);
        let n = self.text.chars().count();
        let shown: Zeroizing<String> = if reveal { Zeroizing::new(self.text.chars().skip(start).collect()) } else { Zeroizing::new("•".repeat(n - start)) };
        put(f, area.col, area.row, &shown, style, area.cols);
        if focused {
            f.cursor = Some(Cursor {
                col: area.col + (self.cursor - start) as u16,
                row: area.row,
                shape: CursorShape::Beam,
                color: c.accent,
                text: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_like_a_text_field() {
        let mut s = SecretField::default();
        for ch in "pässword".chars() {
            s.handle(&KeyEvent::char(ch));
        }
        s.handle(&KeyEvent::named(Named::Home));
        s.handle(&KeyEvent::named(Named::Delete));
        s.handle(&KeyEvent::named(Named::End));
        s.handle(&KeyEvent::named(Named::Backspace));
        assert_eq!(s.text(), "ässwor");
        s.set("new");
        assert_eq!(s.text(), "new");
        s.clear();
        assert!(s.is_empty());
    }
}
