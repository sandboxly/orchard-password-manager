//! Copying secrets: a password put on the clipboard is taken off again
//! after a while, on lock and on quit.
//!
//! One thread owns the clipboard (on X11 and Wayland the copying process
//! has to keep serving its content) and the secret on it. When the time is
//! up it clears the clipboard, but only if it still holds our secret: what
//! the user copied since stays. Copies are marked so clipboard managers
//! leave them out of their history. Our copy of the secret is wiped when it
//! is dropped.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

/// The system clipboard as the worker uses it (a fake one in tests).
pub trait Board {
    fn get(&mut self) -> Option<Zeroizing<String>>;
    /// Put a secret on the clipboard, kept out of clipboard histories.
    fn set_secret(&mut self, text: &str) -> Result<(), String>;
    fn clear(&mut self);
}

enum Cmd {
    Copy(Zeroizing<String>, Duration),
    /// Take our secret off now; the reply says it's done.
    Clear(Option<Sender<()>>),
}

/// The handle the app keeps; dropping it clears the clipboard.
pub struct SecretClipboard {
    tx: Sender<Cmd>,
}

impl SecretClipboard {
    /// The system clipboard, on its own thread.
    pub fn system() -> SecretClipboard {
        SecretClipboard::with(SystemBoard::default())
    }

    pub fn with(board: impl Board + Send + 'static) -> SecretClipboard {
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || worker(board, rx));
        SecretClipboard { tx }
    }

    /// Put `secret` on the clipboard for `keep` (then it's cleared). A
    /// newer copy replaces it and starts the time again.
    pub fn copy(&self, secret: Zeroizing<String>, keep: Duration) {
        let _ = self.tx.send(Cmd::Copy(secret, keep));
    }

    /// Clear our secret now, waiting a moment for it to happen (locking,
    /// quitting: on some systems the clipboard outlives the app).
    pub fn clear_now(&self) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(Cmd::Clear(Some(done))).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(1));
        }
    }
}

impl Drop for SecretClipboard {
    fn drop(&mut self) {
        self.clear_now();
    }
}

fn worker(mut board: impl Board, rx: Receiver<Cmd>) {
    let mut held: Option<(Zeroizing<String>, Instant)> = None;
    loop {
        let cmd = match &held {
            Some((_, until)) => match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    take_off(&mut board, &mut held);
                    return;
                }
            },
            None => match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => return,
            },
        };
        match cmd {
            Some(Cmd::Copy(secret, keep)) => match board.set_secret(&secret) {
                Ok(()) => held = Some((secret, Instant::now() + keep)),
                Err(e) => {
                    log::warn!("clipboard: {e}");
                    held = None;
                }
            },
            Some(Cmd::Clear(done)) => {
                take_off(&mut board, &mut held);
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
            // Time's up.
            None => take_off(&mut board, &mut held),
        }
    }
}

/// Clear the clipboard if it still has our secret; forget (wipe) the secret.
fn take_off(board: &mut impl Board, held: &mut Option<(Zeroizing<String>, Instant)>) {
    if let Some((secret, _)) = held.take()
        && board.get().is_some_and(|now| *now == *secret)
    {
        board.clear();
    }
}

/// The real clipboard, opened on first use.
#[derive(Default)]
struct SystemBoard {
    clipboard: Option<arboard::Clipboard>,
}

impl SystemBoard {
    fn open(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        Ok(self.clipboard.as_mut().expect("opened"))
    }
}

impl Board for SystemBoard {
    fn get(&mut self) -> Option<Zeroizing<String>> {
        self.open().ok()?.get_text().ok().map(Zeroizing::new)
    }

    fn set_secret(&mut self, text: &str) -> Result<(), String> {
        let set = self.open()?.set();
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
        let set = arboard::SetExtLinux::exclude_from_history(set);
        #[cfg(target_os = "macos")]
        let set = arboard::SetExtApple::exclude_from_history(set);
        #[cfg(windows)]
        let set = {
            use arboard::SetExtWindows;
            set.exclude_from_history().exclude_from_cloud().exclude_from_monitoring()
        };
        set.text(text).map_err(|e| e.to_string())
    }

    fn clear(&mut self) {
        if let Ok(c) = self.open()
            && let Err(e) = c.clear()
        {
            log::warn!("clipboard: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A clipboard in memory, shared with the test.
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Option<String>>>);

    impl Board for Fake {
        fn get(&mut self) -> Option<Zeroizing<String>> {
            self.0.lock().unwrap().clone().map(Zeroizing::new)
        }
        fn set_secret(&mut self, text: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(text.to_string());
            Ok(())
        }
        fn clear(&mut self) {
            *self.0.lock().unwrap() = None;
        }
    }

    fn wait_for(fake: &Fake, want: Option<&str>) -> bool {
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if fake.0.lock().unwrap().as_deref() == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn cleared_after_the_time() {
        let fake = Fake::default();
        let c = SecretClipboard::with(fake.clone());
        c.copy(Zeroizing::new("hunter2".into()), Duration::from_millis(50));
        assert!(wait_for(&fake, Some("hunter2")));
        assert!(wait_for(&fake, None));
    }

    #[test]
    fn what_the_user_copied_since_stays() {
        let fake = Fake::default();
        let c = SecretClipboard::with(fake.clone());
        c.copy(Zeroizing::new("hunter2".into()), Duration::from_millis(50));
        assert!(wait_for(&fake, Some("hunter2")));
        *fake.0.lock().unwrap() = Some("a recipe".into());
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.0.lock().unwrap().as_deref(), Some("a recipe"));
        drop(c);
        assert_eq!(fake.0.lock().unwrap().as_deref(), Some("a recipe"));
    }

    #[test]
    fn cleared_at_once_on_lock_and_on_quit() {
        let fake = Fake::default();
        let c = SecretClipboard::with(fake.clone());
        c.copy(Zeroizing::new("hunter2".into()), Duration::from_secs(60));
        c.clear_now();
        assert_eq!(*fake.0.lock().unwrap(), None);
        c.copy(Zeroizing::new("hunter3".into()), Duration::from_secs(60));
        drop(c);
        assert_eq!(*fake.0.lock().unwrap(), None);
    }
}
