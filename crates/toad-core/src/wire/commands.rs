//! What a command does.
//!
//! The room's own memory is the wire's to write: a teammate and a setting are
//! events on the room stream, and this is the only thing that appends them.
//! Everything else — a session, a secret, the models a key can reach — is
//! asked of the [`RoomHandle`], because a running agent and a vault are not
//! things to reimplement behind a door.

use super::RoomHandle;
use crate::contract::{Command, McpPolicy, Persona, PersonaDraft, PolicyMode, Reach};
use crate::log::{Log, StreamId};
use crate::store::{chapters, search};
use crate::{paths, room};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) async fn run(
    command: Command,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
) -> Result<Value, String> {
    match command {
        Command::PersonaCreate { draft } => create_persona(log, draft),
        Command::PersonaUpdate { id, patch } => update_persona(log, &id, &patch),
        Command::PersonaDelete { id } => delete_persona(log, &id),
        Command::SettingsUpdate { patch } => update_settings(log, patch),

        Command::CredentialCreate {
            provider_id,
            label,
            secret,
        } => room
            .credential_create(&provider_id, &label, &secret)
            .map(|credential| json!(credential)),
        Command::CredentialRevoke { id } => room.credential_revoke(&id).map(|()| Value::Null),
        Command::CredentialDelete { id } => room.credential_delete(&id).map(|()| Value::Null),
        Command::CredentialList {} => Ok(json!(room.credentials())),
        Command::ModelsList {} => Ok(json!(room.models())),

        Command::SessionStart { persona_id } => {
            room.start(&persona_id).await.map(|info| json!(info))
        }
        Command::SessionStop { persona_id } => room.stop(&persona_id).map(|()| Value::Null),
        Command::SessionPrompt {
            persona_id,
            text,
            reply_to,
            attachments,
        } => room
            .prompt(&persona_id, &text, reply_to, attachments)
            .await
            .map(|()| Value::Null),
        Command::SessionCancel { persona_id } => room.cancel(&persona_id).map(|()| Value::Null),
        Command::SessionSetModel {
            persona_id,
            model_id,
        } => room
            .set_model(&persona_id, &model_id)
            .await
            .map(|info| json!(info)),

        Command::SearchThread {
            persona_id,
            query,
            limit,
        } => Ok(search::search(log.root(), &persona_id, &query, limit)),
        Command::SearchAll { query, limit } => Ok(search::search_all(log.root(), &query, limit)),
        Command::ChapterList { persona_id } => Ok(json!(chapters::list(log, &persona_id))),
        Command::RoomImport { from } => room
            .import(std::path::Path::new(&from))
            .map(|report| json!(report)),
        Command::ChapterStartFresh { persona_id } => room
            .start_fresh_chapter(&persona_id)
            .await
            .map(|chapter| json!(chapter)),
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A drafted field that is present and not blank, trimmed.
fn given(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// A teammate as the previous Toad's `createPersona` made one: a fresh uuid,
/// the room's default backend, a workspace under the data directory, and
/// every capability its policy can give.
fn create_persona(log: &Log, draft: PersonaDraft) -> Result<Value, String> {
    let id = Uuid::new_v4().to_string();
    let settings = room::settings(log);
    let default_backend = settings
        .get("defaultBackendId")
        .and_then(Value::as_str)
        .unwrap_or("pi")
        .to_string();
    let stamped = now();
    let persona = Persona {
        node: None,
        id: id.clone(),
        name: given(Some(draft.name)).unwrap_or_else(|| "Untitled".to_string()),
        goal: given(draft.goal).unwrap_or_default(),
        face: None,
        team: given(draft.team),
        backend_id: given(draft.backend_id).unwrap_or(default_backend),
        cwd: given(draft.cwd).unwrap_or_else(|| {
            paths::default_workspace(log.root(), &id)
                .to_string_lossy()
                .into_owned()
        }),
        // The workspace is the wall unless the draft asked for the machine,
        // and an absent reach is the workspace, so only the wider one is
        // written down.
        reach: draft.reach.filter(|reach| *reach == Reach::Machine),
        model_id: given(draft.model_id),
        mode_id: None,
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        web_search_policy: None,
        computer: draft.computer,
        subagents: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: stamped,
        updated_at: stamped,
    };
    append_persona(log, &persona)?;
    Ok(json!(persona))
}

/// The patch over the record, and the whole record written again: a stream
/// folds by id, so a line carrying only what changed would leave the fold
/// holding only what changed.
fn update_persona(log: &Log, id: &str, patch: &Value) -> Result<Value, String> {
    let previous = living(log, id)?;
    let mut record = json!(previous);
    let fields = record
        .as_object_mut()
        .expect("a teammate serializes as an object");
    for (key, value) in patch
        .as_object()
        .ok_or("A patch is an object of fields to change.")?
    {
        fields.insert(key.clone(), value.clone());
    }
    fields.insert("id".into(), Value::from(id));
    fields.insert("updatedAt".into(), Value::from(now()));

    let updated: Persona = serde_json::from_value(record)
        .map_err(|error| format!("That patch does not leave a teammate behind: {error}."))?;
    append_persona(log, &updated)?;
    Ok(json!(updated))
}

/// The tombstone is the same kind and id again, so the fold finds it instead
/// of the teammate and a mirror has a line to ship rather than an absence to
/// notice.
fn delete_persona(log: &Log, id: &str) -> Result<Value, String> {
    living(log, id)?;
    append(
        log,
        &json!({ "kind": "persona", "id": id, "deleted": true }),
    )?;
    Ok(Value::Null)
}

/// One event per key, and `null` clears a key rather than setting it to
/// nothing — which puts the room's default back, because a deleted setting is
/// not an override.
fn update_settings(log: &Log, patch: Map<String, Value>) -> Result<Value, String> {
    for (key, value) in patch {
        let event = if value.is_null() {
            json!({ "kind": "setting", "id": key, "deleted": true })
        } else {
            json!({ "kind": "setting", "id": key, "value": value })
        };
        append(log, &event)?;
    }
    Ok(Value::Object(room::settings(log)))
}

fn living(log: &Log, id: &str) -> Result<Persona, String> {
    room::roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
        .ok_or_else(|| format!("There is no teammate {id}."))
}

/// A persona event is the teammate's record with the kind beside it.
fn append_persona(log: &Log, persona: &Persona) -> Result<(), String> {
    let mut event = json!(persona);
    event
        .as_object_mut()
        .expect("a teammate serializes as an object")
        .insert("kind".into(), Value::from("persona"));
    append(log, &event)
}

fn append(log: &Log, event: &Value) -> Result<(), String> {
    log.append(&StreamId::Room, event)
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}
