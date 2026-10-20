//! What the app reads from and changes in an open vault's entries, on top
//! of the `keepass` crate: the list's rows, the groups, and adding,
//! changing and deleting entries the way KDBX expects (a changed entry
//! keeps its previous version in its history; a deleted one goes to the
//! recycle bin, from where it can be restored or deleted for good).

use keepass::Database;
use keepass::db::{EntryId, GroupId, Times};
use zeroize::Zeroizing;

use crate::otp;
use crate::vault::fields;

/// An entry as the list shows it (nothing secret).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: EntryId,
    pub title: String,
    pub username: String,
    pub url: String,
    /// "Group / Subgroup", without the root.
    pub group: String,
    /// The group it is in (in the recycle bin: the bin's group).
    pub group_id: GroupId,
}

/// A group in the tree the group filter shows.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupNode {
    pub id: GroupId,
    /// 0 for the root.
    pub depth: usize,
    pub name: String,
    /// Entries directly in it.
    pub entries: usize,
}

/// An entry being edited: its fields as the editor holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub title: String,
    pub username: String,
    pub password: Zeroizing<String>,
    pub url: String,
    /// The one-time password as stored (see `otp::stored`); empty: none.
    pub otp: Zeroizing<String>,
    pub notes: String,
    pub group: GroupId,
}

/// The recycle bin's entries (in it and in groups in it), by title; the
/// group column says where each came from.
pub fn bin_rows(db: &Database) -> Vec<Row> {
    let Some(bin) = db.recycle_bin().map(|g| g.id()) else { return Vec::new() };
    let paths: std::collections::HashMap<GroupId, String> = groups(db).into_iter().collect();
    let root = db.root().id();
    let mut rows = Vec::new();
    let mut stack = vec![bin];
    while let Some(id) = stack.pop() {
        let Some(g) = db.group(id) else { continue };
        stack.extend(g.groups().map(|s| s.id()));
        for e in g.entries() {
            let from = restore_group(db, e.id()).filter(|&g| g != root);
            rows.push(Row {
                id: e.id(),
                title: e.get_title().unwrap_or("").to_string(),
                username: e.get_username().unwrap_or("").to_string(),
                url: e.get_url().unwrap_or("").to_string(),
                group: from.and_then(|g| paths.get(&g).cloned()).unwrap_or_default(),
                group_id: id,
            });
        }
    }
    rows.sort_by_key(|r| r.title.to_lowercase());
    rows
}

/// Whether the entry is in the recycle bin (or a group in it).
pub fn in_bin(db: &Database, id: EntryId) -> bool {
    let Some(bin) = db.recycle_bin().map(|g| g.id()) else { return false };
    let Some(e) = db.entry(id) else { return false };
    let mut g = Some(e.parent().id());
    while let Some(id) = g {
        if id == bin {
            return true;
        }
        g = db.group(id).and_then(|group| group.parent().map(|p| p.id()));
    }
    false
}

/// Where a binned entry goes back to: the group it was deleted from if
/// that is still there (and not binned itself), else the root.
fn restore_group(db: &Database, id: EntryId) -> Option<GroupId> {
    let e = db.entry(id)?;
    let listed: Vec<GroupId> = groups(db).into_iter().map(|(g, _)| g).collect();
    Some(e.previous_parent().map(|g| g.id()).filter(|g| listed.contains(g)).unwrap_or(db.root().id()))
}

/// Take the entry out of the recycle bin, back where it was. Returns the
/// group it went to.
pub fn restore(db: &mut Database, id: EntryId) -> Option<GroupId> {
    if !in_bin(db, id) {
        return None;
    }
    let to = restore_group(db, id)?;
    move_entry(&mut db.entry_mut(id)?, to);
    Some(to)
}

/// Delete the entry for good (noted as deleted, for apps that merge).
/// `Err` (nothing deleted) if that would take an attachment an older
/// version of another entry uses (see `integrity`).
pub fn delete_for_good(db: &mut Database, id: EntryId) -> Result<(), String> {
    crate::integrity::delete_keeping_attachments(db, |db| remove(db, id))
}

fn remove(db: &mut Database, id: EntryId) {
    if let Some(mut e) = db.entry_mut(id) {
        e.track_changes().remove();
    }
}

/// Delete everything in the recycle bin for good. Returns how many
/// entries; `Err` as for `delete_for_good`.
pub fn empty_bin(db: &mut Database) -> Result<usize, String> {
    let n = bin_rows(db).len();
    let Some(bin) = db.recycle_bin().map(|g| g.id()) else { return Ok(0) };
    let (entries, subs): (Vec<EntryId>, Vec<GroupId>) = {
        let g = db.group(bin).expect("the bin");
        (g.entries().map(|e| e.id()).collect(), g.groups().map(|s| s.id()).collect())
    };
    crate::integrity::delete_keeping_attachments(db, |db| {
        for id in entries {
            remove(db, id);
        }
        for id in subs {
            if let Some(mut g) = db.group_mut(id) {
                let _ = g.track_changes().remove();
            }
        }
    })?;
    Ok(n)
}

/// The built-in icon KDBX apps use for the recycle bin.
const RECYCLE_BIN_ICON: usize = 43;

/// The list's rows: every entry outside the recycle bin, by title.
pub fn rows(db: &Database) -> Vec<Row> {
    let mut rows = Vec::new();
    for (id, path) in groups(db) {
        let Some(g) = db.group(id) else { continue };
        let group = if id == db.root().id() { String::new() } else { path };
        for e in g.entries() {
            rows.push(Row {
                id: e.id(),
                title: e.get_title().unwrap_or("").to_string(),
                username: e.get_username().unwrap_or("").to_string(),
                url: e.get_url().unwrap_or("").to_string(),
                group: group.clone(),
                group_id: id,
            });
        }
    }
    rows.sort_by_key(|r| r.title.to_lowercase());
    rows
}

/// The groups as a tree (the recycle bin left out), in the order of
/// `groups`: a group, then its subgroups by name.
pub fn group_tree(db: &Database) -> Vec<GroupNode> {
    let bin = db.recycle_bin().map(|g| g.id());
    let mut out = Vec::new();
    let mut stack = vec![(db.root().id(), 0)];
    while let Some((id, depth)) = stack.pop() {
        let Some(g) = db.group(id) else { continue };
        out.push(GroupNode { id, depth, name: g.name.clone(), entries: g.entries().count() });
        let mut subs: Vec<_> = g.groups().filter(|s| Some(s.id()) != bin).map(|s| (s.id(), s.name.to_lowercase())).collect();
        subs.sort_by(|a, b| b.1.cmp(&a.1));
        stack.extend(subs.into_iter().map(|(s, _)| (s, depth + 1)));
    }
    out
}

/// Every group but the recycle bin (and what's in it), with its path
/// ("Work / Servers"; the root by its name), in tree order.
pub fn groups(db: &Database) -> Vec<(GroupId, String)> {
    let bin = db.recycle_bin().map(|g| g.id());
    let root = db.root();
    let mut out = Vec::new();
    let mut stack = vec![(root.id(), None::<String>)];
    while let Some((id, path)) = stack.pop() {
        let Some(g) = db.group(id) else { continue };
        let label = path.clone().unwrap_or_else(|| g.name.clone());
        out.push((id, label));
        let mut subs: Vec<_> = g.groups().filter(|s| Some(s.id()) != bin).map(|s| (s.id(), s.name.clone())).collect();
        subs.sort_by_key(|(_, name)| std::cmp::Reverse(name.to_lowercase()));
        for (sub, name) in subs {
            let p = match &path {
                Some(p) => format!("{p} / {name}"),
                None => name,
            };
            stack.push((sub, Some(p)));
        }
    }
    out
}

/// A group's path as `groups` gives it.
pub fn group_path(db: &Database, id: GroupId) -> String {
    groups(db).into_iter().find(|(g, _)| *g == id).map(|(_, p)| p).unwrap_or_default()
}

/// The entry's fields, to edit.
pub fn draft(db: &Database, id: EntryId) -> Option<Draft> {
    let e = db.entry(id)?;
    Some(Draft {
        title: e.get_title().unwrap_or("").to_string(),
        username: e.get_username().unwrap_or("").to_string(),
        password: Zeroizing::new(e.get_password().unwrap_or("").to_string()),
        url: e.get_url().unwrap_or("").to_string(),
        otp: otp::stored(&e).and_then(Result::ok).unwrap_or_default(),
        notes: e.get(fields::NOTES).unwrap_or("").to_string(),
        group: e.parent().id(),
    })
}

/// A new entry from `d`.
pub fn add(db: &mut Database, d: &Draft) -> Option<EntryId> {
    let mut g = db.group_mut(d.group)?;
    let mut e = g.add_entry();
    e.set_unprotected(fields::TITLE, d.title.as_str());
    e.set_unprotected(fields::USERNAME, d.username.as_str());
    e.set_protected(fields::PASSWORD, d.password.as_str());
    e.set_unprotected(fields::URL, d.url.as_str());
    e.set_unprotected(fields::NOTES, d.notes.as_str());
    if !d.otp.is_empty() {
        e.set_protected(fields::OTP, d.otp.as_str());
    }
    Some(e.id())
}

/// Change the entry to `d`; its previous version goes into its history.
/// `false`: nothing changed (no history added).
pub fn update(db: &mut Database, id: EntryId, d: &Draft) -> bool {
    let Some(before) = draft(db, id) else { return false };
    let fields_changed = d.title != before.title
        || d.username != before.username
        || d.password != before.password
        || d.url != before.url
        || d.otp != before.otp
        || d.notes != before.notes;
    let moved = d.group != before.group && db.group(d.group).is_some();
    if !fields_changed && !moved {
        return false;
    }
    let Some(mut e) = db.entry_mut(id) else { return false };
    // Move first: a history version can't keep a group of its own in the
    // file (it takes the entry's on reading), so one made before the move
    // would read back different and fail the save's check.
    if moved {
        move_entry(&mut e, d.group);
    }
    if !fields_changed {
        return true;
    }
    let mut t = e.track_changes();
    if d.title != before.title {
        t.set_unprotected(fields::TITLE, d.title.as_str());
    }
    if d.username != before.username {
        t.set_unprotected(fields::USERNAME, d.username.as_str());
    }
    if d.password != before.password {
        t.set_protected(fields::PASSWORD, d.password.as_str());
    }
    if d.url != before.url {
        t.set_unprotected(fields::URL, d.url.as_str());
    }
    if d.notes != before.notes {
        t.set_unprotected(fields::NOTES, d.notes.as_str());
    }
    // Written as an "otp" link; KeePass's own fields would disagree with it.
    if d.otp != before.otp {
        for f in otp::TIME_OTP_FIELDS {
            t.fields.remove(f);
        }
        if d.otp.is_empty() {
            t.fields.remove(fields::OTP);
        } else {
            t.set_protected(fields::OTP, d.otp.as_str());
        }
    }
    true
}

/// Delete the entry: into the recycle bin (made when first needed), or
/// for good if the vault has the bin turned off. `true`: it went to the bin.
pub fn delete(db: &mut Database, id: EntryId) -> bool {
    if db.entry(id).is_none() {
        return false;
    }
    if db.meta.recyclebin_enabled == Some(false) {
        if let Some(mut e) = db.entry_mut(id) {
            e.track_changes().remove();
        }
        return false;
    }
    let bin = recycle_bin(db);
    if let Some(mut e) = db.entry_mut(id) {
        move_entry(&mut e, bin);
    }
    db.meta.recyclebin_changed = Some(Times::now());
    true
}

/// Move to another group (an existing one), noting when. No history
/// version: see `update`.
fn move_entry(e: &mut keepass::db::EntryMut<'_>, group: GroupId) {
    if e.move_to(group).is_ok() {
        e.times.location_changed = Some(Times::now());
    }
}

/// The recycle bin's group, made (under the root) if there is none.
fn recycle_bin(db: &mut Database) -> GroupId {
    if let Some(g) = db.recycle_bin() {
        return g.id();
    }
    let mut root = db.root_mut();
    let mut g = root.add_group();
    g.name = "Recycle Bin".into();
    g.enable_autotype = Some(false);
    g.enable_searching = Some(false);
    g.set_icon_builtin(RECYCLE_BIN_ICON);
    let id = g.id();
    db.meta.recyclebin_enabled = Some(true);
    db.meta.recyclebin_uuid = Some(id.uuid());
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (Database, GroupId) {
        let mut db = Database::new();
        let work = {
            let mut root = db.root_mut();
            let mut g = root.add_group();
            g.name = "Work".into();
            g.id()
        };
        (db, work)
    }

    fn sample(group: GroupId) -> Draft {
        Draft {
            title: "Mail".into(),
            username: "me".into(),
            password: Zeroizing::new("one".into()),
            url: "https://mail.example.org".into(),
            otp: Zeroizing::default(),
            notes: String::new(),
            group,
        }
    }

    #[test]
    fn add_then_edit_keeps_the_old_version_in_history() {
        let (mut db, work) = db();
        let id = add(&mut db, &sample(work)).unwrap();
        let mut d = draft(&db, id).unwrap();
        assert_eq!(d, sample(work));
        assert!(!update(&mut db, id, &d), "no change, no history");
        d.password = Zeroizing::new("two".into());
        assert!(update(&mut db, id, &d));
        let e = db.entry(id).unwrap();
        assert_eq!(e.get_password(), Some("two"));
        assert!(e.historical(1).is_none(), "one old version");
        assert_eq!(e.historical(0).unwrap().get_password(), Some("one"));
    }

    #[test]
    fn notes_with_line_breaks() {
        let (mut db, work) = db();
        let id = add(&mut db, &sample(work)).unwrap();
        let mut d = draft(&db, id).unwrap();
        d.notes = "line one\nline two".into();
        assert!(update(&mut db, id, &d));
        assert_eq!(draft(&db, id).unwrap().notes, "line one\nline two");
        assert_eq!(db.entry(id).unwrap().historical(0).unwrap().get(fields::NOTES), Some(""));
    }

    #[test]
    fn moving_to_another_group() {
        let (mut db, work) = db();
        let root = db.root().id();
        let id = add(&mut db, &sample(root)).unwrap();
        let mut d = draft(&db, id).unwrap();
        d.group = work;
        assert!(update(&mut db, id, &d));
        assert_eq!(rows(&db)[0].group, "Work");
    }

    #[test]
    fn delete_goes_to_the_recycle_bin_which_the_list_and_groups_leave_out() {
        let (mut db, work) = db();
        let id = add(&mut db, &sample(work)).unwrap();
        assert!(delete(&mut db, id));
        assert!(rows(&db).is_empty());
        let bin = db.recycle_bin().unwrap();
        assert_eq!(bin.entries().count(), 1);
        assert!(groups(&db).iter().all(|(_, name)| name != "Recycle Bin"));
        // A second delete reuses the bin.
        let other = add(&mut db, &sample(work)).unwrap();
        delete(&mut db, other);
        assert_eq!(db.recycle_bin().unwrap().entries().count(), 2);
        assert_eq!(db.root().groups().count(), 2);
    }

    #[test]
    fn delete_for_good_when_the_bin_is_off() {
        let (mut db, work) = db();
        db.meta.recyclebin_enabled = Some(false);
        let id = add(&mut db, &sample(work)).unwrap();
        assert!(!delete(&mut db, id));
        assert!(db.entry(id).is_none());
        assert!(db.deleted_objects.contains_key(&id.uuid()));
    }

    #[test]
    fn moves_and_edits_pass_the_save_check() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = crate::vault::Vault::create(&dir.path().join("v.kdbx"), crate::vault::key("x"), crate::vault::argon2id(1, 64 * 1024, 1));
        let work = {
            let mut root = v.db.root_mut();
            let mut g = root.add_group();
            g.name = "Work".into();
            g.id()
        };
        let root = v.db.root().id();
        let id = add(&mut v.db, &sample(root)).unwrap();
        let other = add(&mut v.db, &sample(root)).unwrap();
        v.save().unwrap();
        let mut d = draft(&v.db, id).unwrap();
        d.group = work;
        d.title = "Moved and renamed".into();
        update(&mut v.db, id, &d);
        delete(&mut v.db, other);
        v.changed();
        v.save().unwrap();
    }

    #[test]
    fn restore_goes_back_to_where_it_was() {
        let (mut db, work) = db();
        let id = add(&mut db, &sample(work)).unwrap();
        delete(&mut db, id);
        assert!(in_bin(&db, id));
        assert_eq!(bin_rows(&db)[0].group, "Work");
        assert_eq!(restore(&mut db, id), Some(work));
        assert!(!in_bin(&db, id));
        assert_eq!(rows(&db)[0].group, "Work");
        // Its group gone meanwhile: back to the root.
        delete(&mut db, id);
        db.group_mut(work).unwrap().track_changes().remove().unwrap();
        assert_eq!(restore(&mut db, id), Some(db.root().id()));
    }

    #[test]
    fn emptying_the_bin_deletes_for_good() {
        let (mut db, work) = db();
        let a = add(&mut db, &sample(work)).unwrap();
        let b = add(&mut db, &sample(work)).unwrap();
        delete(&mut db, a);
        delete(&mut db, b);
        assert_eq!(empty_bin(&mut db), Ok(2));
        assert!(bin_rows(&db).is_empty());
        assert!(db.entry(a).is_none() && db.deleted_objects.contains_key(&b.uuid()));
        assert!(db.recycle_bin().is_some(), "the bin itself stays");
    }

    #[test]
    fn restores_and_bin_changes_pass_the_save_check() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = crate::vault::Vault::create(&dir.path().join("v.kdbx"), crate::vault::key("x"), crate::vault::argon2id(1, 64 * 1024, 1));
        let root = v.db.root().id();
        let a = add(&mut v.db, &sample(root)).unwrap();
        let b = add(&mut v.db, &sample(root)).unwrap();
        delete(&mut v.db, a);
        delete(&mut v.db, b);
        v.changed();
        v.save().unwrap();
        restore(&mut v.db, a);
        empty_bin(&mut v.db).unwrap();
        v.changed();
        v.save().unwrap();
    }

    #[test]
    fn groups_in_tree_order_with_paths() {
        let (mut db, work) = db();
        db.group_mut(work).unwrap().add_group().name = "Servers".into();
        db.root_mut().add_group().name = "Bank".into();
        let names: Vec<String> = groups(&db).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names[1..], ["Bank", "Work", "Work / Servers"]);
    }
}
