//! Import an existing Toad data directory into this room.
//!
//! The source is never written: `store.sqlite` is opened read-only, tapes are
//! copied, and secrets are read out of the old vault. A teammate already in
//! the roster is left alone, and a tape that already exists here is not
//! overwritten, so running the import twice is the same as running it once.
//! A teammate's working directory stays where it is — under the old data
//! directory's `workspaces/` when that was the default — because the
//! workspace is the project, not a copy of it. Backend ids are mapped onto
//! this registry; an id with no counterpart is kept so the session can
//! refuse it in a sentence.

mod personas;
mod records;

use crate::contract::{
    Face, HarnessChoice, Persona, PersonaComputer, PersonaSubagents, Reach, WebSearchPolicy,
};
use crate::log::{Log, StreamId};
use crate::store::search::Indexer;
use crate::vault::Vault;
use crate::{paths, room};
use records::{list_records, open};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use ts_rs::TS;

/// What an import did: how many of each thing came over, and what it left.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Report {
    pub teammates: i64,
    pub tapes: i64,
    pub settings: i64,
    pub keys: i64,
    pub skipped: Vec<Skipped>,
}

/// One note from the import: a thing left behind, or a backend this
/// registry has no harness for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct Skipped {
    pub item: String,
    pub reason: String,
}

const IMPORTED_SETTINGS: [&str; 2] = ["chapterIdleHours", "defaultBackendId"];
const IMPORTED_PROVIDERS: [&str; 3] = ["anthropic", "openai", "openrouter"];

/// Old Toad backend id → this registry's id, for every DEFAULT harness both
/// trees name as the same agent. Evidence is the previous Toad's
/// `src/bun/acp/registry.ts` and this tree's `driver/acp/registry.rs`.
/// An id already equal to its counterpart passes through; an id absent
/// here is imported as written.
///
/// `pi` → `pi`. Toad Agent. The previous Toad's `PI_BACKEND_ID` /
/// `DEFAULT_BACKEND_ID`; this tree's `driver::PI_BACKEND_ID`. Not in the
/// ACP registry: there is no child to launch.
///
/// `cursor` → `cursor`. Cursor. Old `NATIVE_BACKENDS.cursor` launches
/// `cursor-agent acp`. This tree's `NATIVE` row is the same id and launch.
///
/// `opencode` → `opencode`. opencode. Old `NATIVE_BACKENDS.opencode`
/// launches `opencode acp`. This tree's `NATIVE` row is the same.
///
/// `gemini` → `gemini`. Gemini CLI. Old `NATIVE_BACKENDS.gemini` launches
/// `gemini --acp`. This tree's `NATIVE` row is the same.
///
/// `claude-acp` → `claude-acp`. Claude Code. Old `ADAPTED_BACKENDS["claude-acp"]`
/// is the Claude Code ACP adapter (`npx @agentclientprotocol/claude-agent-acp`,
/// client `claude`). This tree's `ADAPTED` row uses the same id, name, and
/// package.
///
/// `codex-acp` → `codex-acp`. Codex. Old `ADAPTED_BACKENDS["codex-acp"]` is
/// the Codex ACP adapter (`npx @agentclientprotocol/codex-acp`, client
/// `codex`). This tree's `ADAPTED` row uses the same id, name, and package.
const BACKEND_COUNTERPARTS: &[(&str, &str)] = &[
    ("pi", "pi"),
    ("cursor", "cursor"),
    ("opencode", "opencode"),
    ("gemini", "gemini"),
    ("claude-acp", "claude-acp"),
    ("codex-acp", "codex-acp"),
];

/// This registry's id for an old Toad backend, when both trees taught the
/// same harness.
fn counterpart(old: &str) -> Option<&'static str> {
    BACKEND_COUNTERPARTS
        .iter()
        .find(|(from, _)| *from == old)
        .map(|(_, to)| *to)
}

/// Copies the previous Toad's roster, tapes, settings and keys into `log` and
/// `vault`. `from` is opened read-only and is never written.
pub fn import(from: &Path, log: &Log, vault: &Vault) -> io::Result<Report> {
    if !from.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a Toad data directory", from.display()),
        ));
    }

    let mut report = Report::default();
    // SQLite's read-only open still creates `-wal`/`-shm` beside a closed
    // WAL file, so the store is read from a copy. Tapes and settings are
    // ordinary files and are read in place.
    let snapshot = snapshot_store(from)?;
    let store_root = snapshot.as_ref().map(StoreSnapshot::path).unwrap_or(from);
    let imported = import_teammates(store_root, from, log, &mut report)?;
    import_tapes(from, log, &imported, &mut report)?;
    import_settings(from, log, &mut report)?;
    import_keys(store_root, from, vault, &mut report)?;

    if !imported.is_empty() {
        let mut indexer = Indexer::open(log).map_err(io::Error::other)?;
        indexer.sync(&imported).map_err(io::Error::other)?;
    }

    Ok(report)
}

fn import_teammates(
    store_root: &Path,
    workspace_root: &Path,
    log: &Log,
    report: &mut Report,
) -> io::Result<Vec<String>> {
    let existing: HashSet<String> = room::roster(log)
        .into_iter()
        .map(|persona| persona.id)
        .collect();
    let mut imported = Vec::new();
    for value in personas::list_local_personas_from(store_root, workspace_root) {
        let Some(id) = value.get("id").and_then(Value::as_str).map(str::to_string) else {
            report.skipped.push(Skipped {
                item: "teammate".into(),
                reason: "a row had no id".into(),
            });
            continue;
        };
        if existing.contains(&id) {
            report.skipped.push(Skipped {
                item: format!("teammate {id}"),
                reason: "already in the roster".into(),
            });
            continue;
        }
        let mut persona = match persona_from_legacy(value) {
            Ok(persona) => persona,
            Err(reason) => {
                report.skipped.push(Skipped {
                    item: format!("teammate {id}"),
                    reason,
                });
                continue;
            }
        };
        match counterpart(&persona.backend_id) {
            Some(mapped) => persona.backend_id = mapped.to_string(),
            None => report.skipped.push(Skipped {
                item: format!("teammate {}", persona.name),
                reason: format!("backend {} has no counterpart here", persona.backend_id),
            }),
        }
        append_persona(log, &persona)?;
        report.teammates += 1;
        imported.push(id.clone());
    }
    Ok(imported)
}

fn import_tapes(
    from: &Path,
    log: &Log,
    imported: &[String],
    report: &mut Report,
) -> io::Result<()> {
    let dest = log.root();
    for id in imported {
        if tape_exists(dest, id) {
            report.skipped.push(Skipped {
                item: format!("tape {id}"),
                reason: "already exists here".into(),
            });
            continue;
        }
        if copy_tape(from, dest, id)? {
            report.tapes += 1;
        }
    }
    Ok(())
}

fn import_settings(from: &Path, log: &Log, report: &mut Report) -> io::Result<()> {
    let Some(settings) = source_settings(from) else {
        return Ok(());
    };
    for key in IMPORTED_SETTINGS {
        let Some(value) = settings.get(key).cloned() else {
            continue;
        };
        if setting_written(log, key) {
            report.skipped.push(Skipped {
                item: format!("setting {key}"),
                reason: "already set".into(),
            });
            continue;
        }
        log.append(
            &StreamId::Room,
            &json!({ "kind": "setting", "id": key, "value": value }),
        )?;
        report.settings += 1;
    }
    Ok(())
}

fn import_keys(
    store_root: &Path,
    from: &Path,
    vault: &Vault,
    report: &mut Report,
) -> io::Result<()> {
    let Some(database) = open(store_root) else {
        return Ok(());
    };
    let secrets = source_secrets(from)?;
    let mut held: HashSet<(String, String)> = vault
        .list()
        .into_iter()
        .filter(|credential| !credential.revoked)
        .map(|credential| (credential.provider_id, credential.label))
        .collect();

    for record in list_records(&database, "credential") {
        let provider = record
            .replicated
            .get("providerId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let label = record
            .replicated
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
            .unwrap_or(provider);
        let item = format!("key {label} ({provider})");
        let kind = record
            .replicated
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("api_key");
        if kind == "oauth" {
            report.skipped.push(Skipped {
                item,
                reason: "oauth credentials are not imported".into(),
            });
            continue;
        }
        if record.replicated.get("revoked").and_then(Value::as_bool) == Some(true) {
            report.skipped.push(Skipped {
                item,
                reason: "revoked".into(),
            });
            continue;
        }
        if !IMPORTED_PROVIDERS.contains(&provider) {
            report.skipped.push(Skipped {
                item,
                reason: "provider is not anthropic, openai or openrouter".into(),
            });
            continue;
        }
        if !held.insert((provider.to_string(), label.to_string())) {
            report.skipped.push(Skipped {
                item,
                reason: "already in this vault".into(),
            });
            continue;
        }
        let Some(secret) = secrets.get(&record.id).filter(|secret| !secret.is_empty()) else {
            held.remove(&(provider.to_string(), label.to_string()));
            report.skipped.push(Skipped {
                item,
                reason: "no secret in the source vault".into(),
            });
            continue;
        };
        vault.create(provider, label, secret)?;
        report.keys += 1;
    }
    Ok(())
}

/// A persona event is the teammate's record with the kind beside it — the
/// same shape `append_persona` on the wire writes.
fn append_persona(log: &Log, persona: &Persona) -> io::Result<()> {
    let mut event = json!(persona);
    event
        .as_object_mut()
        .expect("a teammate serializes as an object")
        .insert("kind".into(), Value::from("persona"));
    log.append(&StreamId::Room, &event).map(|_| ())
}

/// The previous Toad's persona JSON, as a teammate this room can hold.
///
/// Optional fields an older build spelled differently (an emoji face, a
/// `{brave: false}` web-search policy) are dropped rather than taking the
/// whole teammate with them: the roster is the thing being imported, and a
/// face can be chosen again.
fn persona_from_legacy(mut value: Value) -> Result<Persona, String> {
    if let Some(object) = value.as_object_mut() {
        strip_if_invalid::<Face>(object, "face");
        strip_if_invalid::<WebSearchPolicy>(object, "webSearchPolicy");
        strip_if_invalid::<PersonaComputer>(object, "computer");
        strip_if_invalid::<PersonaSubagents>(object, "subagents");
        strip_if_invalid::<HarnessChoice>(object, "harnessOverride");
        strip_if_invalid::<Reach>(object, "reach");
    }
    serde_json::from_value(value)
        .map_err(|error| format!("its record does not read as a teammate ({error})"))
}

fn strip_if_invalid<T: for<'de> Deserialize<'de>>(object: &mut Map<String, Value>, key: &str) {
    let Some(value) = object.get(key) else {
        return;
    };
    if serde_json::from_value::<T>(value.clone()).is_err() {
        object.remove(key);
    }
}

fn tape_exists(root: &Path, id: &str) -> bool {
    paths::transcript_segments_dir(root, id).exists() || paths::transcript_path(root, id).exists()
}

/// Copies the segmented directory if it is there, otherwise the flat file,
/// unchanged. Returns whether anything was copied.
fn copy_tape(from: &Path, to: &Path, id: &str) -> io::Result<bool> {
    let source_dir = paths::transcript_segments_dir(from, id);
    if source_dir.is_dir() {
        copy_dir(&source_dir, &paths::transcript_segments_dir(to, id))?;
        return Ok(true);
    }
    let source_flat = paths::transcript_path(from, id);
    if source_flat.is_file() {
        let dest = paths::transcript_path(to, id);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source_flat, &dest)?;
        return Ok(true);
    }
    Ok(false)
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn source_settings(from: &Path) -> Option<Map<String, Value>> {
    let text = fs::read_to_string(from.join("settings.json")).ok()?;
    match serde_json::from_str(&text).ok()? {
        Value::Object(stored) => stored.get("settings").and_then(Value::as_object).cloned(),
        _ => None,
    }
}

fn setting_written(log: &Log, key: &str) -> bool {
    log.load(&StreamId::Room).iter().any(|event| {
        event.get("kind").and_then(Value::as_str) == Some("setting")
            && event.get("id").and_then(Value::as_str) == Some(key)
    })
}

/// A copy of `store.sqlite` (and its sidecars, when they are there) so the
/// importer can open SQLite without creating files in the source.
struct StoreSnapshot {
    dir: PathBuf,
}

impl StoreSnapshot {
    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for StoreSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn snapshot_store(from: &Path) -> io::Result<Option<StoreSnapshot>> {
    let store = records::store_path(from);
    if !store.exists() {
        return Ok(None);
    }
    let dir = std::env::temp_dir().join(format!(
        "toad-import-store-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir)?;
    fs::copy(&store, dir.join("store.sqlite"))?;
    if let Some(name) = store.file_name() {
        for suffix in ["-wal", "-shm"] {
            let sidecar = store.with_file_name(format!("{}{suffix}", name.to_string_lossy()));
            if sidecar.is_file() {
                fs::copy(&sidecar, dir.join(format!("store.sqlite{suffix}")))?;
            }
        }
    }
    Ok(Some(StoreSnapshot { dir }))
}

fn source_secrets(from: &Path) -> io::Result<BTreeMap<String, String>> {
    let path = from.join("credentials").join("vault.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    let stored: Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    let Some(secrets) = stored.get("secrets").and_then(Value::as_object) else {
        return Ok(BTreeMap::new());
    };
    let mut map = BTreeMap::new();
    for (id, value) in secrets {
        if let Some(secret) = value.as_str() {
            map.insert(id.clone(), secret.to_string());
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::records::fixture::{Put, create, scratch as store_scratch};
    use super::*;
    use crate::store::search;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("toad-core-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Every file under `root`: relative path to (mtime nanos, content hash).
    /// A write that changes bytes or the clock on a file fails the comparison.
    fn fingerprint(root: &Path) -> BTreeMap<String, (u128, u64)> {
        let mut files = BTreeMap::new();
        walk(root, root, &mut files);
        files
    }

    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, (u128, u64)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, files);
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let metadata = fs::metadata(&path).unwrap();
            let mtime = metadata
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let bytes = fs::read(&path).unwrap();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            files.insert(relative, (mtime, hasher.finish()));
        }
    }

    fn write_old_room(from: &Path) {
        let database = create(from, "this-desk");
        Put {
            machine: Some(json!({ "cwd": "/tmp/ada" })),
            ..Put::new(
                "ada",
                "this-desk",
                json!({
                    "name": "Ada",
                    "goal": "Keep the harbour running.",
                    "backendId": "pi",
                }),
            )
        }
        .write(&database);
        Put {
            machine: Some(json!({ "cwd": "/tmp/bob" })),
            ..Put::new(
                "bob",
                "this-desk",
                json!({
                    "name": "Bob",
                    "goal": "Watch the crane.",
                    "backendId": "pi",
                }),
            )
        }
        .write(&database);
        Put {
            machine: Some(json!({ "cwd": "/tmp/cursor" })),
            ..Put::new(
                "cal",
                "this-desk",
                json!({
                    "name": "Cal",
                    "goal": "Drive Cursor.",
                    "backendId": "cursor",
                }),
            )
        }
        .write(&database);
        Put::new("theirs", "peer-desk", json!({ "name": "Theirs" })).write(&database);
        Put {
            deleted: true,
            ..Put::new("buried", "this-desk", json!({ "name": "Buried" }))
        }
        .write(&database);

        Put {
            kind: "credential",
            ..Put::new(
                "cred-ant",
                "this-desk",
                json!({
                    "providerId": "anthropic",
                    "label": "work",
                    "kind": "api_key",
                    "revoked": false,
                }),
            )
        }
        .write(&database);
        Put {
            kind: "credential",
            ..Put::new(
                "cred-oai",
                "this-desk",
                json!({
                    "providerId": "openai",
                    "label": "personal",
                    "kind": "api_key",
                    "revoked": false,
                }),
            )
        }
        .write(&database);
        Put {
            kind: "credential",
            ..Put::new(
                "cred-or",
                "this-desk",
                json!({
                    "providerId": "openrouter",
                    "label": "spare",
                    "kind": "api_key",
                    "revoked": false,
                }),
            )
        }
        .write(&database);
        Put {
            kind: "credential",
            ..Put::new(
                "cred-oauth",
                "this-desk",
                json!({
                    "providerId": "anthropic",
                    "label": "claude-login",
                    "kind": "oauth",
                    "revoked": false,
                }),
            )
        }
        .write(&database);
        Put {
            kind: "credential",
            ..Put::new(
                "cred-revoked",
                "this-desk",
                json!({
                    "providerId": "openai",
                    "label": "old",
                    "kind": "api_key",
                    "revoked": true,
                }),
            )
        }
        .write(&database);
        database.close().unwrap();

        fs::create_dir_all(paths::transcript_segments_dir(from, "ada")).unwrap();
        fs::write(
            paths::transcript_segment_path(from, "ada", 1),
            "{\"kind\":\"user\",\"id\":\"u1\",\"ts\":1000,\"text\":\"hello from ada\"}\n",
        )
        .unwrap();
        fs::write(
            paths::transcript_path(from, "bob"),
            "{\"kind\":\"user\",\"id\":\"u2\",\"ts\":2000,\"text\":\"hello from bob\"}\n",
        )
        .unwrap();

        fs::write(
            from.join("settings.json"),
            json!({
                "version": 1,
                "settings": {
                    "chapterIdleHours": 4,
                    "defaultBackendId": "pi",
                    "theme": "dark",
                },
                "lastPersonaId": "ada",
            })
            .to_string(),
        )
        .unwrap();

        fs::create_dir_all(from.join("credentials")).unwrap();
        fs::write(
            from.join("credentials").join("vault.json"),
            json!({
                "version": 1,
                "secrets": {
                    "cred-ant": "sk-ant-import",
                    "cred-oai": "sk-oai-import",
                    "cred-or": "sk-or-import",
                    "cred-revoked": "sk-revoked-unused",
                }
            })
            .to_string(),
        )
        .unwrap();
    }

    fn dest(name: &str) -> (PathBuf, Log, Vault) {
        let root = scratch(name);
        let log = Log::open(&root);
        let vault = Vault::open(&root, log.clone()).unwrap();
        (root, log, vault)
    }

    #[test]
    fn an_old_data_directory_comes_over_once_and_the_source_is_untouched() {
        let from = store_scratch("import-source");
        write_old_room(&from);
        let before = fingerprint(&from);

        let (to, log, vault) = dest("import-dest");
        let first = import(&from, &log, &vault).unwrap();

        assert_eq!(first.teammates, 3, "{first:?}");
        assert_eq!(first.tapes, 2, "{first:?}");
        assert_eq!(first.settings, 2, "{first:?}");
        assert_eq!(first.keys, 3, "{first:?}");
        let reasons: Vec<(&str, &str)> = first
            .skipped
            .iter()
            .map(|skipped| (skipped.item.as_str(), skipped.reason.as_str()))
            .collect();
        // A teammate on a harness this registry already names comes over
        // under that id. Whether the binary is on PATH is the session's
        // question, not the import's.
        assert!(
            !reasons.iter().any(|(item, _)| *item == "teammate cal"),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|(item, reason)| *item == "key claude-login (anthropic)"
                    && reason.contains("oauth")),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|(item, reason)| *item == "key old (openai)" && *reason == "revoked"),
            "{reasons:?}"
        );

        let roster = room::roster(&log);
        let names: Vec<&str> = roster.iter().map(|persona| persona.name.as_str()).collect();
        assert_eq!(names, ["Ada", "Bob", "Cal"]);
        assert_eq!(roster[2].backend_id, "cursor");
        assert!(roster.iter().all(|persona| persona.id != "theirs"));
        assert!(roster.iter().all(|persona| persona.id != "buried"));

        assert_eq!(
            fs::read(paths::transcript_segment_path(&to, "ada", 1)).unwrap(),
            fs::read(paths::transcript_segment_path(&from, "ada", 1)).unwrap()
        );
        assert_eq!(
            fs::read(paths::transcript_path(&to, "bob")).unwrap(),
            fs::read(paths::transcript_path(&from, "bob")).unwrap()
        );

        let settings = room::settings(&log);
        assert_eq!(settings["chapterIdleHours"], 4);
        assert_eq!(settings["defaultBackendId"], "pi");
        assert!(settings.get("theme").is_none());

        let keys = vault.provider_keys();
        assert_eq!(
            keys.get("anthropic").map(String::as_str),
            Some("sk-ant-import")
        );
        assert_eq!(
            keys.get("openai").map(String::as_str),
            Some("sk-oai-import")
        );
        assert_eq!(
            keys.get("openrouter").map(String::as_str),
            Some("sk-or-import")
        );

        let hits = search::search(log.root(), "ada", "hello from ada", None);
        assert!(
            !hits["hits"].as_array().unwrap().is_empty(),
            "the imported tape should be searchable: {hits}"
        );

        assert_eq!(
            fingerprint(&from),
            before,
            "the source directory's mtime and contents must not change"
        );

        let second = import(&from, &log, &vault).unwrap();
        assert_eq!(second.teammates, 0, "{second:?}");
        assert_eq!(second.tapes, 0, "{second:?}");
        assert_eq!(second.settings, 0, "{second:?}");
        assert_eq!(second.keys, 0, "{second:?}");
        assert!(
            !second.skipped.is_empty(),
            "the second run should skip everything that came over"
        );
        assert!(
            second
                .skipped
                .iter()
                .any(|skipped| skipped.item == "teammate ada"
                    && skipped.reason == "already in the roster"),
            "{second:?}"
        );
        assert_eq!(room::roster(&log).len(), 3);
        assert_eq!(vault.provider_keys().len(), 3);
        assert_eq!(fingerprint(&from), before);
    }

    /// Claude Code's old id is this registry's id, so a teammate that named
    /// `claude-acp` starts here as Claude Code. An id nobody taught is kept,
    /// and the report says so.
    #[test]
    fn a_known_harness_is_mapped_and_an_unknown_one_is_kept() {
        let from = store_scratch("import-backends");
        let database = create(&from, "this-desk");
        Put {
            machine: Some(json!({ "cwd": "/tmp/claude" })),
            ..Put::new(
                "claude",
                "this-desk",
                json!({
                    "name": "Claude",
                    "goal": "Drive Claude Code.",
                    "backendId": "claude-acp",
                }),
            )
        }
        .write(&database);
        Put {
            machine: Some(json!({ "cwd": "/tmp/stranger" })),
            ..Put::new(
                "stranger",
                "this-desk",
                json!({
                    "name": "Stranger",
                    "goal": "A harness this tree never taught.",
                    "backendId": "nonesuch",
                }),
            )
        }
        .write(&database);
        database.close().unwrap();

        let (_to, log, vault) = dest("import-backends-dest");
        let report = import(&from, &log, &vault).unwrap();
        assert_eq!(report.teammates, 2, "{report:?}");

        let roster = room::roster(&log);
        let claude = roster
            .iter()
            .find(|persona| persona.id == "claude")
            .unwrap();
        assert_eq!(claude.backend_id, "claude-acp");
        let stranger = roster
            .iter()
            .find(|persona| persona.id == "stranger")
            .unwrap();
        assert_eq!(stranger.backend_id, "nonesuch");

        assert!(
            report
                .skipped
                .iter()
                .any(|skipped| skipped.item == "teammate Stranger"
                    && skipped.reason == "backend nonesuch has no counterpart here"),
            "{report:?}"
        );
        assert!(
            !report
                .skipped
                .iter()
                .any(|skipped| skipped.item.contains("Claude")),
            "{report:?}"
        );
    }
}
