//! A teammate, assembled out of the three classes its record carries.
//!
//! This is `personaOf` in `src/bun/store/personas.ts`, field by field, and it
//! has to stay that way: the window is handed whichever of the two built the
//! answer, so a field one of them omits and the other spells is a teammate that
//! changes shape depending on who asked.
//!
//! Normalization happens on the way out rather than at write time because the
//! store holds rows written by older builds. A field the store never learned
//! falls back to what a fresh teammate would have had, and a field the store
//! learned and then unlearned is simply not read: a row replicated from a 0.4.x
//! desk still carries `plugins`, and building the teammate key by key is what
//! makes that a non-event rather than a crash.

use super::records::{ResourceRecord, list_records, local_node_id, open};
use crate::paths::default_workspace;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::Path;

/// Toad Agent, which is what a teammate runs when its row names no harness.
/// The literal is `DEFAULT_BACKEND_ID` in `src/bun/acp/registry.ts`; it is
/// written into every persona, so it is an identifier and not a label.
const DEFAULT_BACKEND_ID: &str = "pi";

const MAX_SUBAGENT_EXTRAS: usize = 16;
const MAX_SUBAGENT_NAME: usize = 60;
const MAX_SUBAGENT_DESCRIPTION: usize = 400;
const MAX_SUBAGENT_PROMPT: usize = 4_000;
const MAX_SUBAGENT_ID: usize = 40;
const MAX_SUBAGENT_MODEL_ID: usize = 120;

/// Reserved `kind` for the built-in task runner. Operators cannot take this id.
const GENERIC_SUBAGENT_KIND: &str = "generic";

const DEFAULT_TASK_RUNNER_DESCRIPTION: &str = "A silent coding runner in this workspace. Use for bounded work that would take many tool calls, or for pieces that can run at the same time.";

/// This desk's teammates, in the order the store holds them.
///
/// Only rows this node owns: a linked desktop's teammates reach the rail by
/// another road, and the merge and this desk's arrangement of it are the
/// caller's to apply.
#[cfg(test)]
pub fn list_local_personas(root: &Path) -> Vec<Value> {
    list_local_personas_from(root, root)
}

/// The same listing, with the store opened at `store_root` and workspaces
/// named from `workspace_root`.
///
/// The importer reads a copy of `store.sqlite` so opening SQLite cannot
/// create `-wal`/`-shm` beside a directory it must not write; the workspace
/// default still has to be the source directory's, not the copy's.
pub(super) fn list_local_personas_from(store_root: &Path, workspace_root: &Path) -> Vec<Value> {
    list_personas_from(store_root, workspace_root, true)
}

/// Teammates this store holds that another desk owns.
///
/// Dropping them is the decision — v1 has no fleet — but the importer
/// still has to say so, or a name that was on the rail vanishes without
/// a word.
pub(super) fn list_foreign_personas_from(store_root: &Path, workspace_root: &Path) -> Vec<Value> {
    list_personas_from(store_root, workspace_root, false)
}

fn list_personas_from(store_root: &Path, workspace_root: &Path, local: bool) -> Vec<Value> {
    let Some(database) = open(store_root) else {
        return Vec::new();
    };
    let Some(node_id) = local_node_id(&database) else {
        return Vec::new();
    };
    list_records(&database, "persona")
        .iter()
        .filter(|record| (record.owner_node == node_id) == local)
        // An id with no characters in it cannot name a workspace or a tape, so
        // a row carrying one is not a teammate this room could ever open.
        .filter(|record| !record.id.is_empty())
        .map(|record| persona_of(record, workspace_root))
        .collect()
}

/// A string field, or nothing when the stored value is not one.
fn text(class: &Map<String, Value>, key: &str) -> Option<String> {
    class.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The same, treating an empty string as nothing — the `||` in the original,
/// where a row holding `""` falls back rather than shipping a blank.
fn nonempty(class: &Map<String, Value>, key: &str) -> Option<String> {
    text(class, key).filter(|value| !value.is_empty())
}

/// Whether JavaScript would have taken this value as present.
///
/// Two fields are carried through unexamined — `face`, which is the UI's own
/// vocabulary, and `webSearchPolicy` — and the original guards both with a
/// plain truthiness test. Spelling that test here is what keeps a stored `null`
/// or `""` omitted rather than passed on as a value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(string) => !string.is_empty(),
        _ => true,
    }
}

fn insert_if_truthy(persona: &mut Map<String, Value>, key: &str, class: &Map<String, Value>) {
    if let Some(value) = class.get(key).filter(|value| truthy(value)) {
        persona.insert(key.to_string(), value.clone());
    }
}

/// Carries a portable boolean grant only when it is actually true.
///
/// Unlike the older fields handled by [`insert_if_truthy`], a malformed value
/// here must not make the whole teammate fail deserialization: an upgrade must
/// close an unrecognised grant safely.
fn insert_if_true(persona: &mut Map<String, Value>, key: &str, class: &Map<String, Value>) {
    if class.get(key).and_then(Value::as_bool) == Some(true) {
        persona.insert(key.to_string(), Value::Bool(true));
    }
}

/// `slice(0, max)` as JavaScript counts it: in UTF-16 code units, cut on a
/// character boundary, so a cut that would split a character drops it whole.
fn truncate_utf16(value: &str, max: usize) -> String {
    let mut units = 0;
    let mut end = 0;
    for (index, character) in value.char_indices() {
        units += character.len_utf16();
        if units > max {
            break;
        }
        end = index + character.len_utf8();
    }
    value[..end].to_string()
}

fn clip(value: &str, max: usize) -> String {
    truncate_utf16(value.trim(), max)
}

/// A clipped string field, or nothing when it is absent, not a string, or
/// nothing but whitespace.
fn optional_text(class: &Map<String, Value>, key: &str, max: usize) -> Option<String> {
    let clipped = clip(class.get(key)?.as_str()?, max);
    (!clipped.is_empty()).then_some(clipped)
}

fn object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

/// A stored harness choice, or nothing when missing or malformed.
fn normalize_harness(value: Option<&Value>) -> Option<Value> {
    let candidate = object(value)?;
    let backend_id = nonempty(candidate, "backendId")?;
    let mut choice = Map::new();
    choice.insert("backendId".to_string(), json!(backend_id));
    if let Some(model_id) = nonempty(candidate, "modelId") {
        choice.insert("modelId".to_string(), json!(model_id));
    }
    Some(Value::Object(choice))
}

/// A stored computer setting, or nothing when missing or malformed.
fn normalize_computer(value: Option<&Value>) -> Option<Value> {
    let candidate = object(value)?;
    let enabled = candidate.get("enabled")?.as_bool()?;
    let mut computer = Map::new();
    computer.insert("enabled".to_string(), json!(enabled));
    if let Some(image) = candidate.get("image").and_then(Value::as_str) {
        let image = image.trim();
        if !image.is_empty() {
            computer.insert("image".to_string(), json!(image));
        }
    }
    Some(Value::Object(computer))
}

/// Checkpoints with a usable backend and session id; anything else is dropped.
fn normalize_checkpoints(value: Option<&Value>) -> Value {
    let mut checkpoints = Vec::new();
    for entry in value.and_then(Value::as_array).into_iter().flatten() {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let (Some(backend_id), Some(session_id)) =
            (nonempty(entry, "backendId"), nonempty(entry, "sessionId"))
        else {
            continue;
        };
        checkpoints.push(json!({ "backendId": backend_id, "sessionId": session_id }));
    }
    Value::Array(checkpoints)
}

/// Preserve a stored grant; a missing or malformed mode grants nothing.
fn normalize_policy(value: Option<&Value>) -> Value {
    let candidate = object(value);
    let mode = candidate
        .and_then(|policy| policy.get("mode"))
        .and_then(Value::as_str)
        .filter(|mode| matches!(*mode, "all" | "none" | "some"))
        .unwrap_or("none");
    let server_ids: Vec<&str> = candidate
        .and_then(|policy| policy.get("serverIds"))
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    json!({ "mode": mode, "serverIds": server_ids })
}

fn is_reserved_subagent_id(id: &str) -> bool {
    id == GENERIC_SUBAGENT_KIND
}

/// `/^[a-z][a-z0-9-]{0,39}$/`, and not the reserved kind.
fn is_legal_subagent_id(id: &str) -> bool {
    let mut characters = id.chars();
    let starts = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    starts
        && id.len() <= MAX_SUBAGENT_ID
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !is_reserved_subagent_id(id)
}

/// Kind id from a display name. Empty when nothing legal remains.
fn slugify_subagent_id(name: &str) -> String {
    let lowered = name.to_lowercase();
    let mut slug = String::new();
    for character in lowered.chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    truncate_utf16(slug.trim_matches('-'), MAX_SUBAGENT_ID)
}

/// A free id near the one that was asked for, or nothing when there is none.
fn unique_extra_id(wanted: &str, taken: &HashSet<String>) -> Option<String> {
    let base = if is_legal_subagent_id(wanted) {
        wanted.to_string()
    } else {
        slugify_subagent_id(wanted)
    };
    if base.is_empty() || is_reserved_subagent_id(&base) {
        return None;
    }
    if !taken.contains(&base) && is_legal_subagent_id(&base) {
        return Some(base);
    }
    for suffix in 2..100 {
        let head = truncate_utf16(&base, MAX_SUBAGENT_ID - suffix.to_string().len() - 1);
        let candidate = format!("{head}-{suffix}");
        if !taken.contains(&candidate) && is_legal_subagent_id(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn normalize_defaults(value: Option<&Value>) -> Option<Value> {
    let raw = object(value)?;
    let mut defaults = Map::new();
    for (key, max) in [
        ("name", MAX_SUBAGENT_NAME),
        ("description", MAX_SUBAGENT_DESCRIPTION),
        ("prompt", MAX_SUBAGENT_PROMPT),
        ("modelId", MAX_SUBAGENT_MODEL_ID),
    ] {
        if let Some(field) = optional_text(raw, key, max) {
            defaults.insert(key.to_string(), json!(field));
        }
    }
    (!defaults.is_empty()).then_some(Value::Object(defaults))
}

fn normalize_extra(value: &Value, taken: &mut HashSet<String>) -> Option<Value> {
    let raw = value.as_object()?;
    let name = optional_text(raw, "name", MAX_SUBAGENT_NAME)?;
    let wanted = text(raw, "id").unwrap_or_else(|| name.clone());
    let id = unique_extra_id(&wanted, taken)?;
    let description = optional_text(raw, "description", MAX_SUBAGENT_DESCRIPTION)
        .unwrap_or_else(|| DEFAULT_TASK_RUNNER_DESCRIPTION.to_string());
    taken.insert(id.clone());

    let mut extra = Map::new();
    extra.insert("id".to_string(), json!(id));
    extra.insert("name".to_string(), json!(name));
    extra.insert("description".to_string(), json!(description));
    if let Some(prompt) = optional_text(raw, "prompt", MAX_SUBAGENT_PROMPT) {
        extra.insert("prompt".to_string(), json!(prompt));
    }
    if let Some(model_id) = optional_text(raw, "modelId", MAX_SUBAGENT_MODEL_ID) {
        extra.insert("modelId".to_string(), json!(model_id));
    }
    Some(Value::Object(extra))
}

/// Drop anything a hand-edited config could smuggle in. Missing or empty
/// becomes nothing, which is the same as "task runner only".
fn normalize_subagents(value: Option<&Value>) -> Option<Value> {
    let raw = object(value)?;
    let defaults = normalize_defaults(raw.get("defaults"));
    let mut taken = HashSet::new();
    let mut extras = Vec::new();
    for item in raw
        .get("extras")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if extras.len() >= MAX_SUBAGENT_EXTRAS {
            break;
        }
        if let Some(extra) = normalize_extra(item, &mut taken) {
            extras.push(extra);
        }
    }
    if defaults.is_none() && extras.is_empty() {
        return None;
    }
    let mut subagents = Map::new();
    if let Some(defaults) = defaults {
        subagents.insert("defaults".to_string(), defaults);
    }
    if !extras.is_empty() {
        subagents.insert("extras".to_string(), Value::Array(extras));
    }
    Some(Value::Object(subagents))
}

fn persona_of(record: &ResourceRecord, root: &Path) -> Value {
    let replicated = &record.replicated;
    let empty = Map::new();
    let portable = record.portable.as_ref().unwrap_or(&empty);
    let machine = record.machine.as_ref().unwrap_or(&empty);

    let mut persona = Map::new();
    persona.insert("id".to_string(), json!(record.id));
    persona.insert(
        "name".to_string(),
        json!(text(replicated, "name").unwrap_or_else(|| "Untitled".to_string())),
    );
    persona.insert(
        "goal".to_string(),
        json!(text(replicated, "goal").unwrap_or_default()),
    );
    insert_if_truthy(&mut persona, "face", replicated);
    if let Some(team) = text(replicated, "team") {
        persona.insert("team".to_string(), json!(team));
    }
    persona.insert(
        "backendId".to_string(),
        json!(nonempty(replicated, "backendId").unwrap_or_else(|| DEFAULT_BACKEND_ID.to_string())),
    );
    persona.insert(
        "cwd".to_string(),
        json!(nonempty(machine, "cwd").unwrap_or_else(|| {
            default_workspace(root, &record.id)
                .to_string_lossy()
                .into_owned()
        })),
    );
    if let Some(model_id) = text(replicated, "modelId") {
        persona.insert("modelId".to_string(), json!(model_id));
    }
    // `modeId` was machine-bound until it was found to be a preference like the
    // model beside it. A row written before that still carries it under
    // `machine`, and a field changing class must not wipe what it already held,
    // so both places are read — forever. Moving the value up is the writer's
    // job, and the writer has done it before any window can ask.
    if let Some(mode_id) = text(replicated, "modeId").or_else(|| text(machine, "modeId")) {
        persona.insert("modeId".to_string(), json!(mode_id));
    }
    if let Some(hop_notice) = text(machine, "hopNotice") {
        persona.insert("hopNotice".to_string(), json!(hop_notice));
    }
    if let Some(harness_override) = normalize_harness(replicated.get("harnessOverride")) {
        persona.insert("harnessOverride".to_string(), harness_override);
    }
    // Anything but the one word that opens the machine reads as confined.
    if portable.get("reach").and_then(Value::as_str) == Some("machine") {
        persona.insert("reach".to_string(), json!("machine"));
    }
    persona.insert(
        "mcpPolicy".to_string(),
        normalize_policy(portable.get("mcpPolicy")),
    );
    // Background work is a standing portable grant. Older stores never had
    // this field, so omitting it keeps the new default closed on import.
    insert_if_true(&mut persona, "backgroundWork", portable);
    insert_if_truthy(&mut persona, "webSearchPolicy", portable);
    if let Some(computer) = normalize_computer(portable.get("computer")) {
        persona.insert("computer".to_string(), computer);
    }
    if let Some(subagents) = normalize_subagents(portable.get("subagents")) {
        persona.insert("subagents".to_string(), subagents);
    }
    persona.insert(
        "sessionCheckpoints".to_string(),
        normalize_checkpoints(machine.get("sessionCheckpoints")),
    );
    persona.insert(
        "createdAt".to_string(),
        replicated
            .get("createdAt")
            .filter(|value| value.is_number())
            .cloned()
            .unwrap_or_else(|| json!(record.updated_at)),
    );
    persona.insert("updatedAt".to_string(), json!(record.updated_at));
    Value::Object(persona)
}

#[cfg(test)]
mod tests {
    use super::super::records::fixture::{Put, create, scratch};
    use super::*;
    use crate::contract::Persona;
    use serde_json::json;

    #[test]
    fn imported_mcp_access_requires_a_saved_grant() {
        for value in [None, Some(json!(null)), Some(json!({ "mode": "invalid" }))] {
            assert_eq!(
                normalize_policy(value.as_ref()),
                json!({ "mode": "none", "serverIds": [] })
            );
        }
        for mode in ["none", "some", "all"] {
            let saved = json!({ "mode": mode, "serverIds": ["echo"] });
            assert_eq!(normalize_policy(Some(&saved)), saved);
        }
    }

    #[test]
    fn the_three_classes_assemble_into_one_teammate() {
        let root = scratch("assemble");
        let database = create(&root, "this-desk");
        Put {
            updated_at: 4_000,
            portable: Some(json!({
                "mcpPolicy": { "mode": "some", "serverIds": ["one", 7] },
                "backgroundWork": true,
                "computer": { "enabled": true, "image": "  toad/computer  " },
                "reach": "machine",
                "webSearchPolicy": { "brave": false },
            })),
            machine: Some(json!({
                "cwd": "/tmp/fixture-ada",
                "hopNotice": "you moved desks",
                "sessionCheckpoints": [
                    { "backendId": "pi", "sessionId": "s-pi" },
                    { "backendId": "", "sessionId": "junk" },
                ],
            })),
            ..Put::new(
                "fixture-ada",
                "this-desk",
                json!({
                    "name": "Ada",
                    "goal": "prove the machine",
                    "team": "Maths",
                    "face": { "kind": "emoji", "value": "🐸" },
                    "backendId": "pi",
                    "modelId": "m1",
                    "modeId": "architect",
                    "harnessOverride": { "backendId": "claude", "modelId": "" },
                    "createdAt": 1_000,
                }),
            )
        }
        .write(&database);

        let personas = list_local_personas(&root);
        assert_eq!(personas.len(), 1);
        assert_eq!(
            personas[0],
            json!({
                "id": "fixture-ada",
                "name": "Ada",
                "goal": "prove the machine",
                "face": { "kind": "emoji", "value": "🐸" },
                "team": "Maths",
                "backendId": "pi",
                "cwd": "/tmp/fixture-ada",
                "modelId": "m1",
                "modeId": "architect",
                "hopNotice": "you moved desks",
                "harnessOverride": { "backendId": "claude" },
                "reach": "machine",
                "mcpPolicy": { "mode": "some", "serverIds": ["one"] },
                "backgroundWork": true,
                "webSearchPolicy": { "brave": false },
                "computer": { "enabled": true, "image": "toad/computer" },
                "sessionCheckpoints": [{ "backendId": "pi", "sessionId": "s-pi" }],
                "createdAt": 1_000,
                "updatedAt": 4_000,
            })
        );
    }

    #[test]
    fn a_row_that_says_almost_nothing_still_makes_a_whole_teammate() {
        let root = scratch("defaults");
        let database = create(&root, "this-desk");
        Put {
            updated_at: 7_777,
            machine: Some(json!({ "cwd": "" })),
            portable: Some(json!({ "backgroundWork": "yes" })),
            ..Put::new("bare", "this-desk", json!({ "backendId": "" }))
        }
        .write(&database);

        let personas = list_local_personas(&root);
        assert_eq!(
            personas[0],
            json!({
                "id": "bare",
                "name": "Untitled",
                "goal": "",
                // The one backend a teammate runs when its row names none.
                "backendId": "pi",
                "cwd": default_workspace(&root, "bare").to_string_lossy(),
                "mcpPolicy": { "mode": "none", "serverIds": [] },
                "sessionCheckpoints": [],
                // A row with no creation stamp was created when it was last
                // written, which is the only date anybody can still prove.
                "createdAt": 7_777,
                "updatedAt": 7_777,
            })
        );
        let imported: Persona = serde_json::from_value(personas[0].clone()).unwrap();
        assert!(!imported.background_work);
    }

    #[test]
    fn another_desks_teammate_and_a_tombstone_are_both_left_out() {
        let root = scratch("owners");
        let database = create(&root, "this-desk");
        Put::new("mine", "this-desk", json!({ "name": "Mine" })).write(&database);
        Put::new("theirs", "peer-desk", json!({ "name": "Theirs" })).write(&database);
        Put {
            deleted: true,
            ..Put::new("buried", "this-desk", json!({ "name": "Buried" }))
        }
        .write(&database);

        let personas = list_local_personas(&root);
        let listed: Vec<&str> = personas
            .iter()
            .map(|persona| persona["id"].as_str().unwrap())
            .collect();
        assert_eq!(listed, ["mine"]);

        let foreign = list_foreign_personas_from(&root, &root);
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0]["id"], "theirs");
        assert_eq!(foreign[0]["name"], "Theirs");
    }

    /// A row an older build wrote keeps its thinking level where that build put
    /// it, and reading both places is what stands between such a row and a
    /// silent reset to the default.
    #[test]
    fn a_level_left_in_the_machine_class_is_still_read() {
        let root = scratch("mode-machine");
        let database = create(&root, "this-desk");
        Put {
            machine: Some(json!({ "modeId": "xhigh" })),
            ..Put::new("dialled", "this-desk", json!({ "name": "Dialled" }))
        }
        .write(&database);

        assert_eq!(list_local_personas(&root)[0]["modeId"], "xhigh");
    }

    /// A field this build no longer knows is not carried: the teammate is built
    /// key by key, so a 0.4.x desk's `plugins` is a non-event.
    #[test]
    fn a_field_this_build_never_heard_of_is_dropped() {
        let root = scratch("unknown-field");
        let database = create(&root, "this-desk");
        Put {
            portable: Some(json!({ "plugins": ["ghost"] })),
            ..Put::new(
                "from-the-future",
                "this-desk",
                json!({ "name": "Plugged in", "plugins": ["ghost"] }),
            )
        }
        .write(&database);

        let persona = &list_local_personas(&root)[0];
        assert_eq!(persona["name"], "Plugged in");
        assert!(persona.get("plugins").is_none());
    }

    #[test]
    fn only_the_one_word_that_opens_the_machine_reads_as_reach() {
        let root = scratch("reach");
        let database = create(&root, "this-desk");
        for (id, reach) in [
            ("open", json!("machine")),
            ("confined", json!("workspace")),
            ("nonsense", json!(true)),
        ] {
            Put {
                portable: Some(json!({ "reach": reach })),
                ..Put::new(id, "this-desk", json!({ "name": id }))
            }
            .write(&database);
        }

        let personas = list_local_personas(&root);
        assert_eq!(personas[0]["reach"], "machine");
        assert!(personas[1].get("reach").is_none());
        assert!(personas[2].get("reach").is_none());
    }

    #[test]
    fn subagents_keep_only_what_a_hand_edited_row_could_legally_say() {
        let root = scratch("subagents");
        let database = create(&root, "this-desk");
        Put {
            portable: Some(json!({
                "subagents": {
                    "defaults": { "name": "  Runner  ", "prompt": "", "modelId": 7 },
                    "extras": [
                        { "name": "Reviewer", "description": "  reads diffs  " },
                        { "id": "generic", "name": "Impostor" },
                        { "name": "Reviewer" },
                        { "description": "no name at all" },
                    ],
                },
            })),
            ..Put::new("delegator", "this-desk", json!({ "name": "Delegator" }))
        }
        .write(&database);
        Put {
            portable: Some(json!({ "subagents": { "extras": [{ "name": "" }] } })),
            ..Put::new("solo", "this-desk", json!({ "name": "Solo" }))
        }
        .write(&database);

        let personas = list_local_personas(&root);
        assert_eq!(
            personas[0]["subagents"],
            json!({
                "defaults": { "name": "Runner" },
                "extras": [
                    { "id": "reviewer", "name": "Reviewer", "description": "reads diffs" },
                    // An entry asking for the reserved kind is dropped, and a
                    // second Reviewer is numbered rather than dropped with it.
                    { "id": "reviewer-2", "name": "Reviewer", "description": DEFAULT_TASK_RUNNER_DESCRIPTION },
                ],
            })
        );
        // Nothing usable is the same as no subagents at all.
        assert!(personas[1].get("subagents").is_none());
    }

    #[test]
    fn a_missing_store_is_an_empty_roster() {
        assert!(list_local_personas(&scratch("no-store")).is_empty());
    }

    #[test]
    fn a_garbage_file_at_the_store_path_is_an_empty_roster() {
        let root = scratch("garbage-store");
        std::fs::write(
            super::super::records::store_path(&root),
            "this file is emphatically not a sqlite database\n",
        )
        .unwrap();
        assert!(list_local_personas(&root).is_empty());
    }
}
