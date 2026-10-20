#!/usr/bin/env bash
# Install Orchard Password Manager on Omarchy (or any Arch-based system), per user,
# no root needed except for the runtime libraries:
#
#   curl -fsSL https://raw.githubusercontent.com/sandboxly/orchard-password-manager/main/packaging/omarchy/install.sh | bash
#   ... | bash -s -- --bind --free-f9      # also add a key and free F9 for the app
#   ... | bash -s -- --uninstall
#
# Options
#   --bind        Super+Ctrl+Shift+P opens Orchard Password Manager
#   --free-f9     unbind F9 from Omarchy's push-to-talk dictation, so F9 reaches the
#                 app (Settings); dictation keeps working if you rebind it
#   --version X   install release X (for example 1.0.0) instead of the latest
#   --uninstall   remove the app, the launcher entry and the key binding
#
# The app lands in ~/.local/opt/orchard-password-manager and is linked as
# ~/.local/bin/orchard-password-manager. It ships everything else, so
# nothing else is installed system-wide except the libraries listed in DEPS.
set -euo pipefail

REPO="sandboxly/orchard-password-manager"
ASSET="Orchard-Password-Manager-Linux.tar.gz"
PREFIX="$HOME/.local/opt/orchard-password-manager"
BIN="$HOME/.local/bin/orchard-password-manager"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
BINDINGS="$HOME/.config/hypr/bindings.lua"
MARK="-- Orchard Password Manager (added by its installer)"
MARK_F9="-- Orchard Password Manager: F9 is its settings (added by its installer)"
# Window system and clipboard, keyboard, fonts and GPU loader. Most are on Omarchy already.
DEPS=(wayland libxkbcommon libx11 libxcursor libxi fontconfig vulkan-icd-loader mesa)

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

uninstall() {
  rm -rf "$PREFIX"
  rm -f "$BIN" "$APPS/orchard-password-manager.desktop" "$ICONS/orchard-password-manager.png"
  if [[ -f "$BINDINGS" ]] && grep -qF -- "$MARK" "$BINDINGS"; then
    # The marker line and the bind line after it.
    sed -i "\|^$MARK\$|,+1d" "$BINDINGS"
  fi
  if [[ -f "$BINDINGS" ]] && grep -qF -- "$MARK_F9" "$BINDINGS"; then
    sed -i "\|^$MARK_F9\$|,+1d" "$BINDINGS"
  fi
  update-desktop-database "$APPS" 2>/dev/null || true
  say "Orchard Password Manager removed. Settings in ~/.config/orchard-password-manager stay; your vaults are never touched."
}

install_deps() {
  local missing=()
  for d in "${DEPS[@]}"; do pacman -Qq "$d" &>/dev/null || missing+=("$d"); done
  [[ ${#missing[@]} -eq 0 ]] && return
  say "Installing libraries: ${missing[*]}"
  sudo pacman -S --needed --noconfirm "${missing[@]}"
}

install_app() {
  local version="$1" url tmp
  if [[ -n "$version" ]]; then
    url="https://github.com/$REPO/releases/download/v${version#v}/$ASSET"
  else
    url="https://github.com/$REPO/releases/latest/download/$ASSET"
  fi
  tmp="$(mktemp -d)"
  trap "rm -rf '$tmp'" EXIT
  say "Downloading Orchard Password Manager"
  curl -fL --progress-bar -o "$tmp/$ASSET" "$url" || die "download failed: $url (check the version and your connection)"
  tar xzf "$tmp/$ASSET" -C "$tmp"
  [[ -x "$tmp/orchard-password-manager/orchard-password-manager" ]] || die "the download is not an Orchard Password Manager package"

  mkdir -p "$(dirname "$PREFIX")" "$(dirname "$BIN")" "$APPS" "$ICONS"
  rm -rf "$PREFIX.new"
  mv "$tmp/orchard-password-manager" "$PREFIX.new"
  rm -rf "$PREFIX"
  mv "$PREFIX.new" "$PREFIX"
  ln -sf "$PREFIX/orchard-password-manager" "$BIN"

  curl -fsSL -o "$ICONS/orchard-password-manager.png" \
    "https://raw.githubusercontent.com/$REPO/main/packaging/linux/orchard-password-manager.png" || true
  cat >"$APPS/orchard-password-manager.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Orchard Password Manager
GenericName=Password manager
Comment=Keep your passwords in an encrypted vault
Exec=$PREFIX/orchard-password-manager %f
Icon=orchard-password-manager
Terminal=false
Categories=Utility;Security;
Keywords=password;vault;kdbx;login;otp;security;
MimeType=application/x-keepass2;
StartupWMClass=orchard-password-manager
EOF
  update-desktop-database "$APPS" 2>/dev/null || true
}

add_binding() {
  [[ -f "$BINDINGS" ]] || die "no $BINDINGS: is this Omarchy 4 or later?"
  if grep -qF -- "$MARK" "$BINDINGS"; then return; fi
  printf '\n%s\no.bind("SUPER + CTRL + SHIFT + P", "Orchard Password Manager", "uwsm-app -- orchard-password-manager")\n' \
    "$MARK" >>"$BINDINGS"
  hyprctl reload >/dev/null 2>&1 || true
}

free_f9() {
  [[ -f "$BINDINGS" ]] || die "no $BINDINGS: is this Omarchy 4 or later?"
  if grep -qF -- "$MARK_F9" "$BINDINGS"; then return; fi
  printf '\n%s\nhl.unbind("F9")\n' "$MARK_F9" >>"$BINDINGS"
  hyprctl reload >/dev/null 2>&1 || true
}

main() {
  local free=0 bind=0 version=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --bind) bind=1 ;;
      --free-f9) free=1 ;;
      --version) version="${2:?--version needs a number, for example 1.0.0}"; shift ;;
      --uninstall) uninstall; exit 0 ;;
      -h|--help) sed -n '2,15p' "${BASH_SOURCE[0]:-$0}" 2>/dev/null || true; exit 0 ;;
      *) die "unknown option: $1" ;;
    esac
    shift
  done

  [[ "$(uname -m)" == x86_64 ]] || die "Orchard Password Manager for Linux needs an x86-64 machine"
  command -v pacman >/dev/null || die "this installer is for Arch-based systems such as Omarchy"
  install_deps
  install_app "$version"
  [[ $bind -eq 1 ]] && add_binding
  [[ $free -eq 1 ]] && free_f9

  say "Installed. Start it from the launcher (Super+Space) or with: orchard-password-manager"
  [[ $bind -eq 1 ]] && echo "Key: Super+Ctrl+Shift+P"
  [[ $bind -eq 0 || $free -eq 0 ]] && echo "Options: --bind adds a key; --free-f9 frees F9 from Omarchy's dictation key."
  case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Add ~/.local/bin to your PATH to run it from a terminal." ;; esac
}

main "$@"
