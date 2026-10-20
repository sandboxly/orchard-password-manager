//! Warnings and errors go to `log.txt` in the config directory (next to
//! the crash log), for when something fails and the message alone doesn't
//! say enough. Never log a secret: entry ids, field names and error
//! texts only.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The file starts over once it is this big.
const MAX_BYTES: u64 = 256 * 1024;

struct FileLog(Mutex<File>);

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut f) = self.0.lock() {
            let _ = writeln!(f, "{secs} {} {}: {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.0.lock() {
            let _ = f.flush();
        }
    }
}

/// Log to `dir/log.txt` from now on (a failure to open it is ignored).
pub fn init(dir: &Path) {
    let path = dir.join("log.txt");
    let _ = std::fs::create_dir_all(dir);
    let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES);
    let file = OpenOptions::new().create(true).append(!fresh).write(true).truncate(fresh).open(&path);
    if let Ok(file) = file
        && log::set_logger(Box::leak(Box::new(FileLog(Mutex::new(file))))).is_ok()
    {
        log::set_max_level(log::LevelFilter::Warn);
    }
}
