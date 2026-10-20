use std::sync::Arc;

use orchard_kit::input::Mods;

use super::*;

/// An app that touches neither the real config nor the clipboard.
fn app() -> App {
    let mut app = App::new(Arc::new(|| {}), false);
    app.resized(100, 36, (8.0, 16.0));
    app
}

/// A vault on disk with fast key derivation.
fn vault_file(dir: &Path) -> PathBuf {
    let path = dir.join("v.kdbx");
    let mut v = Vault::create(&path, vault::key("pw"), vault::argon2id(1, 64 * 1024, 1));
    for (title, user) in [("Mail", "me"), ("Bank", "acct")] {
        let mut root = v.db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, user);
        e.set_protected(fields::PASSWORD, "s3cret");
    }
    v.save().unwrap();
    path
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.handle_key(&KeyEvent::char(ch));
    }
}

/// Run background results until `done` or a few seconds have passed.
fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done(app) && Instant::now() < until {
        app.process();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn unlock(app: &mut App, path: &Path, password: &str) {
    app.screen = Screen::Unlock(Unlock::new(Some(path)));
    type_text(app, password);
    app.handle_key(&KeyEvent::named(Named::Enter));
    wait(app, |a| !matches!(&a.screen, Screen::Unlock(u) if u.busy));
}

#[test]
fn unlocks_and_lists_entries_by_title() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    let Screen::Open(o) = &app.screen else { panic!("not unlocked") };
    let titles: Vec<&str> = o.rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Bank", "Mail"]);
    assert_eq!(app.settings.last_vault.as_deref(), Some(path.as_path()));
}

#[test]
fn a_wrong_password_says_so_and_wipes_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "nope");
    let Screen::Unlock(u) = &app.screen else { panic!("unlocked with a wrong password") };
    assert!(u.error.as_deref().unwrap().starts_with("Wrong master password"));
    assert!(u.password.is_empty());
}

#[test]
fn locks_after_the_idle_time_and_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.copy_field(fields::PASSWORD, "password");
    assert!(app.clipboard_until.is_some());

    let at = app.lock_at().unwrap();
    app.tick(at - Duration::from_secs(1));
    assert!(matches!(app.screen, Screen::Open(_)));
    app.tick(at);
    assert!(matches!(app.screen, Screen::Unlock(_)));
    assert_eq!(app.clipboard_until, None);

    unlock(&mut app, &path, "pw");
    app.handle_key(&lock_key());
    let Screen::Unlock(u) = &app.screen else { panic!("⌘⇧L didn't lock") };
    assert_eq!(u.path.text, path.display().to_string());
    assert_eq!(u.focus, UnlockFocus::Password);
}

#[test]
fn typing_filters_and_esc_clears() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    type_text(&mut app, "acc");
    let Screen::Open(o) = &app.screen else { panic!() };
    assert_eq!(o.selected().map(|r| r.title.as_str()), Some("Bank"));
    assert_eq!(o.view().len(), 1);
    app.handle_key(&KeyEvent::named(Named::Escape));
    let Screen::Open(o) = &app.screen else { panic!() };
    assert_eq!(o.view().len(), 2);
}

#[test]
fn a_tilde_in_the_path_means_the_home_folder() {
    let home = Path::new("/home/me");
    assert_eq!(expand_home("~/passwords.kdbx", Some(home)), home.join("passwords.kdbx"));
    assert_eq!(expand_home("~", Some(home)), home);
    assert_eq!(expand_home("~other/v.kdbx", Some(home)), PathBuf::from("~other/v.kdbx"));
    assert_eq!(expand_home("/srv/~/v.kdbx", Some(home)), PathBuf::from("/srv/~/v.kdbx"));
    assert_eq!(expand_home("~/v.kdbx", None), PathBuf::from("~/v.kdbx"));
}

#[test]
fn f2_picks_a_vault_file_to_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    let mut app = app();
    app.screen = Screen::Unlock(Unlock::new(Some(&dir.path().join("missing.kdbx"))));
    press(&mut app, Named::F(2));
    wait(&mut app, |a| matches!(&a.screen, Screen::Unlock(u) if u.picker.as_ref().is_some_and(|p| !p.loading())));
    let Screen::Unlock(u) = &app.screen else { panic!() };
    let names: Vec<_> = u.picker.as_ref().unwrap().entries().unwrap().iter().map(|e| file_name(&e.path)).collect();
    assert_eq!(names, ["v.kdbx"]);
    // Row 0 is "..".
    press(&mut app, Named::Down);
    press(&mut app, Named::Enter);
    let Screen::Unlock(u) = &app.screen else { panic!() };
    assert!(u.picker.is_none());
    assert_eq!(u.vault_path(), path);
    assert_eq!(u.focus, UnlockFocus::Password);
}

#[test]
fn f3_puts_a_new_vault_in_the_chosen_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app();
    app.screen = Screen::Unlock(Unlock::new(None));
    press(&mut app, Named::F(3));
    let Screen::Unlock(u) = &mut app.screen else { panic!() };
    assert!(u.picker.is_some());
    u.picked(picker::Outcome::Folder(dir.path().into()));
    assert!(u.picker.is_none());
    assert_eq!(u.vault_path(), dir.path().join("Passwords.kdbx"));
    assert!(u.creating());
    std::fs::write(dir.path().join("Passwords.kdbx"), "x").unwrap();
    assert_eq!(new_vault_path(dir.path()), dir.path().join("Passwords 2.kdbx"));
    // Esc leaves the path as it was.
    press(&mut app, Named::F(3));
    press(&mut app, Named::Escape);
    let Screen::Unlock(u) = &app.screen else { panic!() };
    assert!(u.picker.is_none());
    assert_eq!(u.vault_path(), dir.path().join("Passwords.kdbx"));
}

#[test]
fn a_new_vault_needs_the_password_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.kdbx");
    let mut app = app();
    app.screen = Screen::Unlock(Unlock::new(Some(&path)));
    type_text(&mut app, "one");
    app.handle_key(&KeyEvent::named(Named::Enter));
    let Screen::Unlock(u) = &app.screen else { panic!() };
    assert_eq!(u.focus, UnlockFocus::Repeat);
    type_text(&mut app, "two");
    app.handle_key(&KeyEvent::named(Named::Enter));
    let Screen::Unlock(u) = &app.screen else { panic!() };
    assert!(u.error.as_deref().unwrap().contains("don't match"));
    assert!(!path.exists());
}

fn open(app: &App) -> &Open {
    match &app.screen {
        Screen::Open(o) => o,
        Screen::Unlock(_) => panic!("locked"),
    }
}

fn wait_saved(app: &mut App) {
    wait(app, |a| !open(a).saving);
}

fn reopen(path: &Path) -> Vault {
    Vault::open(path, vault::key("pw")).unwrap()
}

#[test]
fn editing_an_entry_saves_it_with_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    // "Bank" is selected (first by title).
    app.handle_key(&KeyEvent::named(Named::F(4)));
    assert!(open(&app).editor.is_some());
    // Title field: replace its text, then go to the password and type more.
    for _ in 0..4 {
        app.handle_key(&KeyEvent::named(Named::Backspace));
    }
    type_text(&mut app, "Savings");
    app.handle_key(&KeyEvent::named(Named::Tab));
    app.handle_key(&KeyEvent::named(Named::Tab));
    type_text(&mut app, "!");
    app.handle_key(&KeyEvent::named(Named::Enter));
    assert!(open(&app).editor.is_none());
    assert_eq!(open(&app).selected().unwrap().title, "Savings");
    wait_saved(&mut app);
    assert_eq!(open(&app).save_error, None);

    let v = reopen(&path);
    let e = v.db.iter_all_entries().find(|e| e.get_title() == Some("Savings")).unwrap();
    assert_eq!(e.get_password(), Some("s3cret!"));
    assert_eq!(e.historical(0).unwrap().get_title(), Some("Bank"));
}

#[test]
fn a_new_entry_needs_a_title_and_lands_selected() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&KeyEvent::named(Named::F(7)));
    app.handle_key(&KeyEvent::named(Named::Enter));
    let ed = open(&app).editor.as_ref().expect("closed without a title");
    assert!(ed.error.is_some());
    type_text(&mut app, "Forum");
    app.handle_key(&KeyEvent::named(Named::Enter));
    assert_eq!(open(&app).selected().unwrap().title, "Forum");
    wait_saved(&mut app);
    assert_eq!(reopen(&path).db.num_entries(), 3);
}

#[test]
fn esc_cancels_without_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let before = std::fs::read(&path).unwrap();
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&KeyEvent::named(Named::F(4)));
    // Without changes, Esc just closes.
    app.handle_key(&KeyEvent::named(Named::Escape));
    assert!(open(&app).editor.is_none());
    app.handle_key(&KeyEvent::named(Named::F(4)));
    type_text(&mut app, "xyz");
    app.handle_key(&KeyEvent::named(Named::Escape));
    yes(&mut app);
    assert!(open(&app).editor.is_none());
    assert!(!open(&app).saving);
    assert_eq!(open(&app).rows[0].title, "Bank");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn delete_moves_to_the_recycle_bin_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&KeyEvent::named(Named::F(8)));
    assert_eq!(open(&app).rows.len(), 1);
    wait_saved(&mut app);
    let v = reopen(&path);
    assert_eq!(v.db.recycle_bin().unwrap().entries().count(), 1);
}

#[test]
fn locking_saves_what_is_pending() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&KeyEvent::named(Named::F(8)));
    // Lock at once, without waiting for the background save.
    app.handle_key(&lock_key());
    assert!(matches!(app.screen, Screen::Unlock(_)));
    assert_eq!(reopen(&path).db.recycle_bin().unwrap().entries().count(), 1);
}

#[test]
fn a_failed_save_keeps_the_vault_open_on_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    // A folder where the temporary file goes: every save fails.
    std::fs::create_dir(dir.path().join("v.kdbx.tmp")).unwrap();
    app.handle_key(&KeyEvent::named(Named::F(8)));
    wait_saved(&mut app);
    assert!(open(&app).save_error.is_some());
    app.handle_key(&lock_key());
    assert!(matches!(app.screen, Screen::Open(_)), "locked and lost the change");
    // Inactivity locks anyway.
    let at = app.lock_at().unwrap();
    app.tick(at);
    assert!(matches!(app.screen, Screen::Unlock(_)));
    assert!(app.toast.as_ref().unwrap().error);
}

#[test]
fn notes_take_several_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&KeyEvent::named(Named::F(4)));
    for _ in 0..5 {
        app.handle_key(&KeyEvent::named(Named::Tab));
    }
    // Enter in the notes starts a line; it doesn't save.
    type_text(&mut app, "PIN 1234");
    app.handle_key(&KeyEvent::named(Named::Enter));
    type_text(&mut app, "branch: Main St");
    assert!(open(&app).editor.is_some());
    // Down past the last line goes on to the group, where Enter saves.
    app.handle_key(&KeyEvent::named(Named::Down));
    app.handle_key(&KeyEvent::named(Named::Tab));
    app.handle_key(&KeyEvent::named(Named::Enter));
    assert!(open(&app).editor.is_none());
    wait_saved(&mut app);
    let v = reopen(&path);
    let e = v.db.iter_all_entries().find(|e| e.get_title() == Some("Bank")).unwrap();
    assert_eq!(e.get(fields::NOTES), Some("PIN 1234\nbranch: Main St"));
}

/// ⌘ / Ctrl and a letter.
/// ⌘⇧L / Ctrl+Shift+L.
fn lock_key() -> KeyEvent {
    let mut ev = primary('l');
    ev.mods.shift = true;
    ev
}

#[test]
fn f9_opens_settings_and_plain_l_shortcuts_dont_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&primary('l'));
    assert!(matches!(app.screen, Screen::Open(_)), "⌘L alone no longer locks");
    press(&mut app, Named::F(9));
    assert!(app.settings_dialog.is_some());
    assert!(matches!(app.screen, Screen::Open(_)));
    press(&mut app, Named::Escape);
    assert!(app.settings_dialog.is_none());
    // On the unlock screen too.
    app.handle_key(&lock_key());
    press(&mut app, Named::F(9));
    assert!(app.settings_dialog.is_some());
}

#[test]
fn cmd_x_closes_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.new_entry();
    app.handle_key(&primary('x'));
    assert!(matches!(app.screen, Screen::Open(_)), "in the editor ⌘X is the text field's");
    press(&mut app, Named::Escape);
    app.handle_key(&primary('x'));
    let Screen::Unlock(u) = &app.screen else { panic!("still open") };
    assert!(u.path.text.is_empty());
    assert_eq!(u.focus, UnlockFocus::Path);
    assert!(u.picker.is_none());
}

#[test]
fn cmd_o_locks_and_picks_another_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&primary('o'));
    let Screen::Unlock(u) = &app.screen else { panic!("still open") };
    assert!(u.picker.is_some());
    assert_eq!(u.vault_path(), path, "the path stays for Esc");
}

#[test]
fn f5_shows_only_the_groups_checked() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    // A "Work" group with an entry of its own.
    let Screen::Open(o) = &mut app.screen else { panic!() };
    let work = {
        let mut root = o.vault.db.root_mut();
        let mut g = root.add_group();
        g.name = "Work".into();
        g.id()
    };
    o.vault.db.group_mut(work).unwrap().add_entry().set_unprotected(fields::TITLE, "VPN");
    o.rows = entries::rows(&o.vault.db);
    assert_eq!(open(&app).view().len(), 3);

    press(&mut app, Named::F(5));
    let g = open(&app).group_filter.as_ref().expect("open");
    let names: Vec<&str> = g.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names[1..], ["Work"]);
    // The root unchecks all; then Work alone.
    app.handle_key(&KeyEvent::char(' '));
    press(&mut app, Named::Down);
    app.handle_key(&KeyEvent::char(' '));
    press(&mut app, Named::Enter);
    let o = open(&app);
    assert!(o.group_filter.is_none());
    let titles: Vec<&str> = o.view().iter().map(|&i| o.rows[i].title.as_str()).collect();
    assert_eq!(titles, ["VPN"]);
    // Typing searches within it.
    type_text(&mut app, "mail");
    assert!(open(&app).view().is_empty());
    press(&mut app, Named::Escape);
    // Esc in the dialog keeps the filter; all checked again removes it.
    press(&mut app, Named::F(5));
    press(&mut app, Named::Escape);
    assert!(open(&app).shown_groups.is_some());
    press(&mut app, Named::F(5));
    app.handle_key(&KeyEvent::char(' '));
    press(&mut app, Named::Enter);
    assert!(open(&app).shown_groups.is_none());
    assert_eq!(open(&app).view().len(), 3);
    // F6 no longer copies the username.
    press(&mut app, Named::F(6));
    assert!(app.toast.is_none());
}

fn primary(c: char) -> KeyEvent {
    let mods = if cfg!(target_os = "macos") { Mods { logo: true, ..Mods::default() } } else { Mods { ctrl: true, ..Mods::default() } };
    KeyEvent { text: None, ..KeyEvent::char(c).with(mods) }
}

fn press(app: &mut App, key: Named) {
    app.handle_key(&KeyEvent::named(key));
}

fn confirm(app: &App) -> Option<Ask> {
    open(app).confirm.as_ref().map(|c| c.action)
}

/// Yes to the open question (the buttons start on the safe one).
fn yes(app: &mut App) {
    press(app, Named::Left);
    press(app, Named::Enter);
}

#[test]
fn esc_with_changes_asks_before_discarding() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(4));
    type_text(&mut app, "x");
    press(&mut app, Named::Escape);
    assert_eq!(confirm(&app), Some(Ask::Discard));
    // Esc (or Enter on "Keep editing") goes back to the editor, edit and all.
    press(&mut app, Named::Escape);
    assert_eq!(open(&app).editor.as_ref().unwrap().title.text, "Bankx");
    press(&mut app, Named::Escape);
    yes(&mut app);
    assert!(open(&app).editor.is_none() && confirm(&app).is_none());
    assert!(!open(&app).saving);
}

#[test]
fn undo_puts_a_deleted_entry_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(8));
    assert_eq!(open(&app).rows.len(), 1);
    app.handle_key(&primary('z'));
    assert_eq!(open(&app).rows.len(), 2);
    assert_eq!(open(&app).selected().unwrap().title, "Bank");
    wait_saved(&mut app);
    let v = reopen(&path);
    assert_eq!(v.db.recycle_bin().unwrap().entries().count(), 0);
}

#[test]
fn the_recycle_bin_restores_deletes_and_empties() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(8));
    press(&mut app, Named::F(8));
    press(&mut app, Named::F(2));
    assert!(open(&app).bin);
    assert_eq!(open(&app).rows.len(), 2);
    // F7 restores the selected one ("Bank").
    press(&mut app, Named::F(7));
    assert_eq!(open(&app).rows.len(), 1);
    // F8 deletes for good, after asking.
    press(&mut app, Named::F(8));
    assert!(matches!(confirm(&app), Some(Ask::DeleteForGood(_))));
    yes(&mut app);
    assert!(open(&app).rows.is_empty());
    press(&mut app, Named::F(4));
    assert_eq!(confirm(&app), None, "nothing to empty");
    press(&mut app, Named::F(2));
    let titles: Vec<&str> = open(&app).rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Bank"]);
    wait_saved(&mut app);
    let v = reopen(&path);
    assert_eq!(v.db.num_entries(), 1);
}

#[test]
fn emptying_the_bin_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(8));
    press(&mut app, Named::F(2));
    press(&mut app, Named::F(4));
    assert_eq!(confirm(&app), Some(Ask::EmptyBin));
    press(&mut app, Named::Enter);
    assert_eq!(open(&app).rows.len(), 1, "Enter on Cancel keeps it");
    press(&mut app, Named::F(4));
    yes(&mut app);
    assert!(open(&app).rows.is_empty());
    wait_saved(&mut app);
    assert_eq!(reopen(&path).db.num_entries(), 1);
}

#[test]
fn locking_keeps_what_the_editor_holds() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(4));
    type_text(&mut app, " 2");
    press(&mut app, Named::Escape);
    assert_eq!(confirm(&app), Some(Ask::Discard));
    let at = app.lock_at().unwrap();
    app.tick(at);
    assert!(matches!(app.screen, Screen::Unlock(_)));
    assert!(app.toast.as_ref().unwrap().text.contains("saved first"));
    let v = reopen(&path);
    let e = v.db.iter_all_entries().find(|e| e.get_title() == Some("Bank 2")).unwrap();
    assert_eq!(e.historical(0).unwrap().get_title(), Some("Bank"));
}

#[test]
fn an_old_format_vault_updates_before_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.kdbx");
    let mut v = Vault::old(&path, keepass::config::DatabaseVersion::KDB3(1));
    v.db.root_mut().add_entry().set_unprotected(fields::TITLE, "Mail");
    let mut app = app();
    app.show_vault(v);
    press(&mut app, Named::F(8));
    assert_eq!(confirm(&app), Some(Ask::UpdateFormat(Then::Delete)));
    press(&mut app, Named::Escape);
    assert_eq!(open(&app).rows.len(), 1, "no means no change");
    press(&mut app, Named::F(8));
    yes(&mut app);
    // Updated, then the delete went ahead.
    assert!(open(&app).vault.writable());
    assert!(open(&app).rows.is_empty());
    wait_saved(&mut app);
    assert_eq!(open(&app).save_error, None);
    assert_eq!(reopen(&path).db.recycle_bin().unwrap().entries().count(), 1);
}

#[test]
fn the_generator_fills_the_password_and_remembers_its_choices() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    press(&mut app, Named::F(7));
    type_text(&mut app, "Forum");
    press(&mut app, Named::F(2));
    assert!(open(&app).editor.as_ref().unwrap().generator.is_some());
    // From "Use password" up to the length, five longer, then Enter uses it.
    for _ in 0..6 {
        press(&mut app, Named::Up);
    }
    for _ in 0..5 {
        press(&mut app, Named::Right);
    }
    press(&mut app, Named::Enter);
    let ed = open(&app).editor.as_ref().unwrap();
    assert!(ed.generator.is_none());
    assert_eq!(ed.password.text().chars().count(), 25);
    assert_eq!(app.settings.generator.length, 25);
    press(&mut app, Named::Enter);
    wait_saved(&mut app);
    let v = reopen(&path);
    let e = v.db.iter_all_entries().find(|e| e.get_title() == Some("Forum")).unwrap();
    assert_eq!(e.get_password().unwrap().chars().count(), 25);
}

/// Into the editor's OTP field (Tab from the title, past three fields).
fn focus_otp(app: &mut App) {
    for _ in 0..4 {
        press(app, Named::Tab);
    }
    let ed = open(app).editor.as_ref().unwrap();
    assert_eq!(ed.focus, Some(editor::Field::Otp));
}

#[test]
fn a_pasted_otp_secret_is_saved_as_a_link_and_copies_codes() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    app.handle_key(&primary('t'));
    assert!(app.toast.as_ref().unwrap().text.contains("no one-time password"));

    app.edit_selected();
    focus_otp(&mut app);
    type_text(&mut app, "not a secret!");
    press(&mut app, Named::Enter);
    let ed = open(&app).editor.as_ref().expect("still open");
    assert!(ed.error.as_deref().unwrap().contains("secret isn't valid"));
    assert_eq!(ed.focus, Some(editor::Field::Otp));

    let Screen::Open(o) = &mut app.screen else { panic!() };
    o.editor.as_mut().unwrap().otp.set("gezd gnbv gy3t qojq gezd gnbv gy3t qojq");
    press(&mut app, Named::Enter);
    assert!(open(&app).editor.is_none());
    wait_saved(&mut app);
    let v = reopen(&path);
    let title = open(&app).selected().unwrap().title.clone();
    let e = v.db.iter_all_entries().find(|e| e.get_title() == Some(&title)).unwrap();
    let link = e.get(fields::OTP).unwrap();
    assert!(link.starts_with(&format!("otpauth://totp/{title}?secret=GEZDGNBV")), "{link}");
    assert_eq!(e.historical(0).and_then(|h| h.get(fields::OTP).map(str::to_owned)), None, "the old version had none");

    app.handle_key(&primary('t'));
    assert!(app.toast.as_ref().unwrap().text.starts_with("One-time code copied"));
    assert!(app.clipboard_until.is_some());
}

#[test]
fn clearing_the_otp_removes_keepass_fields_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    let id = open(&app).selected().unwrap().id;
    let Screen::Open(o) = &mut app.screen else { panic!() };
    let mut e = o.vault.db.entry_mut(id).unwrap();
    e.set_protected("TimeOtp-Secret-Base32", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    e.set_unprotected("TimeOtp-Period", "30");
    app.edit_selected();
    let ed = open(&app).editor.as_ref().unwrap();
    assert!(ed.otp.text().starts_with("otpauth://totp/"), "read from KeePass's fields");
    let Screen::Open(o) = &mut app.screen else { panic!() };
    o.editor.as_mut().unwrap().otp.clear();
    press(&mut app, Named::Enter);
    wait_saved(&mut app);
    let v = reopen(&path);
    let e = v.db.entry(id).unwrap();
    assert!(otp::stored(&e).is_none());
    assert!(e.get("TimeOtp-Period").is_none());
}

#[test]
fn copy_icons_line_up_on_fields_with_a_value_and_copy_on_click() {
    use crate::ui::{CopyWhat, copy_icons};
    let dir = tempfile::tempdir().unwrap();
    let path = vault_file(dir.path());
    let mut app = app();
    unlock(&mut app, &path, "pw");
    // "Bank": username and password, no URL or notes.
    let icons = copy_icons(&app);
    assert_eq!(icons.iter().map(|&(_, w)| w).collect::<Vec<_>>(), [CopyWhat::Username, CopyWhat::Password]);
    assert!(icons.iter().all(|&((col, _), _)| col == icons[0].0.0), "one column");
    let Screen::Open(o) = &mut app.screen else { unreachable!() };
    let id = o.selected().unwrap().id;
    let mut e = o.vault.db.entry_mut(id).unwrap();
    e.set_unprotected(fields::USERNAME, "");
    e.set_unprotected(fields::NOTES, "Line one\nLine two");
    let icons = copy_icons(&app);
    assert_eq!(icons.iter().map(|&(_, w)| w).collect::<Vec<_>>(), [CopyWhat::Password, CopyWhat::Notes]);
    let ((col, row), _) = icons[1];
    app.handle_mouse(&MouseEvent { kind: MouseKind::Down, col, row, x: 0.0, y: 0.0, double: false });
    assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Notes copied"));
    assert!(app.clipboard_until.is_some());
}
