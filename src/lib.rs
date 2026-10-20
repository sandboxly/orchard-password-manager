//! Orchard Password Manager: KDBX vaults in a themeable character-grid
//! window (`orchard-grid`, shared with the other Orchard apps).

pub mod app;
pub mod clipboard;
pub mod entries;
pub mod generator;
pub mod integrity;
pub mod logging;
pub mod otp;
pub mod secret;
pub mod settings;
pub mod ui;
pub mod vault;

pub use app::App;
