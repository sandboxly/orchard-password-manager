# Orchard Password Manager

A themeable, keyboard-friendly password manager for KDBX vaults, from the
Orchard family (Orchard Commander, Orchard Music Player, Orchard Download
Manager). It is drawn as a character grid on the GPU, like the other
Orchard apps: same renderer, same color schemes, same frame look. Vaults
stay compatible with other apps that read the KDBX format.

* Opens KDB, KDBX3 and KDBX4 vaults; saves KDBX4, checked before it
  replaces the file, with the previous version kept as `.bak`. An older
  vault asks before its first change updates the format.
* New vaults use Argon2id.
* Locks itself after a while without input, on ⌘⇧L and on quit.
* Copied passwords leave the clipboard again after a while (unless you
  copied something else since), and stay out of clipboard histories.
* Passwords show only on F3, for a few seconds.
* Search as you type; a group filter (F5).
* An entry editor that keeps the previous version in the entry's history,
  with a password generator (F2 in the editor).
* One-time codes, from an `otpauth://` link or the TimeOtp fields other
  apps write.
* A recycle bin (F2), with ⌘Z for the last deletion.
* Settings (F9): locking and clipboard times, the color schemes shared
  with Orchard Commander, the font.

## Install

Downloads for macOS (Apple silicon), Windows and Linux are on the
[website](https://sandboxly.github.io/orchard-password-manager/) and the
[releases page](https://github.com/sandboxly/orchard-password-manager/releases).
On macOS also:

```sh
brew install --cask sandboxly/tap/orchard-password-manager
```

**Omarchy and other Arch-based systems** — one command, no root except for runtime libraries:

```sh
curl -fsSL https://raw.githubusercontent.com/sandboxly/orchard-password-manager/main/packaging/omarchy/install.sh | bash
```

Add `-s -- --bind --free-f9` after `bash` to bind Super+Ctrl+Shift+P and free F9 (Settings) from
Omarchy's dictation key; `-s -- --uninstall` removes it. Rerun the command to update.

## Building

```sh
cargo build --release
./target/release/orchard-password-manager [VAULT.kdbx]
```

Packages (macOS dmg, Windows installer, deb, rpm, AppImage) and the release
steps: [docs/RELEASING.md](docs/RELEASING.md). The website is `site/`
(`python3 site/build.py` builds it into `_site/`).

## Keys

| key                   | does                                              |
|-----------------------|---------------------------------------------------|
| typing, Esc           | search · clear the search                         |
| Enter, ⌘C             | copy the password                                 |
| ⌘B, ⌘U, ⌘T            | copy the username · URL · one-time code           |
| F3                    | show or hide the password                         |
| F4, F7                | edit the entry · new entry                        |
| F8, ⌘Z                | move to the recycle bin · put the last one back   |
| F2                    | the recycle bin (F7 restores, F4 empties it)      |
| F5                    | choose the groups to show                         |
| F2, ⌘G                | in the editor: generate a password                |
| ⌘⇧L                   | lock                                              |
| ⌘O, ⌘X                | open another vault · close the vault              |
| F9, ⌘,                | settings                                          |
| ⌘+ ⌘− ⌘0              | font size                                         |

On Windows and Linux, Ctrl instead of ⌘.

## Files

Settings live in the platform config folder
(`~/Library/Application Support/Orchard Password Manager` on macOS,
`~/.config/orchard-password-manager` on Linux), or in
`$ORCHARD_PASSWORD_MANAGER_CONFIG_DIR`. Vaults stay wherever you keep
them; the settings never hold a secret.

MIT License.
