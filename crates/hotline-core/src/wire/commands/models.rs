//! Which model a teammate runs on, and what a fresh room can run.

use super::persona::{living, update_persona};
use super::settings::update_settings;
use crate::contract::{Persona, SessionConfigCategory, SessionInfo, SessionState};
use crate::driver::HOTLINE_BACKEND_ID;
use crate::driver::acp::catalog;
use crate::log::Log;
use crate::room;
use crate::wire::RoomHandle;
use crate::wire::roster::offered_info;
use serde_json::{Map, Value, json};
use std::sync::Arc;

mod effort;

/// Where a fresh room stands, from what it already knows. A live credential
/// is a way to run Hotline Agent; a startable harness is a way to run only once
/// it is the room's default, because that is what the first teammate lands
/// on. Hotline Agent is not a harness here: it is what the providers are for.
pub(super) fn welcome(
    settings: &Map<String, Value>,
    teammates: usize,
    set_up: bool,
    credentials: &[crate::contract::Credential],
    backends: Vec<crate::contract::BackendChoice>,
) -> crate::contract::Welcome {
    let known = crate::models::providers();
    let mut providers: Vec<String> = credentials
        .iter()
        .filter(|credential| !credential.revoked)
        .map(|credential| {
            known
                .iter()
                .find(|provider| provider.id == credential.provider_id)
                .map_or_else(
                    || credential.label.clone(),
                    |provider| provider.name.clone(),
                )
        })
        .collect();
    providers.sort();
    providers.dedup();
    let harnesses: Vec<crate::contract::BackendChoice> = backends
        .into_iter()
        .filter(|backend| backend.id != HOTLINE_BACKEND_ID && backend.unavailable.is_none())
        .collect();
    let default_backend_id = settings
        .get("defaultBackendId")
        .and_then(Value::as_str)
        .unwrap_or(HOTLINE_BACKEND_ID)
        .to_string();
    let can_run = !providers.is_empty()
        || harnesses
            .iter()
            .any(|backend| backend.id == default_backend_id);
    crate::contract::Welcome {
        providers,
        harnesses,
        default_backend_id,
        can_run,
        teammates,
        set_up,
    }
}

/// Write the teammate's model first, then switch a live session if there is
/// one. The persona is the truth; the live switch is a courtesy to the turn
/// already running. Writing first means an idle teammate keeps the choice,
/// and a live switch that fails still lands on the next start. A teammate at
/// rest answers with what it offers, the new model current.
///
/// The effort moves with the model, by the rules in [`effort`]: written on
/// the teammate with the model and remembered for both, and set on a live
/// session after the model. A model whose levels are not known until a
/// session on it reports them is placed once the live switch answers; at
/// rest it keeps the teammate's effort, as before.
pub(super) async fn set_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    model_id: &str,
) -> Result<SessionInfo, String> {
    let persona = living(log, persona_id)?;
    if persona.backend_id == HOTLINE_BACKEND_ID {
        // An id naming an account must be one the desk lists; a plain id is
        // the provider's default account, which a desk with several accounts
        // of that provider lists only by name.
        if !room
            .models()
            .iter()
            .any(|choice| choice.id == model_id || crate::models::base_id(&choice.id) == model_id)
        {
            return Err(format!("{model_id} is not a model this desk can reach."));
        }
    } else if model_id.is_empty() {
        return Err("A model needs an id.".to_string());
    }

    let before = offered_info(log, room, persona_id);
    let place = Placing::new(log, &persona, &before, model_id);
    let landed = levels_of(&persona, &before, model_id)
        .and_then(|(levels, default)| place.land(&levels, default.as_deref()));
    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        let mut patch = match &landed {
            Some(level) => effort_patch(log, persona_id, &place.key, level)?,
            None => json!({}),
        };
        patch["modelId"] = json!(model_id);
        update_persona(log, room, persona_id, &patch)?;
    }
    if let Some(level) = &landed {
        remember_on_desk(log, &place.key, level)?;
    }
    // The persisted choice counts as a use, so lastModelId is the id
    // just written rather than whatever a live session happens to report.
    if persona.backend_id == HOTLINE_BACKEND_ID {
        write_last_model(log, model_id)?;
    }

    if room.info(persona_id).state != SessionState::Idle {
        let info = room.set_model(persona_id, model_id).await?;
        let info = land_live_effort(log, room, persona_id, &place, landed, info).await?;
        return keep_fast(room, &persona, info).await;
    }
    Ok(offered_info(log, room, persona_id))
}

/// A live session's effort once its model has moved. The session's own
/// levels are the last word: a model the desk could not place, or placed
/// on a level its session does not have, is placed again on what the
/// session reports.
async fn land_live_effort(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    place: &Placing,
    landed: Option<String>,
    info: SessionInfo,
) -> Result<SessionInfo, String> {
    let Some(config) = catalog::effort_config(&info.configs).cloned() else {
        return Ok(info);
    };
    let levels: Vec<String> = config.options.iter().map(|one| one.id.clone()).collect();
    let level = if let Some(level) = landed.filter(|level| levels.contains(level)) {
        level
    } else {
        let Some(level) = place.land(&levels, config.current_id.as_deref()) else {
            return Ok(info);
        };
        keep_effort(log, room, persona_id, &place.key, &level).await?;
        level
    };
    if config.current_id.as_deref() == Some(level.as_str()) {
        return Ok(info);
    }
    room.set_config(persona_id, &config.id, &level).await
}

/// A live session's fast mode and context once its model has moved: the
/// teammate's choices, where the new model offers them. An adapter drops a
/// model's settings on a model without them and brings them back at the
/// model's own.
async fn keep_fast(
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
    mut info: SessionInfo,
) -> Result<SessionInfo, String> {
    let put_back: Vec<(String, String)> = info
        .configs
        .iter()
        .filter(|config| {
            matches!(
                config.category,
                Some(SessionConfigCategory::Fast | SessionConfigCategory::Context)
            )
        })
        .filter_map(|config| {
            let chosen = persona.config_choices.get(&config.id)?;
            (config.current_id.as_ref() != Some(chosen)
                && config.options.iter().any(|option| option.id == *chosen))
            .then(|| (config.id.clone(), chosen.clone()))
        })
        .collect();
    for (config_id, value) in put_back {
        info = room.set_config(&persona.id, &config_id, &value).await?;
    }
    Ok(info)
}

/// What a model switch places the new model's effort by: the teammate's and
/// the desk's memories for that model and the effort it runs at now.
struct Placing {
    key: String,
    teammate: Option<String>,
    desk: Option<String>,
    current: Option<String>,
    current_levels: Vec<String>,
}

impl Placing {
    fn new(log: &Log, persona: &Persona, before: &SessionInfo, model_id: &str) -> Self {
        let key = effort::key(&persona.backend_id, model_id);
        let desk = room::settings(log)
            .get(effort::DESK_SETTING)
            .and_then(|memory| memory.get(&key))
            .and_then(Value::as_str)
            .map(str::to_string);
        // The effort as the session or the teammate's offer draws it; a
        // Hotline Agent teammate at rest has no picker drawn, so its own.
        let (current, current_levels) = match catalog::effort_config(&before.configs) {
            Some(config) => (
                config.current_id.clone().or(persona.effort_id.clone()),
                config.options.iter().map(|one| one.id.clone()).collect(),
            ),
            None if persona.backend_id == HOTLINE_BACKEND_ID => {
                let model = before
                    .current_model_id
                    .clone()
                    .or(persona.model_id.clone())
                    .unwrap_or_default();
                (
                    persona
                        .effort_id
                        .clone()
                        .or_else(|| crate::models::default_effort(&model)),
                    crate::models::efforts(&model),
                )
            }
            None => (persona.effort_id.clone(), Vec::new()),
        };
        Self {
            teammate: persona.effort_by_model.get(&key).cloned(),
            key,
            desk,
            current,
            current_levels,
        }
    }

    fn land(&self, levels: &[String], default: Option<&str>) -> Option<String> {
        effort::land(&effort::Switch {
            levels,
            default,
            teammate: self.teammate.as_deref(),
            desk: self.desk.as_deref(),
            current: self.current.as_deref(),
            current_levels: &self.current_levels,
        })
    }
}

/// A model's levels and default as the desk knows them before switching:
/// Hotline Agent's from the catalogue, a harness's as its offer lists them.
/// None when a harness has not said, which only a session on it can.
fn levels_of(
    persona: &Persona,
    before: &SessionInfo,
    model_id: &str,
) -> Option<(Vec<String>, Option<String>)> {
    if persona.backend_id == HOTLINE_BACKEND_ID {
        return Some((
            crate::models::efforts(model_id),
            crate::models::default_effort(model_id),
        ));
    }
    let model = before.models.iter().find(|model| model.id == model_id)?;
    let levels = model.efforts.as_ref()?;
    Some((
        levels.iter().map(|level| level.id.clone()).collect(),
        model.default_effort.clone(),
    ))
}

/// An effort a switch landed on once the session said the model's levels:
/// the teammate's effort, remembered on it and on the desk for the model.
async fn keep_effort(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    key: &str,
    level: &str,
) -> Result<(), String> {
    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        update_persona(
            log,
            room,
            persona_id,
            &effort_patch(log, persona_id, key, level)?,
        )?;
    }
    remember_on_desk(log, key, level)
}

/// The teammate's effort set to `level`, remembered for the model `key`
/// names. Read under the policy lock, so no other write is lost.
fn effort_patch(log: &Log, persona_id: &str, key: &str, level: &str) -> Result<Value, String> {
    let mut patch = json!({ "effortId": level });
    if let Some(memory) = effort::remembered(living(log, persona_id)?.effort_by_model, key, level) {
        patch["effortByModel"] = json!(memory);
    }
    Ok(patch)
}

/// The desk's last effort for a model, across teammates.
fn remember_on_desk(log: &Log, key: &str, level: &str) -> Result<(), String> {
    let memory: std::collections::BTreeMap<String, String> = room::settings(log)
        .get(effort::DESK_SETTING)
        .and_then(|memory| serde_json::from_value(memory.clone()).ok())
        .unwrap_or_default();
    let Some(memory) = effort::remembered(memory, key, level) else {
        return Ok(());
    };
    let mut patch = Map::new();
    patch.insert(effort::DESK_SETTING.into(), json!(memory));
    update_settings(log, patch).map(|_| ())
}

/// How long a switch of agent waits for a busy teammate to finish its turn
/// before it gives up and leaves the record to take effect on the next start.
const SWITCH_PATIENCE: std::time::Duration = std::time::Duration::from_hours(6);

/// Moves a teammate to another agent, with a model when one is named.
///
/// The record is written at once, so a desk that restarts starts the
/// teammate on the new agent. The new agent's saved session is withdrawn
/// first: the teammate starts it fresh, seeded from the chapter like any
/// new session, and never reopens one it left earlier (A→B→A). A turn in
/// progress is not cut short: the live session restarts on the new agent
/// once the teammate is idle, and the switch lands on its next turn.
pub(super) async fn set_backend(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    backend_id: &str,
    model_id: Option<&str>,
) -> Result<SessionInfo, String> {
    let persona = living(log, persona_id)?;
    match room
        .backends()
        .await
        .into_iter()
        .find(|backend| backend.id == backend_id)
    {
        None => return Err(format!("There is no agent {backend_id} on this desk.")),
        Some(backend) if backend.unavailable.is_some() => {
            return Err(format!(
                "{} is not ready: {}",
                backend.name,
                backend.unavailable.unwrap_or_default()
            ));
        }
        Some(_) => {}
    }
    let model_id = model_id.filter(|model| !model.is_empty());
    if backend_id == HOTLINE_BACKEND_ID
        && let Some(model) = model_id
        && !room
            .models()
            .iter()
            .any(|choice| choice.id == model || crate::models::base_id(&choice.id) == model)
    {
        return Err(format!("{model} is not a model this desk can reach."));
    }
    if persona.backend_id == backend_id {
        return match model_id {
            Some(model) => set_model(log, room, persona_id, model).await,
            None => Ok(room.info(persona_id)),
        };
    }
    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        let mut moved = living(log, persona_id)?;
        moved.backend_id = backend_id.to_string();
        // A model is the new agent's when named; otherwise the agent's own.
        // Its effort and settings are its own too: the old agent's ids mean
        // nothing to it.
        moved.model_id = model_id.map(str::to_string);
        moved.effort_id = None;
        moved.config_choices.clear();
        moved.updated_at = chrono::Utc::now().timestamp_millis();
        room::append_persona(log, &moved)?;
        room::clear_checkpoint(log, persona_id, backend_id)?;
    }
    if backend_id == HOTLINE_BACKEND_ID
        && let Some(model) = model_id
    {
        write_last_model(log, model)?;
    }
    if busy(room.info(persona_id).state) {
        let room = room.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            while busy(room.info(&persona_id).state) && started.elapsed() < SWITCH_PATIENCE {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            if let Err(error) = room.reattach(&persona_id).await {
                eprintln!("{persona_id} did not restart on its new agent: {error}");
            }
        });
    } else {
        room.reattach(persona_id).await?;
    }
    Ok(offered_info(log, room, persona_id))
}

/// A teammate in the middle of a turn, or of starting one.
fn busy(state: SessionState) -> bool {
    matches!(state, SessionState::Thinking | SessionState::Starting)
}

/// Write a teammate's effort first, then switch a live session if there is
/// one. A harness's settings are kept by its own config ids, see
/// [`set_harness_config`].
pub(super) async fn set_config(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    config_id: &str,
    value: &str,
) -> Result<SessionInfo, String> {
    let persona = living(log, persona_id)?;
    if persona.backend_id != HOTLINE_BACKEND_ID {
        return set_harness_config(log, room, &persona, config_id, value).await;
    }
    if config_id != "effort" {
        if room.info(persona_id).state != SessionState::Idle {
            return room.set_config(persona_id, config_id, value).await;
        }
        return Err("This agent does not offer that setting.".to_string());
    }

    let model_id = room
        .info(persona_id)
        .current_model_id
        .or(persona.model_id.clone());
    if let Some(model_id) = &model_id
        && !value.is_empty()
    {
        let listed = crate::models::efforts(model_id);
        if !listed.iter().any(|id| id == value) {
            let label = crate::models::label_of(model_id).unwrap_or(model_id.clone());
            return Err(format!("{value} is not an effort {label} offers."));
        }
    }

    // A level chosen is remembered for the model, on the teammate and the
    // desk; going back to the default is no choice to remember.
    let key = model_id
        .as_deref()
        .filter(|_| !value.is_empty())
        .map(|model_id| effort::key(HOTLINE_BACKEND_ID, model_id));
    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        let patch = match &key {
            Some(key) => effort_patch(log, persona_id, key, value)?,
            None => json!({ "effortId": (!value.is_empty()).then_some(value) }),
        };
        update_persona(log, room, persona_id, &patch)?;
    }
    if let Some(key) = &key {
        remember_on_desk(log, key, value)?;
    }

    if room.info(persona_id).state != SessionState::Idle {
        return room.set_config(persona_id, config_id, value).await;
    }
    Ok(room.info(persona_id))
}

/// A harness teammate's setting, kept on the teammate so its next session
/// opens on it: an effort picker as its effort, any other by the harness's
/// own id. A value the agent does not offer, live or as it last offered, is
/// refused; an empty one goes back to the agent's own default. With a
/// session up the setting is put on it too, and the answer is the
/// session's; at rest it is what the teammate offers, the choice current.
async fn set_harness_config(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
    config_id: &str,
    value: &str,
) -> Result<SessionInfo, String> {
    let live = room.info(&persona.id);
    let offered = super::super::roster::offered(room, persona, live.clone());
    let config = offered.configs.iter().find(|config| config.id == config_id);
    if let Some(config) = config
        && !value.is_empty()
        && !config.options.iter().any(|option| option.id == value)
    {
        return Err(format!(
            "{value} is not a choice of {} this agent offers.",
            config.name
        ));
    }
    let running = live.state != SessionState::Idle;
    let effort = config.map_or(config_id == "effort", |config| {
        config.category == Some(SessionConfigCategory::Effort)
    });
    // A harness may offer its model as a config, which is no setting of the
    // teammate's: a live session's answer names the model, remembered below.
    // An effort chosen is remembered for the model it is on, on the
    // teammate and the desk.
    let key = offered
        .current_model_id
        .as_deref()
        .filter(|model| effort && !value.is_empty() && !model.is_empty())
        .map(|model| effort::key(&persona.backend_id, model));
    if config.is_some() || effort || !running {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        let patch = if let Some(key) = &key {
            effort_patch(log, &persona.id, key, value)?
        } else if effort {
            json!({ "effortId": (!value.is_empty()).then_some(value) })
        } else {
            let mut choices = living(log, &persona.id)?.config_choices;
            if value.is_empty() {
                choices.remove(config_id);
            } else {
                choices.insert(config_id.to_string(), value.to_string());
            }
            json!({ "configChoices": choices })
        };
        update_persona(log, room, &persona.id, &patch)?;
    }
    if let Some(key) = &key {
        remember_on_desk(log, key, value)?;
    }
    if running {
        let info = room.set_config(&persona.id, config_id, value).await?;
        // A setting chosen can move the model; what it moved to is chosen too.
        keep_reported_model(log, room, persona, true).await?;
        return Ok(info);
    }
    Ok(offered_info(log, room, &persona.id))
}

/// The model a Hotline Agent teammate most recently ran on. The wire is the
/// only writer of the room stream's settings, so this lives here rather
/// than on the session. A prompt on an idle teammate starts it, so start
/// and prompt both come through.
/// What a live session reports it runs on is remembered once the session
/// is up: for Hotline Agent as the room's last model, for a harness on the
/// teammate itself only when it has none, so the band can name the model
/// before the child is started again. A harness picks its own default and
/// only says so once running; without this the teammate at rest has no
/// model at all. A model the teammate has is its choice, which a report
/// never replaces: its next session is set to it again (docs/sessions.md).
pub(in crate::wire) async fn remember_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
) -> Result<(), String> {
    keep_reported_model(log, room, persona, false).await
}

/// [`remember_model`], or with `chosen` the session's model as the answer
/// to a person's choice, kept on the teammate over the one it had.
async fn keep_reported_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
    chosen: bool,
) -> Result<(), String> {
    // Every prompt comes through here, and nearly every one has nothing new to
    // remember. The answer is read again under the lock before anything is
    // written, so this look is only to keep the lock off the common path.
    if room
        .info(&persona.id)
        .current_model_id
        .is_none_or(|model_id| {
            model_id.is_empty() || already_remembered(log, persona, &model_id, chosen)
        })
    {
        return Ok(());
    }
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let Some(model_id) = room.info(&persona.id).current_model_id else {
        return Ok(());
    };
    if model_id.is_empty() {
        return Ok(());
    }
    if persona.backend_id == HOTLINE_BACKEND_ID {
        return write_last_model(log, &model_id);
    }
    if already_remembered(log, &living(log, &persona.id)?, &model_id, chosen) {
        return Ok(());
    }
    update_persona(log, room, &persona.id, &json!({ "modelId": model_id })).map(|_| ())
}

/// Whether there is nothing for [`keep_reported_model`] to write: `model_id`
/// is already where it would go, or a harness teammate has a model of its
/// own and this is a report, not an answer to a choice.
fn already_remembered(log: &Log, persona: &Persona, model_id: &str, chosen: bool) -> bool {
    if persona.backend_id == HOTLINE_BACKEND_ID {
        room::settings(log)
            .get("lastModelId")
            .and_then(Value::as_str)
            == Some(model_id)
    } else {
        persona.model_id.as_deref() == Some(model_id) || (!chosen && persona.model_id.is_some())
    }
}

fn write_last_model(log: &Log, model_id: &str) -> Result<(), String> {
    let settings = room::settings(log);
    let current = settings.get("lastModelId").and_then(Value::as_str);
    if current == Some(model_id) {
        return Ok(());
    }
    let mut patch = Map::new();
    patch.insert("lastModelId".into(), json!(model_id));
    update_settings(log, patch).map(|_| ())
}

#[cfg(test)]
mod welcome_tests {
    use super::welcome;
    use crate::contract::{BackendChoice, Credential, CredentialKind};
    use serde_json::{Map, Value};

    fn credential(provider_id: &str, revoked: bool) -> Credential {
        Credential {
            id: format!("{provider_id}-key"),
            provider_id: provider_id.to_string(),
            credential_kind: CredentialKind::ApiKey,
            base_url: None,
            custom: None,
            label: provider_id.to_string(),
            revoked,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn backend(id: &str, unavailable: Option<&str>) -> BackendChoice {
        BackendChoice {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            unavailable: unavailable.map(str::to_string),
            models: None,
            sign_in: None,
        }
    }

    fn settings(default_backend_id: &str) -> Map<String, Value> {
        let mut settings = Map::new();
        settings.insert("defaultBackendId".into(), Value::from(default_backend_id));
        settings
    }

    #[test]
    fn a_fresh_room_cannot_run_and_a_live_key_is_the_way_in() {
        let fresh = welcome(
            &settings("hotline"),
            0,
            false,
            &[],
            vec![backend("hotline", None)],
        );
        assert!(!fresh.can_run);
        assert_eq!(fresh.providers, [] as [String; 0]);
        assert!(fresh.harnesses.is_empty(), "Hotline Agent is not a harness");
        assert_eq!(fresh.teammates, 0);

        let revoked = welcome(
            &settings("hotline"),
            0,
            false,
            &[credential("anthropic", true)],
            vec![backend("hotline", None)],
        );
        assert!(!revoked.can_run, "a revoked key runs nothing");

        let keyed = welcome(
            &settings("hotline"),
            0,
            false,
            &[
                credential("anthropic", false),
                credential("anthropic", false),
            ],
            vec![backend("hotline", None)],
        );
        assert!(keyed.can_run);
        assert_eq!(
            keyed.providers,
            vec!["Anthropic".to_string()],
            "named once, by its catalogue name"
        );
    }

    #[test]
    fn a_harness_is_a_way_in_only_as_the_rooms_default() {
        let backends = || {
            vec![
                backend("hotline", None),
                backend("cursor", None),
                backend("gemini", Some("Not installed")),
            ]
        };
        let installed = welcome(&settings("hotline"), 0, false, &[], backends());
        assert!(
            !installed.can_run,
            "a harness on the machine is not yet the room's"
        );
        assert_eq!(installed.harnesses.len(), 1);
        assert_eq!(installed.harnesses[0].id, "cursor");

        let chosen = welcome(&settings("cursor"), 0, false, &[], backends());
        assert!(chosen.can_run);
        assert_eq!(chosen.default_backend_id, "cursor");

        let missing = welcome(&settings("gemini"), 2, true, &[], backends());
        assert!(
            !missing.can_run,
            "a default this machine cannot start is no way in"
        );
        assert_eq!(missing.teammates, 2);
    }
}
