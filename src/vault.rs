//! A KDBX vault on disk: opening it with its key, and saving it so that a
//! failed or bad write can never cost the user their passwords.
//!
//! The `keepass` crate does all of the crypto. Its KDBX4 writer is marked
//! experimental, so every save is checked before it replaces anything:
//! write "<name>.tmp", open that again with the same key and compare it with
//! what is in memory, keep the previous file as "<name>.bak", then move the
//! new one into place.
//!
//! The decrypted database and the key live only in [`Vault`] (and in a
//! [`Snapshot`] while it is being saved); dropping them wipes them (the
//! crate zeroizes protected fields and the key on drop).
//!
//! Saves run off the UI thread on a snapshot. One lock per vault orders
//! them and a change counter skips a snapshot older than what is already
//! on disk, so a slow save can never overwrite a newer one.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use keepass::config::{DatabaseConfig, DatabaseVersion, KdfConfig};
use keepass::db::{DatabaseOpenError, DatabaseSaveError};
use keepass::error::DatabaseKeyError;
use keepass::{Database, DatabaseKey};

pub use keepass::db::fields;

/// An open vault: the decrypted database, its key and where it lives.
pub struct Vault {
    pub path: PathBuf,
    pub db: Database,
    key: DatabaseKey,
    /// Counts changes to `db` ([`Vault::changed`]).
    generation: u64,
    /// The generation on disk; held while a save writes.
    written: Arc<Mutex<u64>>,
}

/// The vault as it was at one change, for saving on another thread.
pub struct Snapshot {
    path: PathBuf,
    db: Database,
    key: DatabaseKey,
    generation: u64,
    written: Arc<Mutex<u64>>,
}

/// Why a vault didn't open, in terms the UI can explain.
#[derive(Debug)]
pub enum OpenError {
    /// Wrong password or key file.
    WrongKey,
    NotFound,
    /// Not a KeePass file, or one this version can't read.
    NotAVault(String),
    Io(io::Error),
}

#[derive(Debug)]
pub enum SaveError {
    /// KDB and KDBX3 files open but can't be written.
    OldFormat,
    /// The file written didn't read back the same: nothing was replaced.
    Verify(String),
    Write(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::WrongKey => write!(f, "wrong key"),
            OpenError::NotFound => write!(f, "not found"),
            OpenError::NotAVault(e) => write!(f, "not a vault: {e}"),
            OpenError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::OldFormat => write!(f, "older vault format, can't be written"),
            SaveError::Verify(e) => write!(f, "the saved file didn't read back the same: {e}"),
            SaveError::Write(e) => write!(f, "{e}"),
        }
    }
}

/// The key from a master password (and later key files).
pub fn key(password: &str) -> DatabaseKey {
    DatabaseKey::new().with_password(password)
}

/// The key derivation for new vaults (and vaults updated from an older
/// format): Argon2id with 64 MiB, which takes a noticeable moment to open
/// (to be tuned). (The crate's default uses 1 MiB, far too little against
/// guessing.) Tests get a fast one.
pub fn new_vault_config() -> DatabaseConfig {
    if cfg!(test) { argon2id(1, 64 * 1024, 1) } else { argon2id(10, 64 * 1024 * 1024, 4) }
}

/// The version a vault is saved in. The crate writes KDBX 4.1 only, and
/// refuses a 4.0 vault; 4.1 only adds optional parts to 4.0, and apps that
/// read 4.0 read 4.1, so a 4.0 vault becomes 4.1 when it is opened (written
/// with the next change, without asking).
fn writable_version(version: DatabaseVersion) -> DatabaseVersion {
    let current = DatabaseConfig::default().version;
    match (&version, &current) {
        (DatabaseVersion::KDB4(minor), DatabaseVersion::KDB4(written)) if minor < written => current.clone(),
        _ => version,
    }
}

/// Formats that open but can't be saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldFormat {
    /// KDBX 3 (and its pre-release): updated in place, the old file kept
    /// as "<name>.bak".
    Kdbx3,
    /// The first-generation format (".kdb"): a copy in the current
    /// format goes next to it, the old file left alone.
    Kdb,
}

/// The crate's defaults with Argon2id (memory in bytes).
pub fn argon2id(iterations: u64, memory: u64, parallelism: u32) -> DatabaseConfig {
    let mut config = DatabaseConfig::default();
    config.kdf_config = KdfConfig::Argon2id { iterations, memory, parallelism, version: argon2::Version::Version13 };
    config
}

impl Vault {
    /// Open and decrypt the vault at `path`. Slow on purpose (the key
    /// derivation): call it off the UI thread.
    pub fn open(path: &Path, key: DatabaseKey) -> Result<Vault, OpenError> {
        let file = File::open(path).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => OpenError::NotFound,
            _ => OpenError::Io(e),
        })?;
        let mut db = Database::open(&mut BufReader::new(file), key.clone()).map_err(open_error)?;
        db.config.version = writable_version(db.config.version);
        Ok(Vault { path: path.to_path_buf(), db, key, generation: 0, written: Arc::new(Mutex::new(0)) })
    }

    /// A new, empty vault at `path` (not written until saved).
    pub fn create(path: &Path, key: DatabaseKey, config: DatabaseConfig) -> Vault {
        let mut db = Database::with_config(config);
        db.root_mut().name = "Passwords".into();
        Vault { path: path.to_path_buf(), db, key, generation: 1, written: Arc::new(Mutex::new(0)) }
    }

    /// Note a change to `db` (it needs saving).
    pub fn changed(&mut self) {
        self.generation += 1;
    }

    /// Changes not on disk yet (waits for a save that is writing).
    pub fn unsaved(&self) -> bool {
        *self.written.lock().unwrap_or_else(|e| e.into_inner()) < self.generation
    }

    /// Whether changes can be saved: only KDBX4 can be written.
    pub fn writable(&self) -> bool {
        matches!(self.db.config.version, keepass::config::DatabaseVersion::KDB4(_))
    }

    /// Why the vault can't be saved as it is, if it can't.
    pub fn old_format(&self) -> Option<OldFormat> {
        use keepass::config::DatabaseVersion as V;
        match self.db.config.version {
            V::KDB4(_) => None,
            V::KDB(_) => Some(OldFormat::Kdb),
            V::KDB2(_) | V::KDB3(_) => Some(OldFormat::Kdbx3),
        }
    }

    /// Where an old-format vault will be saved once updated: the same file,
    /// or for a ".kdb" the same name ending in ".kdbx".
    pub fn updated_path(&self) -> PathBuf {
        match self.old_format() {
            Some(OldFormat::Kdb) => self.path.with_extension("kdbx"),
            _ => self.path.clone(),
        }
    }

    /// Switch an old-format vault to the current format (in memory: the
    /// next save writes it, to `updated_path`). Fails if a ".kdb"'s copy
    /// would replace a file that is already there.
    pub fn update_format(&mut self) -> Result<(), PathBuf> {
        if self.old_format().is_none() {
            return Ok(());
        }
        let path = self.updated_path();
        if path != self.path && path.exists() {
            return Err(path);
        }
        // Inner encryption, key derivation and header all become KDBX4's;
        // the protected values are decrypted in memory, so nothing is lost.
        self.db.config = new_vault_config();
        if path != self.path {
            self.path = path;
            // Nothing of this vault is in the new file yet.
            self.written = Arc::new(Mutex::new(0));
        }
        self.changed();
        Ok(())
    }

    /// The vault as it is now, to save on another thread.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            path: self.path.clone(),
            db: self.db.clone(),
            key: self.key.clone(),
            generation: self.generation,
            written: self.written.clone(),
        }
    }

    /// Save now, on this thread ([`Snapshot::save`]).
    pub fn save(&self) -> Result<(), SaveError> {
        self.snapshot().save()
    }
}

impl Snapshot {
    /// Write the vault, checked (see the module comment), unless a newer
    /// snapshot is already on disk. On any error the file on disk is left
    /// as it was. Slow (the key derivation, twice): off the UI thread.
    pub fn save(self) -> Result<(), SaveError> {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if *written >= self.generation {
            return Ok(());
        }
        let tmp = sibling(&self.path, "tmp");
        let result = self.write_checked(&tmp);
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
            return result;
        }
        if self.path.exists() {
            fs::copy(&self.path, sibling(&self.path, "bak")).map_err(|e| SaveError::Write(format!("backup: {e}")))?;
        }
        fs::rename(&tmp, &self.path).map_err(|e| SaveError::Write(format!("replace: {e}")))?;
        *written = self.generation;
        Ok(())
    }

    fn write_checked(&self, tmp: &Path) -> Result<(), SaveError> {
        let file = File::create(tmp).map_err(|e| SaveError::Write(e.to_string()))?;
        let mut out = BufWriter::new(file);
        self.db.save(&mut out, self.key.clone()).map_err(|e| match e {
            DatabaseSaveError::UnsupportedVersion => SaveError::OldFormat,
            e => SaveError::Write(e.to_string()),
        })?;
        let file = out.into_inner().map_err(|e| SaveError::Write(e.to_string()))?;
        file.sync_all().map_err(|e| SaveError::Write(e.to_string()))?;
        drop(file);

        let back = File::open(tmp).map_err(|e| SaveError::Verify(e.to_string()))?;
        let reread = Database::open(&mut BufReader::new(back), self.key.clone()).map_err(|e| SaveError::Verify(e.to_string()))?;
        // Compared part by part: the crate may number attachments anew.
        if let Some(what) = crate::integrity::differences(&self.db, &reread) {
            return Err(SaveError::Verify(format!("{what} read back different")));
        }
        Ok(())
    }
}

#[cfg(test)]
impl Vault {
    /// A vault as if opened from a file in an older format.
    pub fn old(path: &Path, version: keepass::config::DatabaseVersion) -> Vault {
        let mut v = Vault::create(path, key("pw"), new_vault_config());
        v.db.config.version = version;
        v.generation = 0;
        v
    }
}

fn open_error(e: DatabaseOpenError) -> OpenError {
    match e {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => OpenError::WrongKey,
        DatabaseOpenError::Io(e) => OpenError::Io(e),
        e => OpenError::NotAVault(e.to_string()),
    }
}

/// "vault.kdbx" → "vault.kdbx.<ext>" in the same folder (so a rename is
/// atomic).
fn sibling(path: &Path, ext: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(ext);
    path.with_file_name(name)
}

/// `--self-test`'s check: a vault saves, checks and opens again here (in a
/// temporary folder, removed again).
pub fn self_test() -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("orchard-password-manager-self-test-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("self-test.kdbx");
    let result = (|| {
        let mut v = Vault::create(&path, key("self-test"), argon2id(1, 64 * 1024, 1));
        v.db.root_mut().add_entry().set_protected(fields::PASSWORD, "self-test");
        v.save().map_err(|e| e.to_string())?;
        let back = Vault::open(&path, key("self-test")).map_err(|e| e.to_string())?;
        match back.db.iter_all_entries().next().and_then(|e| e.get_password().map(str::to_string)) {
            Some(p) if p == "self-test" => Ok("a vault saves and opens again".to_string()),
            _ => Err("the saved vault lost its entry".to_string()),
        }
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fast key derivation: tests open and save many times.
    fn quick() -> DatabaseConfig {
        argon2id(1, 64 * 1024, 1)
    }

    fn sample(path: &Path) -> Vault {
        let mut v = Vault::create(path, key("correct horse"), quick());
        let mut root = v.db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, "Mail");
        e.set_unprotected(fields::USERNAME, "me@example.org");
        e.set_protected(fields::PASSWORD, "s3cret!");
        v
    }

    #[test]
    fn save_and_open_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        sample(&path).save().unwrap();
        let v = Vault::open(&path, key("correct horse")).unwrap();
        let e = v.db.iter_all_entries().next().unwrap();
        assert_eq!(e.get_title(), Some("Mail"));
        assert_eq!(e.get_password(), Some("s3cret!"));
        assert!(!sibling(&path, "tmp").exists());
    }

    #[test]
    fn wrong_password_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        sample(&path).save().unwrap();
        assert!(matches!(Vault::open(&path, key("wrong")), Err(OpenError::WrongKey)));
        assert!(matches!(Vault::open(&dir.path().join("none.kdbx"), key("x")), Err(OpenError::NotFound)));
    }

    #[test]
    fn second_save_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        let mut v = sample(&path);
        v.save().unwrap();
        v.db.root_mut().add_entry().set_unprotected(fields::TITLE, "Bank");
        v.changed();
        v.save().unwrap();
        let old = Vault::open(&sibling(&path, "bak"), key("correct horse")).unwrap();
        assert_eq!(old.db.num_entries(), 1);
        assert_eq!(Vault::open(&path, key("correct horse")).unwrap().db.num_entries(), 2);
    }

    #[test]
    fn an_older_snapshot_never_overwrites_a_newer_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        let mut v = sample(&path);
        let old = v.snapshot();
        v.db.root_mut().add_entry().set_unprotected(fields::TITLE, "Bank");
        v.changed();
        assert!(v.unsaved());
        v.save().unwrap();
        assert!(!v.unsaved());
        old.save().unwrap();
        assert_eq!(Vault::open(&path, key("correct horse")).unwrap().db.num_entries(), 2);
    }

    #[test]
    fn a_failed_save_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        let mut v = sample(&path);
        v.save().unwrap();
        let before = fs::read(&path).unwrap();
        v.db.root_mut().add_entry().set_unprotected(fields::TITLE, "Bank");
        v.changed();
        // The temporary file can't be created: a folder is in its way.
        fs::create_dir(sibling(&path, "tmp")).unwrap();
        assert!(v.save().is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(v.unsaved());
    }

    #[test]
    fn a_kdbx_4_0_vault_is_saved_as_4_1() {
        assert_eq!(writable_version(DatabaseVersion::KDB4(0)), DatabaseVersion::KDB4(1));
        assert_eq!(writable_version(DatabaseVersion::KDB4(1)), DatabaseVersion::KDB4(1));
        assert_eq!(writable_version(DatabaseVersion::KDB3(1)), DatabaseVersion::KDB3(1));
        // The crate can't write 4.0 itself: once it says 4.1, the save goes through.
        let dir = tempfile::tempdir().unwrap();
        let mut v = sample(&dir.path().join("v.kdbx"));
        v.db.config.version = DatabaseVersion::KDB4(0);
        assert!(matches!(v.save(), Err(SaveError::OldFormat)));
        v.db.config.version = writable_version(v.db.config.version.clone());
        v.save().unwrap();
    }

    #[test]
    fn an_old_format_vault_updates_in_place() {
        use keepass::config::DatabaseVersion;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        let mut v = Vault::old(&path, DatabaseVersion::KDB3(1));
        v.db.root_mut().add_entry().set_protected(fields::PASSWORD, "kept");
        v.changed();
        assert_eq!(v.old_format(), Some(OldFormat::Kdbx3));
        assert!(matches!(v.save(), Err(SaveError::OldFormat)));
        v.update_format().unwrap();
        assert!(v.writable());
        assert!(v.unsaved());
        v.save().unwrap();
        let back = Vault::open(&path, key("pw")).unwrap();
        assert_eq!(back.db.iter_all_entries().next().unwrap().get_password(), Some("kept"));
    }

    #[test]
    fn a_first_generation_vault_updates_into_a_copy() {
        use keepass::config::DatabaseVersion;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdb");
        fs::write(&path, b"old").unwrap();
        let mut v = Vault::old(&path, DatabaseVersion::KDB(0));
        // A copy would replace a file that's there: refused.
        fs::write(dir.path().join("v.kdbx"), b"other").unwrap();
        assert_eq!(v.update_format(), Err(dir.path().join("v.kdbx")));
        fs::remove_file(dir.path().join("v.kdbx")).unwrap();
        v.update_format().unwrap();
        assert_eq!(v.path, dir.path().join("v.kdbx"));
        v.save().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"old");
        Vault::open(&v.path, key("pw")).unwrap();
    }
}
