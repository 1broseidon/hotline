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

use crate::contract::{Persona, ScheduleKind, ScheduledJob, SessionCheckpoint};
use crate::log::{Log, StreamId};
use serde::Deserialize;
use serde_json::{Map, Value, json};

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
    settings.insert("enabledModels".into(), json!({}));
    settings
}

/// The one id every `models` event shares.
pub(crate) const MODELS_CHANGED: &str = "models-changed";

/// Tells the room its model choices changed. A failed write only costs the
/// prompt update: the list is read fresh on the next start either way.
pub(crate) fn models_changed(log: &Log) {
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
pub(crate) fn changes_models(event: &Value) -> bool {
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
pub(crate) fn room_event(kind: &str, body: Value) -> Value {
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

fn personas(events: &[Value]) -> Vec<Persona> {
    events
        .iter()
        .filter(|event| is_kind(event, "persona") && !is_deleted(event))
        .filter_map(|event| Persona::deserialize(event).ok())
        .filter(|persona| !persona.id.is_empty())
        .collect()
}

/// Every setting the room has, the ones nobody set included.
///
/// A deleted setting is not an override, so its default stands again — which
/// is what "delete" means to somebody clearing a preference.
pub fn settings(log: &Log) -> Map<String, Value> {
    let mut settings = defaults();
    for event in log.load(&StreamId::Room) {
        if !is_kind(&event, "setting") || is_deleted(&event) {
            continue;
        }
        let Some(key) = event.get("id").and_then(Value::as_str) else {
            continue;
        };
        let value = event.get("value").cloned().unwrap_or(Value::Null);
        settings.insert(key.to_string(), value);
    }
    // The raw list is what was stored; a half-written entry costs that
    // entry on the way out, so a teammate never sees a server that cannot
    // be started.
    let normalised = crate::mcp::normalize_servers(
        settings
            .get("mcpServers")
            .unwrap_or(&Value::Array(Vec::new())),
    );
    settings.insert("mcpServers".into(), Value::Array(normalised));
    settings
}

/// The jobs still waiting to fire, soonest first.
///
/// The event's `kind` is always `schedule` — that is the stream's kind, and
/// overwriting a job's own `kind` is what lets a loop share the kind with a
/// one-shot. A loop is recovered from `every` being present. An event that
/// does not read as a job is skipped rather than fatal, the same as a
/// teammate that does not read as a persona.
pub fn schedules(log: &Log) -> Vec<ScheduledJob> {
    jobs(&log.load(&StreamId::Room))
}

fn jobs(events: &[Value]) -> Vec<ScheduledJob> {
    let mut jobs: Vec<ScheduledJob> = events
        .iter()
        .filter(|event| is_kind(event, "schedule") && !is_deleted(event))
        .filter_map(job_from_event)
        .collect();
    jobs.sort_by(|a, b| a.next_at.cmp(&b.next_at).then(a.id.cmp(&b.id)));
    jobs
}

/// One teammate's jobs, soonest first, for a reader that will take the
/// answer as the whole truth: `None` when the room holds no living teammate
/// by that id, and an error when the room could not be read. Neither may
/// come back looking like a teammate with nothing scheduled.
pub(crate) fn teammate_schedules(
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
pub(crate) fn append_schedule(log: &Log, job: &ScheduledJob) -> Result<(), String> {
    log.append(&StreamId::Room, &event_of(job))
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// The tombstone is the same kind and id again, so the fold finds it instead
/// of the job and a mirror has a line to ship rather than an absence to
/// notice.
pub(crate) fn tombstone_schedule(log: &Log, id: &str) -> Result<(), String> {
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
        operator_created: event
            .get("operatorCreated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        next_at: event.get("nextAt")?.as_i64()?,
        created_at: event.get("createdAt")?.as_i64()?,
    })
}

/// What has been brought over to a teammate's computer from the host's
/// browsers: the latest `cookie-imports` event for that teammate, whole,
/// since the stream folds by teammate and a line carrying one import would
/// leave the fold holding one import. Nothing recorded is an empty list.
pub fn cookie_imports(log: &Log, persona_id: &str) -> Vec<crate::contract::CookieImport> {
    log.load(&StreamId::Room)
        .into_iter()
        .filter(|event| is_kind(event, "cookie-imports"))
        .filter(|event| event.get("personaId").and_then(Value::as_str) == Some(persona_id))
        .filter_map(|event| {
            serde_json::from_value::<Vec<crate::contract::CookieImport>>(
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
pub(crate) fn merge_cookie_import(
    imports: &mut Vec<crate::contract::CookieImport>,
    import: crate::contract::CookieImport,
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
pub(crate) fn record_cookie_imports(
    log: &Log,
    persona_id: &str,
    imports: &[crate::contract::CookieImport],
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

#[cfg(test)]
mod cookie_import_tests {
    use super::tests::scratch;
    use super::*;
    use crate::contract::{CookieImport, CookieSite};

    fn import(browser: &str, profile: &str, sites: &[(&str, u32)], at: i64) -> CookieImport {
        CookieImport {
            browser_id: browser.to_owned(),
            browser_name: browser.to_uppercase(),
            profile_id: profile.to_owned(),
            profile_name: profile.to_owned(),
            imported_at: at,
            sites: sites
                .iter()
                .map(|(domain, cookies)| CookieSite {
                    domain: (*domain).to_owned(),
                    cookies: *cookies,
                })
                .collect(),
        }
    }

    #[test]
    fn an_import_from_the_same_browser_and_profile_merges_and_another_is_listed_beside_it() {
        let mut imports = Vec::new();
        merge_cookie_import(
            &mut imports,
            import("chrome", "Default", &[("github.com", 3)], 1),
        );
        merge_cookie_import(
            &mut imports,
            import(
                "chrome",
                "Default",
                &[("gitlab.com", 2), ("github.com", 5)],
                2,
            ),
        );
        merge_cookie_import(
            &mut imports,
            import("firefox", "default", &[("github.com", 1)], 3),
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].imported_at, 2);
        assert_eq!(
            imports[0]
                .sites
                .iter()
                .map(|site| (site.domain.as_str(), site.cookies))
                .collect::<Vec<_>>(),
            vec![("github.com", 5), ("gitlab.com", 2)],
            "the union, sorted, with the latest count"
        );
        assert_eq!(imports[1].browser_id, "firefox");
    }

    #[test]
    fn the_record_is_the_latest_whole_list_per_teammate() {
        let log = scratch("cookie-imports");
        assert!(cookie_imports(&log, "ada").is_empty());
        let first = vec![import("chrome", "Default", &[("github.com", 3)], 1)];
        record_cookie_imports(&log, "ada", &first).unwrap();
        record_cookie_imports(
            &log,
            "bob",
            &[import("firefox", "default", &[("x.test", 1)], 2)],
        )
        .unwrap();
        assert_eq!(cookie_imports(&log, "ada"), first);
        record_cookie_imports(&log, "ada", &[]).unwrap();
        assert!(
            cookie_imports(&log, "ada").is_empty(),
            "the latest record wins"
        );
        assert_eq!(cookie_imports(&log, "bob").len(), 1);
    }
}

/// A persona event is the teammate's record with the kind beside it, and the
/// whole record every time: a stream folds by id, so a line carrying only what
/// changed would leave the fold holding only what changed.
pub(crate) fn append_persona(log: &Log, persona: &Persona) -> Result<(), String> {
    log.append(&StreamId::Room, &room_event("persona", json!(persona)))
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// Remembers an agent's own id for this teammate's conversation, so a later
/// session can ask that same backend to reopen it.
///
/// One entry per backend, replaced in place: a teammate that moves between
/// harnesses and back finds both conversations where it left them, and Cursor
/// is never handed Claude's session id.
pub(crate) fn checkpoint_session(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
    session_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| checkpoint.backend_id != backend_id);
        checkpoints.push(SessionCheckpoint {
            backend_id: backend_id.to_string(),
            session_id: session_id.to_string(),
        });
    })
}

/// Withdraws the promise to reopen that backend's session. The session itself
/// is not touched; what is gone is Hotline's intention to return to it, which is
/// what closing a chapter means.
pub(crate) fn clear_checkpoint(
    log: &Log,
    persona_id: &str,
    backend_id: &str,
) -> Result<(), String> {
    with_checkpoints(log, persona_id, |checkpoints| {
        checkpoints.retain(|checkpoint| checkpoint.backend_id != backend_id);
    })
}

/// The teammate's record with its checkpoints changed and nothing else, or
/// nothing at all when they did not change.
fn with_checkpoints(
    log: &Log,
    persona_id: &str,
    change: impl FnOnce(&mut Vec<SessionCheckpoint>),
) -> Result<(), String> {
    let mut persona = roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
        .ok_or_else(|| format!("There is no teammate {persona_id}."))?;
    let before = persona.session_checkpoints.clone();
    change(&mut persona.session_checkpoints);
    if persona.session_checkpoints == before {
        return Ok(());
    }
    persona.updated_at = chrono::Utc::now().timestamp_millis();
    append_persona(log, &persona)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{McpPolicy, PolicyMode};
    use serde_json::json;

    pub(super) fn scratch(name: &str) -> Log {
        let root =
            std::env::temp_dir().join(format!("hotline-core-room-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Log::open(root)
    }

    fn persona(id: &str, name: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: name.to_string(),
            goal: "Keep the harbour running.".to_string(),
            face: None,
            team: None,
            backend_id: "hotline".to_string(),
            cwd: "/tmp/harbour".to_string(),
            reach: None,
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            skill_policy: Default::default(),
            background_work: false,
            allowed_senders: Vec::new(),
            web_search_policy: None,
            computer: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    /// A persona event is the teammate's record with the kind beside it.
    fn persona_event(persona: &Persona) -> Value {
        let mut event = serde_json::to_value(persona).unwrap();
        let fields = event.as_object_mut().unwrap();
        fields.insert("kind".into(), Value::from("persona"));
        event
    }

    fn setting(key: &str, value: Value) -> Value {
        json!({"kind": "setting", "id": key, "value": value})
    }

    fn tombstone(kind: &str, id: &str) -> Value {
        json!({"kind": kind, "id": id, "deleted": true})
    }

    fn append(log: &Log, event: &Value) {
        log.append(&StreamId::Room, event).unwrap();
    }

    #[test]
    fn a_persona_event_is_the_contracts_persona_beside_its_kind() {
        let log = scratch("persona-round-trip");
        let ada = persona("ada", "Ada");
        let written = persona_event(&ada);
        append(&log, &written);

        // Out of the stream and back into the struct the wire is defined by,
        // and out again as the same bytes: a field spelled differently here
        // than the contract spells it fails on one side or the other.
        assert_eq!(roster(&log), [ada]);
        assert_eq!(log.load(&StreamId::Room), [written]);
    }

    #[test]
    fn the_roster_is_the_living_teammates_in_the_order_they_arrived() {
        let log = scratch("roster");
        let ada = persona("ada", "Ada");
        let bob = persona("bob", "Bob");
        append(&log, &persona_event(&ada));
        append(&log, &persona_event(&bob));

        // A rename is the same id again, and keeps the place it arrived in.
        let mut renamed = ada.clone();
        renamed.name = "Ada Lovelace".to_string();
        renamed.updated_at = 1_700_000_001_000;
        append(&log, &persona_event(&renamed));

        assert_eq!(roster(&log), [renamed, bob]);
    }

    #[test]
    fn a_tombstone_is_the_last_word_on_a_teammate() {
        let log = scratch("tombstone");
        let ada = persona("ada", "Ada");
        let bob = persona("bob", "Bob");
        append(&log, &persona_event(&ada));
        append(&log, &persona_event(&bob));
        append(&log, &tombstone("persona", "ada"));

        assert_eq!(roster(&log), [bob]);
        // The stream still carries the delete, which is what a mirror ships.
        let ids: Vec<String> = log
            .load(&StreamId::Room)
            .iter()
            .map(|event| event["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["ada", "bob"]);
    }

    #[test]
    fn a_setting_nobody_set_is_the_default_and_a_set_one_stands_over_it() {
        let log = scratch("settings");
        assert_eq!(settings(&log)["defaultBackendId"], "hotline");
        assert_eq!(settings(&log)["chapterIdleHours"], 8);
        assert_eq!(settings(&log)["enabledModels"], json!({}));

        append(&log, &setting("chapterIdleHours", Value::from(2)));
        append(&log, &setting("theme", Value::from("dark")));
        assert_eq!(settings(&log)["chapterIdleHours"], 2);
        assert_eq!(settings(&log)["theme"], "dark");
        assert_eq!(settings(&log)["defaultBackendId"], "hotline");

        // Clearing a setting is a tombstone, and puts the default back. A
        // setting that never had one is simply gone.
        append(&log, &tombstone("setting", "chapterIdleHours"));
        append(&log, &tombstone("setting", "theme"));
        assert_eq!(settings(&log)["chapterIdleHours"], 8);
        assert!(!settings(&log).contains_key("theme"));
    }

    #[test]
    fn a_malformed_mcp_server_entry_is_skipped_on_read() {
        let log = scratch("mcp-bad-entry");
        append(
            &log,
            &setting(
                "mcpServers",
                json!([
                    { "id": "no-name", "type": "stdio", "command": "echo" },
                    { "id": "good", "type": "stdio", "name": "Good", "command": "run" },
                ]),
            ),
        );
        let folded = settings(&log);
        let servers = folded["mcpServers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["id"], "good");
    }

    fn job(
        id: &str,
        persona_id: &str,
        kind: ScheduleKind,
        prompt: &str,
        quiet: bool,
    ) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            persona_id: persona_id.to_string(),
            kind,
            when: (kind == ScheduleKind::Schedule).then_some(1_700_000_100_000),
            every: (kind == ScheduleKind::Loop).then_some(15_000),
            prompt: prompt.to_string(),
            quiet: quiet.then_some(true),
            operator_created: false,
            next_at: 1_700_000_100_000,
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_schedule_event_folds_as_the_job_it_was() {
        let log = scratch("schedule-create");
        let once = job(
            "job-1",
            "ada",
            ScheduleKind::Schedule,
            "check the crane",
            false,
        );
        append_schedule(&log, &once).unwrap();

        assert_eq!(&schedules(&log)[0], &once);
        // The stream's kind is always `schedule`, even though this job's kind
        // is too — a loop has to share the slot, so the job's own kind is not
        // what `kind` says on disk.
        assert_eq!(log.load(&StreamId::Room)[0]["kind"], "schedule");
        assert_eq!(log.load(&StreamId::Room)[0].get("every"), None);
        assert_eq!(log.load(&StreamId::Room)[0]["when"], once.when.unwrap());
    }

    #[test]
    fn operator_schedule_provenance_survives_the_room_fold() {
        let log = scratch("schedule-operator");
        let mut operator = job(
            "job-operator",
            "ada",
            ScheduleKind::Schedule,
            "operator check",
            false,
        );
        operator.operator_created = true;
        append_schedule(&log, &operator).unwrap();

        assert_eq!(schedules(&log), vec![operator]);
        assert_eq!(log.load(&StreamId::Room)[0]["operatorCreated"], true);
    }

    #[test]
    fn a_loop_is_the_schedule_event_that_carries_every() {
        let log = scratch("schedule-loop");
        let loop_job = job("job-2", "ada", ScheduleKind::Loop, "sweep the inbox", true);
        append_schedule(&log, &loop_job).unwrap();

        assert_eq!(schedules(&log)[0], loop_job);
        assert_eq!(log.load(&StreamId::Room)[0]["kind"], "schedule");
        assert_eq!(log.load(&StreamId::Room)[0]["every"], 15_000);
        assert_eq!(log.load(&StreamId::Room)[0]["quiet"], true);
    }

    /// A teammate whose id names no file could never be opened, and the
    /// startup fold opens every teammate's tape before it serves one.
    #[test]
    fn a_record_with_an_empty_id_is_not_a_teammate() {
        let log = scratch("empty-id");
        append(&log, &persona_event(&persona("", "Nobody")));
        append(&log, &persona_event(&persona("ada", "Ada")));

        let ids: Vec<String> = roster(&log).into_iter().map(|persona| persona.id).collect();
        assert_eq!(ids, ["ada"]);
    }

    #[test]
    fn kind_is_the_first_key_on_every_room_event_this_writes() {
        let log = scratch("kind-leads");
        append_persona(&log, &persona("ada", "Ada")).unwrap();
        append_schedule(
            &log,
            &job(
                "job-1",
                "ada",
                ScheduleKind::Schedule,
                "check the crane",
                false,
            ),
        )
        .unwrap();
        tombstone_schedule(&log, "job-1").unwrap();

        let raw = std::fs::read_to_string(log.root().join("room.jsonl")).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 3, "{raw}");
        for line in &lines {
            assert!(
                line.starts_with("{\"kind\":"),
                "kind was not the leading key: {line}"
            );
        }
        assert!(
            lines[0].starts_with("{\"kind\":\"persona\""),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("{\"kind\":\"schedule\""),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("{\"kind\":\"schedule\""),
            "{}",
            lines[2]
        );
    }

    #[test]
    fn one_teammates_schedules_are_its_own_and_none_is_an_answer() {
        let log = scratch("teammate-schedules");
        append_persona(&log, &persona("ada", "Ada")).unwrap();
        append_persona(&log, &persona("bob", "Bob")).unwrap();

        // A teammate with nothing waiting is a list with nothing in it; a
        // teammate the room does not hold is no list at all.
        assert_eq!(teammate_schedules(&log, "ada").unwrap(), Some(Vec::new()));
        assert_eq!(teammate_schedules(&log, "nobody").unwrap(), None);

        let mut sweep = job("job-1", "ada", ScheduleKind::Loop, "sweep the inbox", true);
        let once = job(
            "job-2",
            "ada",
            ScheduleKind::Schedule,
            "check the crane",
            false,
        );
        let theirs = job(
            "job-3",
            "bob",
            ScheduleKind::Schedule,
            "count the boats",
            false,
        );
        for job in [&sweep, &once, &theirs] {
            append_schedule(&log, job).unwrap();
        }
        assert_eq!(
            teammate_schedules(&log, "ada").unwrap(),
            Some(vec![sweep.clone(), once.clone()])
        );
        assert_eq!(teammate_schedules(&log, "bob").unwrap(), Some(vec![theirs]));

        // A loop that ran is the same job with a later next run, once; a
        // one-shot that fired is gone.
        sweep.next_at += 15_000;
        append_schedule(&log, &sweep).unwrap();
        tombstone_schedule(&log, "job-2").unwrap();
        assert_eq!(teammate_schedules(&log, "ada").unwrap(), Some(vec![sweep]));

        // A teammate deleted is no longer one whose jobs can be read, though
        // the clock has not yet cleared them away.
        append(&log, &tombstone("persona", "ada"));
        assert_eq!(teammate_schedules(&log, "ada").unwrap(), None);
    }

    #[test]
    fn a_room_that_cannot_be_read_is_an_error_not_an_empty_list() {
        let log = scratch("teammate-schedules-unreadable");
        // Nobody has written to the room yet: that is a room with no
        // teammates, which is an answer.
        assert_eq!(teammate_schedules(&log, "ada").unwrap(), None);
        // A room stream that is there but cannot be read is not.
        std::fs::create_dir_all(log.root().join("room.jsonl")).unwrap();
        let error = teammate_schedules(&log, "ada").unwrap_err();
        assert!(error.contains("could not be read"), "{error}");
        assert!(log.try_load(&StreamId::Room).is_err());
        assert!(log.load(&StreamId::Room).is_empty());
    }

    #[test]
    fn a_tombstone_cancels_a_job_and_a_rewrite_flips_its_silence() {
        let log = scratch("schedule-cancel-quiet");
        let mut loud = job("job-1", "ada", ScheduleKind::Loop, "check the order", false);
        let quiet = job("job-2", "bob", ScheduleKind::Schedule, "say the word", true);
        append_schedule(&log, &loud).unwrap();
        append_schedule(&log, &quiet).unwrap();
        assert_eq!(schedules(&log), [loud.clone(), quiet.clone()]);

        loud.quiet = Some(true);
        append_schedule(&log, &loud).unwrap();
        assert_eq!(schedules(&log)[0].quiet, Some(true));

        loud.quiet = None;
        append_schedule(&log, &loud).unwrap();
        assert_eq!(schedules(&log)[0].quiet, None);

        tombstone_schedule(&log, "job-1").unwrap();
        assert_eq!(schedules(&log), [quiet]);
        // The stream still carries the delete, which is what a mirror ships.
        let ids: Vec<String> = log
            .load(&StreamId::Room)
            .iter()
            .map(|event| event["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["job-1", "job-2"]);
    }
}
