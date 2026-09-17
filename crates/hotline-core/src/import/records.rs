//! Owner-stamped records, read out of the same `store.sqlite` the main writes.
//!
//! A row carries the node that owns it, a fencing `owner_epoch`, a `version`
//! that orders edits inside one epoch, and the three classes of state that
//! decide how far each field is allowed to travel: `replicated` goes
//! everywhere, `portable` travels only when the agent does, `machine` never
//! leaves. Deletes leave tombstones so a peer that was offline still learns
//! them.
//!
//! Listing helpers used by the roster reader still answer empty rather than
//! failing. A store that is missing, will not open, or holds bytes that are
//! not a database is the only copy of a roster somebody typed: the main
//! latches it damaged and refuses to write, and an empty rail is survivable
//! where a broken window is not. A fault on that path is an empty answer, and
//! the bytes stay exactly where they are for someone to look at.
//!
//! The importer's path is fallible. A `store.sqlite` that exists but cannot
//! be opened or queried is an error naming the file, because a report of
//! zeros is silence about a roster that is still on disk.

use rusqlite::{Connection, OpenFlags, Row};
use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The previous edition's record store. This tree never writes it; the importer
/// is the only reader, and it opens the file read-only.
pub(crate) fn store_path(root: &Path) -> PathBuf {
    root.join("store.sqlite")
}

/// One row of `resources`, with the JSON columns parsed.
///
/// `portable` and `machine` are nullable because a tombstone releases the state
/// this node's copy held, and a row may never have had either. The stamp
/// fields the importer does not read are still the row: the tests assert
/// them, and dropping them here would be a second, quieter reader.
#[allow(dead_code)]
pub struct ResourceRecord {
    pub kind: String,
    pub id: String,
    pub owner_node: String,
    pub owner_epoch: i64,
    pub version: i64,
    pub updated_at: i64,
    pub deleted: bool,
    pub replicated: Map<String, Value>,
    pub portable: Option<Map<String, Value>>,
    pub machine: Option<Map<String, Value>>,
}

/// Opens the store read-only, or answers `None` for a store that cannot be read.
///
/// Read-only is the invariant, not a precaution: the main process migrates this
/// file and fences its writes, and a second writer would have neither. The busy
/// timeout is for the main's own writes — the file is in WAL, so a reader is
/// only ever blocked by a checkpoint, and waiting out one beats answering an
/// empty roster because a teammate was being renamed at that moment.
pub fn open(root: &Path) -> Option<Connection> {
    open_readonly(root).ok()
}

fn open_readonly(root: &Path) -> rusqlite::Result<Connection> {
    let database = Connection::open_with_flags(
        store_path(root),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    database.busy_timeout(Duration::from_secs(5))?;
    Ok(database)
}

/// Opens the store read-only for the importer.
///
/// A missing file is `Ok(None)`: a data directory with no store is an empty
/// roster, not an error. A file that exists but will not open is an error
/// naming it.
pub(crate) fn open_for_import(root: &Path) -> io::Result<Option<Connection>> {
    let path = store_path(root);
    if !path.exists() {
        return Ok(None);
    }
    open_readonly(root)
        .map(Some)
        .map_err(|error| io::Error::other(format!("{} cannot be read ({error})", path.display())))
}

/// Confirms a store copy can be opened and queried. `named` is the path the
/// error should mention — the source file, not a temporary copy.
///
/// `PRAGMA quick_check` is the torn-copy detector: a `-wal` copied without
/// its `-shm` still reads, and SQLite rebuilds the index, but a copy whose
/// frames do not hang together must not become a smaller roster. The
/// personas count is the same question asked of the table the importer
/// actually reads.
pub(crate) fn require_readable(root: &Path, named: &Path) -> io::Result<()> {
    if !store_path(root).exists() {
        return Err(io::Error::other(format!(
            "{} cannot be read",
            named.display()
        )));
    }
    let database = open_readonly(root).map_err(|error| {
        io::Error::other(format!("{} cannot be read ({error})", named.display()))
    })?;
    let status: String = database
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| {
            io::Error::other(format!("{} cannot be read ({error})", named.display()))
        })?;
    if status != "ok" {
        return Err(io::Error::other(format!(
            "{} did not copy intact ({status})",
            named.display()
        )));
    }
    database
        .query_row(
            "SELECT count(*) FROM resources WHERE kind = 'persona'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            io::Error::other(format!("{} cannot be read ({error})", named.display()))
        })?;
    Ok(())
}

/// A JSON column as an object, or nothing when it is absent or not one.
fn object_of(text: Option<String>) -> Option<Map<String, Value>> {
    match serde_json::from_str(&text?) {
        Ok(Value::Object(object)) => Some(object),
        _ => None,
    }
}

fn record_of(row: &Row) -> rusqlite::Result<ResourceRecord> {
    Ok(ResourceRecord {
        kind: row.get("kind")?,
        id: row.get("id")?,
        owner_node: row.get("owner_node")?,
        owner_epoch: row.get("owner_epoch")?,
        version: row.get("version")?,
        updated_at: row.get("updated_at")?,
        deleted: row.get::<_, i64>("deleted")? == 1,
        replicated: object_of(row.get("replicated")?).unwrap_or_default(),
        portable: object_of(row.get("portable")?),
        machine: object_of(row.get("machine")?),
    })
}

/// The node this store belongs to, as the main stamped it on first open.
///
/// Read from the store rather than resolved a second way: the question a reader
/// asks is which rows are this desk's own, and that is the id rows are
/// *stamped* with.
pub fn local_node_id(database: &Connection) -> Option<String> {
    database
        .query_one("SELECT value FROM meta WHERE key = 'node_id'", [], |row| {
            row.get(0)
        })
        .ok()
}

/// Every live record of one kind, in the order the rows were inserted.
///
/// Tombstones are hidden. Order is insertion order because a roster nobody ever
/// dragged is shown in the order `config.json` had; where a row sits after a
/// drag is view state the caller applies.
pub fn list_records(database: &Connection, kind: &str) -> Vec<ResourceRecord> {
    try_list_records(database, kind).unwrap_or_default()
}

/// The same listing, failing when the store cannot be queried.
pub(crate) fn try_list_records(
    database: &Connection,
    kind: &str,
) -> rusqlite::Result<Vec<ResourceRecord>> {
    let mut statement = database
        .prepare("SELECT * FROM resources WHERE kind = ? AND deleted = 0 ORDER BY rowid")?;
    let rows = statement.query_map([kind], record_of)?;
    rows.collect()
}

/// One record, tombstone included: whether a deleted row still counts is the
/// caller's question, and a caller that cannot see the tombstone cannot tell a
/// teammate that was deleted from one that never existed.
#[cfg(test)]
pub fn get_record(database: &Connection, kind: &str, id: &str) -> Option<ResourceRecord> {
    database
        .query_one(
            "SELECT * FROM resources WHERE kind = ? AND id = ?",
            [kind, id],
            record_of,
        )
        .ok()
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A store built the way the main builds one.
    //!
    //! The statements are `createSchema` in `src/bun/store/records.ts`, copied
    //! rather than shared: they are what a Rust reader must keep agreeing with,
    //! so a divergence should show up as a failing test here and not as a
    //! quietly adjusted constant.

    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    /// A data directory of this test's own, emptied first so a rerun is a run.
    pub fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hotline-core-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn create(root: &Path, node_id: &str) -> Connection {
        std::fs::create_dir_all(root).unwrap();
        let database = Connection::open(super::store_path(root)).unwrap();
        database.pragma_update(None, "journal_mode", "WAL").unwrap();
        for statement in [
            "CREATE TABLE IF NOT EXISTS meta (
		key   TEXT PRIMARY KEY,
		value TEXT NOT NULL
	) STRICT",
            "CREATE TABLE IF NOT EXISTS resources (
		kind        TEXT    NOT NULL,
		id          TEXT    NOT NULL,
		owner_node  TEXT    NOT NULL,
		owner_epoch INTEGER NOT NULL,
		version     INTEGER NOT NULL,
		updated_at  INTEGER NOT NULL,
		deleted     INTEGER NOT NULL DEFAULT 0,
		replicated  TEXT    NOT NULL,
		portable    TEXT,
		machine     TEXT,
		PRIMARY KEY (kind, id)
	) STRICT",
            "CREATE TABLE IF NOT EXISTS oplog (
		seq         INTEGER PRIMARY KEY AUTOINCREMENT,
		owner_node  TEXT    NOT NULL,
		kind        TEXT    NOT NULL,
		id          TEXT    NOT NULL,
		owner_epoch INTEGER NOT NULL,
		version     INTEGER NOT NULL,
		op          TEXT    NOT NULL CHECK (op IN ('put','tombstone')),
		payload     TEXT    NOT NULL,
		at          INTEGER NOT NULL
	) STRICT",
            "CREATE UNIQUE INDEX IF NOT EXISTS oplog_idempotent ON oplog (kind, id, owner_epoch, version)",
            "CREATE INDEX IF NOT EXISTS oplog_by_owner ON oplog (owner_node, seq)",
            "CREATE TABLE IF NOT EXISTS applied_cursor (
		owner_node  TEXT    PRIMARY KEY,
		applied_seq INTEGER NOT NULL
	) STRICT",
            "INSERT INTO meta (key, value) VALUES ('schema_version', '1') ON CONFLICT(key) DO NOTHING",
        ] {
            database.execute(statement, []).unwrap();
        }
        database
            .execute(
                "INSERT INTO meta (key, value) VALUES ('node_id', ?)",
                [node_id],
            )
            .unwrap();
        database
    }

    /// One row, with the three class payloads spelled as the caller wants them.
    pub struct Put<'a> {
        pub kind: &'a str,
        pub id: &'a str,
        pub owner_node: &'a str,
        pub updated_at: i64,
        pub deleted: bool,
        pub replicated: serde_json::Value,
        pub portable: Option<serde_json::Value>,
        pub machine: Option<serde_json::Value>,
    }

    impl<'a> Put<'a> {
        pub fn new(id: &'a str, owner_node: &'a str, replicated: serde_json::Value) -> Self {
            Self {
                kind: "persona",
                id,
                owner_node,
                updated_at: 2_000,
                deleted: false,
                replicated,
                portable: None,
                machine: None,
            }
        }

        pub fn write(self, database: &Connection) {
            database
                .execute(
                    "INSERT INTO resources
                       (kind, id, owner_node, owner_epoch, version, updated_at, deleted, replicated, portable, machine)
                     VALUES (?, ?, ?, 1, 1, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        self.kind,
                        self.id,
                        self.owner_node,
                        self.updated_at,
                        i64::from(self.deleted),
                        self.replicated.to_string(),
                        self.portable.map(|value| value.to_string()),
                        self.machine.map(|value| value.to_string()),
                    ],
                )
                .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{Put, create, scratch};
    use super::*;
    use serde_json::json;

    #[test]
    fn a_row_reads_back_with_its_stamp_and_all_three_classes() {
        let root = scratch("classes");
        let database = create(&root, "this-desk");
        Put {
            portable: Some(json!({ "mcpPolicy": { "mode": "all" } })),
            machine: Some(json!({ "cwd": "/tmp/ada" })),
            ..Put::new("ada", "this-desk", json!({ "name": "Ada" }))
        }
        .write(&database);

        let reader = open(&root).unwrap();
        assert_eq!(local_node_id(&reader).as_deref(), Some("this-desk"));
        let records = list_records(&reader, "persona");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.kind, "persona");
        assert_eq!(record.id, "ada");
        assert_eq!(record.owner_node, "this-desk");
        assert_eq!(record.owner_epoch, 1);
        assert_eq!(record.version, 1);
        assert_eq!(record.updated_at, 2_000);
        assert!(!record.deleted);
        assert_eq!(record.replicated["name"], "Ada");
        assert_eq!(
            record.portable.as_ref().unwrap()["mcpPolicy"]["mode"],
            "all"
        );
        assert_eq!(record.machine.as_ref().unwrap()["cwd"], "/tmp/ada");
    }

    #[test]
    fn a_tombstone_is_hidden_from_the_listing_and_still_fetched_by_id() {
        let root = scratch("tombstone");
        let database = create(&root, "this-desk");
        Put::new("listed", "this-desk", json!({ "name": "Listed" })).write(&database);
        Put {
            deleted: true,
            ..Put::new("buried", "this-desk", json!({ "name": "Buried" }))
        }
        .write(&database);

        let reader = open(&root).unwrap();
        let live = list_records(&reader, "persona");
        let listed: Vec<&str> = live.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(listed, ["listed"]);

        let buried = get_record(&reader, "persona", "buried").unwrap();
        assert!(buried.deleted);
        assert!(get_record(&reader, "persona", "never-existed").is_none());
    }

    #[test]
    fn a_missing_store_and_a_garbage_one_both_read_empty() {
        let missing = scratch("missing");
        assert!(open(&missing).is_none());

        let garbage = scratch("garbage");
        std::fs::write(
            store_path(&garbage),
            "this file is emphatically not a sqlite database\n",
        )
        .unwrap();
        let reader = open(&garbage).expect("sqlite opens lazily; the read is what fails");
        assert!(local_node_id(&reader).is_none());
        assert!(list_records(&reader, "persona").is_empty());
        assert!(get_record(&reader, "persona", "anyone").is_none());
    }

    /// A row written while the reader is open is a row the reader can see: the
    /// main is the writer, and it writes whenever somebody edits a teammate.
    #[test]
    fn a_reader_sees_what_the_writer_commits_beside_it() {
        let root = scratch("concurrent");
        let database = create(&root, "this-desk");
        let reader = open(&root).unwrap();
        assert!(list_records(&reader, "persona").is_empty());

        Put::new("late", "this-desk", json!({ "name": "Late" })).write(&database);
        assert_eq!(list_records(&reader, "persona").len(), 1);
    }

    /// A WAL database with no writer attached is still readable. SQLite needs a
    /// shared-memory file to read a WAL that has not been checkpointed, and a
    /// read-only connection cannot create one — so a store nobody has open is
    /// exactly the case that would answer an empty roster if this were wrong.
    #[test]
    fn a_store_the_writer_has_closed_still_reads() {
        let root = scratch("closed");
        let database = create(&root, "this-desk");
        Put::new("kept", "this-desk", json!({ "name": "Kept" })).write(&database);
        database.close().unwrap();

        let reader = open(&root).unwrap();
        assert_eq!(local_node_id(&reader).as_deref(), Some("this-desk"));
        assert_eq!(list_records(&reader, "persona").len(), 1);
    }
}
