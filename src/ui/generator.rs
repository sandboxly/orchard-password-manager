//! The password generator: a dialog over the entry editor. Every change to
//! the choices makes a new password; F2 makes another; "Use password" puts
//! it in the editor's password field. Like the editor's, the password is
//! hidden unless F3 shows it.

use orchard_grid::{CellRect, Frame, Style};
use orchard_kit::form::{self, Act, Ctl, Form, Kind, TextKind};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, Named};
use orchard_kit::widgets::{DrawCtx, put_elided};
use zeroize::Zeroizing;

use crate::generator::{self, MAX_LENGTH, MIN_LENGTH, Options};
use crate::secret::SecretField;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Length,
    Lower,
    Upper,
    Digits,
    Symbols,
    NoLookalikes,
    Use,
    Cancel,
}

pub struct Generator {
    pub options: Options,
    /// The password made last.
    pub password: SecretField,
    pub reveal: bool,
    pub focus: Option<Field>,
    pub error: Option<String>,
}

/// What a key or click asks of the editor.
pub enum Outcome {
    None,
    /// Use this password.
    Use(Zeroizing<String>),
    Cancel,
}

const WIDTH: u16 = 60;
const ROWS: u16 = 13;
const LABEL_W: u16 = 10;

impl Generator {
    pub fn new(options: Options) -> Generator {
        let mut g = Generator { options, password: SecretField::default(), reveal: false, focus: Some(Field::Use), error: None };
        g.generate();
        g
    }

    fn generate(&mut self) {
        match generator::generate(&self.options) {
            Some(p) => self.password.set(&p),
            None => self.password.clear(),
        }
    }

    fn rects(screen: CellRect) -> (CellRect, CellRect) {
        form::dialog_rects(screen, WIDTH, ROWS)
    }

    fn form(&self, r: CellRect) -> Form<Field> {
        let o = &self.options;
        let mut f = Form::default();
        let line = |n: u16| CellRect::new(r.col, r.row + n, r.cols, 1);
        f.label(CellRect { cols: LABEL_W, ..line(2) }, "Length", TextKind::Label);
        let spin = CellRect::new(r.col + LABEL_W, r.row + 2, 12, 1);
        f.ctls.push(Ctl::new(Field::Length, spin, Kind::Spin(o.length.to_string())));
        let checks = [
            (Field::Lower, "Lowercase letters (a–z)", o.lower),
            (Field::Upper, "Capital letters (A–Z)", o.upper),
            (Field::Digits, "Digits (0–9)", o.digits),
            (Field::Symbols, "Symbols (!#$%…)", o.symbols),
            (Field::NoLookalikes, "Leave out look-alikes (l I 1 O 0 o |)", o.no_lookalikes),
        ];
        for (n, (id, label, on)) in checks.into_iter().enumerate() {
            f.ctls.push(Ctl::new(id, line(3 + n as u16), Kind::Check(label.into(), on)));
        }
        let buttons = form::ButtonRow::new(&["Use password", "Cancel"]);
        for (id, rect) in [Field::Use, Field::Cancel].into_iter().zip(buttons.rects(line(ROWS - 1))) {
            let label = if id == Field::Use { "Use password" } else { "Cancel" };
            f.ctls.push(Ctl::new(id, rect, Kind::Button(label.into())));
        }
        f
    }

    pub fn handle_key(&mut self, ev: &KeyEvent, screen: CellRect) -> Outcome {
        let (_, r) = Generator::rects(screen);
        if ev.is(Named::F(3)) {
            self.reveal = !self.reveal;
            return Outcome::None;
        }
        if ev.is(Named::F(2)) || (ev.mods.primary() && ev.key == Key::Char('g')) {
            self.generate();
            return Outcome::None;
        }
        // Enter uses the password from anywhere but Cancel (Space ticks
        // the boxes).
        if ev.is(Named::Enter) && self.focus != Some(Field::Cancel) {
            return self.use_it();
        }
        let form = self.form(r);
        let mut focus = self.focus;
        let act = form.handle_key(&mut focus, ev, None);
        self.focus = focus;
        self.act(act)
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent, screen: CellRect) -> Outcome {
        let (_, r) = Generator::rects(screen);
        let form = self.form(r);
        let mut focus = self.focus;
        let act = form.handle_mouse(&mut focus, ev);
        self.focus = focus;
        self.act(act)
    }

    fn use_it(&self) -> Outcome {
        if self.password.is_empty() { Outcome::None } else { Outcome::Use(Zeroizing::new(self.password.text().to_string())) }
    }

    fn act(&mut self, act: Act<Field>) -> Outcome {
        let o = &mut self.options;
        let before = *o;
        match act {
            Act::Step(Field::Length, d) => o.length = (o.length as i32 + d).clamp(MIN_LENGTH.into(), MAX_LENGTH.into()) as u16,
            Act::Toggle(id) => match id {
                Field::Lower => o.lower = !o.lower,
                Field::Upper => o.upper = !o.upper,
                Field::Digits => o.digits = !o.digits,
                Field::Symbols => o.symbols = !o.symbols,
                Field::NoLookalikes => o.no_lookalikes = !o.no_lookalikes,
                _ => {}
            },
            Act::Press(Field::Use) | Act::Submit => return self.use_it(),
            Act::Press(Field::Cancel) | Act::Escape => return Outcome::Cancel,
            _ => {}
        }
        if *o != before {
            // At least one kind of character.
            if !(o.lower || o.upper || o.digits || o.symbols) {
                *o = before;
                self.error = Some("Keep at least one kind of character".into());
                return Outcome::None;
            }
            self.error = None;
            self.generate();
        }
        Outcome::None
    }

    pub fn draw(&self, f: &mut Frame, screen: CellRect, ctx: &DrawCtx) {
        let c = &ctx.theme.colors;
        // On top: the cursor of what is underneath doesn't show through.
        f.cursor = None;
        let (outer, _) = Generator::rects(screen);
        let r = form::dialog_box(f, outer, "Generate password", ctx);
        self.password.draw(f, CellRect::new(r.col, r.row, r.cols, 1), ctx, false, self.reveal);
        let form = self.form(r);
        form.draw(f, self.focus, ctx);
        let bits = self.options.bits();
        let (text, style) = match &self.error {
            Some(e) => (e.clone(), Style::new(c.error, c.surface)),
            None => (format!("Strength: {} ({bits} bits)", generator::strength(bits)), Style::new(c.muted, c.surface)),
        };
        put_elided(f, r.col, r.row + 9, &text, style, r.cols);
        let show = if self.reveal { "F3 hides it" } else { "F3 shows it" };
        put_elided(f, r.col, r.row + 10, &format!("F2 makes another · {show} · Enter uses it"), Style::new(c.muted, c.surface), r.cols);
    }
}
