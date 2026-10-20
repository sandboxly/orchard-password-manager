//! A question before something that can't be taken back (discarding an
//! edit, deleting for good, updating a vault's format): a small dialog with
//! a sentence or two and two buttons, the safe one focused.

use orchard_grid::{CellRect, Frame};
use orchard_kit::form::{self, ButtonRow, RowOutcome};
use orchard_kit::input::{KeyEvent, MouseEvent, MouseKind, Named};
use orchard_kit::widgets::DrawCtx;

pub struct Confirm<A> {
    pub title: String,
    pub text: String,
    /// What a yes does.
    pub action: A,
    /// [yes, no]; starts on no.
    pub buttons: ButtonRow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    None,
    Yes,
    No,
}

const WIDTH: u16 = 60;

impl<A> Confirm<A> {
    pub fn new(title: &str, text: impl Into<String>, yes: &str, no: &str, action: A) -> Self {
        let mut buttons = ButtonRow::new(&[yes, no]);
        buttons.focus = 1;
        Confirm { title: title.into(), text: text.into(), action, buttons }
    }

    fn lines(&self) -> Vec<String> {
        form::wrap(&self.text, (WIDTH - 4) as usize)
    }

    /// The box and the buttons' row.
    fn rects(&self, screen: CellRect) -> (CellRect, CellRect) {
        let n = self.lines().len() as u16;
        let (outer, r) = form::dialog_rects(screen, WIDTH, n + 2);
        (outer, CellRect::new(r.col, r.row + r.rows.saturating_sub(1), r.cols, 1))
    }

    /// ←/→ and Tab pick a button, Enter presses it, Esc says no.
    pub fn handle_key(&mut self, ev: &KeyEvent) -> Answer {
        if ev.is(Named::Escape) {
            return Answer::No;
        }
        if ev.is(Named::Tab) {
            self.buttons.focus = 1 - self.buttons.focus;
            return Answer::None;
        }
        match self.buttons.handle_key(ev) {
            RowOutcome::Pressed(0) => Answer::Yes,
            RowOutcome::Pressed(_) => Answer::No,
            _ => Answer::None,
        }
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent, screen: CellRect) -> Answer {
        if ev.kind != MouseKind::Down {
            return Answer::None;
        }
        let (_, row) = self.rects(screen);
        match self.buttons.hit(row, ev.col, ev.row) {
            Some(0) => Answer::Yes,
            Some(_) => Answer::No,
            None => Answer::None,
        }
    }

    pub fn draw(&self, f: &mut Frame, screen: CellRect, ctx: &DrawCtx) {
        // On top: the cursor of what is underneath doesn't show through.
        f.cursor = None;
        let (outer, row) = self.rects(screen);
        let r = form::dialog_box(f, outer, &self.title, ctx);
        for (i, line) in self.lines().iter().enumerate() {
            form::label(f, CellRect::new(r.col, r.row + i as u16, r.cols, 1), line, ctx);
        }
        self.buttons.draw(f, row, ctx, true);
    }
}
