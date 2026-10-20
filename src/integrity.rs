//! Guards around two gaps in the keepass crate (0.15) that concern
//! attachments; everything else is compared and kept as the crate has it.
//!
//! - The crate numbers attachments, and its writer renumbers them when
//!   there are gaps (after an entry with one was deleted for good), so a
//!   file read back can hold the same attachments under other numbers.
//!   [`differences`] is the save's check: every part of the database
//!   compared, attachments by file name and content.
//! - Each attachment keeps a list of the entries that use it, and an entry
//!   deleted for good takes the attachments whose list is then empty with
//!   it. The crate's reader leaves those lists empty (it fills them only
//!   for icons), so after opening a file a deletion could take an
//!   attachment another entry shares. [`delete_keeping_attachments`] puts
//!   such attachments back.
//!
//! What these report never holds a value: entry ids, field and file names.

use std::collections::{BTreeMap, HashMap};

use keepass::Database;
use keepass::db::{AttachmentId, EntryId, EntryRef, Value};

/// The first way `b` differs from `a`, if any (`None`: the same).
pub fn differences(a: &Database, b: &Database) -> Option<String> {
    if a.config != b.config {
        return Some("the database settings".into());
    }
    if a.meta != b.meta {
        return Some("the database's metadata".into());
    }
    if a.deleted_objects != b.deleted_objects {
        return Some("the list of deleted objects".into());
    }
    if a.root().id() != b.root().id() {
        return Some("the root group".into());
    }
    let groups = |db: &Database| db.iter_all_groups().map(|g| (g.id(), (*g).clone())).collect::<HashMap<_, _>>();
    let (ga, gb) = (groups(a), groups(b));
    if ga.len() != gb.len() {
        return Some(format!("the number of groups ({} → {})", ga.len(), gb.len()));
    }
    for (id, g) in &ga {
        if gb.get(id) != Some(g) {
            return Some(format!("group {}", id.uuid()));
        }
    }
    let icons = |db: &Database| {
        db.iter_all_custom_icons().map(|i| (i.id().uuid(), (i.name.clone(), i.last_modification_time, i.data.clone()))).collect::<HashMap<_, _>>()
    };
    if icons(a) != icons(b) {
        return Some("the custom icons".into());
    }
    let ids = |db: &Database| db.iter_all_entries().map(|e| e.id()).collect::<std::collections::HashSet<_>>();
    let (ea, eb) = (ids(a), ids(b));
    if ea != eb {
        return Some(format!("the entries ({} → {})", ea.len(), eb.len()));
    }
    for id in ea {
        let (Some(x), Some(y)) = (a.entry(id), b.entry(id)) else { return Some(format!("entry {}", id.uuid())) };
        if let Some(what) = entry_differences(&x, &y) {
            return Some(format!("entry {}: {what}", id.uuid()));
        }
        let n = x.history.as_ref().map_or(0, |h| h.get_entries().len());
        if n != y.history.as_ref().map_or(0, |h| h.get_entries().len()) {
            return Some(format!("entry {}: the number of history versions", id.uuid()));
        }
        for i in 0..n {
            let (Some(hx), Some(hy)) = (x.historical(i), y.historical(i)) else { continue };
            if let Some(what) = entry_differences(&hx, &hy) {
                return Some(format!("entry {} history version {i}: {what}", id.uuid()));
            }
        }
    }
    None
}

/// One version of an entry against another (not its history).
fn entry_differences(a: &EntryRef, b: &EntryRef) -> Option<String> {
    if a.fields != b.fields {
        let mut names: Vec<&String> = a.fields.keys().chain(b.fields.keys()).filter(|k| a.fields.get(*k) != b.fields.get(*k)).collect();
        names.sort();
        names.dedup();
        return Some(format!("field {}", names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")));
    }
    let checks: [(&str, bool); 11] = [
        ("auto-type", a.autotype == b.autotype),
        ("tags", a.tags == b.tags),
        ("times", a.times == b.times),
        ("custom data", a.custom_data == b.custom_data),
        ("icon", a.icon() == b.icon()),
        ("colors", a.foreground_color == b.foreground_color && a.background_color == b.background_color),
        ("URL override", a.override_url == b.override_url),
        ("quality check", a.quality_check == b.quality_check),
        ("group", a.parent().id() == b.parent().id()),
        ("previous group", a.previous_parent().map(|g| g.id()) == b.previous_parent().map(|g| g.id())),
        ("attachments", attachments(a) == attachments(b)),
    ];
    checks.iter().find(|(_, same)| !same).map(|(what, _)| what.to_string())
}

/// File name → content.
fn attachments(e: &EntryRef) -> BTreeMap<String, Value<Vec<u8>>> {
    e.attachments_named().map(|(name, a)| (name.to_string(), a.data.clone())).collect()
}

/// The attachments an entry version uses: (entry, history version) →
/// [(file name, id, content)].
type Uses = Vec<((EntryId, Option<usize>), Vec<(String, AttachmentId, Value<Vec<u8>>)>)>;

fn uses(db: &Database) -> Uses {
    let mut out = Vec::new();
    for e in db.iter_all_entries() {
        let n = e.history.as_ref().map_or(0, |h| h.get_entries().len());
        let versions = std::iter::once((None, db.entry(e.id()))).chain((0..n).map(|i| (Some(i), e.historical(i))));
        for (i, v) in versions {
            let Some(v) = v else { continue };
            let list: Vec<_> = v.attachments_named().map(|(name, a)| (name.to_string(), a.id(), a.data.clone())).collect();
            if !list.is_empty() {
                out.push(((e.id(), i), list));
            }
        }
    }
    out
}

/// Run `delete` (entries or groups deleted for good), then give the entries
/// that are left back any attachment it took from them. `Err` (and the
/// database as it was) when one of their older versions lost one: those
/// can't be given one back through the crate.
pub fn delete_keeping_attachments<T>(db: &mut Database, delete: impl FnOnce(&mut Database) -> T) -> Result<T, String> {
    let before = db.clone();
    let used = uses(db);
    let out = delete(db);
    for ((id, version), list) in used {
        if db.entry(id).is_none() {
            continue; // deleted itself
        }
        for (name, attachment, data) in list {
            if db.attachment(attachment).is_some() {
                continue;
            }
            match version {
                None => {
                    if let Some(mut e) = db.entry_mut(id) {
                        e.add_attachment(name, data);
                    }
                }
                Some(_) => {
                    *db = before;
                    return Err(format!("entry {}: an older version shares an attachment with what was deleted", id.uuid()));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;

    use keepass::db::fields;

    use super::*;
    use crate::entries;
    use crate::vault::{self, Vault};

    /// A file with three entries, each with an attachment (and A with a
    /// history version sharing it), opened again: the way a vault from
    /// another app arrives.
    fn opened(dir: &Path) -> Vault {
        let path = dir.join("v.kdbx");
        let mut v = Vault::create(&path, vault::key("pw"), vault::argon2id(1, 64 * 1024, 1));
        for t in ["A", "B", "C"] {
            let mut root = v.db.root_mut();
            let mut e = root.add_entry();
            e.set_unprotected(fields::TITLE, t);
            e.add_attachment(format!("{t}.txt"), Value::unprotected(t.as_bytes().to_vec()));
        }
        let a = id(&v, "A");
        v.db.entry_mut(a).unwrap().track_changes().set_unprotected(fields::USERNAME, "me");
        // Written without the check: new attachments fail the crate's own
        // comparison (their entry lists aren't in the file).
        let mut out = File::create(&path).unwrap();
        v.db.save(&mut out, vault::key("pw")).unwrap();
        drop(out);
        Vault::open(&path, vault::key("pw")).unwrap()
    }

    fn id(v: &Vault, title: &str) -> EntryId {
        v.db.iter_all_entries().find(|e| e.get_title() == Some(title)).unwrap().id()
    }

    fn files(v: &Vault, title: &str) -> Vec<(String, Vec<u8>)> {
        let e = v.db.entry(id(v, title)).unwrap();
        e.attachments_named().map(|(n, a)| (n.to_string(), a.data.get().to_vec())).collect()
    }

    #[test]
    fn deleting_for_good_with_attachments_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = opened(dir.path());
        let b = id(&v, "B");
        entries::delete_for_good(&mut v.db, b).unwrap();
        v.changed();
        v.save().expect("saved, though the attachments are numbered anew");
        let back = Vault::open(&dir.path().join("v.kdbx"), vault::key("pw")).unwrap();
        assert!(back.db.iter_all_entries().all(|e| e.get_title() != Some("B")));
        assert_eq!(files(&back, "A"), [("A.txt".to_string(), b"A".to_vec())]);
        assert_eq!(files(&back, "C"), [("C.txt".to_string(), b"C".to_vec())]);
        assert_eq!(back.db.iter_all_attachments().count(), 2);
    }

    #[test]
    fn emptying_the_bin_with_attachments_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = opened(dir.path());
        for t in ["A", "C"] {
            let e = id(&v, t);
            entries::delete(&mut v.db, e);
        }
        assert_eq!(entries::empty_bin(&mut v.db), Ok(2));
        v.changed();
        v.save().expect("saved");
        let back = Vault::open(&dir.path().join("v.kdbx"), vault::key("pw")).unwrap();
        assert_eq!(files(&back, "B"), [("B.txt".to_string(), b"B".to_vec())]);
    }

    #[test]
    fn a_difference_is_named_without_its_value() {
        let dir = tempfile::tempdir().unwrap();
        let v = opened(dir.path());
        assert_eq!(differences(&v.db, &v.db.clone()), None);
        let mut other = v.db.clone();
        let a = id(&v, "A");
        other.entry_mut(a).unwrap().set_protected(fields::PASSWORD, "s3cret");
        let what = differences(&v.db, &other).unwrap();
        assert_eq!(what, format!("entry {}: field \"Password\"", a.uuid()));
        assert!(!what.contains("s3cret"));
        let mut other = v.db.clone();
        other.entry_mut(a).unwrap().add_attachment("A.txt", Value::unprotected(b"changed".to_vec()));
        assert_eq!(differences(&v.db, &other).unwrap(), format!("entry {}: attachments", a.uuid()));
    }
}
