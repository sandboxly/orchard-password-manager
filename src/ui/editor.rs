//! The entry editor: a dialog over the list for a new entry or a change to
//! one. The password goes into a `SecretField`, hidden unless F3 shows it;
//! the notes into a `NotesField` (several lines); everything else is a kit
//! form. Saving hands a `Draft` to the app.

use keepass::db::{EntryId, GroupId};
use orchard_grid::{CellRect, Frame, Style};
use orchard_kit::form::{self, Act, Choice, Ctl, Form, Kind, Popup, PopupOutcome, TextKind};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, Named, primary_label};
use orchard_kit::widgets::{DrawCtx, TextField, put_elided};
use zeroize::Zeroizing;

use crate::entries::Draft;
use crate::generator::Options;
use crate::secret::SecretField;
use crate::ui::generator::{self, Generator};
use crate::ui::notes::NotesField;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Username,
    Password,
    Url,
    Otp,
    Notes,
    Group,
    Save,
    Cancel,
}

pub struct Editor {
    /// `None`: a new entry.
    pub id: Option<EntryId>,
    pub title: TextField,
    pub username: TextField,
    pub password: SecretField,
    pub url: TextField,
    /// The one-time password link or secret (shown on F3, like the password).
    pub otp: SecretField,
    pub notes: NotesField,
    pub groups: Vec<(GroupId, String)>,
    /// Into `groups`.
    pub group: usize,
    pub reveal: bool,
    pub focus: Option<Field>,
    pub popup: Option<Popup>,
    pub error: Option<String>,
    /// The password generator, over the editor.
    pub generator: Option<Generator>,
    /// The generator's choices (the app keeps them in the settings).
    pub gen_options: Options,
    /// The fields as they were opened (to tell whether there are changes).
    initial: Draft,
}

/// What a key or click asks of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Save,
    Cancel,
}

const WIDTH: u16 = 76;
const ROWS: u16 = 15;
const NOTES_ROWS: u16 = 5;
const LABEL_W: u16 = 11;

impl Editor {
    /// Edit `draft` (an existing entry's if `id` is set).
    pub fn new(id: Option<EntryId>, draft: &Draft, groups: Vec<(GroupId, String)>, gen_options: Options) -> Editor {
        let mut password = SecretField::default();
        password.set(&draft.password);
        let mut otp = SecretField::default();
        otp.set(&draft.otp);
        let group = groups.iter().position(|(g, _)| *g == draft.group).unwrap_or(0);
        let mut ed = Editor {
            id,
            title: TextField::new(draft.title.as_str()),
            username: TextField::new(draft.username.as_str()),
            password,
            url: TextField::new(draft.url.as_str()),
            otp,
            notes: NotesField::new(&draft.notes).home(),
            groups,
            group,
            reveal: false,
            focus: Some(Field::Title),
            popup: None,
            error: None,
            generator: None,
            gen_options,
            initial: draft.clone(),
        };
        ed.initial = ed.draft();
        ed
    }

    /// Something differs from how the editor opened.
    pub fn changed(&self) -> bool {
        self.draft() != self.initial
    }

    /// The one-time password differs from how the editor opened.
    pub fn otp_changed(&self) -> bool {
        self.otp.text().trim() != self.initial.otp.as_str()
    }

    /// The fields as they are now.
    pub fn draft(&self) -> Draft {
        Draft {
            title: self.title.text.trim().to_string(),
            username: self.username.text.clone(),
            password: Zeroizing::new(self.password.text().to_string()),
            url: self.url.text.trim().to_string(),
            otp: Zeroizing::new(self.otp.text().trim().to_string()),
            notes: self.notes.text(),
            group: self.groups[self.group].0,
        }
    }

    fn items(&self) -> Vec<Choice> {
        self.groups.iter().map(|(_, name)| Some(name.clone())).collect()
    }

    /// The dialog's box and its content, centered in `screen`.
    pub fn rects(screen: CellRect) -> (CellRect, CellRect) {
        form::dialog_rects(screen, WIDTH, ROWS)
    }

    /// The controls, laid out in the content area `r`.
    fn form(&self, r: CellRect) -> Form<Field> {
        let mut f = Form::default();
        let field_w = r.cols.saturating_sub(LABEL_W);
        let line = |n: u16| CellRect::new(r.col, r.row + n, LABEL_W, 1);
        let field = |n: u16| CellRect::new(r.col + LABEL_W, r.row + n, field_w, 1);
        let text = |t: &TextField| Kind::Text(t.clone(), String::new());
        let group_row = 5 + NOTES_ROWS;
        for (n, label) in [(0, "Title"), (1, "Username"), (2, "Password"), (3, "URL"), (4, "OTP"), (5, "Notes"), (group_row, "Group")] {
            f.label(line(n), label, TextKind::Label);
        }
        f.ctls.push(Ctl::new(Field::Title, field(0), text(&self.title)));
        f.ctls.push(Ctl::new(Field::Username, field(1), text(&self.username)));
        // Drawn by the editor itself (a secret field); the form only places it.
        f.ctls.push(Ctl::new(Field::Password, field(2), Kind::Text(TextField::default(), String::new())));
        f.ctls.push(Ctl::new(Field::Url, field(3), Kind::Text(self.url.clone(), "https://…".into())));
        // A secret too.
        f.ctls.push(Ctl::new(Field::Otp, field(4), Kind::Text(TextField::default(), String::new())));
        // Drawn by the editor too (several lines).
        let notes = CellRect { rows: NOTES_ROWS, ..field(5) };
        f.ctls.push(Ctl::new(Field::Notes, notes, Kind::Text(TextField::default(), String::new())));
        f.ctls.push(Ctl::new(Field::Group, field(group_row), Kind::Combo { items: self.items(), current: self.group }));
        let buttons = form::ButtonRow::new(&["Save", "Cancel"]);
        let row = CellRect::new(r.col, r.row + ROWS - 1, r.cols, 1);
        for (id, rect) in [Field::Save, Field::Cancel].into_iter().zip(buttons.rects(row)) {
            let label = if id == Field::Save { "Save" } else { "Cancel" };
            f.ctls.push(Ctl::new(id, rect, Kind::Button(label.into())));
        }
        f
    }

    fn text_field(&mut self, id: Field) -> Option<&mut TextField> {
        match id {
            Field::Title => Some(&mut self.title),
            Field::Username => Some(&mut self.username),
            Field::Url => Some(&mut self.url),
            _ => None,
        }
    }

    /// F2 / ⌘G: the generator.
    fn open_generator(&mut self) {
        self.popup = None;
        self.generator = Some(Generator::new(self.gen_options));
    }

    fn generator_outcome(&mut self, out: generator::Outcome) -> Outcome {
        let done = match out {
            generator::Outcome::None => false,
            generator::Outcome::Use(p) => {
                self.password.set(&p);
                self.focus = Some(Field::Password);
                self.error = None;
                true
            }
            generator::Outcome::Cancel => true,
        };
        if done && let Some(g) = self.generator.take() {
            self.gen_options = g.options;
        }
        Outcome::None
    }

    pub fn handle_key(&mut self, ev: &KeyEvent, screen: CellRect) -> Outcome {
        if let Some(g) = &mut self.generator {
            let out = g.handle_key(ev, screen);
            return self.generator_outcome(out);
        }
        if ev.is(Named::F(2)) || (ev.mods.primary() && ev.key == Key::Char('g')) {
            self.open_generator();
            return Outcome::None;
        }
        let (_, r) = Editor::rects(screen);
        let form = self.form(r);
        let items = self.items();
        if let Some(p) = &mut self.popup {
            match p.handle_key(ev, &items) {
                PopupOutcome::Chosen(i) => {
                    self.group = i;
                    self.popup = None;
                }
                PopupOutcome::Closed => self.popup = None,
                PopupOutcome::Consumed => {}
            }
            return Outcome::None;
        }
        if ev.is(Named::F(3)) {
            self.reveal = !self.reveal;
            return Outcome::None;
        }
        if ev.mods.primary() && ev.key == Key::Char('s') {
            return Outcome::Save;
        }
        form.fix_focus(&mut self.focus);
        // The password field takes its own edit keys; moving and Enter are
        // the form's.
        let moves = matches!(ev.key, Key::Named(Named::Tab | Named::Up | Named::Down | Named::Enter | Named::Escape));
        if self.focus == Some(Field::Password) && !moves && self.password.handle(ev) {
            self.error = None;
            return Outcome::None;
        }
        if self.focus == Some(Field::Otp) && !moves && self.otp.handle(ev) {
            self.error = None;
            return Outcome::None;
        }
        // The notes take Enter (a new line), and Up and Down until the
        // first or last line.
        let leaves = matches!(ev.key, Key::Named(Named::Tab | Named::Escape));
        if self.focus == Some(Field::Notes) && !leaves && self.notes.handle(ev) {
            self.error = None;
            return Outcome::None;
        }
        let mut focus = self.focus;
        let field = focus.and_then(|id| self.text_field(id));
        let act = form.handle_key(&mut focus, ev, field);
        self.focus = focus;
        self.act(act, &form)
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent, screen: CellRect) -> Outcome {
        if let Some(g) = &mut self.generator {
            let out = g.handle_mouse(ev, screen);
            return self.generator_outcome(out);
        }
        let (_, r) = Editor::rects(screen);
        let form = self.form(r);
        let items = self.items();
        if let Some(p) = &mut self.popup {
            match p.handle_mouse(ev, &items, screen) {
                PopupOutcome::Chosen(i) => {
                    self.group = i;
                    self.popup = None;
                }
                PopupOutcome::Closed => self.popup = None,
                PopupOutcome::Consumed => {}
            }
            return Outcome::None;
        }
        let mut focus = self.focus;
        let act = form.handle_mouse(&mut focus, ev);
        self.focus = focus;
        if let Act::Caret(Field::Notes, rect, col) = act {
            self.notes.click(rect, col, ev.row);
            return Outcome::None;
        }
        self.act(act, &form)
    }

    fn act(&mut self, act: Act<Field>, form: &Form<Field>) -> Outcome {
        match act {
            Act::Edited(_) => self.error = None,
            Act::Caret(id, rect, col) => {
                if let Some(t) = self.text_field(id) {
                    t.click(rect, col);
                }
            }
            Act::Step(Field::Group, d) => self.group = form::step(&self.items(), self.group, d),
            Act::Open(Field::Group) => {
                if let Some(c) = form.ctl(Field::Group) {
                    self.popup = Some(Popup::open(c.rect, &self.items(), self.group));
                }
            }
            Act::Press(Field::Save) | Act::Submit => return Outcome::Save,
            Act::Press(Field::Cancel) | Act::Escape => return Outcome::Cancel,
            _ => {}
        }
        Outcome::None
    }

    pub fn draw(&self, f: &mut Frame, screen: CellRect, ctx: &DrawCtx) {
        let c = &ctx.theme.colors;
        let (outer, _) = Editor::rects(screen);
        let title = if self.id.is_some() { "Edit entry" } else { "New entry" };
        let r = form::dialog_box(f, outer, title, ctx);
        let form = self.form(r);
        form.draw(f, self.focus, ctx);
        if let Some(ctl) = form.ctl(Field::Password) {
            self.password.draw(f, ctl.rect, ctx, self.focus == Some(Field::Password), self.reveal);
        }
        if let Some(ctl) = form.ctl(Field::Otp) {
            let focused = self.focus == Some(Field::Otp);
            self.otp.draw(f, ctl.rect, ctx, focused, self.reveal);
            if self.otp.is_empty() && !focused {
                let hint = CellRect::new(ctl.rect.col + 1, ctl.rect.row, ctl.rect.cols.saturating_sub(2), 1);
                put_elided(f, hint.col, hint.row, "setup link or secret, to show sign-in codes", Style::new(c.muted, c.background), hint.cols);
            }
        }
        if let Some(ctl) = form.ctl(Field::Notes) {
            self.notes.draw(f, ctl.rect, ctx, self.focus == Some(Field::Notes));
        }
        let (text, style) = match &self.error {
            Some(e) => (e.clone(), Style::new(c.error, c.surface)),
            None if self.focus == Some(Field::Notes) => (primary_label("Enter adds a line · ⌘S saves · Esc cancels"), Style::new(c.muted, c.surface)),
            None if self.focus == Some(Field::Otp) => {
                (primary_label("⌘V pastes the otpauth:// link or the secret the site shows · Enter saves"), Style::new(c.muted, c.surface))
            }
            None => {
                let show = if self.reveal { "F3 hides it" } else { "F3 shows it" };
                (format!("F2 generates a password · {show} · Enter saves · Esc cancels"), Style::new(c.muted, c.surface))
            }
        };
        put_elided(f, r.col, r.row + ROWS - 3, &text, style, r.cols);
        if let Some(p) = &self.popup {
            p.draw(f, &self.items(), screen, ctx);
        }
        if let Some(g) = &self.generator {
            g.draw(f, screen, ctx);
        }
    }
}
