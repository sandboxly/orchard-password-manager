//! `--screenshot OUT.png|OUT.webp`: a state drawn headlessly (docs, and
//! checking the look without a window): a demo vault (made up, in memory,
//! never written), or the unlock screen.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use orchard_grid::Frame;
use orchard_kit::input::KeyEvent;
use orchard_password_manager::App;
use orchard_password_manager::app::{Screen, Unlock, UnlockFocus};
use orchard_password_manager::ui;
use orchard_password_manager::ui::settings;
use orchard_password_manager::vault::{self, Vault, fields};
use orchard_shell::shot;

/// The options only this app's screenshots have (the common ones are
/// `orchard_shell::shot`'s).
#[derive(Default)]
struct Options {
    cursor: usize,
    reveal: bool,
    search: Option<String>,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut o = Options::default();
    let shot = match shot::parse(args, (110, 36), |a, it| match a {
        "--cursor" => o.cursor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0),
        "--reveal" => o.reveal = true,
        "--search" => o.search = it.next().cloned(),
        _ => {}
    }) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut app = App::new(Arc::new(|| {}), false);
    if let Some(t) = &shot.theme {
        app.settings.color_scheme = t.clone();
        app.apply_settings();
    }
    let mut r = match shot::renderer(app.theme.font.clone()) {
        Ok(r) => r,
        Err(code) => return code,
    };
    let (cols, rows) = shot.size;
    app.resized(cols, rows, shot::cell_px(&mut r));

    match shot.dialog.as_deref() {
        Some("unlock") | Some("error") => {
            let mut u = Unlock::new(Some(std::path::Path::new(&demo_path())));
            for ch in "correct horse".chars() {
                u.password.handle(&KeyEvent::char(ch));
            }
            if shot.dialog.as_deref() == Some("error") {
                u.password.clear();
                u.error = Some("Wrong master password. Check it and try again".into());
            }
            app.screen = Screen::Unlock(u);
        }
        Some(d @ ("open" | "folder")) => {
            let mut u = Unlock::new(Some(std::path::Path::new(&demo_path())));
            u.choose(d == "folder", false, app.tasks.waker());
            app.screen = Screen::Unlock(u);
            // Until the listing and places are in (a few seconds at most).
            let until = Instant::now() + Duration::from_secs(5);
            while matches!(&app.screen, Screen::Unlock(u) if u.picker.as_ref().is_some_and(|p| p.loading())) && Instant::now() < until {
                app.process();
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Some("create") => {
            let mut u = Unlock::new(Some(std::path::Path::new("/nonexistent/orchard-demo/new-vault.kdbx")));
            for ch in "a long new passphrase".chars() {
                u.password.handle(&KeyEvent::char(ch));
            }
            u.focus = UnlockFocus::Repeat;
            app.screen = Screen::Unlock(u);
        }
        _ => {
            app.show_vault(demo());
            if let Screen::Open(open) = &mut app.screen {
                if let Some(s) = &o.search {
                    open.search.set_text(s);
                }
                open.cursor = o.cursor;
                if o.reveal {
                    open.reveal_until = Some(Instant::now() + Duration::from_secs(10));
                }
            }
            app.scroll_to_cursor();
            match shot.dialog.as_deref() {
                Some(d @ ("settings" | "security" | "scheme-editor" | "color-picker")) => open_settings(&mut app, d),
                Some("edit") => app.edit_selected(),
                Some("filter") => {
                    // Email and Shopping unchecked, the cursor on Social.
                    use orchard_kit::input::Named;
                    app.open_group_filter();
                    for key in [Named::Down, Named::Insert, Named::Down, Named::Down, Named::Insert] {
                        app.handle_key(&KeyEvent::named(key));
                    }
                }
                Some("new") => app.new_entry(),
                Some("toast") => app.toast("The saved copy didn't read back the same, so the vault file was left as it was. log.txt in the settings folder says what differed", true),
                Some("groups") => {
                    app.edit_selected();
                    // Focus the group box and open its list.
                    for _ in 0..5 {
                        app.handle_key(&KeyEvent::named(orchard_kit::input::Named::Tab));
                    }
                    app.handle_key(&KeyEvent::named(orchard_kit::input::Named::Enter));
                }
                Some("generate") => {
                    app.edit_selected();
                    app.handle_key(&KeyEvent::named(orchard_kit::input::Named::F(2)));
                }
                Some("discard") => {
                    app.edit_selected();
                    app.handle_key(&KeyEvent::char('x'));
                    app.handle_key(&KeyEvent::named(orchard_kit::input::Named::Escape));
                }
                Some(d @ ("bin" | "empty")) => {
                    // Straight into the database: nothing is saved here.
                    if let Screen::Open(open) = &mut app.screen {
                        for title in ["Forum", "Grocery delivery", "VPN"] {
                            let id = open.rows.iter().find(|r| r.title == title).map(|r| r.id);
                            if let Some(id) = id {
                                orchard_password_manager::entries::delete(&mut open.vault.db, id);
                            }
                        }
                    }
                    app.toggle_bin();
                    if d == "empty" {
                        app.ask_empty_bin();
                    }
                }
                Some("update") => {
                    if let Screen::Open(open) = &mut app.screen {
                        open.vault.db.config.version = keepass::config::DatabaseVersion::KDB3(1);
                    }
                    app.edit_selected();
                }
                _ => {}
            }
            if o.reveal
                && let Screen::Open(open) = &mut app.screen
                && let Some(ed) = &mut open.editor
            {
                ed.reveal = true;
            }
        }
    }

    let mut frame = Frame::new(cols, rows, app.theme.colors.background);
    ui::draw(&app, &mut frame);
    shot::finish(&mut r, &frame, &shot.out)
}

/// Settings over the demo vault: the Appearance page with the scheme list
/// focused, the Security page, or the scheme editor (with its color picker).
fn open_settings(app: &mut App, which: &str) {
    settings::load_fonts_now(app);
    app.open_settings();
    let Some(mut d) = app.settings_dialog.take() else { return };
    match which {
        "security" => {
            d.page = 1;
            d.focus = settings::Focus::Ctl("lock_after");
        }
        "settings" => d.focus = settings::Focus::Ctl("scheme"),
        _ => {
            let mut editor = settings::scheme_editor::SchemeEditor::open(app, settings::Slot::Color, false);
            if which == "color-picker" {
                // "Headers and titles".
                editor.open_picker(4);
            }
            d.editor = Some(editor);
        }
    }
    app.settings_dialog = Some(d);
}

/// As typed (the unlock screen expands "~"): screenshots never show the
/// home folder of whoever took them.
fn demo_path() -> String {
    "~/Documents/Passwords.kdbx".into()
}

/// (group, title, username, password, URL)
const DEMO: &[(&str, &str, &str, &str, &str)] = &[
    ("Email", "Personal mail", "alex@example.org", "tulip-Harbor-93-quiet", "https://mail.example.org"),
    ("Email", "Work mail", "alex.morgan@example.com", "Q7#vLm2!pZr9", "https://webmail.example.com"),
    ("Finance", "Bank", "alexmorgan", "r1ver-Stone-candle-8", "https://bank.example.net"),
    ("Finance", "Credit card portal", "alex.m", "u&3Ks!92xLq@", "https://cards.example.net"),
    ("Finance", "Tax office", "AM-4471-2290", "Orchard-lantern-61", "https://tax.example.gov"),
    ("Shopping", "Bookshop", "alex@example.org", "page-Turner-2026", "https://books.example.shop"),
    ("Shopping", "Grocery delivery", "alex@example.org", "basil-Tomato-17", "https://fresh.example.shop"),
    ("Social", "Photo sharing", "alex_in_orchards", "8mF!q2Lz#Wv4", "https://photos.example.social"),
    ("Social", "Forum", "orchardfan", "maple-Kettle-55-ink", "https://forum.example.org"),
    ("Work", "Code hosting", "amorgan", "gH4$kP9!eR2w", "https://code.example.dev"),
    ("Work", "VPN", "amorgan", "copper-Fern-lamp-30", "vpn.example.com"),
    ("Work", "Wiki", "amorgan", "Sunny-meadow-404", "https://wiki.example.com"),
    ("Home", "Wi-Fi", "", "apple-Blossom-garden-22", ""),
    ("Home", "Router admin", "admin", "Zx9!fK2#mQ7p", "http://192.168.1.1"),
    ("Home", "Streaming", "alex@example.org", "popcorn-Velvet-88", "https://watch.example.tv"),
];

fn demo() -> Vault {
    let mut v = Vault::create(std::path::Path::new(&demo_path()), vault::key("demo"), vault::argon2id(1, 64 * 1024, 1));
    for &(group, title, user, password, url) in DEMO {
        let found = v.db.root().groups().find(|g| g.name == group).map(|g| g.id());
        let gid = found.unwrap_or_else(|| {
            let mut root = v.db.root_mut();
            let mut g = root.add_group();
            g.name = group.into();
            g.id()
        });
        let mut g = v.db.group_mut(gid).expect("just added");
        let mut e = g.add_entry();
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, user);
        e.set_protected(fields::PASSWORD, password);
        e.set_unprotected(fields::URL, url);
        if title == "Code hosting" {
            // A made-up secret: the code changes every 30 seconds.
            e.set_protected(fields::OTP, "otpauth://totp/Code%20hosting:amorgan?secret=JBSWY3DPEHPK3PXP&issuer=Code%20hosting");
        }
        if title == "Bank" {
            e.set_unprotected(fields::NOTES, "Customer number 4471-2290\nPhone banking PIN is in the safe\nBranch: 12 Orchard Road");
        }
    }
    v
}
