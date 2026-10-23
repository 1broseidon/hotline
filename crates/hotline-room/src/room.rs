//! The room, folded out of its stream: who is on the team, how the room is
//! set, and which jobs will wake a teammate later.
//!
//! There is no roster table and no settings file. Everything the room
//! remembers is an event on [`StreamId::Room`], shaped `{kind, id, …}`, and
//! these functions are the whole of reading it back:
//!
//! - `persona` carries a teammate's record — the [`Persona`] fields beside the
//!   kind, `id` being the teammate's — and [`roster`] is every one of them
//!   that is not deleted.
//! - `setting` carries one preference, `id` naming it and `value` holding it,
//!   and [`settings`] is those laid over the room's defaults.
//! - `schedule` carries a job — who to wake, when, with what prompt — and
//!   [`schedules`] is every one that is not deleted. The event's kind is
//!   always `schedule`; a loop is the job that carries `every`.
//!
//! - `exchange_pair` carries the durable queue and message brake for two
//!   teammates. `session::exchanges` owns its transitions.
//!
//! - `models` says the room's model choices changed where no other event
//!   shows it: a discovery refresh, a manual model id, a sign-in's account
//!   list. It carries no data. It always has the same id, so the stream
//!   folds it to one line, and a live session and the window each read the
//!   list again when it arrives.
//!
//! A delete is not a new kind: it is the same kind and id again with
//! `deleted: true`. The stream folds by id, so the tombstone is the last word
//! and a reader finds it instead of what it replaced — which is also what
//! makes a delete something a mirror can ship, rather than an absence it has
//! to notice.

use hotline_contract::{
    AgentAccount, ImageSettings, Persona, ScheduleKind, ScheduledJob, SessionCheckpoint,
    SpendingSettings,
};
use hotline_store::log::{Log, StreamId};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The setting that keeps the room's agent accounts, an array of
/// [`AgentAccount`].
pub const AGENT_ACCOUNTS: &str = "agentAccounts";

/// What a setting means before anybody has set it. `hotline` is the built-in Hotline
/// Agent, which is what a new teammate runs on, and a chapter closes after
/// eight hours of quiet — a working day's gap, so yesterday's context does not
/// follow you into this morning. `enabledModels` is empty: a provider nobody
/// has filtered shows every model, because a missing filter is not an empty
/// one. `defaultModelId` and `lastModelId` stay out of this map: a desk that
/// has never named a model has no preference, not an empty string.
fn defaults() -> Map<String, Value> {
    let mut settings = Map::new();
    settings.insert("defaultBackendId".into(), Value::from("hotline"));
    settings.insert("chapterIdleHours".into(), Value::from(8));
    settings.insert("mcpServers".into(), Value::Array(Vec::new()));
    settings.insert(AGENT_ACCOUNTS.into(), Value::Array(Vec::new()));
    settings.insert("enabledModels".into(), json!({}));
    settings.insert("images".into(), json!(ImageSettings::default()));
    settings.insert("spending".into(), json!(SpendingSettings::default()));
    settings
}

pub fn normalize_setting(key: &str, value: &Value) -> Result<Value, String> {
    if key == "personName" {
        let name = value.as_str().ok_or("Your name must be text.")?.trim();
        if name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err("Your name must be at most 100 characters on one line.".into());
        }
        return Ok(json!(name));
    }
    if key == "webSearch" {
        return hotline_websearch::normalize_setting(value);
    }
    if matches!(key, "images" | "spending") && !value.is_object() {
        return Err(format!("The {key} setting must be an object."));
    }
    match key {
        "images" => {
            let settings: ImageSettings = serde_json::from_value(value.clone()).map_err(|_| {
                "Images settings must be an object with optional provider and model strings."
                    .to_string()
            })?;
            for (name, selection) in [
                ("provider", settings.provider.as_deref()),
                ("model", settings.model.as_deref()),
            ] {
                if selection.is_some_and(|selection| selection.trim().is_empty()) {
                    return Err(format!("The images {name} must not be blank."));
                }
            }
            Ok(json!(settings))
        }
        "spending" => {
            let settings: SpendingSettings =
                serde_json::from_value(value.clone()).map_err(|_| {
                    "Spending settings must give chat, voice and images budgets, each with optional numeric dayUsd and monthUsd limits."
                        .to_string()
                })?;
            settings.validate()?;
            Ok(json!(settings))
        }
        "pinnedTeammates" => {
            let ids = value
                .as_array()
                .and_then(|ids| ids.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                .filter(|ids| ids.iter().all(|id| !id.is_empty()))
                .ok_or("Pinned teammates must be a list of teammate ids.")?;
            let mut unique: Vec<&str> = Vec::new();
            for id in ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            if unique.len() > MAX_PINS {
                return Err(too_many_pins());
            }
            Ok(json!(unique))
        }
        _ => Ok(value.clone()),
    }
}

/// How many teammates a desk can pin. Three faces fit a phone's header and
/// the desktop rail's width, and a fourth would stop being a favourite.
pub const MAX_PINS: usize = 3;

pub fn too_many_pins() -> String {
    format!("A desk pins up to {MAX_PINS} teammates. Unpin one first.")
}

/// The pinned teammates, in order, out of the `pinnedTeammates` setting's
/// stored value.
///
/// Read leniently and settled against who is on the team, so a stored list
/// that names a teammate who is gone, repeats one, or runs long (an older
/// or hand-edited stream) still reads as pins that can be shown.
pub fn settled_pins(value: Option<&Value>, team: &[Persona]) -> Vec<String> {
    let mut pins: Vec<String> = Vec::new();
    for id in value.and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = id.as_str() else { continue };
        if team.iter().any(|persona| persona.id == id) && !pins.iter().any(|pin| pin == id) {
            pins.push(id.to_string());
        }
    }
    pins.truncate(MAX_PINS);
    pins
}

/// The desk's pinned teammates, in the order they sit.
pub fn pinned_teammates(log: &Log) -> Vec<String> {
    let events = log.load(&StreamId::Room);
    settled_pins(
        settings_from_events(&events).get("pinnedTeammates"),
        &personas(&events),
    )
}

/// The setting naming the desk's lead: one teammate's id.
pub const TEAM_LEAD: &str = "teamLead";

/// The lead out of the `teamLead` setting's stored value, if it names
/// somebody still on the team.
pub fn settled_lead(value: Option<&Value>, team: &[Persona]) -> Option<String> {
    let id = value?.as_str()?;
    team.iter()
        .any(|persona| persona.id == id)
        .then(|| id.to_string())
}

/// The desk's lead.
pub fn team_lead(log: &Log) -> Option<String> {
    let events = log.load(&StreamId::Room);
    settled_lead(
        settings_from_events(&events).get(TEAM_LEAD),
        &personas(&events),
    )
}

/// Writes the lead the team implies, and answers it.
///
/// A lead who has left is cleared, and a desk with exactly one teammate and
/// no lead makes that one lead: so the first teammate leads, a desk upgraded
/// with one teammate gets its lead, and deleting down to one leaves that one
/// leading. Among two or more nobody is picked; that is the person's choice.
/// Nothing is written when the stored value already says it.
pub fn settle_lead(log: &Log) -> Result<Option<String>, String> {
    let events = log.load(&StreamId::Room);
    let team = personas(&events);
    let stored = settings_from_events(&events).remove(TEAM_LEAD);
    let lead = settled_lead(stored.as_ref(), &team).or_else(|| match team.as_slice() {
        [only] => Some(only.id.clone()),
        _ => None,
    });
    if stored.as_ref().and_then(Value::as_str) != lead.as_deref() {
        write_lead(log, lead.as_deref())?;
    }
    Ok(lead)
}

/// The `teamLead` setting set to `id`, or deleted when there is none.
pub fn write_lead(log: &Log, id: Option<&str>) -> Result<(), String> {
    let event = match id {
        Some(id) => json!({ "kind": "setting", "id": TEAM_LEAD, "value": id }),
        None => json!({ "kind": "setting", "id": TEAM_LEAD, "deleted": true }),
    };
    log.append(&StreamId::Room, &event)
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// The one id every `models` event shares.
pub const MODELS_CHANGED: &str = "models-changed";

/// Tells the room its model choices changed. A failed write only costs the
/// prompt update: the list is read fresh on the next start either way.
pub fn models_changed(log: &Log) {
    let _ = log.append(
        &StreamId::Room,
        &room_event(
            "models",
            json!({
                "id": MODELS_CHANGED,
                "ts": chrono::Utc::now().timestamp_millis(),
            }),
        ),
    );
}

/// Whether a room event can change which models Hotline Agent offers: a
/// connection added, revoked or removed, the "Models shown" filter, or the
/// `models` event itself.
pub fn changes_models(event: &Value) -> bool {
    is_kind(event, "credential")
        || is_kind(event, "models")
        || (is_kind(event, "setting")
            && event.get("id").and_then(Value::as_str) == Some("enabledModels"))
}

fn is_kind(event: &Value, kind: &str) -> bool {
    event.get("kind").and_then(Value::as_str) == Some(kind)
}

fn is_deleted(event: &Value) -> bool {
    event.get("deleted").and_then(Value::as_bool) == Some(true)
}

/// A room event with `kind` as the first key.
///
/// serde_json preserves insert order, so putting `kind` on after the body's
/// fields would leave it last, and a reader of the raw file would not see
/// the field a fold discriminates on first.
pub fn room_event(kind: &str, body: Value) -> Value {
    let Value::Object(fields) = body else {
        unreachable!("a room event is an object");
    };
    let mut event = serde_json::Map::new();
    event.insert("kind".into(), Value::from(kind));
    for (key, value) in fields {
        if key != "kind" {
            event.insert(key, value);
        }
    }
    Value::Object(event)
}

/// The team, in the order its teammates first appeared on the stream.
///
/// An event that does not read as a `Persona` is skipped rather than fatal: a
/// line a newer build wrote, or one a half-written record left behind, costs
/// its own teammate and not the whole roster. A record whose `id` is empty is
/// one of those: an id with no characters in it cannot name the files that
/// hold a tape, so a teammate that has one could never be opened at all.
pub fn roster(log: &Log) -> Vec<Persona> {
    personas(&log.load(&StreamId::Room))
}

pub fn personas(events: &[Value]) -> Vec<Persona> {
    events
        .iter()
        .filter(|event| is_kind(event, "persona") && !is_deleted(event))
        .filter_map(|event| Persona::deserialize(event).ok())
        .filter(|persona| !persona.id.is_empty())
        .collect()
}

/// Whether a teammate has ever been made in this room, whether or not one
/// is left. Deleting a teammate writes a `persona` tombstone, and the fold
/// keeps it in the teammate's place, so the stream still says one was here:
/// that is how the room knows it has been set up without storing that it was.
pub fn has_had_a_teammate(events: &[Value]) -> bool {
    events.iter().any(|event| {
        is_kind(event, "persona")
            && event
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
    })
}

/// Every setting the room has, the ones nobody set included.
///
/// A deleted setting is not an override, so its default stands again — which
/// is what "delete" means to somebody clearing a preference.
pub fn settings(log: &Log) -> Map<String, Value> {
    settings_from_events(&log.load(&StreamId::Room))
}

/// Whether the owner has set `key`, as opposed to it standing at its default.
pub fn is_set(log: &Log, key: &str) -> bool {
    log.load(&StreamId::Room).iter().any(|event| {
        is_kind(event, "setting")
            && !is_deleted(event)
            && event.get("id").and_then(Value::as_str) == Some(key)
    })
}

pub fn try_settings(log: &Log) -> Result<Map<String, Value>, String> {
    let events = log.try_load_strict(&StreamId::Room).map_err(|_| {
        "The room's settings could not be safely read. Images and spending are unavailable."
            .to_string()
    })?;
    if events.iter().any(|event| {
        matches!(
            event.get("id").and_then(Value::as_str),
            Some("images" | "spending")
        ) && !is_kind(event, "setting")
    }) {
        return Err("An image or spending setting has an invalid event kind.".into());
    }
    Ok(settings_from_events(&events))
}

pub fn settings_from_events(events: &[Value]) -> Map<String, Value> {
    let mut settings = defaults();
    for event in events {
        if !is_kind(event, "setting") || is_deleted(event) {
            continue;
        }
        let Some(key) = event.get("id").and_then(Value::as_str) else {
            continue;
        };
        let value = event.get("value").cloned().unwrap_or(Value::Null);
        let value = normalize_setting(key, &value).unwrap_or(value);
        settings.insert(key.to_string(), value);
    }
    // The raw list is what was stored; a half-written entry costs that
    // entry on the way out, so a teammate never sees a server that cannot
    // be started.
    let normalised = crate::mcp_servers::normalize_servers(
        settings
            .get("mcpServers")
            .unwrap_or(&Value::Array(Vec::new())),
    );
    settings.insert("mcpServers".into(), Value::Array(normalised));
    let accounts = crate::agent_accounts::normalize_accounts(
        settings.get(AGENT_ACCOUNTS).unwrap_or(&Value::Null),
    );
    settings.insert(AGENT_ACCOUNTS.into(), json!(accounts));
    settings
}

/// The room's agent accounts, in the order they were added.
pub fn agent_accounts(log: &Log) -> Vec<AgentAccount> {
    agent_accounts_from(&log.load(&StreamId::Room))
}

/// The same, from room events the caller has already loaded.
pub fn agent_accounts_from(events: &[Value]) -> Vec<AgentAccount> {
    crate::agent_accounts::normalize_accounts(
        settings_from_events(events)
            .get(AGENT_ACCOUNTS)
            .unwrap_or(&Value::Null),
    )
}

/// Stores `accounts` as the whole list, in one `setting` event. It does not
/// check them: whoever changes the list checks that the candidate reads back
/// unchanged through `normalize_accounts`, under the policy lock, before
/// calling this.
pub fn write_agent_accounts(log: &Log, accounts: &[AgentAccount]) -> std::io::Result<()> {
    log.append(
        &StreamId::Room,
        &room_event(
            "setting",
            json!({ "id": AGENT_ACCOUNTS, "value": accounts }),
        ),
    )
    .map(|_| ())
}

/// The jobs still waiting to fire, soonest first: neither deleted nor ended.
///
/// The event's `kind` is always `schedule` — that is the stream's kind, and
/// overwriting a job's own `kind` is what lets a loop share the kind with a
/// one-shot. A loop is recovered from `every` being present. An event that
/// does not read as a job is skipped rather than fatal, the same as a
/// teammate that does not read as a persona.
pub fn schedules(log: &Log) -> Vec<ScheduledJob> {
    jobs(&log.load(&StreamId::Room))
        .into_iter()
        .filter(|job| job.ended_at.is_none())
        .collect()
}

/// Every job not deleted, ended ones included, soonest first: an ended job's
/// thread stays readable, so it is still listed.
pub fn all_jobs(log: &Log) -> Vec<ScheduledJob> {
    jobs(&log.load(&StreamId::Room))
}

pub fn jobs(events: &[Value]) -> Vec<ScheduledJob> {
    let mut jobs: Vec<ScheduledJob> = events
        .iter()
        .filter(|event| is_kind(event, "schedule") && !is_deleted(event))
        .filter_map(job_from_event)
        .collect();
    jobs.sort_by(|a, b| a.next_at.cmp(&b.next_at).then(a.id.cmp(&b.id)));
    jobs
}

/// One teammate's jobs, ended ones included, soonest first, for a reader
/// that will take the answer as the whole truth: `None` when the room holds no living teammate
/// by that id, and an error when the room could not be read. Neither may
/// come back looking like a teammate with nothing scheduled.
pub fn teammate_schedules(
    log: &Log,
    persona_id: &str,
) -> Result<Option<Vec<ScheduledJob>>, String> {
    let events = log
        .try_load(&StreamId::Room)
        .map_err(|error| format!("The room's stream could not be read: {error}."))?;
    if personas(&events)
        .iter()
        .all(|persona| persona.id != persona_id)
    {
        return Ok(None);
    }
    Ok(Some(
        jobs(&events)
            .into_iter()
            .filter(|job| job.persona_id == persona_id)
            .collect(),
    ))
}

/// A job event is the job's record with the stream kind beside it. The job's
/// own kind is not written as `kind` — that slot is the stream's — and is
/// recovered on the way back from `every`.
pub fn append_schedule(log: &Log, job: &ScheduledJob) -> Result<(), String> {
    log.append(&StreamId::Room, &event_of(job))
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// The tombstone is the same kind and id again, so the fold finds it instead
/// of the job and a mirror has a line to ship rather than an absence to
/// notice.
pub fn tombstone_schedule(log: &Log, id: &str) -> Result<(), String> {
    log.append(
        &StreamId::Room,
        &json!({"kind": "schedule", "id": id, "deleted": true}),
    )
    .map(|_| ())
    .map_err(|error| format!("The room's stream could not be written: {error}."))
}

fn event_of(job: &ScheduledJob) -> Value {
    room_event(
        "schedule",
        serde_json::to_value(job).expect("a job serializes as JSON"),
    )
}

fn job_from_event(event: &Value) -> Option<ScheduledJob> {
    let id = event.get("id")?.as_str()?.to_string();
    let persona_id = event.get("personaId")?.as_str()?.to_string();
    let prompt = event.get("prompt")?.as_str()?.to_string();
    if id.is_empty() || persona_id.is_empty() || prompt.trim().is_empty() {
        return None;
    }
    Some(ScheduledJob {
        id,
        persona_id,
        kind: if event.get("every").and_then(Value::as_i64).is_some() {
            ScheduleKind::Loop
        } else {
            ScheduleKind::Schedule
        },
        when: event.get("when").and_then(Value::as_i64),
        every: event.get("every").and_then(Value::as_i64),
        prompt,
        quiet: event
            .get("quiet")
            .and_then(Value::as_bool)
            .filter(|quiet| *quiet),
        reminder: event
            .get("reminder")
            .and_then(Value::as_bool)
            .filter(|reminder| *reminder),
        goal: event
            .get("goal")
            .and_then(Value::as_str)
            .map(str::to_string),
        until: event.get("until").and_then(Value::as_i64),
        times: event
            .get("times")
            .and_then(Value::as_u64)
            .and_then(|times| u32::try_from(times).ok()),
        without: event.get("without").map(without_of),
        operator_created: event
            .get("operatorCreated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        next_at: event.get("nextAt")?.as_i64()?,
        created_at: event.get("createdAt")?.as_i64()?,
        last_post: event
            .get("lastPost")
            .and_then(|post| serde_json::from_value(post.clone()).ok()),
        ended_at: event.get("endedAt").and_then(Value::as_i64),
    })
}

/// What a job was narrowed by. A list this desk cannot read, written by a
/// newer one with an ability it does not know, leaves every ability out:
/// a narrowing it misread must not become a job with more than it was given.
fn without_of(value: &Value) -> Vec<hotline_contract::Ability> {
    use hotline_contract::Ability;
    serde_json::from_value(value.clone())
        .unwrap_or_else(|_| vec![Ability::Computer, Ability::Folders, Ability::McpServers])
}

/// What has been brought over to a teammate's computer from the host's
/// browsers: the latest `cookie-imports` event for that teammate, whole,
/// since the stream folds by teammate and a line carrying one import would
/// leave the fold holding one import. Nothing recorded is an empty list.
pub fn cookie_imports(log: &Log, persona_id: &str) -> Vec<hotline_contract::CookieImport> {
    log.load(&StreamId::Room)
        .into_iter()
        .filter(|event| is_kind(event, "cookie-imports"))
        .filter(|event| event.get("personaId").and_then(Value::as_str) == Some(persona_id))
        .filter_map(|event| {
            serde_json::from_value::<Vec<hotline_contract::CookieImport>>(
                event.get("imports").cloned().unwrap_or(Value::Null),
            )
            .ok()
        })
        .next_back()
        .unwrap_or_default()
}

/// Folds one import into the record: the same browser and profile again
/// keeps one entry, with its sites the union — a site brought over twice
/// carries the latest count — and its time the latest; another browser or
/// profile is another entry.
pub fn merge_cookie_import(
    imports: &mut Vec<hotline_contract::CookieImport>,
    import: hotline_contract::CookieImport,
) {
    if let Some(earlier) = imports.iter_mut().find(|earlier| {
        earlier.browser_id == import.browser_id && earlier.profile_id == import.profile_id
    }) {
        for site in import.sites {
            match earlier
                .sites
                .iter_mut()
                .find(|known| known.domain == site.domain)
            {
                Some(known) => known.cookies = site.cookies,
                None => earlier.sites.push(site),
            }
        }
        earlier.sites.sort_by(|a, b| a.domain.cmp(&b.domain));
        earlier.browser_name = import.browser_name;
        earlier.profile_name = import.profile_name;
        earlier.imported_at = import.imported_at;
    } else {
        let mut import = import;
        import.sites.sort_by(|a, b| a.domain.cmp(&b.domain));
        imports.push(import);
    }
}

/// Records the whole list of what a teammate's computer has been handed,
/// replacing the earlier record; domains and counts only, never a value.
pub fn record_cookie_imports(
    log: &Log,
    persona_id: &str,
    imports: &[hotline_contract::CookieImport],
) -> Result<(), String> {
    log.append(
        &StreamId::Room,
        &room_event(
            "cookie-imports",
            // The stream folds by id, so one entry per teammate is what the
            // fold keeps, and the latest record is the whole record.
            json!({"id": format!("cookie-imports:{persona_id}"), "personaId": persona_id, "imports": imports}),
        ),
    )
    .map(|_| ())
    .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// A persona event is the teammate's record with the kind beside it, and the
/// whole record every time: a stream folds by id, so a line carrying only what
/// changed would leave the fold holding only what changed.
pub fn append_persona(log: &Log, persona: &Persona) -> Result<(), String> {
    log.append(&StreamId::Room, &room_event("persona", json!(persona)))
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// Held while a teammate's `persona` record is read, changed and appended
/// again. The stream folds by id and the last whole record wins, so two
/// changes that each read the record before the other appended would leave
/// only one of them.
static PERSONA_WRITES: Mutex<()> = Mutex::new(());

/// The lock that keeps those changes from overwriting each other. Every
/// writer that reads a record and appends it again takes it.
///
/// It is the innermost lock: never held across an `.await`, and only the
/// log's own locks are taken under it. The four checkpoint functions below
/// take it themselves, so a holder never calls them, and `Room::checkpoint`
/// reaches it while holding `lifecycle`. Every other writer but a minted
/// passkey's grant takes it under `policy_update_lock`.
pub fn persona_write_lock() -> MutexGuard<'static, ()> {
    PERSONA_WRITES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Remembers an agent's own id for this teammate's conversation, so a later
/// session can ask that same backend to reopen it.
///
/// One entry per backend and account, replaced in place: a teammate that moves
/// between harnesses and back finds each conversation where it left it, and
/// Cursor is never handed Claude's session id nor one account the other's.
/// Moving between accounts is not such a return: the switch clears that
/// backend's entries for every account (`clear_backend_checkpoints`). No
/// account is This computer, the agent's own sign-in.
pub fn checkpoint_session(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
    account_id: Option<&str>,
    session_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| !same(checkpoint, backend_id, account_id));
        checkpoints.push(SessionCheckpoint {
            backend_id: backend_id.to_string(),
            session_id: session_id.to_string(),
            account_id: account_id.map(str::to_string),
        });
    })
}

/// Withdraws the promise to reopen that backend's session on that account.
/// The session itself is not touched; what is gone is Hotline's intention to
/// return to it, which is what closing a chapter means.
pub fn clear_checkpoint(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
    account_id: Option<&str>,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| !same(checkpoint, backend_id, account_id));
    })
}

/// Withdraws every teammate's checkpoints on one account, and writes only the
/// records that had one.
pub fn clear_account_checkpoints(log: &Log, account_id: &str) -> Result<(), String> {
    let _writes = persona_write_lock();
    for persona in roster(log) {
        write_changed_checkpoints(log, persona, |checkpoints| {
            checkpoints.retain(|checkpoint| checkpoint.account_id.as_deref() != Some(account_id));
        })?;
    }
    Ok(())
}

/// Withdraws a teammate's checkpoints on one backend, whatever account they
/// ran under: once the teammate's account has changed, none of them continues
/// what was said.
pub fn clear_backend_checkpoints(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| checkpoint.backend_id != backend_id);
    })
}

/// Whether a checkpoint is the one a backend and an account keep.
fn same(checkpoint: &SessionCheckpoint, backend: &str, account: Option<&str>) -> bool {
    checkpoint.backend_id == backend && checkpoint.account_id.as_deref() == account
}

/// The teammate's record with its checkpoints changed and nothing else, or
/// nothing at all when they did not change.
fn with_checkpoints(
    log: &Log,
    persona_id: &str,
    change: impl FnOnce(&mut Vec<SessionCheckpoint>),
) -> Result<(), String> {
    let _writes = persona_write_lock();
    let persona = roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .ok_or_else(|| format!("There is no teammate {persona_id}."))?;
    write_changed_checkpoints(log, persona, change)
}

/// Appends `persona` with `change` made to its checkpoints, if it changed
/// any. The caller holds [`persona_write_lock`] from reading the record.
fn write_changed_checkpoints(
    log: &Log,
    mut persona: Persona,
    change: impl FnOnce(&mut Vec<SessionCheckpoint>),
) -> Result<(), String> {
    let before = persona.session_checkpoints.clone();
    change(&mut persona.session_checkpoints);
    if persona.session_checkpoints == before {
        return Ok(());
    }
    persona.updated_at = chrono::Utc::now().timestamp_millis();
    append_persona(log, &persona)
}

#[cfg(test)]
mod tests;
