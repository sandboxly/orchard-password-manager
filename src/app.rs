//! The app's state and logic: the unlock screen or the open vault, keys,
//! timers. Drawing is `ui`'s, the window `main.rs`'s.
//!
//! Secrets have short lives here: the open [`Vault`] is the only place the
//! decrypted database and its key exist, and locking drops it (wiping
//! them). The app locks after a while without input and when it quits; a
//! copied password goes off the clipboard after a while, on lock and on
//! quit (`clipboard`).
//!
//! Changes (the entry editor, delete, the recycle bin) go into the open
//! vault and are saved right away on a worker (`Vault::snapshot`); locking
//! and quitting keep what is in the editor and finish a pending save first.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use keepass::db::{EntryId, GroupId};
use orchard_kit::input::{Key, KeyEvent, MouseEvent, MouseKind, Named, primary_label};
use orchard_kit::picker::{self, Picker};
use orchard_kit::tasks::{Tasks, Waker};
use orchard_kit::widgets::TextField;
use orchard_theme::{ColorScheme, Theme, schemes};
use zeroize::Zeroizing;

use crate::clipboard::{Board, SecretClipboard};
use crate::entries::{self, Row};
use crate::otp;
use crate::secret::SecretField;
use crate::settings::{self, Settings};
use crate::ui::confirm::{Answer, Confirm};
use crate::ui::editor::{self, Editor};
use crate::ui::groups::{self, GroupFilter};
use crate::ui::settings::{FontInfo, SettingsDialog};
use crate::vault::{self, OldFormat, OpenError, SaveError, Vault, fields};

pub enum Screen {
    Unlock(Unlock),
    Open(Box<Open>),
}

/// The unlock (or create) screen.
pub struct Unlock {
    pub path: TextField,
    pub password: SecretField,
    /// For a new vault: the password again.
    pub repeat: SecretField,
    pub focus: UnlockFocus,
    /// Opening (the key derivation takes a moment).
    pub busy: bool,
    pub error: Option<String>,
    /// The file picker over the dialog: F2 a vault to open, F3 a folder
    /// for a new one.
    pub picker: Option<Box<Picker>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockFocus {
    Path,
    Password,
    Repeat,
}

pub struct Open {
    pub vault: Vault,
    /// All entries, sorted by title.
    pub rows: Vec<Row>,
    pub search: TextField,
    /// Into `view()`.
    pub cursor: usize,
    pub top: usize,
    /// The selected entry's password shows until then.
    pub reveal_until: Option<Instant>,
    /// The entry editor, over the list.
    pub editor: Option<Box<Editor>>,
    /// A save is running on a worker.
    pub saving: bool,
    /// Why the last save failed (cleared by one that works).
    pub save_error: Option<String>,
    /// The list shows the recycle bin's entries instead of the vault's.
    pub bin: bool,
    /// A question over everything else.
    pub confirm: Option<Box<Confirm<Ask>>>,
    /// The last entry moved to the recycle bin: ⌘Z puts it back.
    pub undo: Option<EntryId>,
    /// The groups whose entries the list shows (F5); `None`: every group's.
    /// Kept in memory only, until the vault locks.
    pub shown_groups: Option<HashSet<GroupId>>,
    /// The group filter dialog, over the list.
    pub group_filter: Option<Box<GroupFilter>>,
}

/// What a yes to the open question does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Close the editor without saving.
    Discard,
    DeleteForGood(EntryId),
    EmptyBin,
    /// Update an old-format vault, then go on with what needed it.
    UpdateFormat(Then),
}

/// A change that waited for the vault's format update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Edit,
    New,
    Delete,
    Restore,
    EmptyBin,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct App {
    pub settings: Settings,
    pub theme: Theme,
    pub system_dark: bool,
    pub screen: Screen,
    pub toast: Option<Toast>,
    pub tasks: Tasks<App>,
    pub clipboard: SecretClipboard,
    /// When a copied secret comes off the clipboard (for the status line).
    pub clipboard_until: Option<Instant>,
    pub last_input: Instant,
    pub cols: u16,
    pub rows: u16,
    pub cell_px: (f32, f32),
    pub quit: bool,
    /// Settings (F9, ⌘,), over whatever screen is showing.
    pub settings_dialog: Option<Box<SettingsDialog>>,
    /// The color schemes as last read (built-ins and the shared folder's).
    pub schemes: Vec<ColorScheme>,
    /// The color-schemes folder Orchard Commander and the other apps share.
    pub schemes_dir: PathBuf,
    /// The installed fonts, for Settings (looked up when it first opens).
    pub fonts: FontInfo,
    config_dir: PathBuf,
    /// Read and write the settings (not in tests and screenshots).
    persist: bool,
}

impl App {
    /// `persist`: read and write the settings and use the real clipboard
    /// (tests and screenshots pass `false`).
    pub fn new(wake: Waker, persist: bool) -> App {
        let config_dir = settings::config_dir();
        let settings = if persist { Settings::load(&config_dir) } else { Settings::default() };
        let schemes_dir = orchard_theme::dirs::schemes_dir();
        let schemes = schemes::all_schemes(&schemes_dir);
        let theme = Theme::new(&settings.appearance(), scheme_in(&schemes, &settings.color_scheme));
        let clipboard = if persist { SecretClipboard::system() } else { SecretClipboard::with(NoClipboard) };
        let unlock = Unlock::new(settings.last_vault.as_deref());
        let mut app = App {
            settings,
            theme,
            system_dark: true,
            screen: Screen::Unlock(unlock),
            toast: None,
            tasks: Tasks::new(2, wake),
            clipboard,
            clipboard_until: None,
            last_input: Instant::now(),
            cols: 100,
            rows: 36,
            cell_px: (8.0, 16.0),
            quit: false,
            settings_dialog: None,
            schemes,
            schemes_dir,
            fonts: FontInfo::default(),
            config_dir,
            persist,
        };
        app.apply_settings();
        app
    }

    /// Read and write the settings in `dir` (tests).
    #[cfg(test)]
    pub(crate) fn persist_in(&mut self, dir: &Path) {
        self.config_dir = dir.to_path_buf();
        self.persist = true;
    }

    pub fn save_settings(&mut self) {
        if self.persist
            && let Err(e) = self.settings.save(&self.config_dir)
        {
            log::warn!("saving settings: {e}");
        }
    }

    /// Quitting: secrets off the clipboard, the vault locked.
    pub fn shutdown(&mut self) {
        self.lock(true);
        self.save_settings();
    }

    /// Change settings: the app follows at once and they are saved.
    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.apply_settings();
        self.save_settings();
    }

    /// Look as the settings say; also ends a scheme preview.
    pub fn apply_settings(&mut self) {
        let a = self.settings.appearance();
        self.theme = Theme::new(&a, self.scheme(a.scheme_name(self.system_dark)));
    }

    /// The color scheme with this name (as last read), or the default one.
    pub fn scheme(&self, name: &str) -> ColorScheme {
        scheme_in(&self.schemes, name)
    }

    /// Read the color-schemes folder again (another Orchard app may have saved one).
    pub fn refresh_schemes(&mut self) {
        self.schemes = schemes::all_schemes(&self.schemes_dir);
    }

    /// Show `scheme` on the whole window without choosing it (the scheme editor's preview).
    pub fn preview_scheme(&mut self, scheme: ColorScheme) {
        self.theme = Theme::new(&self.settings.appearance(), scheme);
    }

    /// The system switched between light and dark mode.
    pub fn set_system_dark(&mut self, dark: bool) {
        if self.system_dark != dark {
            self.system_dark = dark;
            let editing = self.settings_dialog.as_ref().is_some_and(|d| d.editor.is_some());
            if self.settings.follow_system_appearance && !editing {
                self.apply_settings();
            }
        }
    }

    /// F9 / ⌘,: Settings.
    pub fn open_settings(&mut self) {
        crate::ui::settings::open(self);
    }

    /// ⌘+ / ⌘− / ⌘0: a bigger or smaller font, or the default size.
    pub fn zoom(&mut self, step: i64) {
        let (lo, hi) = schemes::FONT_SIZE_RANGE;
        let size = if step == 0 { settings::DEFAULT_FONT_SIZE } else { (self.settings.font_size + step).clamp(lo, hi) };
        self.update_settings(|s| s.font_size = size);
    }

    pub fn resized(&mut self, cols: u16, rows: u16, cell_px: (f32, f32)) {
        self.cols = cols;
        self.rows = rows;
        self.cell_px = cell_px;
        self.scroll_to_cursor();
    }

    pub fn process(&mut self) -> bool {
        let mut changed = false;
        if let Screen::Unlock(u) = &mut self.screen {
            changed |= u.picker.as_mut().is_some_and(|p| p.poll());
        }
        for done in self.tasks.take() {
            done(self);
            changed = true;
        }
        changed
    }

    pub fn tick(&mut self, now: Instant) {
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
        }
        if self.clipboard_until.is_some_and(|t| now >= t) {
            self.clipboard_until = None;
        }
        if let Screen::Open(o) = &mut self.screen
            && o.reveal_until.is_some_and(|t| now >= t)
        {
            o.reveal_until = None;
        }
        if let Some(at) = self.lock_at()
            && now >= at
        {
            let had_error = self.toast.as_ref().is_some_and(|t| t.error);
            let editing = matches!(&self.screen, Screen::Open(o) if o.editor.as_ref().is_some_and(|e| e.changed()));
            self.lock(true);
            if !had_error && !self.toast.as_ref().is_some_and(|t| t.error) {
                let text = if editing { "Locked after inactivity. The entry you were editing was saved first" } else { "Locked after inactivity" };
                self.toast(text, false);
            }
        }
    }

    /// When the vault locks unless there is input before then.
    pub fn lock_at(&self) -> Option<Instant> {
        let secs = self.settings.lock_after_idle_secs;
        (matches!(self.screen, Screen::Open(_)) && secs > 0).then(|| self.last_input + Duration::from_secs(secs.into()))
    }

    pub fn next_frame(&self, now: Instant) -> Option<Instant> {
        let toast = self.toast.as_ref().map(|t| t.until);
        let reveal = match &self.screen {
            Screen::Open(o) => o.reveal_until,
            Screen::Unlock(_) => None,
        };
        // Countdowns in the status line, and the one-time code's, tick once a second.
        let second = (self.clipboard_until.is_some() || self.lock_at().is_some() || self.selected_has_otp()).then(|| now + Duration::from_secs(1));
        [toast, reveal, self.clipboard_until, self.lock_at(), second].into_iter().flatten().min()
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        // Long enough to read: a second for every few words on top.
        let words = text.split_whitespace().count() as u64;
        let secs = if error { 7 } else { 3 } + words / 4;
        self.toast = Some(Toast { text, error, until: Instant::now() + Duration::from_secs(secs) });
    }

    pub fn title(&self) -> String {
        match &self.screen {
            Screen::Open(o) => format!("{} — Orchard Password Manager", file_name(&o.vault.path)),
            Screen::Unlock(_) => "Orchard Password Manager".into(),
        }
    }

    // -- locking and unlocking ---------------------------------------------

    /// Drop the open vault (wiping it) and take our secret off the
    /// clipboard. The unlock screen offers the same vault again. Unsaved
    /// changes, the editor's included, are saved first (waiting for a save
    /// that is running); if that fails, the vault stays open, unless
    /// `force` (inactivity, quitting), which locks anyway and says what was
    /// lost.
    pub fn lock(&mut self, force: bool) {
        self.leave_vault(force, true);
    }

    /// ⌘X: close the vault (saving first, as locking does) and go back to
    /// opening or creating one, with the path left empty.
    pub fn close_vault(&mut self) {
        self.leave_vault(false, false);
    }

    /// Lock, back to the unlock screen with this vault's path (`keep_path`)
    /// or an empty one.
    fn leave_vault(&mut self, force: bool, keep_path: bool) {
        self.keep_editor_changes();
        let Screen::Open(o) = &self.screen else { return };
        let path = o.vault.path.clone();
        if o.vault.writable()
            && o.vault.unsaved()
            && let Err(e) = o.vault.save()
        {
            log::warn!("saving before locking: {e}");
            if !force {
                self.toast(format!("Couldn't save the changes, so the vault stays open. {}", save_message(&e)), true);
                return;
            }
            self.toast(format!("Locked. The last changes couldn't be saved. {}", save_message(&e)), true);
        }
        self.clipboard.clear_now();
        self.clipboard_until = None;
        self.screen = Screen::Unlock(Unlock::new(keep_path.then_some(path.as_path())));
    }

    /// F5: which groups the list shows.
    pub fn open_group_filter(&mut self) {
        let Some(o) = self.open() else { return };
        let nodes = entries::group_tree(&o.vault.db);
        o.group_filter = Some(Box::new(GroupFilter::new(nodes, o.shown_groups.as_ref())));
    }

    fn groups_chosen(&mut self, out: groups::Outcome) {
        let Some(o) = self.open() else { return };
        match out {
            groups::Outcome::None => return,
            groups::Outcome::Cancel => {}
            groups::Outcome::Show(shown) => {
                o.shown_groups = shown;
                o.cursor = 0;
                o.top = 0;
                o.reveal_until = None;
            }
        }
        o.group_filter = None;
    }

    /// ⌘O: lock this vault (saving what is pending first) and choose
    /// another in the file picker. A failed save keeps this one open.
    pub fn open_another(&mut self) {
        self.lock(false);
        if let Screen::Unlock(u) = &mut self.screen {
            u.choose(false, true, self.tasks.waker());
        }
    }

    /// Enter on the unlock screen: open the vault, or create it when the
    /// file doesn't exist yet (the password typed twice).
    pub fn submit_unlock(&mut self) {
        let Screen::Unlock(u) = &mut self.screen else { return };
        if u.busy {
            return;
        }
        let path = u.vault_path();
        if path.as_os_str().is_empty() {
            u.error = Some("Enter the path of a vault file".into());
            u.focus = UnlockFocus::Path;
            return;
        }
        if u.password.is_empty() {
            u.error = Some("Enter the master password".into());
            u.focus = UnlockFocus::Password;
            return;
        }
        let create = !path.exists();
        if create && u.focus != UnlockFocus::Repeat && u.repeat.is_empty() {
            u.focus = UnlockFocus::Repeat;
            u.error = None;
            return;
        }
        if create && u.password.text() != u.repeat.text() {
            u.error = Some("The passwords don't match. Type the same one twice".into());
            u.repeat.clear();
            u.focus = UnlockFocus::Repeat;
            return;
        }
        let key = vault::key(u.password.text());
        u.password.clear();
        u.repeat.clear();
        u.busy = true;
        u.error = None;
        self.tasks.spawn(
            move || {
                if create {
                    let v = Vault::create(&path, key, vault::new_vault_config());
                    v.save().map(|()| v).map_err(|e| (path, format!("Couldn't create the vault: {e}")))
                } else {
                    Vault::open(&path, key).map_err(|e| (path, open_message(&e)))
                }
            },
            |app, result| app.unlocked(result),
        );
    }

    fn unlocked(&mut self, result: Result<Vault, (PathBuf, String)>) {
        match result {
            Ok(vault) => self.show_vault(vault),
            Err((path, message)) => {
                let mut u = Unlock::new(Some(&path));
                u.error = Some(message);
                self.screen = Screen::Unlock(u);
            }
        }
    }

    /// Show an unlocked vault's entries.
    pub fn show_vault(&mut self, vault: Vault) {
        self.settings.last_vault = Some(vault.path.clone());
        self.save_settings();
        let rows = entries::rows(&vault.db);
        let old = vault.old_format().is_some();
        self.screen = Screen::Open(Box::new(Open {
            vault,
            rows,
            search: TextField::new("").plain(),
            cursor: 0,
            top: 0,
            reveal_until: None,
            editor: None,
            saving: false,
            save_error: None,
            bin: false,
            confirm: None,
            undo: None,
            shown_groups: None,
            group_filter: None,
        }));
        if old {
            self.toast("This vault uses an older format: changing it updates the format first", false);
        }
        self.last_input = Instant::now();
        self.scroll_to_cursor();
    }

    // -- the open vault ------------------------------------------------------

    fn open(&mut self) -> Option<&mut Open> {
        match &mut self.screen {
            Screen::Open(o) => Some(o),
            Screen::Unlock(_) => None,
        }
    }

    /// Changes can be saved; else asks to update the vault's format first
    /// (and `then` goes on after a yes).
    fn check_writable(&mut self, then: Then) -> bool {
        let Some(o) = self.open() else { return false };
        let Some(old) = o.vault.old_format() else { return true };
        let name = file_name(&o.vault.path);
        let (text, yes) = match old {
            OldFormat::Kdbx3 => (
                format!("This vault uses an older format that this app can't save. To make changes, update it to the current format. Apps that only read the older format won't open it afterwards. The file as it is now is kept as {name}.bak."),
                "Update format",
            ),
            OldFormat::Kdb => (
                format!(
                    "This vault uses an old format that this app can't save. To make changes, save it in the current format as {}. The old file stays as it is; changes go to the new one.",
                    file_name(&o.vault.updated_path())
                ),
                "Save in new format",
            ),
        };
        o.confirm = Some(Box::new(Confirm::new("Update vault format?", text, yes, "Cancel", Ask::UpdateFormat(then))));
        false
    }

    /// Yes to updating the format: update, save, and go on.
    fn update_format(&mut self, then: Then) {
        let Some(o) = self.open() else { return };
        if let Err(taken) = o.vault.update_format() {
            let text = format!("Couldn't save in the new format: {} already exists. Move or rename it, then try again", file_name(&taken));
            return self.toast(text, true);
        }
        let path = o.vault.path.clone();
        self.settings.last_vault = Some(path);
        self.save_settings();
        self.vault_changed(None);
        match then {
            Then::Edit => self.edit_selected(),
            Then::New => self.new_entry(),
            Then::Delete => self.delete_selected(),
            Then::Restore => self.restore_selected(),
            Then::EmptyBin => self.ask_empty_bin(),
        }
    }

    /// F4: edit the selected entry.
    pub fn edit_selected(&mut self) {
        if !self.check_writable(Then::Edit) {
            return;
        }
        let options = self.settings.generator;
        let Some(o) = self.open() else { return };
        let Some(id) = o.selected().map(|r| r.id) else { return };
        let Some(draft) = entries::draft(&o.vault.db, id) else { return };
        let groups = entries::groups(&o.vault.db);
        o.editor = Some(Box::new(Editor::new(Some(id), &draft, groups, options)));
        o.reveal_until = None;
    }

    /// F7: a new entry, in the selected entry's group.
    pub fn new_entry(&mut self) {
        if !self.check_writable(Then::New) {
            return;
        }
        let options = self.settings.generator;
        let Some(o) = self.open() else { return };
        let db = &o.vault.db;
        let group = o.selected().and_then(|r| db.entry(r.id)).map_or(db.root().id(), |e| e.parent().id());
        let draft = entries::Draft {
            title: String::new(),
            username: String::new(),
            password: Zeroizing::new(String::new()),
            url: String::new(),
            otp: Zeroizing::default(),
            notes: String::new(),
            group,
        };
        let groups = entries::groups(db);
        o.editor = Some(Box::new(Editor::new(None, &draft, groups, options)));
    }

    /// The generator's choices, once it closes with new ones, go into the
    /// settings.
    fn remember_generator(&mut self) {
        let Screen::Open(o) = &self.screen else { return };
        let Some(options) = o.editor.as_ref().map(|e| e.gen_options) else { return };
        if options != self.settings.generator {
            self.settings.generator = options;
            self.save_settings();
        }
    }

    /// The editor's Cancel: closes it, after asking if there are changes.
    fn cancel_editor(&mut self) {
        let Some(o) = self.open() else { return };
        match &o.editor {
            Some(ed) if ed.changed() => {
                let text = "Your changes to this entry haven't been saved.";
                o.confirm = Some(Box::new(Confirm::new("Discard changes?", text, "Discard", "Keep editing", Ask::Discard)));
            }
            _ => o.editor = None,
        }
    }

    /// Locking with the editor open: what it holds goes into the vault
    /// (an edit keeps the previous version in the history, so nothing is
    /// lost either way), to be saved with the rest.
    fn keep_editor_changes(&mut self) {
        let Some(o) = self.open() else { return };
        o.confirm = None;
        let Some(ed) = o.editor.take() else { return };
        if !ed.changed() || !o.vault.writable() {
            return;
        }
        let draft = ed.draft();
        let changed = match ed.id {
            Some(id) => entries::update(&mut o.vault.db, id, &draft),
            None => entries::add(&mut o.vault.db, &draft).is_some(),
        };
        if changed {
            o.vault.changed();
            o.rows = o.list_rows();
        }
    }

    /// The editor's Save: write the draft into the vault and save it.
    fn save_editor(&mut self) {
        let Some(o) = self.open() else { return };
        let Some(ed) = &mut o.editor else { return };
        let mut draft = ed.draft();
        if draft.title.is_empty() {
            ed.error = Some("Enter a title for the entry".into());
            ed.focus = Some(editor::Field::Title);
            return;
        }
        // A one-time password typed or pasted: stored as a full link.
        if ed.otp_changed() && !draft.otp.is_empty() {
            match otp::normalize(&draft.otp, &draft.title) {
                Ok(link) => draft.otp = link,
                Err(e) => {
                    ed.error = Some(e.message().into());
                    ed.focus = Some(editor::Field::Otp);
                    return;
                }
            }
        }
        let id = ed.id;
        o.editor = None;
        // Saved into a group the list leaves out: it shows that group too.
        if let Some(shown) = &mut o.shown_groups {
            shown.insert(draft.group);
        }
        let (id, changed) = match id {
            Some(id) => (Some(id), entries::update(&mut o.vault.db, id, &draft)),
            None => (entries::add(&mut o.vault.db, &draft), true),
        };
        if changed {
            self.vault_changed(id);
        }
    }

    /// F8: the selected entry goes to the recycle bin (⌘Z puts it back).
    /// In the recycle bin, or in a vault without one, it is deleted for
    /// good, after asking.
    pub fn delete_selected(&mut self) {
        if !self.check_writable(Then::Delete) {
            return;
        }
        let Some(o) = self.open() else { return };
        let Some(row) = o.selected().cloned() else { return };
        if o.bin || o.vault.db.meta.recyclebin_enabled == Some(false) {
            let text = format!("{} will be deleted for good. This can't be undone.", entry_name(&row.title));
            o.confirm = Some(Box::new(Confirm::new("Delete for good?", text, "Delete", "Cancel", Ask::DeleteForGood(row.id))));
            return;
        }
        entries::delete(&mut o.vault.db, row.id);
        o.undo = Some(row.id);
        self.vault_changed(None);
        self.toast(primary_label(&format!("{} moved to the recycle bin · ⌘Z undoes", entry_name(&row.title))), false);
    }

    /// ⌘Z: the last entry moved to the recycle bin comes back.
    pub fn undo_delete(&mut self) {
        let Some(o) = self.open() else { return };
        let Some(id) = o.undo.take().filter(|&id| entries::in_bin(&o.vault.db, id)) else {
            return self.toast("Nothing to undo", false);
        };
        let title = o.vault.db.entry(id).and_then(|e| e.get_title().map(str::to_string)).unwrap_or_default();
        entries::restore(&mut o.vault.db, id);
        self.vault_changed(Some(id));
        self.toast(format!("{} is back", entry_name(&title)), false);
    }

    /// F2: the recycle bin's entries instead of the vault's, or back.
    pub fn toggle_bin(&mut self) {
        let Some(o) = self.open() else { return };
        if !o.bin && o.vault.db.meta.recyclebin_enabled == Some(false) {
            return self.toast("This vault deletes entries for good, so it has no recycle bin", false);
        }
        o.bin = !o.bin;
        o.rows = o.list_rows();
        o.search.set_text("");
        o.cursor = 0;
        o.top = 0;
        o.reveal_until = None;
        self.scroll_to_cursor();
    }

    /// F7 in the recycle bin: the selected entry goes back where it was.
    pub fn restore_selected(&mut self) {
        if !self.check_writable(Then::Restore) {
            return;
        }
        let Some(o) = self.open() else { return };
        let Some(row) = o.selected().cloned() else { return };
        let Some(to) = entries::restore(&mut o.vault.db, row.id) else { return };
        let group = if to == o.vault.db.root().id() { String::new() } else { format!(" to {}", entries::group_path(&o.vault.db, to)) };
        self.vault_changed(None);
        self.toast(format!("{} restored{group}", entry_name(&row.title)), false);
    }

    /// F4 in the recycle bin: delete everything in it, after asking.
    pub fn ask_empty_bin(&mut self) {
        if !self.check_writable(Then::EmptyBin) {
            return;
        }
        let Some(o) = self.open() else { return };
        let n = o.rows.len();
        if n == 0 {
            return self.toast("The recycle bin is already empty", false);
        }
        let what = if n == 1 { "The entry in it".to_string() } else { format!("The {n} entries in it") };
        let text = format!("{what} will be deleted for good. This can't be undone.");
        o.confirm = Some(Box::new(Confirm::new("Empty the recycle bin?", text, "Empty bin", "Cancel", Ask::EmptyBin)));
    }

    /// A yes to the open question.
    fn answer_yes(&mut self, ask: Ask) {
        match ask {
            Ask::Discard => {
                if let Some(o) = self.open() {
                    o.editor = None;
                }
            }
            Ask::DeleteForGood(id) => {
                let Some(o) = self.open() else { return };
                let title = o.vault.db.entry(id).and_then(|e| e.get_title().map(str::to_string)).unwrap_or_default();
                if let Err(why) = entries::delete_for_good(&mut o.vault.db, id) {
                    log::warn!("deleting for good: {why}");
                    return self.toast(SHARED_ATTACHMENT, true);
                }
                self.vault_changed(None);
                self.toast(format!("{} deleted", entry_name(&title)), false);
            }
            Ask::EmptyBin => {
                let Some(o) = self.open() else { return };
                let n = match entries::empty_bin(&mut o.vault.db) {
                    Ok(n) => n,
                    Err(why) => {
                        log::warn!("emptying the recycle bin: {why}");
                        return self.toast(SHARED_ATTACHMENT, true);
                    }
                };
                o.undo = None;
                self.vault_changed(None);
                self.toast(if n == 1 { "1 entry deleted".to_string() } else { format!("{n} entries deleted") }, false);
            }
            Ask::UpdateFormat(then) => self.update_format(then),
        }
    }

    /// A key or click on the open question.
    fn answer(&mut self, answer: Answer) {
        let Some(o) = self.open() else { return };
        if answer == Answer::None {
            return;
        }
        let Some(c) = o.confirm.take() else { return };
        if answer == Answer::Yes {
            self.answer_yes(c.action);
        }
    }

    /// After a change to the vault: the list again (on `select` if given,
    /// else where it was), and a save.
    fn vault_changed(&mut self, select: Option<EntryId>) {
        let Some(o) = self.open() else { return };
        o.vault.changed();
        o.rows = o.list_rows();
        if let Some(id) = select {
            let at = o.view().iter().position(|&i| o.rows[i].id == id);
            match at {
                Some(at) => o.cursor = at,
                // Not in the search results: show everything.
                None => {
                    o.search.set_text("");
                    o.cursor = o.view().iter().position(|&i| o.rows[i].id == id).unwrap_or(0);
                }
            }
        }
        self.scroll_to_cursor();
        self.save_vault();
    }

    /// Save the open vault on a worker. One at a time; a change made while
    /// one runs is saved when it is done.
    pub fn save_vault(&mut self) {
        let Some(o) = self.open() else { return };
        if o.saving {
            return;
        }
        o.saving = true;
        let snapshot = o.vault.snapshot();
        self.tasks.spawn(move || snapshot.save(), |app, result| app.saved(result));
    }

    fn saved(&mut self, result: Result<(), SaveError>) {
        let error = result.as_ref().err().map(save_message);
        if let Err(e) = &result {
            log::warn!("saving the vault: {e}");
        }
        let Some(o) = self.open() else {
            // Locked meanwhile (which saved again, or said why not).
            return;
        };
        o.saving = false;
        let again = error.is_none() && o.vault.unsaved();
        o.save_error = error.clone();
        if let Some(e) = error {
            self.toast(format!("Couldn't save the vault. {e}"), true);
        } else if again {
            self.save_vault();
        }
    }

    /// Copy a field of the selected entry; it comes off the clipboard again
    /// after the time set.
    pub fn copy_field(&mut self, field: &str, what: &str) {
        let Screen::Open(o) = &self.screen else { return };
        let Some(row) = o.view().get(o.cursor).map(|&i| &o.rows[i]) else { return };
        let Some(entry) = o.vault.db.entry(row.id) else { return };
        let Some(value) = entry.get(field).filter(|v| !v.is_empty()) else {
            self.toast(format!("This entry has no {what}"), true);
            return;
        };
        let secret = Zeroizing::new(value.to_string());
        let keep = Duration::from_secs(self.settings.clear_clipboard_secs.max(1).into());
        self.clipboard.copy(secret, keep);
        self.clipboard_until = Some(Instant::now() + keep);
        self.toast(format!("{} copied", capitalized(what)), false);
    }

    /// A click on a copy icon in the details.
    pub fn copy(&mut self, what: crate::ui::CopyWhat) {
        use crate::ui::CopyWhat as W;
        match what {
            W::Username => self.copy_field(fields::USERNAME, "username"),
            W::Password => self.copy_field(fields::PASSWORD, "password"),
            W::Url => self.copy_field(fields::URL, "URL"),
            W::Otp => self.copy_otp(),
            W::Notes => self.copy_field(fields::NOTES, "notes"),
        }
    }

    /// ⌘T: the selected entry's current one-time code.
    pub fn copy_otp(&mut self) {
        let Screen::Open(o) = &self.screen else { return };
        let Some(row) = o.view().get(o.cursor).map(|&i| &o.rows[i]) else { return };
        let Some(entry) = o.vault.db.entry(row.id) else { return };
        let code = match otp::code_now(&entry) {
            None => return self.toast("This entry has no one-time password. F4 adds one in the editor", true),
            Some(Err(e)) => return self.toast(e.message(), true),
            Some(Ok(c)) => c,
        };
        let keep = Duration::from_secs(self.settings.clear_clipboard_secs.max(1).into());
        self.clipboard.copy(code.code.clone(), keep);
        self.clipboard_until = Some(Instant::now() + keep);
        self.toast(format!("One-time code copied · valid for {} s", code.left), false);
    }

    /// The selected entry has a one-time password (its code shows).
    fn selected_has_otp(&self) -> bool {
        let Screen::Open(o) = &self.screen else { return false };
        o.editor.is_none() && o.selected().and_then(|r| o.vault.db.entry(r.id)).is_some_and(|e| otp::stored(&e).is_some())
    }

    pub fn toggle_reveal(&mut self) {
        let secs = self.settings.reveal_secs.max(1);
        if let Screen::Open(o) = &mut self.screen {
            o.reveal_until = match o.reveal_until {
                Some(_) => None,
                None => Some(Instant::now() + Duration::from_secs(secs.into())),
            };
        }
    }

    /// Rows of the list (for paging); the layout's.
    pub fn list_height(&self) -> usize {
        crate::ui::layout(self).list.rows as usize
    }

    pub fn scroll_to_cursor(&mut self) {
        let h = self.list_height().max(1);
        if let Screen::Open(o) = &mut self.screen {
            let n = o.view().len();
            o.cursor = o.cursor.min(n.saturating_sub(1));
            if o.cursor < o.top {
                o.top = o.cursor;
            } else if o.cursor >= o.top + h {
                o.top = o.cursor + 1 - h;
            }
            o.top = o.top.min(n.saturating_sub(h));
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if let Screen::Open(o) = &mut self.screen {
            let n = o.view().len() as isize;
            o.cursor = (o.cursor as isize + delta).clamp(0, (n - 1).max(0)) as usize;
            o.reveal_until = None;
        }
        self.scroll_to_cursor();
    }

    // -- input ---------------------------------------------------------------

    pub fn handle_key(&mut self, ev: &KeyEvent) {
        self.last_input = Instant::now();
        if ev.is(Named::F(10)) || (ev.mods.primary() && ev.key == Key::Char('q')) {
            self.quit = true;
            return;
        }
        if let Some(mut d) = self.settings_dialog.take() {
            if d.handle_key(self, ev) {
                self.settings_dialog = Some(d);
            }
            return;
        }
        if ev.mods.primary() && !ev.mods.shift {
            match ev.key {
                Key::Char(',') => return self.open_settings(),
                Key::Char('=' | '+') => return self.zoom(1),
                Key::Char('-') => return self.zoom(-1),
                Key::Char('0') => return self.zoom(0),
                _ => {}
            }
        }
        // Not while a dialog of the vault's is open: it has F9 to itself
        // (none so far), and its edits aren't settings.
        if ev.is(Named::F(9)) && !self.open().is_some_and(|o| o.editor.is_some() || o.confirm.is_some() || o.group_filter.is_some()) {
            return self.open_settings();
        }
        match &self.screen {
            Screen::Unlock(_) => self.unlock_key(ev),
            Screen::Open(_) => self.open_key(ev),
        }
    }

    fn unlock_key(&mut self, ev: &KeyEvent) {
        let screen = self.screen_rect();
        let Screen::Unlock(u) = &mut self.screen else { return };
        if u.busy {
            return;
        }
        if let Some(p) = &mut u.picker {
            let out = p.key(ev, screen);
            return u.picked(out);
        }
        let creating = u.creating();
        match ev.key {
            Key::Named(Named::Enter) => return self.submit_unlock(),
            Key::Named(Named::F(2)) => return u.choose(false, true, self.tasks.waker()),
            Key::Named(Named::F(3)) => return u.choose(true, true, self.tasks.waker()),
            Key::Named(Named::Tab | Named::Down | Named::Up) => {
                let back = ev.is(Named::Up) || ev.mods.shift;
                u.focus = match (u.focus, back, creating) {
                    (UnlockFocus::Path, false, _) => UnlockFocus::Password,
                    (UnlockFocus::Password, false, true) => UnlockFocus::Repeat,
                    (UnlockFocus::Password, true, _) => UnlockFocus::Path,
                    (UnlockFocus::Repeat, true, _) => UnlockFocus::Password,
                    (f, _, _) => f,
                };
                return;
            }
            _ => {}
        }
        let edited = match u.focus {
            UnlockFocus::Path => u.path.handle(ev),
            UnlockFocus::Password => u.password.handle(ev),
            UnlockFocus::Repeat => u.repeat.handle(ev),
        };
        if edited {
            u.error = None;
        }
    }

    fn open_key(&mut self, ev: &KeyEvent) {
        let screen = self.screen_rect();
        if let Some(c) = self.open().and_then(|o| o.confirm.as_mut()) {
            let answer = c.handle_key(ev);
            return self.answer(answer);
        }
        if let Some(g) = self.open().and_then(|o| o.group_filter.as_mut()) {
            let out = g.handle_key(ev, screen);
            return self.groups_chosen(out);
        }
        if let Some(ed) = self.open().and_then(|o| o.editor.as_mut()) {
            match ed.handle_key(ev, screen) {
                editor::Outcome::Save => self.save_editor(),
                editor::Outcome::Cancel => self.cancel_editor(),
                editor::Outcome::None => self.remember_generator(),
            }
            return;
        }
        let bin = self.open().is_some_and(|o| o.bin);
        let page = self.list_height().max(1) as isize;
        if ev.mods.primary() {
            match ev.key {
                Key::Char('c') => self.copy_field(fields::PASSWORD, "password"),
                Key::Char('b') => self.copy_field(fields::USERNAME, "username"),
                Key::Char('u') => self.copy_field(fields::URL, "URL"),
                Key::Char('t') => self.copy_otp(),
                Key::Char('l' | 'L') if ev.mods.shift => self.lock(false),
                Key::Char('o') => self.open_another(),
                Key::Char('x') => self.close_vault(),
                Key::Char('z') if !bin => self.undo_delete(),
                _ => {}
            }
            return;
        }
        match ev.key {
            Key::Named(Named::Up) => self.move_cursor(-1),
            Key::Named(Named::Down) => self.move_cursor(1),
            Key::Named(Named::PageUp) => self.move_cursor(-page),
            Key::Named(Named::PageDown) => self.move_cursor(page),
            Key::Named(Named::Enter) => self.copy_field(fields::PASSWORD, "password"),
            Key::Named(Named::F(5)) if !bin => self.open_group_filter(),
            Key::Named(Named::F(3)) => self.toggle_reveal(),
            Key::Named(Named::F(2)) => self.toggle_bin(),
            Key::Named(Named::F(4)) if bin => self.ask_empty_bin(),
            Key::Named(Named::F(7)) if bin => self.restore_selected(),
            Key::Named(Named::Insert) if bin => {}
            Key::Named(Named::F(4)) => self.edit_selected(),
            Key::Named(Named::F(7) | Named::Insert) => self.new_entry(),
            Key::Named(Named::F(8)) => self.delete_selected(),
            Key::Named(Named::Escape) => {
                if let Screen::Open(o) = &mut self.screen {
                    o.search.set_text("");
                    o.cursor = 0;
                }
                self.scroll_to_cursor();
            }
            _ => {
                // Everything else types into the search.
                if let Screen::Open(o) = &mut self.screen
                    && o.search.handle(ev)
                {
                    o.cursor = 0;
                    o.top = 0;
                    o.reveal_until = None;
                }
            }
        }
    }

    /// The area dialogs center in: the window above the key bar.
    pub fn screen_rect(&self) -> orchard_grid::CellRect {
        orchard_grid::CellRect::new(0, 0, self.cols, crate::ui::layout(self).footer.row)
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent) {
        self.last_input = Instant::now();
        let screen = self.screen_rect();
        if let Some(mut d) = self.settings_dialog.take() {
            if d.handle_mouse(self, ev) {
                self.settings_dialog = Some(d);
            }
            return;
        }
        if let Screen::Unlock(u) = &mut self.screen {
            if let Some(p) = &mut u.picker {
                let out = p.mouse(ev, screen);
                u.picked(out);
            }
            return;
        }
        let Some(o) = self.open() else { return };
        if let Some(c) = &mut o.confirm {
            let answer = c.handle_mouse(ev, screen);
            return self.answer(answer);
        }
        if let Some(g) = &mut o.group_filter {
            let out = g.handle_mouse(ev, screen);
            return self.groups_chosen(out);
        }
        if let Some(ed) = &mut o.editor {
            match ed.handle_mouse(ev, screen) {
                editor::Outcome::Save => self.save_editor(),
                editor::Outcome::Cancel => self.cancel_editor(),
                editor::Outcome::None => self.remember_generator(),
            }
            return;
        }
        if ev.kind == MouseKind::Down
            && let Some(&(_, what)) = crate::ui::copy_icons(self).iter().find(|&&(cell, _)| cell == (ev.col, ev.row))
        {
            return self.copy(what);
        }
        let list = crate::ui::layout(self).list;
        match ev.kind {
            MouseKind::Scroll(rows) => self.move_cursor(-(rows.round() as isize)),
            MouseKind::Down if orchard_kit::widgets::contains(list, ev.col, ev.row) => {
                if let Screen::Open(o) = &mut self.screen {
                    let i = o.top + (ev.row - list.row) as usize;
                    if i < o.view().len() {
                        o.cursor = i;
                        o.reveal_until = None;
                    }
                }
                if ev.double {
                    self.copy_field(fields::PASSWORD, "password");
                }
            }
            _ => {}
        }
    }
}

impl Unlock {
    pub fn new(path: Option<&Path>) -> Unlock {
        let text = path.map(|p| p.display().to_string()).unwrap_or_default();
        let focus = if text.is_empty() { UnlockFocus::Path } else { UnlockFocus::Password };
        Unlock {
            path: TextField::new(text),
            password: SecretField::default(),
            repeat: SecretField::default(),
            focus,
            busy: false,
            error: None,
            picker: None,
        }
    }

    /// The path typed, "~" meaning the home folder.
    pub fn vault_path(&self) -> PathBuf {
        expand_home(self.path.text.trim(), dirs::home_dir().as_deref())
    }

    /// The path names no file yet: Enter creates a vault there.
    pub fn creating(&self) -> bool {
        let p = self.vault_path();
        !p.as_os_str().is_empty() && !p.exists()
    }

    /// Open the picker at the folder of the path typed (home without one):
    /// a vault file, or with `new` a folder for a new vault. Screenshots
    /// leave out the bookmarks (`bookmarks` false), tests always do.
    pub fn choose(&mut self, new: bool, bookmarks: bool, waker: Waker) {
        let start = self.vault_path().parent().map(Path::to_path_buf).unwrap_or_default();
        let opts = if new {
            picker::Options::folder(start).title("Choose a folder for the new vault").accept("Create vault here")
        } else {
            picker::Options::files(start).title("Open a vault").accept("Open vault").extensions(&["kdbx", "kdb"])
        };
        let opts = if bookmarks && !cfg!(test) { opts } else { opts.bookmarks(None) };
        self.picker = Some(Box::new(Picker::new(opts, waker)));
    }

    /// The picker closed (or not yet): a file to unlock, or a new vault's
    /// path in the chosen folder, then on to the password.
    fn picked(&mut self, out: picker::Outcome) {
        let path = match out {
            picker::Outcome::Keep | picker::Outcome::AddFolder(_) => return,
            picker::Outcome::Cancel => None,
            picker::Outcome::Files(files) => files.into_iter().next(),
            picker::Outcome::Folder(dir) => Some(new_vault_path(&dir)),
        };
        self.picker = None;
        if let Some(path) = path {
            self.path.set_text(&path.display().to_string());
            self.focus = UnlockFocus::Password;
            self.error = None;
        }
    }
}

impl Open {
    /// Indexes into `rows` that match the search (all words, in the title,
    /// username, URL or group).
    pub fn view(&self) -> Vec<usize> {
        let words: Vec<String> = self.search.text.split_whitespace().map(str::to_lowercase).collect();
        let shown = self.shown_groups.as_ref().filter(|_| !self.bin);
        (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                if shown.is_some_and(|s| !s.contains(&r.group_id)) {
                    return false;
                }
                let hay = format!("{} {} {} {}", r.title, r.username, r.url, r.group).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect()
    }

    pub fn selected(&self) -> Option<&Row> {
        self.view().get(self.cursor).map(|&i| &self.rows[i])
    }

    /// The rows for what the list shows: the vault's entries or the bin's.
    fn list_rows(&self) -> Vec<Row> {
        if self.bin { entries::bin_rows(&self.vault.db) } else { entries::rows(&self.vault.db) }
    }
}

/// Deleting for good refused (see `integrity::delete_keeping_attachments`).
const SHARED_ATTACHMENT: &str = "Nothing was deleted: an older version of another entry uses the same attachment. Delete that entry too, or keep this one";

fn open_message(e: &OpenError) -> String {
    match e {
        OpenError::WrongKey => "Wrong master password. Check it and try again".into(),
        OpenError::NotFound => "No vault at this path. Check the path, or enter a new one to create a vault".into(),
        OpenError::NotAVault(why) => {
            log::warn!("opening the vault: {why}");
            "This file isn't a vault this app can read. Choose a KDBX file".into()
        }
        OpenError::Io(e) => format!("Couldn't read the vault: {e}"),
    }
}

fn save_message(e: &SaveError) -> String {
    match e {
        SaveError::OldFormat => "This vault uses an older format that can't be saved".into(),
        SaveError::Verify(_) => "The saved copy didn't read back the same, so the vault file was left as it was. log.txt in the settings folder says what differed".into(),
        SaveError::Write(why) => format!("Check that the vault's folder is writable and has space ({why})"),
    }
}

/// An entry in a message: its title in quotes, or "Entry".
fn entry_name(title: &str) -> String {
    if title.is_empty() { "Entry".into() } else { format!("\"{title}\"") }
}

/// A typed path with a leading "~" meaning the home folder, as in a shell.
fn expand_home(text: &str, home: Option<&Path>) -> PathBuf {
    let seps = ['/', std::path::MAIN_SEPARATOR];
    match (text.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(seps) => home.join(rest.trim_start_matches(seps)),
        _ => PathBuf::from(text),
    }
}

/// "Passwords.kdbx" in `dir`, or "Passwords 2.kdbx" and on when taken.
fn new_vault_path(dir: &Path) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { "Passwords.kdbx".into() } else { format!("Passwords {n}.kdbx") }))
        .find(|p| !p.exists())
        .unwrap_or_default()
}

fn scheme_in(schemes: &[ColorScheme], name: &str) -> ColorScheme {
    schemes
        .iter()
        .find(|s| s.name == name)
        .or_else(|| schemes::builtin_scheme(schemes::DEFAULT_SCHEME))
        .cloned()
        .unwrap_or_else(|| Theme::default_dark().scheme)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Tests and screenshots never touch the real clipboard.
struct NoClipboard;

impl Board for NoClipboard {
    fn get(&mut self) -> Option<Zeroizing<String>> {
        None
    }
    fn set_secret(&mut self, _text: &str) -> Result<(), String> {
        Ok(())
    }
    fn clear(&mut self) {}
}

#[cfg(test)]
mod tests;
