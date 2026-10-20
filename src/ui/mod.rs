//! Drawing: a pure function of the app state into an `orchard_grid::Frame`.
//! The layout is recomputed, never stored; mouse hit testing uses it too.

pub mod confirm;
pub mod editor;
pub mod generator;
pub mod groups;
pub mod notes;
pub mod settings;

use std::time::Instant;

use orchard_grid::{CellRect, Frame, Layer, PxRect, Rgba, Style, elide_right};
use orchard_kit::form;
use orchard_kit::input::primary_label;
use orchard_kit::widgets::{self, BoxFrame, DrawCtx, Scrollbar, draw_key_bar, fill, inset, put, put_elided, width};

use crate::app::{App, Open, Screen, Unlock, UnlockFocus};
use crate::vault::fields;

pub const FKEYS: &[(&str, &str)] = &[
    ("F2", "Bin"),
    ("F3", "Show"),
    ("F4", "Edit"),
    ("F5", "Groups"),
    ("F7", "New"),
    ("F8", "Delete"),
    ("F9", "Settings"),
    ("F10", "Quit"),
];

/// The keys while the list shows the recycle bin.
pub const BIN_FKEYS: &[(&str, &str)] = &[
    ("F2", "Entries"),
    ("F3", "Show"),
    ("F4", "Empty bin"),
    ("F7", "Restore"),
    ("F8", "Delete"),
    ("F9", "Settings"),
    ("F10", "Quit"),
];

pub const UNLOCK_FKEYS: &[(&str, &str)] = &[("F2", "Open"), ("F3", "New"), ("F9", "Settings"), ("F10", "Quit")];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    /// The entry list's box.
    pub main: CellRect,
    pub header: CellRect,
    pub list: CellRect,
    /// The search line under the rows.
    pub search: CellRect,
    pub details: Option<CellRect>,
    pub status: CellRect,
    pub footer: CellRect,
}

/// Rows of the details box, frame included: three lines of notes in a
/// tall window, one in a short one.
const DETAILS_H: u16 = 8;
const DETAILS_SHORT_H: u16 = 6;

pub fn layout(app: &App) -> Layout {
    let (w, h) = (app.cols.max(30), app.rows.max(10));
    let keys = fkeys(app);
    let footer_h = widgets::key_bar_rows(w, keys).min(h / 4).max(1);
    let footer = CellRect::new(0, h - footer_h, w, footer_h);
    let mut avail = h - footer_h - 1;
    let status = CellRect::new(0, avail, w, 1);
    let details_h = [DETAILS_H, DETAILS_SHORT_H].into_iter().find(|&dh| avail >= dh + 8 + 2 * (dh - DETAILS_SHORT_H));
    let details = details_h.map(|dh| {
        avail -= dh;
        CellRect::new(0, avail, w, dh)
    });
    let main = CellRect::new(0, 0, w, avail);
    let inner = inset(main, 1, 1);
    let header = CellRect::new(inner.col, inner.row, inner.cols, 1);
    let list = CellRect::new(inner.col, inner.row + 1, inner.cols, inner.rows.saturating_sub(2));
    let search = CellRect::new(inner.col, inner.row + inner.rows.saturating_sub(1), inner.cols, 1);
    Layout { main, header, list, search, details, status, footer }
}

pub fn draw(app: &App, f: &mut Frame) {
    let ctx = DrawCtx { theme: &app.theme, cell_px: app.cell_px };
    f.clear(app.theme.colors.background);
    f.cursor = None;
    let l = layout(app);
    match &app.screen {
        Screen::Unlock(u) => {
            draw_unlock(app, u, f, &ctx);
            draw_key_bar(f, l.footer, UNLOCK_FKEYS, &ctx);
        }
        Screen::Open(o) => {
            draw_list(o, f, &l, &ctx);
            if let Some(r) = l.details {
                draw_details(app, o, f, r, &ctx);
            }
            draw_status(app, o, f, l.status, &ctx);
            draw_key_bar(f, l.footer, fkeys(app), &ctx);
            if let Some(ed) = &o.editor {
                ed.draw(f, app.screen_rect(), &ctx);
            }
            if let Some(g) = &o.group_filter {
                g.draw(f, app.screen_rect(), &ctx);
            }
            if let Some(c) = &o.confirm {
                c.draw(f, app.screen_rect(), &ctx);
            }
        }
    }
    if let Some(d) = &app.settings_dialog {
        // No caret from the screen underneath showing through.
        f.cursor = None;
        d.draw(app, f, &ctx);
    }
    draw_toast(app, f, &l, &ctx);
}

/// Shortcuts as this platform writes them: "⌘⇧L" on macOS, "Ctrl+Shift+L" elsewhere.
pub fn shortcut_label(text: &str) -> String {
    if cfg!(target_os = "macos") { text.to_string() } else { primary_label(&text.replace('⇧', "Shift+")) }
}

/// The key bar's keys for what is on screen.
pub fn fkeys(app: &App) -> &'static [(&'static str, &'static str)] {
    match &app.screen {
        Screen::Open(o) if o.bin => BIN_FKEYS,
        Screen::Open(_) => FKEYS,
        Screen::Unlock(_) => UNLOCK_FKEYS,
    }
}

/// Columns of the list: title, username, URL, group (a share of the width
/// each, the title widest).
fn columns(r: CellRect) -> [CellRect; 4] {
    let w = r.cols;
    let title = w * 32 / 100;
    let user = w * 24 / 100;
    let url = w * 26 / 100;
    let group = w - title - user - url;
    let mut x = r.col;
    [title, user, url, group].map(|cw| {
        let c = CellRect::new(x, r.row, cw, r.rows);
        x += cw;
        c
    })
}

fn draw_list(o: &Open, f: &mut Frame, l: &Layout, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let view = o.view();
    let n = o.rows.len();
    let count = match (view.len() == n, n) {
        (true, 1) => "1 entry".to_string(),
        (true, _) => format!("{n} entries"),
        (false, _) => format!("{} of {n}", view.len()),
    };
    let name = o.vault.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = match (&o.shown_groups, o.bin) {
        (_, true) => format!("{name} · Recycle bin"),
        (Some(shown), false) => format!("{name} · {}", shown_groups_label(o, shown)),
        (None, false) => name,
    };
    BoxFrame { title: &title, subtitle: &count, active: true }.draw(f, l.main, ctx);

    let head = Style::new(c.accent, c.surface).bold();
    let group = if o.bin { "Deleted from" } else { "Group" };
    for (col, label) in columns(l.header).iter().zip(["Title", "Username", "URL", group]) {
        put(f, col.col + 1, col.row, label, head, col.cols.saturating_sub(2));
    }

    if view.is_empty() {
        let text = match (o.rows.is_empty(), o.bin) {
            (true, true) => "The recycle bin is empty · F2 goes back to the entries",
            (true, false) => "No entries yet · F7 adds one",
            (false, false) if o.shown_groups.is_some() && o.search.text.is_empty() => "No entries in the groups shown · F5 shows others",
            (false, _) => "No entries match the search · Esc clears it",
        };
        put_elided(f, l.list.col + 2, l.list.row + 1, text, Style::new(c.muted, c.surface), l.list.cols.saturating_sub(4));
    }
    for (line, &i) in view.iter().skip(o.top).take(l.list.rows as usize).enumerate() {
        let row = &o.rows[i];
        let y = l.list.row + line as u16;
        let selected = o.top + line == o.cursor;
        let style = if selected { Style::new(c.cursor_fg, c.cursor_bg) } else { Style::new(c.foreground, c.surface) };
        if selected {
            fill(f, CellRect::new(l.list.col, y, l.list.cols, 1), style.bg);
        }
        let muted = if selected { style } else { Style::new(c.muted, c.surface) };
        let title = if row.title.is_empty() { "(no title)" } else { &row.title };
        for (col, (text, st)) in columns(l.list).iter().zip([(title, style), (&row.username, style), (&row.url, muted), (&row.group, muted)]) {
            put(f, col.col + 1, y, &elide_right(text, col.cols.saturating_sub(2) as usize), st, col.cols.saturating_sub(2));
        }
    }
    Scrollbar { total: view.len(), visible: l.list.rows as usize, top: o.top }.draw(f, l.list, ctx);

    // The search line: whatever is typed filters the list.
    let s = l.search;
    let label = "Search ";
    put(f, s.col + 1, s.row, label, Style::new(c.muted, c.surface), s.cols);
    let field = CellRect::new(s.col + 1 + width(label), s.row, s.cols.saturating_sub(2 + width(label)), 1);
    o.search.draw_field_hint(f, field, ctx, true, "type to filter");
}

/// "Work", "Work, Email", "4 groups": what the group filter shows.
fn shown_groups_label(o: &Open, shown: &std::collections::HashSet<keepass::db::GroupId>) -> String {
    let names: Vec<String> = crate::entries::group_tree(&o.vault.db).into_iter().filter(|n| shown.contains(&n.id)).map(|n| n.name).collect();
    match names.len() {
        0 => "No groups".into(),
        1 | 2 => names.join(", "),
        n => format!("{n} groups"),
    }
}

/// What a copy icon in the details copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyWhat {
    Username,
    Password,
    Url,
    Otp,
    Notes,
}

/// The details' copy icons for the selected entry: (cell, what it copies),
/// one column at the right edge, only on rows with a value.
pub fn copy_icons(app: &App) -> Vec<((u16, u16), CopyWhat)> {
    let (Screen::Open(o), Some(r)) = (&app.screen, layout(app).details) else { return Vec::new() };
    let inner = inset(r, 1, 1);
    let Some(entry) = o.selected().and_then(|row| o.vault.db.entry(row.id)) else { return Vec::new() };
    let has = |field: &str| entry.get(field).is_some_and(|v| !v.is_empty());
    let mut rows = Vec::new();
    for (line, field, what) in [(0, fields::USERNAME, CopyWhat::Username), (1, fields::PASSWORD, CopyWhat::Password), (2, fields::URL, CopyWhat::Url)] {
        if has(field) {
            rows.push((line, what));
        }
    }
    let mut notes = 3;
    if let Some(code) = crate::otp::code_now(&entry) {
        if code.is_ok() {
            rows.push((3, CopyWhat::Otp));
        }
        notes = 4;
    }
    if has(fields::NOTES) && notes < inner.rows {
        rows.push((notes, CopyWhat::Notes));
    }
    let col = (inner.col + inner.cols).saturating_sub(2);
    rows.into_iter().filter(|&(line, _)| line < inner.rows).map(|(line, what)| ((col, inner.row + line), what)).collect()
}

/// A copy icon, two overlapping squares, drawn in pixels into one cell (no
/// font has a copy symbol everywhere).
fn draw_copy_icon(f: &mut Frame, col: u16, row: u16, fg: Rgba, bg: Rgba, ctx: &DrawCtx) {
    let cell = widgets::px(CellRect::new(col, row, 1, 1), ctx);
    let line = (cell.w / 10.0).round().max(1.0);
    let side = (cell.w * 0.7).min(cell.h * 0.5).round();
    let step = (side * 0.3).round().max(2.0);
    let x = (cell.x + (cell.w - side - step) / 2.0).round();
    let y = (cell.y + (cell.h - side - step) / 2.0).round();
    let outline = |f: &mut Frame, x: f32, y: f32| {
        for r in [
            PxRect::new(x, y, side, line),
            PxRect::new(x, y + side - line, side, line),
            PxRect::new(x, y, line, side),
            PxRect::new(x + side - line, y, line, side),
        ] {
            f.add_rect(r, fg, Layer::BelowText);
        }
    };
    outline(f, x, y);
    // The front square hides the back one's lines under it.
    f.add_rect(PxRect::new(x + step, y + step, side, side), bg, Layer::BelowText);
    outline(f, x + step, y + step);
}

fn draw_details(app: &App, o: &Open, f: &mut Frame, r: CellRect, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let inner = BoxFrame::new("Entry").draw(f, r, ctx);
    let Some(row) = o.selected() else { return };
    let Some(entry) = o.vault.db.entry(row.id) else { return };
    let label_w = 10;
    // Values end before the copy icons' column (and a space).
    let value_w = inner.cols.saturating_sub(label_w + 4);
    let value = |f: &mut Frame, line: u16, label: &str, text: &str, style: Style| {
        let y = inner.row + line;
        put(f, inner.col + 1, y, label, Style::new(c.muted, c.surface), label_w);
        put_elided(f, inner.col + 1 + label_w, y, text, style, value_w);
    };
    let text = Style::new(c.foreground, c.surface);
    value(f, 0, "Username", entry.get_username().unwrap_or(""), text);
    let password = entry.get_password().unwrap_or("");
    if o.reveal_until.is_some() {
        value(f, 1, "Password", password, Style::new(c.warning, c.surface));
    } else {
        let shown = if password.is_empty() { String::new() } else { "••••••••••••".into() };
        value(f, 1, "Password", &shown, text);
    }
    value(f, 2, "URL", entry.get_url().unwrap_or(""), text);
    // The one-time code, hidden like the password, and its seconds left.
    let mut next = 3;
    if let Some(code) = crate::otp::code_now(&entry) {
        match code {
            Ok(code) => {
                let digits = if o.reveal_until.is_some() { code.grouped() } else { crate::otp::group(&"•".repeat(code.code.len())) };
                let style = if o.reveal_until.is_some() { Style::new(c.warning, c.surface) } else { text };
                value(f, 3, "OTP", &digits, style);
                let at = inner.col + 1 + label_w + width(&digits) + 2;
                let left = format!("{} s", code.left);
                put(f, at, inner.row + 3, &left, Style::new(c.muted, c.surface), (inner.col + 1 + label_w + value_w).saturating_sub(at));
            }
            Err(_) => value(f, 3, "OTP", "Can't be read · F4 sets it up again", Style::new(c.error, c.surface)),
        }
        next = 4;
    }
    // The notes on the rows left; "…" when there are more lines.
    let room = inner.rows.saturating_sub(next) as usize;
    let lines: Vec<&str> = entry.get(fields::NOTES).unwrap_or("").lines().collect();
    for (i, line) in lines.iter().take(room).enumerate() {
        let more = i + 1 == room && lines.len() > room;
        let text = if more { format!("{line} …") } else { line.to_string() };
        value(f, next + i as u16, if i == 0 { "Notes" } else { "" }, &text, Style::new(c.muted, c.surface));
    }
    for ((col, row), _) in copy_icons(app) {
        draw_copy_icon(f, col, row, c.muted, c.surface, ctx);
    }
}

fn draw_status(app: &App, o: &Open, f: &mut Frame, r: CellRect, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let now = Instant::now();
    let mut parts = Vec::new();
    if o.saving {
        parts.push("Saving…".to_string());
    } else if o.save_error.is_some() {
        parts.push("Not saved".to_string());
    }
    if let Some(t) = app.clipboard_until {
        parts.push(format!("Clipboard clears in {} s", t.saturating_duration_since(now).as_secs() + 1));
    }
    if let Some(t) = o.reveal_until {
        parts.push(format!("Password hides in {} s", t.saturating_duration_since(now).as_secs() + 1));
    }
    if let Some(t) = app.lock_at() {
        let left = t.saturating_duration_since(now).as_secs();
        // Only the last minute: earlier it's noise.
        if left < 60 {
            parts.push(format!("Locks in {} s", left + 1));
        }
    }
    let text = parts.join(" · ");
    let w = width(&text).min(r.cols.saturating_sub(2));
    // The shortcuts that fit left of the countdowns, the least needed dropped first.
    let mut keys = vec!["⌘C password", "⌘B username", "⌘T code", "⌘⇧L lock", "⌘O open another", "⌘X close"];
    let room = r.cols.saturating_sub(w + if w > 0 { 4 } else { 2 });
    let hint = loop {
        let hint = shortcut_label(&keys.join(" · "));
        if width(&hint) <= room || keys.len() <= 1 {
            break hint;
        }
        keys.pop();
    };
    put(f, r.col + 1, r.row, &hint, Style::new(c.muted, c.background), room);
    let fg = if o.save_error.is_some() && !o.saving { c.error } else { c.foreground };
    put(f, r.col + r.cols.saturating_sub(w + 1), r.row, &text, Style::new(fg, c.background), w);
}

/// The unlock (or create) dialog in the middle of the window.
fn draw_unlock(app: &App, u: &Unlock, f: &mut Frame, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let screen = CellRect::new(0, 0, app.cols, app.rows.saturating_sub(1));
    let creating = u.creating();
    let rows = if creating { 10 } else { 8 };
    let (outer, _) = form::dialog_rects(screen, 64, rows);
    let title = if creating { "Create vault" } else { "Unlock vault" };
    let r = form::dialog_box(f, outer, title, ctx);
    let line = |n: u16| CellRect::new(r.col, r.row + n, r.cols, 1);
    let field = |n: u16| CellRect::new(r.col, r.row + n, r.cols, 1);

    // Under the picker: no caret showing through it.
    let active = !u.busy && u.picker.is_none();
    form::label(f, line(0), "Vault file", ctx);
    u.path.draw_field_hint(f, field(1), ctx, u.focus == UnlockFocus::Path && active, "path to a .kdbx file");
    form::label(f, line(3), if creating { "New master password" } else { "Master password" }, ctx);
    u.password.draw(f, field(4), ctx, u.focus == UnlockFocus::Password && active, false);
    let mut next = 6;
    if creating {
        form::label(f, line(6), "Repeat master password", ctx);
        u.repeat.draw(f, field(7), ctx, u.focus == UnlockFocus::Repeat && active, false);
        next = 9;
    }
    let (text, style) = if u.busy {
        ("Opening…".to_string(), Style::new(c.muted, c.surface))
    } else if let Some(e) = &u.error {
        (e.clone(), Style::new(c.error, c.surface))
    } else if u.path.text.trim().is_empty() {
        ("F2 chooses a vault · F3 creates a new one".to_string(), Style::new(c.muted, c.surface))
    } else if creating {
        ("No file here yet · Enter creates a new vault".to_string(), Style::new(c.muted, c.surface))
    } else {
        ("Enter unlocks · Tab moves between fields".to_string(), Style::new(c.muted, c.surface))
    };
    put_elided(f, r.col, r.row + next, &text, style, r.cols);
    if let Some(p) = &u.picker {
        p.draw(f, app.screen_rect(), ctx);
    }
}

/// The toast: a box bottom right above the key bar, wrapped to fit.
fn draw_toast(app: &App, f: &mut Frame, l: &Layout, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let Some(t) = &app.toast else { return };
    let w = 60.min(l.footer.cols.saturating_sub(4));
    if w < 10 {
        return;
    }
    let lines = form::wrap(&t.text, (w - 4) as usize);
    let h = (lines.len() as u16 + 2).min(l.footer.row.saturating_sub(1));
    if h < 3 {
        return;
    }
    let r = CellRect::new(l.footer.cols.saturating_sub(w + 2), l.footer.row.saturating_sub(h + 1), w, h);
    form::cover(f, r, ctx);
    fill(f, r, c.footer_bg);
    let bar = if t.error { c.error } else { c.border_active };
    for row in r.row..r.row + r.rows {
        put(f, r.col, row, "▌", Style::new(bar, c.footer_bg), 1);
    }
    for (i, line) in lines.iter().enumerate().take((h - 2) as usize) {
        put(f, r.col + 2, r.row + 1 + i as u16, line, Style::new(c.foreground, c.footer_bg), w - 3);
    }
}
