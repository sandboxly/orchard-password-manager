//! Settings (`settings.json`) in the app's config directory. The appearance
//! keys mean what they mean in Orchard Commander's settings; everything
//! else is the password manager's own. Missing keys take their defaults, so
//! older files keep loading. Nothing secret is ever stored here.

use std::path::{Path, PathBuf};

use orchard_theme::schemes::Appearance;
use serde::{Deserialize, Serialize};

/// `$ORCHARD_PASSWORD_MANAGER_CONFIG_DIR`, or the platform's config folder.
pub fn config_dir() -> PathBuf {
    orchard_shell::config::app_config_dir("ORCHARD_PASSWORD_MANAGER_CONFIG_DIR", "orchard-password-manager", "Orchard Password Manager")
}

/// The font size unless changed (like the other Orchard apps).
pub const DEFAULT_FONT_SIZE: i64 = 14;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // -- appearance (Commander's keys) ------------------------------------
    pub color_scheme: String,
    /// Use `light_scheme` / `dark_scheme` by the system's light or dark mode.
    pub follow_system_appearance: bool,
    pub light_scheme: String,
    pub dark_scheme: String,
    pub frame_style: String,
    pub font_family: String,
    pub font_style: String,
    pub font_size: i64,
    pub line_spacing: f64,

    // -- vault ------------------------------------------------------------
    /// The vault opened last, offered on the unlock screen.
    pub last_vault: Option<PathBuf>,

    // -- security ---------------------------------------------------------
    /// Lock after this long without input (0: never).
    pub lock_after_idle_secs: u32,
    /// Take a copied password off the clipboard after this long.
    pub clear_clipboard_secs: u32,
    /// A shown password hides again after this long.
    pub reveal_secs: u32,

    /// The password generator's choices, as last used.
    pub generator: crate::generator::Options,

    pub window_size: Option<(u32, u32)>,
}

impl Default for Settings {
    fn default() -> Self {
        let a = Appearance::default();
        Settings {
            color_scheme: a.color_scheme,
            follow_system_appearance: a.follow_system_appearance,
            light_scheme: a.light_scheme,
            dark_scheme: a.dark_scheme,
            frame_style: a.frame_style,
            font_family: a.font_family,
            font_style: a.font_style,
            font_size: DEFAULT_FONT_SIZE,
            line_spacing: a.line_spacing,
            last_vault: None,
            lock_after_idle_secs: 300,
            clear_clipboard_secs: 20,
            reveal_secs: 10,
            generator: Default::default(),
            window_size: None,
        }
    }
}

impl Settings {
    pub fn appearance(&self) -> Appearance {
        Appearance {
            color_scheme: self.color_scheme.clone(),
            follow_system_appearance: self.follow_system_appearance,
            light_scheme: self.light_scheme.clone(),
            dark_scheme: self.dark_scheme.clone(),
            frame_style: self.frame_style.clone(),
            font_family: self.font_family.clone(),
            font_style: self.font_style.clone(),
            font_size: self.font_size,
            line_spacing: self.line_spacing,
        }
    }

    pub fn load(dir: &Path) -> Settings {
        orchard_shell::config::load_json(&dir.join("settings.json"))
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        orchard_shell::config::save_json(&dir.join("settings.json"), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults_for_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings { lock_after_idle_secs: 60, last_vault: Some("/v.kdbx".into()), ..Settings::default() };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        std::fs::write(dir.path().join("settings.json"), r#"{"clear_clipboard_secs": 5}"#).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!((s.clear_clipboard_secs, s.lock_after_idle_secs), (5, 300));
    }
}
