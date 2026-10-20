//! A text field of several lines, for an entry's notes (the kit's
//! `TextField` holds one line). Lines don't wrap: the view scrolls to keep
//! the cursor in sight, across and down. Enter starts a new line; Up and
//! Down move between lines and leave the field past its first or last.

use orchard_grid::{CellRect, Cursor, CursorShape, Frame, Style};
use orchard_kit::input::{Key, KeyEvent, Named};
use orchard_kit::widgets::{DrawCtx, put};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesField {
    /// At least one (empty) line.
    lines: Vec<String>,
    /// The cursor: line, and char in it.
    line: usize,
    col: usize,
    /// The text had "\r\n" line breaks; `text` gives them back that way.
    crlf: bool,
}

impl NotesField {
    pub fn new(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        let line = lines.len() - 1;
        let col = lines[line].chars().count();
        NotesField { lines, line, col, crlf }
    }

    pub fn text(&self) -> String {
        self.lines.join(if self.crlf { "\r\n" } else { "\n" })
    }

    /// The cursor at the start (a field opened for editing).
    pub fn home(mut self) -> Self {
        (self.line, self.col) = (0, 0);
        self
    }

    fn len(&self, line: usize) -> usize {
        self.lines[line].chars().count()
    }

    fn byte(&self, line: usize, col: usize) -> usize {
        let l = &self.lines[line];
        l.char_indices().nth(col).map_or(l.len(), |(b, _)| b)
    }

    /// Insert at the cursor; line breaks in `text` start new lines.
    pub fn insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        let mut parts = text.split('\n').map(|p| p.chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect::<String>());
        let at = self.byte(self.line, self.col);
        let rest = self.lines[self.line].split_off(at);
        let first = parts.next().unwrap_or_default();
        self.lines[self.line].push_str(&first);
        self.col += first.chars().count();
        for part in parts {
            self.line += 1;
            self.col = part.chars().count();
            self.lines.insert(self.line, part);
        }
        self.lines[self.line].push_str(&rest);
    }

    /// Edit and move keys; returns whether the key was taken. Up on the
    /// first line and Down on the last aren't (the form moves on). ⌘V /
    /// Ctrl+V pastes, line breaks and all.
    pub fn handle(&mut self, ev: &KeyEvent) -> bool {
        if ev.mods.primary() {
            match &ev.key {
                Key::Char('v') => {
                    if let Some(t) = orchard_kit::clipboard::text() {
                        self.insert(&t);
                    }
                    return true;
                }
                Key::Named(Named::Home) => (self.line, self.col) = (0, 0),
                Key::Named(Named::End) => {
                    self.line = self.lines.len() - 1;
                    self.col = self.len(self.line);
                }
                _ => return false,
            }
            return true;
        }
        let n = self.len(self.line);
        match &ev.key {
            Key::Named(Named::Left) if self.col > 0 => self.col -= 1,
            Key::Named(Named::Left) if self.line > 0 => {
                self.line -= 1;
                self.col = self.len(self.line);
            }
            Key::Named(Named::Right) if self.col < n => self.col += 1,
            Key::Named(Named::Right) if self.line + 1 < self.lines.len() => (self.line, self.col) = (self.line + 1, 0),
            Key::Named(Named::Left | Named::Right) => {}
            Key::Named(Named::Up) if self.line > 0 => {
                self.line -= 1;
                self.col = self.col.min(self.len(self.line));
            }
            Key::Named(Named::Down) if self.line + 1 < self.lines.len() => {
                self.line += 1;
                self.col = self.col.min(self.len(self.line));
            }
            Key::Named(Named::Up | Named::Down) => return false,
            Key::Named(Named::Home) => self.col = 0,
            Key::Named(Named::End) => self.col = n,
            Key::Named(Named::Enter) => self.insert("\n"),
            Key::Named(Named::Backspace) if self.col > 0 => {
                let (a, b) = (self.byte(self.line, self.col - 1), self.byte(self.line, self.col));
                self.lines[self.line].replace_range(a..b, "");
                self.col -= 1;
            }
            Key::Named(Named::Backspace) if self.line > 0 => {
                let l = self.lines.remove(self.line);
                self.line -= 1;
                self.col = self.len(self.line);
                self.lines[self.line].push_str(&l);
            }
            Key::Named(Named::Delete) if self.col < n => {
                let (a, b) = (self.byte(self.line, self.col), self.byte(self.line, self.col + 1));
                self.lines[self.line].replace_range(a..b, "");
            }
            Key::Named(Named::Delete) if self.line + 1 < self.lines.len() => {
                let l = self.lines.remove(self.line + 1);
                self.lines[self.line].push_str(&l);
            }
            Key::Named(Named::Backspace | Named::Delete) => {}
            _ => match ev.typed() {
                Some(t) => self.insert(t),
                None => return false,
            },
        }
        true
    }

    /// Where the text goes in the field `r`: one cell in from both ends,
    /// like the kit's fields.
    fn area(r: CellRect) -> CellRect {
        if r.cols < 4 { r } else { CellRect::new(r.col + 1, r.row, r.cols - 2, r.rows) }
    }

    /// The first line and char shown in `area`: the cursor in sight.
    fn scroll(&self, area: CellRect) -> (usize, usize) {
        let top = (self.line + 1).saturating_sub(area.rows.max(1) as usize);
        let left = self.col.saturating_sub(area.cols.saturating_sub(1) as usize);
        (top, left)
    }

    /// Put the cursor at a click on `col`, `row` of the field drawn in `r`.
    pub fn click(&mut self, r: CellRect, col: u16, row: u16) {
        let area = NotesField::area(r);
        let (top, left) = self.scroll(area);
        self.line = (top + row.saturating_sub(area.row) as usize).min(self.lines.len() - 1);
        self.col = (left + col.saturating_sub(area.col) as usize).min(self.len(self.line));
    }

    /// Draw into `r` (several rows) on the window background like the kit's
    /// form fields; the cursor shows while `focused`.
    pub fn draw(&self, f: &mut Frame, r: CellRect, ctx: &DrawCtx, focused: bool) {
        let c = &ctx.theme.colors;
        let style = Style::new(c.foreground, c.background);
        f.fill(r, ' ', style);
        let area = NotesField::area(r);
        if area.cols == 0 || area.rows == 0 {
            return;
        }
        let (top, left) = self.scroll(area);
        for (i, line) in self.lines.iter().skip(top).take(area.rows as usize).enumerate() {
            // Tabs and the like would break the grid.
            let shown: String = line.chars().skip(left).map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
            put(f, area.col, area.row + i as u16, &shown, style, area.cols);
        }
        if focused {
            f.cursor = Some(Cursor {
                col: area.col + (self.col - left) as u16,
                row: area.row + (self.line - top) as u16,
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

    fn keys(n: &mut NotesField, keys: &[Named]) {
        for k in keys {
            n.handle(&KeyEvent::named(*k));
        }
    }

    #[test]
    fn edits_across_lines() {
        let mut n = NotesField::new("one\ntwo").home();
        keys(&mut n, &[Named::End, Named::Enter]);
        n.insert("ä");
        assert_eq!(n.text(), "one\nä\ntwo");
        // Backspace at a line's start joins it to the one above.
        keys(&mut n, &[Named::Down, Named::Home, Named::Backspace]);
        assert_eq!(n.text(), "one\nätwo");
        // Delete at a line's end joins the next one.
        keys(&mut n, &[Named::Up, Named::End, Named::Delete]);
        assert_eq!(n.text(), "oneätwo");
        // Up on the first line is the form's.
        assert!(!n.handle(&KeyEvent::named(Named::Up)));
    }

    #[test]
    fn keeps_the_line_breaks_it_was_given() {
        let n = NotesField::new("a\r\nb");
        assert_eq!(n.text(), "a\r\nb");
        let mut n = NotesField::new("");
        n.insert("pasted\r\ntext");
        assert_eq!(n.text(), "pasted\ntext");
    }

    #[test]
    fn up_and_down_keep_the_column_where_they_can() {
        let mut n = NotesField::new("long line\nab\nlong line").home();
        keys(&mut n, &[Named::End, Named::Down]);
        assert_eq!((n.line, n.col), (1, 2));
        keys(&mut n, &[Named::Down]);
        assert_eq!((n.line, n.col), (2, 2));
    }

    #[test]
    fn a_click_lands_on_the_line_and_char() {
        let mut n = NotesField::new("one\ntwo\nthree");
        n.click(CellRect::new(10, 5, 20, 3), 13, 7);
        assert_eq!((n.line, n.col), (2, 2));
        n.click(CellRect::new(10, 5, 20, 3), 29, 5);
        assert_eq!((n.line, n.col), (0, 3));
    }
}
