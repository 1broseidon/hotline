//! Full-text search over every teammate's conversation, in SQLite FTS5.
//!
//! The JSONL transcript stays the record; this is an index of it, rebuilt from
//! the file whenever the two disagree, and kept current by indexing each event
//! as it is appended. Nothing in here is a source of truth, which is what makes
//! it safe to delete.
//!
//! Two things are indexed, because they answer different questions. Messages
//! answer "where did we say X". Chapters — their titles, notes and tags — are
//! summaries an agent wrote, so they answer "what was that thing we did in
//! June" even when the conversation never used the word the searcher reaches
//! for. Chapter hits come first for that reason.
//!
//! **Asking and answering open the file differently.** A question is asked over
//! a read-only connection that creates nothing and migrates nothing — the query
//! side runs in whichever process holds the window, and a reader that repaired
//! what it read would be a second writer with no transaction. Every fault on
//! that side answers no hits, because a missing or damaged index costs a search
//! and never a record. The one connection that creates the file and its schema
//! is [`Indexer`], and there is one of those, in the process that owns the
//! tapes.

use crate::log::{Log, StreamId, open_epoch};
use crate::paths::{index_path, transcript_path, transcript_segment_path};
use crate::store::chapters::{chapters_of, open_chapter, slice_of};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, Row};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

/// Whose conversation is searched. The two methods differ in exactly this: a
/// thread search filters by teammate, and a search across everyone has to say
/// on each hit whose conversation it came from.
enum Scope<'a> {
    Thread(&'a str),
    Everyone,
}

/// Longer than this and the tail is dropped. The cap is on what a person
/// typed, not on what the index holds.
const MAX_QUERY: usize = 200;

/// Opens the index read-only, or answers `None` when there is nothing to open.
///
/// The busy timeout is for the main's own writes: the file is in WAL, so a
/// reader is only ever blocked by a checkpoint, and waiting one out beats
/// telling somebody their conversation contains nothing.
fn open(root: &Path) -> Option<Connection> {
    let database = Connection::open_with_flags(
        index_path(root),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    database.busy_timeout(Duration::from_secs(5)).ok()?;
    Some(database)
}

/// The first 200 UTF-16 code units of the query.
///
/// UTF-16 because that is what `String.prototype.slice` counts in the main, and
/// a cap that disagreed would search a different query than the one the main
/// answered before this moved. A character is kept whole rather than split at
/// the boundary, so the cut can never produce half a pair.
fn capped(query: &str) -> &str {
    let mut units = 0;
    for (offset, character) in query.char_indices() {
        units += character.len_utf16();
        if units > MAX_QUERY {
            return &query[..offset];
        }
    }
    query
}

/// An FTS5 query from what a person typed.
///
/// Each word becomes a quoted prefix term, so punctuation cannot reach the
/// parser and "contain" finds "container".
fn terms_of(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|word| word.replace(['"', '*'], ""))
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\"*"))
        .collect()
}

/// The hits one MATCH expression finds, chapters and messages kept apart
/// because whether *anything* matched is what decides the OR retry.
struct Found {
    chapters: Vec<Value>,
    messages: Vec<Value>,
}

fn arguments(scope: &Scope, match_expression: &str, limit: i64) -> Vec<SqlValue> {
    let mut arguments = vec![
        SqlValue::Text(match_expression.to_string()),
        SqlValue::Integer(limit),
    ];
    if let Scope::Thread(persona_id) = scope {
        arguments.push(SqlValue::Text((*persona_id).to_string()));
    }
    arguments
}

/// A JSON string field, absent when the column is null or empty — the main
/// spells these with a truthiness test, so an empty string is no value.
fn insert_if_present(hit: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        hit.insert(key.to_string(), Value::String(value));
    }
}

fn chapter_hit(row: &Row, scope: &Scope) -> rusqlite::Result<Value> {
    let mut hit = Map::new();
    hit.insert("kind".into(), json!("chapter"));
    if matches!(scope, Scope::Everyone) {
        hit.insert(
            "personaId".into(),
            json!(row.get::<_, String>("persona_id")?),
        );
    }
    hit.insert("chapterId".into(), json!(row.get::<_, String>("id")?));
    hit.insert("ts".into(), json!(row.get::<_, i64>("started_at")?));
    hit.insert(
        "title".into(),
        json!(
            row.get::<_, Option<String>>("title")?
                .unwrap_or_else(|| "Untitled chapter".into())
        ),
    );
    hit.insert("excerpt".into(), json!(row.get::<_, String>("excerpt")?));
    insert_if_present(&mut hit, "status", row.get("status")?);
    Ok(Value::Object(hit))
}

fn message_hit(row: &Row, scope: &Scope) -> rusqlite::Result<Value> {
    let mut hit = Map::new();
    hit.insert("kind".into(), json!("message"));
    if matches!(scope, Scope::Everyone) {
        hit.insert(
            "personaId".into(),
            json!(row.get::<_, String>("persona_id")?),
        );
    }
    hit.insert("eventId".into(), json!(row.get::<_, String>("event_id")?));
    insert_if_present(&mut hit, "chapterId", row.get("chapter_id")?);
    hit.insert("ts".into(), json!(row.get::<_, i64>("ts")?));
    let from = if row.get::<_, String>("kind")? == "user" {
        "me"
    } else {
        "them"
    };
    hit.insert("from".into(), json!(from));
    hit.insert("excerpt".into(), json!(row.get::<_, String>("excerpt")?));
    Ok(Value::Object(hit))
}

fn rows(
    database: &Connection,
    statement: &str,
    arguments: Vec<SqlValue>,
    hit: impl Fn(&Row) -> rusqlite::Result<Value>,
) -> Vec<Value> {
    let Ok(mut prepared) = database.prepare(statement) else {
        return Vec::new();
    };
    let Ok(found) = prepared.query_map(rusqlite::params_from_iter(arguments), hit) else {
        return Vec::new();
    };
    found.flatten().collect()
}

/// One MATCH expression, asked of both tables.
///
/// Messages are fetched one past the limit so the caller can tell a full page
/// from a page that happens to end there; chapters are not, because they are
/// never the reason a result set is called truncated.
fn attempt(database: &Connection, scope: &Scope, match_expression: &str, limit: i64) -> Found {
    let (chapter_filter, message_filter) = match scope {
        Scope::Thread(_) => (" AND chapters_fts.persona_id = ?3", " AND persona_id = ?3"),
        Scope::Everyone => ("", ""),
    };
    Found {
        chapters: rows(
            database,
            &format!(
                "SELECT c.id, c.persona_id, c.started_at, c.title, c.status,
				        snippet(chapters_fts, 2, '', '', '…', 24) AS excerpt
				 FROM chapters_fts JOIN chapters c ON c.id = chapters_fts.chapter_id
				 WHERE chapters_fts MATCH ?1{chapter_filter}
				 ORDER BY bm25(chapters_fts) LIMIT ?2"
            ),
            arguments(scope, match_expression, limit),
            |row| chapter_hit(row, scope),
        ),
        messages: rows(
            database,
            &format!(
                "SELECT persona_id, event_id, chapter_id, kind, ts,
				        snippet(messages, 5, '', '', '…', 24) AS excerpt
				 FROM messages WHERE messages MATCH ?1{message_filter}
				 ORDER BY bm25(messages) LIMIT ?2"
            ),
            arguments(scope, match_expression, limit.saturating_add(1)),
            |row| message_hit(row, scope),
        ),
    }
}

fn no_hits() -> Value {
    json!({ "hits": [], "truncated": false })
}

/// Chapters first, then messages, each ranked by BM25.
///
/// The words are ANDed; if nothing matches and there was more than one, they
/// are ORed, because the searcher was describing a memory rather than quoting
/// it.
fn run(root: &Path, scope: Scope, query: &str, limit: i64) -> Value {
    let terms = terms_of(capped(query));
    if terms.is_empty() {
        return no_hits();
    }
    let Some(database) = open(root) else {
        return no_hits();
    };
    let mut found = attempt(&database, &scope, &terms.join(" "), limit);
    if found.chapters.is_empty() && found.messages.is_empty() && terms.len() > 1 {
        found = attempt(&database, &scope, &terms.join(" OR "), limit);
    }
    let truncated = i64::try_from(found.messages.len()).unwrap_or(i64::MAX) > limit;
    let mut hits = found.chapters;
    hits.extend(
        found
            .messages
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(0)),
    );
    json!({ "hits": hits, "truncated": truncated })
}

/// One teammate's conversation. The clamp is here rather than at the door
/// because this is the only implementation, and a limit of zero or of a
/// million is a caller's slip either way.
pub fn search(root: &Path, persona_id: &str, query: &str, limit: Option<i64>) -> Value {
    run(
        root,
        Scope::Thread(persona_id),
        query,
        limit.unwrap_or(20).clamp(1, 40),
    )
}

/// The same search, across every teammate at once. One index already holds
/// them all — per-conversation search was a WHERE clause, and removing it is
/// the whole feature.
pub fn search_all(root: &Path, query: &str, limit: Option<i64>) -> Value {
    run(root, Scope::Everyone, query, limit.unwrap_or(30))
}

/// The index's schema, and the only copy of it.
///
/// Word for word `open` in `src/bun/store/search.ts`, tabs included, because
/// SQLite stores a table's `CREATE` text verbatim and a build that spelled it
/// differently would leave two shapes of the same file behind. Neither process
/// migrates this file: a schema change is a new statement here and a rebuild.
const SCHEMA: [&str; 5] = [
    "CREATE VIRTUAL TABLE IF NOT EXISTS messages USING fts5(
		persona_id UNINDEXED, event_id UNINDEXED, chapter_id UNINDEXED, kind UNINDEXED, ts UNINDEXED, text,
		tokenize = 'porter unicode61'
	)",
    "CREATE TABLE IF NOT EXISTS chapters (
		id TEXT PRIMARY KEY, persona_id TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER,
		title TEXT, note TEXT, status TEXT, session_id TEXT, backend_id TEXT
	)",
    "CREATE INDEX IF NOT EXISTS chapters_persona ON chapters(persona_id, started_at)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS chapters_fts USING fts5(
		chapter_id UNINDEXED, persona_id UNINDEXED, text, tokenize = 'porter unicode61'
	)",
    "CREATE TABLE IF NOT EXISTS index_state (persona_id TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL)",
];

/// The size and modification time of the file a teammate is being written to.
///
/// The open epoch's segment, or the legacy flat file when that segment does not
/// exist yet. What is stamped is only ever compared against itself, so what
/// matters is that both processes measure the same file the same way: the
/// milliseconds are rounded the way Node rounds `mtimeMs`, or a tape the main
/// indexed would look changed to this build and be re-read for nothing.
fn file_stamp(root: &Path, persona_id: &str) -> Option<(i64, i64)> {
    let active = transcript_segment_path(root, persona_id, open_epoch(root, persona_id));
    let file = if active.exists() {
        active
    } else {
        transcript_path(root, persona_id)
    };
    let metadata = fs::metadata(&file).ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    let milliseconds =
        (modified.as_secs() as f64 * 1000.0 + f64::from(modified.subsec_nanos()) / 1e6).round();
    Some((metadata.len() as i64, milliseconds as i64))
}

fn stamp(database: &Connection, root: &Path, persona_id: &str) -> rusqlite::Result<()> {
    let Some((size, mtime)) = file_stamp(root, persona_id) else {
        return Ok(());
    };
    database.execute(
        "INSERT INTO index_state (persona_id, size, mtime) VALUES (?, ?, ?)
		 ON CONFLICT(persona_id) DO UPDATE SET size = excluded.size, mtime = excluded.mtime",
        rusqlite::params![persona_id, size, mtime],
    )?;
    Ok(())
}

fn text_of(event: &Value, key: &str) -> Option<String> {
    event
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// One message, if it is one and it said anything.
fn index_message(
    database: &Connection,
    persona_id: &str,
    chapter_id: Option<&str>,
    event: &Value,
) -> rusqlite::Result<()> {
    let kind = event
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != "user" && kind != "agent" {
        return Ok(());
    }
    let text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if text.is_empty() {
        return Ok(());
    }
    database.execute(
        "INSERT INTO messages (persona_id, event_id, chapter_id, kind, ts, text) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            persona_id,
            event.get("id").and_then(Value::as_str).unwrap_or_default(),
            chapter_id,
            kind,
            event.get("ts").and_then(Value::as_i64).unwrap_or_default(),
            text
        ],
    )?;
    Ok(())
}

/// One chapter marker, replacing whatever the row and its text held before.
///
/// A marker is written twice — once when the chapter opens, again when it
/// closes with a title and a note — so this upserts the row and rewrites the
/// searchable text from scratch rather than adding a second copy of it.
fn index_chapter(database: &Connection, persona_id: &str, chapter: &Value) -> rusqlite::Result<()> {
    let id = chapter
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    database.execute(
        "INSERT INTO chapters (id, persona_id, started_at, ended_at, title, note, status, session_id, backend_id)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
		 ON CONFLICT(id) DO UPDATE SET ended_at = excluded.ended_at, title = excluded.title, note = excluded.note,
		   status = excluded.status, session_id = excluded.session_id, backend_id = excluded.backend_id",
        rusqlite::params![
            id,
            persona_id,
            chapter.get("ts").and_then(Value::as_i64).unwrap_or_default(),
            chapter.get("endedAt").and_then(Value::as_i64),
            chapter.get("title").and_then(Value::as_str),
            chapter.get("note").and_then(Value::as_str),
            chapter.get("status").and_then(Value::as_str),
            chapter.get("sessionId").and_then(Value::as_str),
            chapter.get("backendId").and_then(Value::as_str),
        ],
    )?;
    database.execute("DELETE FROM chapters_fts WHERE chapter_id = ?", [id])?;
    let mut parts: Vec<String> = [text_of(chapter, "title"), text_of(chapter, "note")]
        .into_iter()
        .flatten()
        .collect();
    if let Some(tags) = chapter.get("tags").and_then(Value::as_array) {
        parts.extend(tags.iter().filter_map(|tag| {
            tag.as_str()
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
        }));
    }
    let text = parts.join("\n");
    if !text.is_empty() {
        database.execute(
            "INSERT INTO chapters_fts (chapter_id, persona_id, text) VALUES (?, ?, ?)",
            rusqlite::params![id, persona_id, text],
        )?;
    }
    Ok(())
}

/// The one connection that may create this file, migrate it and write to it.
///
/// It holds the open chapter per teammate so that indexing an event does not
/// have to re-read the tape to know which chapter the message belongs to; that
/// is the whole reason this is a value somebody owns rather than a set of free
/// functions. Every method answers a `rusqlite::Result` rather than swallowing
/// its faults the way the main does: the index is rebuildable, so the caller
/// that appended to the tape should log a failure here and carry on, but which
/// caller and which log is the caller's to decide.
pub struct Indexer {
    log: Log,
    database: Connection,
    /// Absent means "not looked up yet"; `Some(None)` means "looked, and no
    /// chapter is open". The main draws the same distinction with `Map.has`.
    open_chapters: HashMap<String, Option<String>>,
}

impl Indexer {
    /// Opens the index for writing, creating the file and its schema. The
    /// index is an index *of that log*: every rebuild re-reads its tapes.
    pub fn open(log: &Log) -> rusqlite::Result<Self> {
        // A directory that cannot be made is a file that cannot be opened, and
        // the open below is the one that says so properly.
        let _ = fs::create_dir_all(log.root());
        let database = Connection::open(index_path(log.root()))?;
        database.pragma_update(None, "journal_mode", "WAL")?;
        for statement in SCHEMA {
            database.execute(statement, [])?;
        }
        Ok(Self {
            log: log.clone(),
            database,
            open_chapters: HashMap::new(),
        })
    }

    /// The teammate's tape, folded — what a rebuild reads.
    fn tape(&self, persona_id: &str) -> Vec<Value> {
        self.log.load(&StreamId::Tape(persona_id.to_string()))
    }

    /// Indexes one event as it lands.
    ///
    /// A chapter marker moves the "current chapter" pointer for the messages
    /// that follow it; its close updates the row in place. A message id already
    /// in the table is a replay rather than a change, because a message is
    /// written once and stands forever.
    pub fn index_event(&mut self, persona_id: &str, event: &Value) -> rusqlite::Result<()> {
        let kind = event
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "chapter" {
            index_chapter(&self.database, persona_id, event)?;
            let still_open = event.get("endedAt").is_none();
            let id = event.get("id").and_then(Value::as_str).map(str::to_string);
            self.open_chapters
                .insert(persona_id.to_string(), id.filter(|_| still_open));
            return stamp(&self.database, self.log.root(), persona_id);
        }
        if kind != "user" && kind != "agent" {
            return Ok(());
        }
        if !self.open_chapters.contains_key(persona_id) {
            let events = self.tape(persona_id);
            let open = open_chapter(&events)
                .and_then(|chapter| chapter.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            self.open_chapters.insert(persona_id.to_string(), open);
        }
        let seen = self
            .database
            .query_one(
                "SELECT 1 FROM messages WHERE persona_id = ? AND event_id = ? LIMIT 1",
                rusqlite::params![
                    persona_id,
                    event.get("id").and_then(Value::as_str).unwrap_or_default()
                ],
                |row| row.get::<_, i64>(0),
            )
            .is_ok();
        if seen {
            return Ok(());
        }
        let chapter_id = self.open_chapters.get(persona_id).cloned().flatten();
        index_message(&self.database, persona_id, chapter_id.as_deref(), event)?;
        stamp(&self.database, self.log.root(), persona_id)
    }

    /// Throws the teammate's rows away and re-reads the tape.
    pub fn reindex(&mut self, persona_id: &str) -> rusqlite::Result<()> {
        let events = self.tape(persona_id);
        let transaction = self.database.transaction()?;
        for table in ["messages", "chapters", "chapters_fts"] {
            transaction.execute(
                &format!("DELETE FROM {table} WHERE persona_id = ?"),
                [persona_id],
            )?;
        }
        let chapters = chapters_of(&events);
        for chapter in &chapters {
            index_chapter(&transaction, persona_id, chapter)?;
        }
        // Messages before the first marker belong to no chapter.
        let unchaptered = match events
            .iter()
            .position(|event| event.get("kind").and_then(Value::as_str) == Some("chapter"))
        {
            Some(first) => &events[..first],
            None => &events[..],
        };
        for event in unchaptered {
            index_message(&transaction, persona_id, None, event)?;
        }
        for chapter in &chapters {
            let id = chapter.get("id").and_then(Value::as_str);
            for event in slice_of(&events, chapter) {
                index_message(&transaction, persona_id, id, event)?;
            }
        }
        stamp(&transaction, self.log.root(), persona_id)?;
        transaction.commit()?;
        let open = open_chapter(&events)
            .and_then(|chapter| chapter.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.open_chapters.insert(persona_id.to_string(), open);
        Ok(())
    }

    /// Forgets a teammate entirely — the rows and the stamp that would stop
    /// them being rebuilt.
    pub fn forget(&mut self, persona_id: &str) -> rusqlite::Result<()> {
        for table in ["messages", "chapters", "chapters_fts", "index_state"] {
            self.database.execute(
                &format!("DELETE FROM {table} WHERE persona_id = ?"),
                [persona_id],
            )?;
        }
        self.open_chapters.remove(persona_id);
        Ok(())
    }

    /// Brings the index in line with the files at startup.
    ///
    /// A transcript whose size or modification time differs from what was last
    /// indexed — the startup fold rewrites them, and a crash can leave events
    /// unindexed — is re-read whole.
    pub fn sync(&mut self, persona_ids: &[String]) -> rusqlite::Result<()> {
        for persona_id in persona_ids {
            let Some(stamp) = file_stamp(self.log.root(), persona_id) else {
                continue;
            };
            let known = self
                .database
                .query_one(
                    "SELECT size, mtime FROM index_state WHERE persona_id = ?",
                    [persona_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .ok();
            if known == Some(stamp) {
                continue;
            }
            self.reindex(persona_id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! An index built the way this crate's own writer builds one.
    //!
    //! The schema comes from `Indexer::open` because there is one copy of it,
    //! which is the point: a reader in these tests is asked of a file the
    //! writer built, so a schema that drifted from what a query expects fails
    //! here rather than in front of somebody searching.

    use super::Indexer;
    use crate::log::Log;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// A data directory, its log, and an index of it, emptied first so a rerun
    /// is a run.
    pub fn indexer(name: &str) -> (PathBuf, Log, Indexer) {
        let root =
            std::env::temp_dir().join(format!("toad-core-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let log = Log::open(&root);
        let indexer = Indexer::open(&log).unwrap();
        (root, log, indexer)
    }

    /// The same index, for the tests that put rows in by hand because what
    /// they are about is the question and not the indexing.
    pub fn index(name: &str) -> (PathBuf, Connection) {
        let (root, _log, indexer) = indexer(name);
        (root, indexer.database)
    }

    pub fn message(database: &Connection, persona_id: &str, id: &str, kind: &str, text: &str) {
        database
            .execute(
                "INSERT INTO messages (persona_id, event_id, chapter_id, kind, ts, text) VALUES (?, ?, NULL, ?, 1, ?)",
                [persona_id, id, kind, text],
            )
            .unwrap();
    }

    pub fn chapter(database: &Connection, persona_id: &str, id: &str, title: &str, note: &str) {
        database
            .execute(
                "INSERT INTO chapters (id, persona_id, started_at, title, status) VALUES (?, ?, 100, ?, 'done')",
                [id, persona_id, title],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO chapters_fts (chapter_id, persona_id, text) VALUES (?, ?, ?)",
                [id, persona_id, &format!("{title}\n{note}")],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{chapter, index, indexer, message};
    use super::*;

    fn hits(answer: &Value) -> &Vec<Value> {
        answer["hits"].as_array().unwrap()
    }

    fn ids(answer: &Value) -> Vec<String> {
        hits(answer)
            .iter()
            .map(|hit| {
                hit.get("eventId")
                    .or_else(|| hit.get("chapterId"))
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    /// Ranking decides the order within a kind, so a test that only cares
    /// which rows came back compares them sorted.
    fn sorted_ids(answer: &Value) -> Vec<String> {
        let mut found = ids(answer);
        found.sort();
        found
    }

    #[test]
    fn two_terms_are_anded_and_a_prefix_matches() {
        let (root, database) = index("and");
        message(&database, "ada", "m1", "user", "the harbour crane is stuck");
        message(&database, "ada", "m2", "agent", "the crane arrived");

        assert_eq!(ids(&search(&root, "ada", "harbour crane", None)), ["m1"]);

        // "cran" reaches "crane" because every term is a prefix term.
        assert_eq!(
            sorted_ids(&search(&root, "ada", "cran", None)),
            ["m1", "m2"]
        );
    }

    #[test]
    fn nothing_matching_every_term_falls_back_to_any_of_them() {
        let (root, database) = index("or");
        message(&database, "ada", "m1", "user", "the harbour is quiet");

        assert!(hits(&search(&root, "ada", "harbour zeppelin", None)).len() == 1);
        // One term that matches nothing has nothing to fall back to.
        assert!(hits(&search(&root, "ada", "zeppelin", None)).is_empty());
    }

    #[test]
    fn a_chapter_hit_comes_before_a_message_hit() {
        let (root, database) = index("order");
        message(&database, "ada", "m1", "user", "the harbour again");
        chapter(
            &database,
            "ada",
            "c1",
            "Harbour week",
            "we fixed the harbour",
        );

        let answer = search(&root, "ada", "harbour", None);
        assert_eq!(ids(&answer), ["c1", "m1"]);
        let chapter_hit = &hits(&answer)[0];
        assert_eq!(chapter_hit["kind"], "chapter");
        assert_eq!(chapter_hit["title"], "Harbour week");
        assert_eq!(chapter_hit["status"], "done");
        assert_eq!(chapter_hit["ts"], 100);
        assert!(chapter_hit.get("personaId").is_none());
    }

    #[test]
    fn a_message_says_which_side_wrote_it() {
        let (root, database) = index("from");
        message(&database, "ada", "m1", "user", "the harbour");
        message(&database, "ada", "m2", "agent", "the harbour, again");

        let answer = search(&root, "ada", "harbour", None);
        let sides: Vec<(&str, &str)> = hits(&answer)
            .iter()
            .map(|hit| {
                (
                    hit["eventId"].as_str().unwrap(),
                    hit["from"].as_str().unwrap(),
                )
            })
            .collect();
        assert!(sides.contains(&("m1", "me")));
        assert!(sides.contains(&("m2", "them")));
        // A message outside any chapter carries no chapter id at all.
        assert!(
            hits(&answer)
                .iter()
                .all(|hit| hit.get("chapterId").is_none())
        );
    }

    #[test]
    fn more_messages_than_the_limit_marks_the_answer_truncated() {
        let (root, database) = index("truncated");
        for number in 0..5 {
            message(
                &database,
                "ada",
                &format!("m{number}"),
                "user",
                "harbour again",
            );
        }

        let two = search(&root, "ada", "harbour", Some(2));
        assert_eq!(hits(&two).len(), 2);
        assert_eq!(two["truncated"], true);

        let all = search(&root, "ada", "harbour", Some(5));
        assert_eq!(hits(&all).len(), 5);
        assert_eq!(all["truncated"], false);
    }

    #[test]
    fn a_thread_search_sees_one_teammate_and_a_global_search_sees_them_all() {
        let (root, database) = index("scope");
        message(&database, "ada", "m1", "user", "harbour");
        message(&database, "bob", "m2", "user", "harbour");
        chapter(&database, "bob", "c1", "Harbour week", "the harbour again");

        assert_eq!(ids(&search(&root, "ada", "harbour", None)), ["m1"]);

        let everyone = search_all(&root, "harbour", None);
        assert_eq!(hits(&everyone)[0]["kind"], "chapter");
        let mut named: Vec<(&str, &str)> = hits(&everyone)
            .iter()
            .map(|hit| {
                (
                    hit["personaId"].as_str().unwrap(),
                    hit.get("eventId")
                        .or_else(|| hit.get("chapterId"))
                        .and_then(Value::as_str)
                        .unwrap(),
                )
            })
            .collect();
        named.sort_unstable();
        assert_eq!(named, [("ada", "m1"), ("bob", "c1"), ("bob", "m2")]);
    }

    #[test]
    fn a_query_past_two_hundred_characters_is_cut_where_the_main_cuts_it() {
        let (root, database) = index("cap");
        message(&database, "ada", "m1", "user", "harbour");
        message(&database, "ada", "m2", "user", "zeppelin");

        // 200 characters of one word, then a second word past the cut. Cut,
        // this asks for the harbour alone; uncut, the zeppelin term would fail
        // the AND and the OR fallback would answer both messages instead.
        let long = format!("{}zeppelin", "harbour ".repeat(25));
        assert_eq!(capped(&long).len(), MAX_QUERY);
        assert_eq!(ids(&search(&root, "ada", &long, None)), ["m1"]);
        assert_eq!(
            sorted_ids(&search(&root, "ada", "harbour zeppelin", None)),
            ["m1", "m2"]
        );

        // An astral character costs the two units the main charges for it.
        let pair = format!("{}🐸", "a".repeat(199));
        assert_eq!(capped(&pair).chars().count(), 199);
    }

    #[test]
    fn an_index_that_is_not_there_answers_no_hits() {
        let missing =
            std::env::temp_dir().join(format!("toad-core-search-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        std::fs::create_dir_all(&missing).unwrap();

        assert_eq!(search(&missing, "ada", "harbour", None), no_hits());
        assert_eq!(search_all(&missing, "harbour", None), no_hits());

        // An index that is bytes rather than a database is the same answer:
        // SQLite opens it lazily, so the fault lands on the first read.
        std::fs::write(index_path(&missing), "not a database\n").unwrap();
        assert_eq!(search(&missing, "ada", "harbour", None), no_hits());
    }

    // --- the writer --------------------------------------------------------

    fn message_event(id: &str, ts: i64, kind: &str, text: &str) -> Value {
        json!({"kind": kind, "id": id, "ts": ts, "text": text})
    }

    /// A chapter marker as the room writes one: the same id twice, second time
    /// carrying what the summariser wrote and the moment it closed.
    fn chapter_event(id: &str, ts: i64, closed: bool) -> Value {
        let mut marker = json!({
            "kind": "chapter", "id": id, "ts": ts, "backendId": "cursor",
            "title": "Harbour week", "note": "we fixed the crane", "tags": ["harbour"]
        });
        if closed {
            marker["endedAt"] = json!(ts + 40);
            marker["status"] = json!("done");
        }
        marker
    }

    /// The rows a test compares, in the order the index holds them.
    fn rows_of(database: &Connection, statement: &str) -> Vec<Vec<SqlValue>> {
        let mut prepared = database.prepare(statement).unwrap();
        let count = prepared.column_count();
        let found = prepared
            .query_map([], |row| (0..count).map(|column| row.get(column)).collect())
            .unwrap();
        found.map(Result::unwrap).collect()
    }

    fn messages(database: &Connection) -> Vec<Vec<SqlValue>> {
        rows_of(
            database,
            "SELECT persona_id, event_id, chapter_id, kind, ts, text FROM messages ORDER BY rowid",
        )
    }

    fn chapter_rows(database: &Connection) -> Vec<Vec<SqlValue>> {
        rows_of(database, "SELECT * FROM chapters ORDER BY id")
    }

    fn chapter_text(database: &Connection) -> Vec<Vec<SqlValue>> {
        rows_of(
            database,
            "SELECT chapter_id, persona_id, text FROM chapters_fts",
        )
    }

    fn text(value: &str) -> SqlValue {
        SqlValue::Text(value.into())
    }

    fn number(value: i64) -> SqlValue {
        SqlValue::Integer(value)
    }

    /// A tape written through the writer this crate ships, so the indexer and
    /// a reindex are looking at the same file.
    fn tape(log: &Log, persona_id: &str, events: &[Value]) {
        for event in events {
            log.append(&StreamId::Tape(persona_id.to_string()), event)
                .unwrap();
        }
    }

    #[test]
    fn a_message_lands_under_the_open_chapter_and_a_replay_of_it_lands_nowhere() {
        let (_root, log, mut indexer) = indexer("write-message");
        let events = [
            chapter_event("c1", 100, false),
            message_event("m1", 110, "user", "the harbour crane is stuck"),
            message_event("m2", 120, "agent", "   "),
        ];
        tape(&log, "ada", &events);
        for event in &events {
            indexer.index_event("ada", event).unwrap();
        }
        // The same message twice is a replay of a line that is written once and
        // stands forever, so the second one changes nothing.
        indexer.index_event("ada", &events[1]).unwrap();

        assert_eq!(
            messages(&indexer.database),
            [vec![
                text("ada"),
                text("m1"),
                text("c1"),
                text("user"),
                number(110),
                text("the harbour crane is stuck"),
            ]]
        );
    }

    #[test]
    fn closing_a_chapter_updates_its_row_rather_than_adding_a_second_one() {
        let (_root, log, mut indexer) = indexer("write-chapter");
        let events = [
            chapter_event("c1", 100, false),
            message_event("m1", 110, "user", "the harbour crane is stuck"),
            chapter_event("c1", 100, true),
            message_event("m2", 150, "user", "after the chapter"),
        ];
        tape(&log, "ada", &events);
        for event in &events {
            indexer.index_event("ada", event).unwrap();
        }

        assert_eq!(
            chapter_rows(&indexer.database),
            [vec![
                text("c1"),
                text("ada"),
                number(100),
                number(140),
                text("Harbour week"),
                text("we fixed the crane"),
                text("done"),
                SqlValue::Null,
                text("cursor"),
            ]]
        );
        // Rewritten from scratch, not appended to: one row, title, note and tag.
        assert_eq!(
            chapter_text(&indexer.database),
            [vec![
                text("c1"),
                text("ada"),
                text("Harbour week\nwe fixed the crane\nharbour"),
            ]]
        );
        // The chapter closed, so what came after it belongs to no chapter.
        let after = messages(&indexer.database);
        assert_eq!(after[1][2], SqlValue::Null);
    }

    #[test]
    fn a_teammate_the_index_forgets_leaves_no_row_and_no_stamp() {
        let (_root, log, mut indexer) = indexer("forget");
        // A chapter id apiece: the id is the primary key, so two teammates
        // sharing one would be one row and this test would prove nothing.
        for (persona_id, chapter_id) in [("ada", "c1"), ("bob", "c2")] {
            let events = [
                chapter_event(chapter_id, 100, false),
                message_event("m1", 110, "user", "the harbour crane is stuck"),
            ];
            tape(&log, persona_id, &events);
            for event in &events {
                indexer.index_event(persona_id, event).unwrap();
            }
        }

        indexer.forget("ada").unwrap();

        assert_eq!(messages(&indexer.database).len(), 1);
        assert_eq!(messages(&indexer.database)[0][0], text("bob"));
        assert_eq!(chapter_rows(&indexer.database).len(), 1);
        assert_eq!(chapter_text(&indexer.database).len(), 1);
        assert_eq!(
            rows_of(
                &indexer.database,
                "SELECT persona_id FROM index_state ORDER BY persona_id"
            ),
            [vec![text("bob")]]
        );
    }

    #[test]
    fn a_stamp_that_matches_the_file_stops_a_sync_and_a_changed_file_restarts_it() {
        let (_root, log, mut indexer) = indexer("sync");
        tape(
            &log,
            "ada",
            &[message_event(
                "m1",
                110,
                "user",
                "the harbour crane is stuck",
            )],
        );
        indexer.sync(&["ada".to_string()]).unwrap();
        assert_eq!(messages(&indexer.database).len(), 1);

        // A row nobody indexed, to see whether the next sync rebuilds or not.
        indexer
            .database
            .execute(
                "INSERT INTO messages (persona_id, event_id, chapter_id, kind, ts, text) VALUES ('ada', 'ghost', NULL, 'user', 1, 'ghost')",
                [],
            )
            .unwrap();
        indexer.sync(&["ada".to_string()]).unwrap();
        assert_eq!(messages(&indexer.database).len(), 2);

        // The file changed, so the whole teammate is re-read and the ghost goes.
        tape(
            &log,
            "ada",
            &[message_event("m2", 120, "agent", "the crane is fixed")],
        );
        indexer.sync(&["ada".to_string()]).unwrap();
        let found: Vec<String> = messages(&indexer.database)
            .iter()
            .map(|row| match &row[1] {
                SqlValue::Text(id) => id.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(found, ["m1", "m2"]);

        // A teammate with no tape at all has nothing to stamp and nothing to do.
        indexer.sync(&["nobody".to_string()]).unwrap();
        assert_eq!(messages(&indexer.database).len(), 2);
    }

    /// The rows below came out of the main's own indexer. Produced by running,
    /// against a throwaway `TOAD_DATA_DIR`, a script that appends these six
    /// events with `src/bun/store/transcript.ts` and hands each to
    /// `src/bun/store/search.ts`'s `indexEvent`, then dumps `sqlite_master`,
    /// `messages`, `chapters`, `chapters_fts` and `index_state`, and then calls
    /// `reindex` and dumps them again.
    ///
    /// The two dumps differ in one cell, and that difference is the pin: read
    /// event by event, a message after a chapter closed belongs to no chapter,
    /// because the marker's close cleared the pointer. Rebuilt from the file,
    /// it belongs to that chapter, because the close superseded the marker by
    /// id and the fold leaves one marker with everything after it in its slice.
    /// The main has answered searches that way since chapters shipped; this
    /// build has to answer them the same way, so the quirk is pinned rather
    /// than fixed.
    #[test]
    fn an_index_this_writes_is_the_one_the_main_writes() {
        let (_root, log, mut indexer) = indexer("pin");
        let events = [
            chapter_event("c1", 100, false),
            message_event("m1", 110, "user", "the harbour crane is stuck"),
            message_event("m2", 120, "agent", "  "),
            message_event("m3", 130, "agent", "the crane is fixed"),
            chapter_event("c1", 100, true),
            message_event("m4", 150, "user", "after the chapter"),
        ];
        tape(&log, "pin-index", &events);
        for event in &events {
            indexer.index_event("pin-index", event).unwrap();
        }

        assert_eq!(
            rows_of(
                &indexer.database,
                "SELECT type, name FROM sqlite_master ORDER BY name"
            ),
            [
                ["table", "chapters"],
                ["table", "chapters_fts"],
                ["table", "chapters_fts_config"],
                ["table", "chapters_fts_content"],
                ["table", "chapters_fts_data"],
                ["table", "chapters_fts_docsize"],
                ["table", "chapters_fts_idx"],
                ["index", "chapters_persona"],
                ["table", "index_state"],
                ["table", "messages"],
                ["table", "messages_config"],
                ["table", "messages_content"],
                ["table", "messages_data"],
                ["table", "messages_docsize"],
                ["table", "messages_idx"],
                ["index", "sqlite_autoindex_chapters_1"],
                ["index", "sqlite_autoindex_index_state_1"],
            ]
            .map(|row| row.map(text).to_vec())
        );

        let indexed = [
            vec![
                text("pin-index"),
                text("m1"),
                text("c1"),
                text("user"),
                number(110),
                text("the harbour crane is stuck"),
            ],
            vec![
                text("pin-index"),
                text("m3"),
                text("c1"),
                text("agent"),
                number(130),
                text("the crane is fixed"),
            ],
            vec![
                text("pin-index"),
                text("m4"),
                SqlValue::Null,
                text("user"),
                number(150),
                text("after the chapter"),
            ],
        ];
        assert_eq!(messages(&indexer.database), indexed);
        assert_eq!(
            chapter_rows(&indexer.database),
            [vec![
                text("c1"),
                text("pin-index"),
                number(100),
                number(140),
                text("Harbour week"),
                text("we fixed the crane"),
                text("done"),
                SqlValue::Null,
                text("cursor"),
            ]]
        );
        assert_eq!(
            chapter_text(&indexer.database),
            [vec![
                text("c1"),
                text("pin-index"),
                text("Harbour week\nwe fixed the crane\nharbour"),
            ]]
        );
        // The tape the main wrote for these six events was 533 bytes long, and
        // the stamp is what the next start compares against.
        assert_eq!(
            rows_of(
                &indexer.database,
                "SELECT persona_id, size FROM index_state ORDER BY persona_id"
            ),
            [vec![text("pin-index"), number(533)]]
        );

        indexer.reindex("pin-index").unwrap();
        let mut rebuilt = indexed;
        rebuilt[2][2] = text("c1");
        assert_eq!(messages(&indexer.database), rebuilt);
        assert_eq!(
            chapter_rows(&indexer.database),
            [vec![
                text("c1"),
                text("pin-index"),
                number(100),
                number(140),
                text("Harbour week"),
                text("we fixed the crane"),
                text("done"),
                SqlValue::Null,
                text("cursor"),
            ]]
        );
    }
}
