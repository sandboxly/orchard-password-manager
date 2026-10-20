//! The sample on the Appearance page: a tiny vault in the current colors,
//! frame style and font (the dialog covers much of the window behind it,
//! which previews the rest).

use orchard_grid::{CellRect, Flags, Frame, Style};
use orchard_kit::widgets::{BoxFrame, DrawCtx, bottom, fill, put, put_elided, right, width};

/// (title, username)
const ENTRIES: [(&str, &str); 3] = [("Bank", "alexmorgan"), ("Mail", "alex@example.org"), ("Wiki", "amorgan")];
const CURSOR: usize = 1;
const KEYS: [(&str, &str); 2] = [("F4", "Edit"), ("F9", "Settings")];

/// The narrowest it is drawn.
pub const MIN_WIDTH: u16 = 22;
pub const MAX_WIDTH: u16 = 30;
/// Two boxes of 3 inner rows, the key bar.
pub const HEIGHT: u16 = 5 + 5 + 1;

/// Draw the sample into `r` (at most [`HEIGHT`] rows).
pub fn draw(f: &mut Frame, r: CellRect, ctx: &DrawCtx) {
    let c = &ctx.theme.colors;
    let r = CellRect::new(r.col, r.row, r.cols.min(MAX_WIDTH), r.rows.min(HEIGHT));
    if r.rows < HEIGHT || r.cols < MIN_WIDTH {
        return;
    }
    let list = CellRect::new(r.col, r.row, r.cols, 5);
    let inner = BoxFrame { title: "Vault.kdbx", subtitle: "", active: true }.draw(f, list, ctx);
    let user_x = inner.col + 1 + inner.cols / 2;
    for (i, (title, user)) in ENTRIES.iter().enumerate() {
        let row = CellRect::new(inner.col, inner.row + i as u16, inner.cols, 1);
        let selected = i == CURSOR;
        let bg = if selected { c.cursor_bg } else { c.surface };
        fill(f, row, bg);
        let style = Style::new(if selected { c.cursor_fg } else { c.foreground }, bg);
        put_elided(f, row.col + 1, row.row, title, style, user_x.saturating_sub(row.col + 2));
        let muted = Style::new(if selected { c.cursor_fg } else { c.muted }, bg);
        put_elided(f, user_x, row.row, user, muted, right(row).saturating_sub(user_x + 1));
    }

    let details = CellRect::new(r.col, bottom(list), r.cols, 5);
    let inner = BoxFrame { title: "Entry", subtitle: "", active: false }.draw(f, details, ctx);
    let label = Style::new(c.muted, c.surface);
    let x = inner.col + 1;
    let (vx, vw) = (x + 9, right(inner).saturating_sub(x + 10));
    put(f, x, inner.row, "Username", label, 8);
    put_elided(f, vx, inner.row, ENTRIES[CURSOR].1, Style::new(c.foreground, c.surface), vw);
    put(f, x, inner.row + 1, "Password", label, 8);
    put(f, vx, inner.row + 1, "••••••••", Style::new(c.foreground, c.surface), vw);
    put(f, x, inner.row + 2, "OTP", label, 8);
    put(f, vx, inner.row + 2, "381 027", Style::new(c.warning, c.surface), vw);

    let bar = CellRect::new(r.col, bottom(details), r.cols, 1);
    fill(f, bar, c.footer_bg);
    let mut x = bar.col + 1;
    for (key, label) in KEYS {
        if x + width(key) + 1 + width(label) > right(bar) {
            break;
        }
        x = put(f, x, bar.row, key, Style::new(c.footer_key, c.footer_bg).with(Flags::BOLD), width(key)) + 1;
        x = put(f, x, bar.row, label, Style::new(c.footer_fg, c.footer_bg), width(label)) + 2;
    }
}

#[cfg(test)]
mod tests {
    use orchard_theme::Theme;

    use super::*;

    #[test]
    fn sample_shows_entries_cursor_and_keys() {
        let theme = Theme::default_dark();
        let ctx = DrawCtx { theme: &theme, cell_px: (8.0, 16.0) };
        let mut f = Frame::new(MAX_WIDTH, HEIGHT, theme.colors.background);
        draw(&mut f, CellRect::new(0, 0, MAX_WIDTH, HEIGHT), &ctx);
        assert!(f.row_text(0).contains("Vault.kdbx"));
        assert!(f.row_text(2).contains("Mail"));
        assert_eq!(f.cell(2, 2).unwrap().bg, theme.colors.cursor_bg, "Mail has the cursor");
        assert!(f.row_text(8).contains("381 027"));
        assert!(f.row_text(HEIGHT - 1).starts_with(" F4 Edit  F9 Settings"));
    }
}
