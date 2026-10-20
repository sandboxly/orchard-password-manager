# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Orchard Password Manager: a password manager from the Orchard family, for KDBX vaults (the KeePass format). Like Orchard Commander, Orchard Music Player and Orchard Download Manager it is a GPU-drawn desktop app (winit + wgpu) styled as a character grid, not a TUI. It uses the Orchard apps' shared crates from Orchard Commons (`orchard-grid` renderer; `orchard-theme` color schemes and the color-schemes folder shared with Commander; `orchard-kit` background tasks, input events, widgets and dialog forms; `orchard-shell` the window and event loop, `--help`/`--version`/`--self-test`/`--screenshot`, the crash log and the settings files) as git dependencies pinned to a release tag; to work on them together, add a `[patch."https://github.com/sandboxly/orchard-commons"]` section pointing at a local checkout (`../orchard-commons/crates/…`) in an uncommitted cargo config (`.cargo/config.toml` is tracked here) and pass it as `cargo <command> --config <file>`. A patched build rewrites the orchard entries in `Cargo.lock` as path packages and fails with `--locked`: never commit that.

## Commands

```sh
cargo run --release -- [VAULT.kdbx]                                # run
cargo test                                                         # all tests
cargo clippy --all-targets -- -D warnings                          # lint
scripts/third-party-licenses.sh                                    # licenses/THIRD-PARTY-RUST.txt after dependency or version changes
cargo run -- --screenshot /tmp/x.png --size 110x36 --cursor 2 --reveal   # look at the UI headless
packaging/macos/package.sh [--notarize]                            # dist/Orchard-Password-Manager-macOS.dmg (CI builds every package)
python3 site/build.py                                              # the website into _site/
```

- `ORCHARD_PASSWORD_MANAGER_CONFIG_DIR=<empty dir>` runs with default settings, without touching the real config.
- `--screenshot` shows a made-up demo vault (in memory, never written); `--dialog unlock|create|error` the unlock screen instead (`open|folder` with its file picker for a vault or a new vault's folder, without bookmarks), `edit|new|groups|generate|discard` the entry editor (on `--cursor N`'s entry; `groups` with its group list open, `filter` the group filter (F5), `generate` with the password generator, `discard` asking to discard), `bin|empty` the recycle bin (three demo entries deleted; `empty` asking to empty it), `update` the question an older-format vault asks before a change, `toast` an error toast; `settings|security|scheme-editor|color-picker` Settings over the vault; `--search TEXT`, `--reveal`. The demo vault's path is `~/Documents/Passwords.kdbx` as typed, so no screenshot shows a real home folder; the unlock screen says "Create vault" unless that file exists (for the website's `unlock.webp`, run with `HOME` pointing at a scratch folder holding an empty `Documents/Passwords.kdbx`).
- Packages come from CI (`.github/workflows/ci.yml`, `packaging/stage.sh`, Inno Setup, nfpm, appimagetool, `packaging/macos/package.sh`), the same as the other Orchard apps'; release steps and the macOS signing secrets: `docs/RELEASING.md`. Keep the Windows installer's AppId. `packaging/omarchy/install.sh` installs per user on Arch (`--bind`: Super+Ctrl+Shift+P). The icons are `packaging/icons/` (`app-1024.png` with a transparent surround, `app.icns`, `app.ico` embedded by `build.rs`) and `packaging/linux/orchard-password-manager.png` (256 px). The website is `site/` (`pages.yml` deploys it): screenshots in `site/screenshots/` are `--screenshot` WebPs, `og.png` is the hero screenshot.

## Architecture

- `app.rs` owns all state and logic (`Screen::Unlock` or `Screen::Open`, keys, timers); `ui/` draws it as a pure function into an `orchard_grid::Frame`; `main.rs` puts `App` in `orchard_shell`'s window and event loop.
- `vault.rs`: the `keepass` crate does all the crypto and the file format; never write our own. Its KDBX4 writer is experimental, so `Vault::save` writes "<file>.tmp", opens it again with the same key and compares it with what's in memory, keeps the old file as "<file>.bak", then renames. Only KDBX 4.1 can be written (a 4.0 vault becomes 4.1 on opening, without asking: 4.1 only adds optional parts); a KDBX3 (or first-generation) vault opens as it is, and the first change asks to update it (`Vault::update_format`: KDBX3 in place with the old file as ".bak"; a ".kdb" into a new ".kdbx" beside it, never replacing a file). New vaults use Argon2id (`new_vault_config`); tests use `argon2id(1, 64 * 1024, 1)` to stay fast.
- `integrity.rs`: the save's check (`differences`: every part compared, attachments by file name and content, since the crate's writer renumbers them) and `delete_keeping_attachments` around deleting for good (the crate's reader leaves attachments' entry lists empty, so a deletion could take an attachment another entry shares: it is attached again, or for an older version the deletion is undone and refused). Warnings go to `log.txt` in the config dir (`logging.rs`); never log a value.
- Opening and saving a vault (the key derivation) go through `app.tasks`, never the UI thread. A change (editor, delete) marks the vault changed (`Vault::changed`) and saves a `Snapshot` on a worker; a per-vault lock and a change counter make sure an older snapshot never overwrites a newer save. Locking puts what the editor holds into the vault (an edit keeps the old version in the history) and saves what is pending first; a manual lock whose save fails leaves the vault open, inactivity and quit lock anyway.
- `entries.rs`: rows, groups and changes to entries. An edit keeps the previous version in the entry's history; delete moves to the recycle bin (made when needed; ⌘Z puts the last one back) or deletes for good if the vault turned the bin off. F2 shows the bin: F7 restores to the group it came from (the entry's previous parent, else the root), F8 deletes for good, F4 empties it. Move an entry *before* tracking changes: KDBX doesn't store a history version's group, so a version made before a move reads back different and fails the save's check.
- The details pane has a copy icon (drawn in pixels: no font has a copy symbol everywhere) in one column at its right edge on each field with a value; `ui::copy_icons` places them for both drawing and clicks (`App::copy`).
- `ui/settings/`: Settings (F9, ⌘,), ported from Orchard Music Player's: pages of declarative controls (`appearance_page`, `security_page`; see the module docs to add one) that apply and save as they change, the scheme editor and color picker saving into the color-schemes folder the Orchard apps share, a sample vault on the Appearance page. `App::settings_dialog` holds it over either screen. Lock is ⌘⇧L (Ctrl+Shift+L), not F9. ⌘O locks and opens the file picker for another vault; ⌘X closes the vault (saving first, like locking) back to the unlock screen with an empty path.
- `ui/groups.rs`: the group filter (F5): the group tree with checkboxes (checking a group checks its subgroups; the root stands for the whole vault); `Open::shown_groups` (`None`: all) narrows `Open::view`, the search works within it. In memory only. Copying is ⌘C / ⌘B / ⌘T / Enter, not F5/F6.
- `ui/confirm.rs`: the question before anything that can't be undone (discarding an edit, deleting for good, emptying the bin, updating the format); `Open::confirm` holds it with an `Ask` saying what a yes does. The safe button has the focus.
- `generator.rs` (random passwords from the OS's secure source, rejection sampled, one of each kind chosen) and `ui/generator.rs` (its dialog, F2 / ⌘G in the editor; the choices are kept in the settings).
- `otp.rs`: time-based one-time passwords, read from an "otp" field (an `otpauth://totp/…` link) or KeePass's "TimeOtp-*" fields into one link with every setting spelled out (6 digits, 30 s, SHA-1 unless it says otherwise: the keepass crate's own default is 8 digits), which the crate's `TOTP` computes the code from. The editor's OTP field (a `SecretField`) takes a link or a bare secret and saves an "otp" link, removing the TimeOtp fields. The code shows in the details on F3 like the password, with its seconds left; ⌘T copies it.
- `ui/editor.rs`: the entry editor (kit form plus a `SecretField` for the password and a `ui/notes.rs` `NotesField` for the notes, which the kit has no multi-line field for). In the notes Enter starts a line, Up/Down leave the field past the first/last line, and ⌘S saves; line breaks round-trip as the entry had them ("\r\n" stays).

## Secrets (must hold)

- The decrypted database and its key live only in the open `Vault`; `App::lock` drops it (the crate wipes protected fields and the key on drop). The app locks after `lock_after_idle_secs` without input, on ⌘⇧L (Ctrl+Shift+L) and on quit (`ShellApp::save` → `shutdown`).
- Copied secrets go through `clipboard::SecretClipboard`: one thread owns the clipboard, marks copies to stay out of clipboard histories, and clears it after `clear_clipboard_secs`, on lock and on quit, but only if it still holds our secret. Tests and screenshots use a no-op clipboard (`App::new(…, false)`).
- The master password is typed into `secret::SecretField` (wiped on clear and drop, drawn as "•"), never the kit's `TextField`, whose text forms clone every frame. Keep our own copies of secrets in `Zeroizing`.
- Never log, toast or show secrets; never write them to settings. A password shows only on F3, for `reveal_secs`.

## Gotchas

- **UI copy** follows `docs/ai-microcopy-guide.md`: sentence case, front-loaded and short, errors that say what went wrong, why and how to fix it, empty states that say how to start, no periods on fragments. Developer details go to the log, not into messages.
- **No third-party product names** in user-visible text, docs or code comments (describe the behaviour), except where a license requires it.
- Glyphs must exist in the grid font: emoji-presentation symbols fall back to an emoji font and break the layout.
- Tests must never touch the real config or clipboard: `App::new(…, false)`.
