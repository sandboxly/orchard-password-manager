//! The group filter (F5): every group as a tree with a checkbox, to show
//! the entries of all groups, one, or any selection. Checking a group
//! checks its subgroups too (unchecking likewise); single ones can then be
//! changed. The root stands for the whole vault.
//!
//! Keys: ↑↓ PgUp PgDn Home End move, Space checks, Enter shows the checked
//! groups, Tab goes to the buttons and back, Esc cancels.

use std::collections::HashSet;

use keepass::db::GroupId;
use orchard_grid::{CellRect, Frame, Style};
use orchard_kit::form::{self, ButtonRow, RowOutcome};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, MouseKind, Named};
use orchard_kit::widgets::{DrawCtx, Scrollbar, contains, fill, put, put_elided, width};

use crate::entries::GroupNode;

pub struct GroupFilter {
    pub nodes: Vec<GroupNode>,
    pub checked: Vec<bool>,
    pub cursor: usize,
    top: usize,
    /// The keyboard is on the buttons (else the list).
    on_buttons: bool,
    /// [Show, Cancel].
    buttons: ButtonRow,
    pub error: Option<String>,
}

/// What a key or click asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Show these groups' entries; `None`: every group's.
    Show(Option<HashSet<GroupId>>),
    Cancel,
}

const WIDTH: u16 = 62;
/// The most list rows; fewer in a short window.
const MAX_ROWS: u16 = 16;

impl GroupFilter {
    /// The tree, with the groups `shown` checked (all of them for `None`).
    pub fn new(nodes: Vec<GroupNode>, shown: Option<&HashSet<GroupId>>) -> GroupFilter {
        let checked = nodes.iter().map(|n| shown.is_none_or(|s| s.contains(&n.id))).collect();
        GroupFilter { nodes, checked, cursor: 0, top: 0, on_buttons: false, buttons: ButtonRow::new(&["Show", "Cancel"]), error: None }
    }

    /// The box, the list, the hint line and the buttons' row.
    fn rects(&self, screen: CellRect) -> (CellRect, CellRect, CellRect, CellRect) {
        let list_rows = (self.nodes.len() as u16).clamp(1, MAX_ROWS).min(screen.rows.saturating_sub(8)).max(1);
        let (outer, r) = form::dialog_rects(screen, WIDTH, list_rows + 4);
        let list = CellRect::new(r.col, r.row, r.cols, r.rows.saturating_sub(4));
        let hint = CellRect::new(r.col, r.row + list.rows + 1, r.cols, 1);
        let buttons = CellRect::new(r.col, r.row + r.rows.saturating_sub(1), r.cols, 1);
        (outer, list, hint, buttons)
    }

    /// The node and every one below it in the tree.
    fn subtree(&self, i: usize) -> std::ops::Range<usize> {
        let depth = self.nodes[i].depth;
        let end = self.nodes[i + 1..].iter().position(|n| n.depth <= depth).map_or(self.nodes.len(), |p| i + 1 + p);
        i..end
    }

    fn toggle(&mut self, i: usize) {
        let on = !self.checked[i];
        for j in self.subtree(i) {
            self.checked[j] = on;
        }
        self.error = None;
    }

    fn show(&mut self) -> Outcome {
        if self.checked.iter().all(|&c| c) {
            return Outcome::Show(None);
        }
        if !self.checked.contains(&true) {
            self.error = Some("Check at least one group, or Esc to keep the list as it was".into());
            return Outcome::None;
        }
        Outcome::Show(Some(self.nodes.iter().zip(&self.checked).filter(|(_, c)| **c).map(|(n, _)| n.id).collect()))
    }

    fn move_to(&mut self, i: usize, rows: usize) {
        self.cursor = i.min(self.nodes.len().saturating_sub(1));
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + rows {
            self.top = self.cursor + 1 - rows;
        }
    }

    pub fn handle_key(&mut self, ev: &KeyEvent, screen: CellRect) -> Outcome {
        let rows = self.rects(screen).1.rows.max(1) as usize;
        match ev.key {
            Key::Named(Named::Escape) => return Outcome::Cancel,
            Key::Named(Named::Tab) => {
                self.on_buttons = !self.on_buttons;
                return Outcome::None;
            }
            _ => {}
        }
        if self.on_buttons {
            return match self.buttons.handle_key(ev) {
                RowOutcome::Pressed(0) => self.show(),
                RowOutcome::Pressed(_) => Outcome::Cancel,
                _ => Outcome::None,
            };
        }
        let page = rows.saturating_sub(1).max(1);
        match ev.key {
            Key::Named(Named::Up) => self.move_to(self.cursor.saturating_sub(1), rows),
            Key::Named(Named::Down) => self.move_to(self.cursor + 1, rows),
            Key::Named(Named::PageUp) => self.move_to(self.cursor.saturating_sub(page), rows),
            Key::Named(Named::PageDown) => self.move_to(self.cursor + page, rows),
            Key::Named(Named::Home) => self.move_to(0, rows),
            Key::Named(Named::End) => self.move_to(self.nodes.len(), rows),
            Key::Named(Named::Enter) => return self.show(),
            Key::Char(' ') if ev.mods.none() => self.toggle(self.cursor),
            Key::Named(Named::Insert) => {
                self.toggle(self.cursor);
                self.move_to(self.cursor + 1, rows);
            }
            _ => {}
        }
        Outcome::None
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent, screen: CellRect) -> Outcome {
        let (_, list, _, buttons) = self.rects(screen);
        let rows = list.rows.max(1) as usize;
        match ev.kind {
            MouseKind::Scroll(lines) => {
                let n = self.nodes.len().saturating_sub(rows);
                self.top = (self.top as isize - lines.round() as isize).clamp(0, n as isize) as usize;
            }
            MouseKind::Down if contains(list, ev.col, ev.row) => {
                let i = self.top + (ev.row - list.row) as usize;
                if i < self.nodes.len() {
                    self.on_buttons = false;
                    self.cursor = i;
                    self.toggle(i);
                }
            }
            MouseKind::Down => match self.buttons.hit(buttons, ev.col, ev.row) {
                Some(0) => return self.show(),
                Some(_) => return Outcome::Cancel,
                None => {}
            },
            _ => {}
        }
        Outcome::None
    }

    pub fn draw(&self, f: &mut Frame, screen: CellRect, ctx: &DrawCtx) {
        let c = &ctx.theme.colors;
        f.cursor = None;
        let (outer, list, hint, buttons) = self.rects(screen);
        form::dialog_box(f, outer, "Show groups", ctx);
        for (line, (node, &on)) in self.nodes.iter().zip(&self.checked).enumerate().skip(self.top).take(list.rows as usize) {
            let y = list.row + (line - self.top) as u16;
            let row = CellRect::new(list.col, y, list.cols.saturating_sub(1), 1);
            let selected = line == self.cursor && !self.on_buttons;
            let style = if selected { Style::new(c.cursor_fg, c.cursor_bg) } else { Style::new(c.foreground, c.surface) };
            fill(f, row, style.bg);
            let count = node.entries.to_string();
            let count_x = row.col + row.cols.saturating_sub(width(&count) + 1);
            let x = row.col + 1 + 2 * node.depth.min(8) as u16;
            // "[-]": some subgroups aren't as it is.
            let mixed = self.subtree(line).any(|j| self.checked[j] != on);
            let mark = match (on, mixed) {
                (_, true) => "[-] ",
                (true, false) => "[x] ",
                (false, false) => "[ ] ",
            };
            let x = put(f, x, y, mark, style, count_x.saturating_sub(x));
            put_elided(f, x, y, &node.name, style, count_x.saturating_sub(x + 1));
            let muted = if selected { style } else { Style::new(c.muted, c.surface) };
            put(f, count_x, y, &count, muted, width(&count));
        }
        Scrollbar { total: self.nodes.len(), visible: list.rows as usize, top: self.top }.draw(f, list, ctx);
        let (text, style) = match &self.error {
            Some(e) => (e.as_str(), Style::new(c.error, c.surface)),
            None => ("Space checks a group and its subgroups · Enter shows them", Style::new(c.muted, c.surface)),
        };
        put_elided(f, hint.col, hint.row, text, style, hint.cols);
        self.buttons.draw(f, buttons, ctx, self.on_buttons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (keepass::Database, Vec<GroupNode>) {
        let mut db = keepass::Database::new();
        db.root_mut().name = "Vault".into();
        let work = {
            let mut root = db.root_mut();
            let mut g = root.add_group();
            g.name = "Work".into();
            g.id()
        };
        db.group_mut(work).unwrap().add_group().name = "Servers".into();
        db.root_mut().add_group().name = "Email".into();
        let nodes = crate::entries::group_tree(&db);
        (db, nodes)
    }

    #[test]
    fn checking_a_group_checks_its_subgroups() {
        let (_, nodes) = tree();
        let names: Vec<(usize, &str)> = nodes.iter().map(|n| (n.depth, n.name.as_str())).collect();
        assert_eq!(names, [(0, "Vault"), (1, "Email"), (1, "Work"), (2, "Servers")]);
        let mut g = GroupFilter::new(nodes.clone(), None);
        let screen = CellRect::new(0, 0, 100, 36);
        assert_eq!(g.checked, [true; 4]);
        // The root unchecks everything; Enter then asks for a group.
        g.handle_key(&KeyEvent::char(' '), screen);
        assert_eq!(g.checked, [false; 4]);
        assert_eq!(g.handle_key(&KeyEvent::named(Named::Enter), screen), Outcome::None);
        assert!(g.error.is_some());
        // Work and its Servers.
        g.handle_key(&KeyEvent::named(Named::Down), screen);
        g.handle_key(&KeyEvent::named(Named::Down), screen);
        g.handle_key(&KeyEvent::char(' '), screen);
        assert_eq!(g.checked, [false, false, true, true]);
        let want: HashSet<GroupId> = [nodes[2].id, nodes[3].id].into();
        assert_eq!(g.handle_key(&KeyEvent::named(Named::Enter), screen), Outcome::Show(Some(want.clone())));
        // Opened again, it shows what is shown; all checked means no filter.
        let mut g = GroupFilter::new(nodes, Some(&want));
        assert_eq!(g.checked, [false, false, true, true]);
        g.handle_key(&KeyEvent::char(' '), screen);
        assert_eq!(g.handle_key(&KeyEvent::named(Named::Enter), screen), Outcome::Show(None));
    }
}
