//! The window: the shared shell (`orchard_shell`: winit event loop, GPU
//! surface, input conversion, wake-ups from workers) around `App`, plus
//! what only this app does there. Everything else is in `App`.

// Windows: a GUI program, no console window behind it. Output still reaches
// a redirected stdout; in a console window the flags that print attach to it
// (`orchard_shell::cli::attach_parent_console`).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use orchard_grid::{FontConfig, Frame, GridRenderer, Rgba};
use orchard_kit::input::{KeyEvent, MouseEvent};
use orchard_password_manager::App;
use orchard_password_manager::app::{Screen, Unlock};
use orchard_password_manager::ui;
use orchard_shell::winit::window::Window;
use orchard_shell::{AppInfo, ShellApp, cli};

mod screenshot;

const INFO: AppInfo = AppInfo {
    name: "Orchard Password Manager",
    bin: "orchard-password-manager",
    version: env!("CARGO_PKG_VERSION"),
    default_size: (1000, 680),
    min_size: (560, 360),
    exit_after_start_env: "ORCHARD_PASSWORD_MANAGER_EXIT_AFTER_START",
};

/// The app in the shell's window.
struct Shell {
    app: App,
    /// The window title shown.
    title: String,
}

impl ShellApp for Shell {
    fn font(&self) -> &FontConfig {
        &self.app.theme.font
    }

    fn background(&self) -> Rgba {
        self.app.theme.colors.background
    }

    fn grid(&self) -> (u16, u16) {
        (self.app.cols, self.app.rows)
    }

    fn window_size(&self) -> Option<(u32, u32)> {
        self.app.settings.window_size
    }

    fn set_window_size(&mut self, logical: (u32, u32)) {
        self.app.settings.window_size = Some(logical);
    }

    fn quit_requested(&self) -> bool {
        self.app.quit
    }

    fn set_system_dark(&mut self, dark: bool) {
        self.app.set_system_dark(dark);
    }

    fn resized(&mut self, cols: u16, rows: u16, cell_px: (f32, f32)) {
        self.app.resized(cols, rows, cell_px);
    }

    fn tick(&mut self, now: Instant) {
        self.app.tick(now);
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw(&self.app, frame);
    }

    fn process(&mut self) -> bool {
        self.app.process()
    }

    fn next_frame(&self, now: Instant) -> Option<Instant> {
        self.app.next_frame(now)
    }

    fn handle_key(&mut self, ev: &KeyEvent) {
        self.app.handle_key(ev);
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) {
        self.app.handle_mouse(ev);
    }

    /// Quitting: locks the vault and takes secrets off the clipboard.
    fn save(&mut self) {
        self.app.shutdown();
    }

    fn before_draw(&mut self, window: &Window, _renderer: &mut GridRenderer) {
        let title = self.app.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }
}

fn usage() -> &'static str {
    "usage: orchard-password-manager [VAULT.kdbx]\n       orchard-password-manager --version | --self-test\n       orchard-password-manager --screenshot OUT.png [--size COLSxROWS] [--theme NAME] [--dialog unlock|create|error|edit|new|groups|generate|discard|bin|empty|update] [--reveal] [--search TEXT] [--cursor N]"
}

/// `--self-test`: drawing and the vault format work in this build (for
/// packaging checks; no window).
fn self_test() -> ExitCode {
    cli::self_test(
        &INFO,
        vec![
            ("render", Box::new(|| cli::render_check(&INFO, screenshot::run))),
            ("vault", Box::new(orchard_password_manager::vault::self_test)),
        ],
    )
}

fn main() -> ExitCode {
    #[cfg(windows)]
    cli::attach_parent_console();
    cli::log_panics(&INFO, orchard_password_manager::settings::config_dir);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::common_args(&INFO, &args, usage(), self_test, screenshot::run) {
        return code;
    }
    // macOS passes -psn_… to apps started from the Finder.
    let vault: Option<PathBuf> = args.into_iter().find(|a| !a.starts_with('-')).map(PathBuf::from);

    orchard_password_manager::logging::init(&orchard_password_manager::settings::config_dir());
    orchard_shell::run(&INFO, |wake| {
        let mut app = App::new(wake, true);
        if let Some(path) = vault {
            app.screen = Screen::Unlock(Unlock::new(Some(&path)));
        }
        Shell { app, title: String::new() }
    })
}
