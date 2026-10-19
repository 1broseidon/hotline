//! Which model a teammate runs on, and what a fresh room can run.

use super::persona::{living, update_persona};
use super::settings::update_settings;
use crate::contract::{Persona, SessionInfo, SessionState};
use crate::driver::HOTLINE_BACKEND_ID;
use crate::log::Log;
use crate::room;
use crate::wire::RoomHandle;
use serde_json::{Map, Value, json};
use std::sync::Arc;

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
/// and a live switch that fails still lands on the next start.
pub(super) async fn set_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    model_id: &str,
) -> Result<SessionInfo, String> {
    let persona = living(log, persona_id)?;
    if persona.backend_id == HOTLINE_BACKEND_ID {
        if !room.models().iter().any(|choice| choice.id == model_id) {
            return Err(format!("{model_id} is not a model this desk can reach."));
        }
    } else if model_id.is_empty() {
        return Err("A model needs an id.".to_string());
    }

    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        update_persona(log, room, persona_id, &json!({ "modelId": model_id }))?;
    }
    // The persisted choice counts as a use, so lastModelId is the id
    // just written rather than whatever a live session happens to report.
    if persona.backend_id == HOTLINE_BACKEND_ID {
        write_last_model(log, model_id)?;
    }

    if room.info(persona_id).state != SessionState::Idle {
        return room.set_model(persona_id, model_id).await;
    }
    Ok(room.info(persona_id))
}

/// Write a Hotline Agent teammate's effort first, then switch a live session
/// if there is one. An ACP teammate has no stored effort: the harness owns
/// its config ids, and a live session is required.
pub(super) async fn set_config(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    config_id: &str,
    value: &str,
) -> Result<SessionInfo, String> {
    let persona = living(log, persona_id)?;
    if persona.backend_id != HOTLINE_BACKEND_ID {
        // A harness may offer its model as a config; what it reports after
        // the change is remembered the same way a start's report is.
        let info = room.set_config(persona_id, config_id, value).await?;
        remember_model(log, room, &persona).await?;
        return Ok(info);
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
    if let Some(model_id) = model_id
        && !value.is_empty()
    {
        let listed = crate::models::efforts(&model_id);
        if !listed.iter().any(|id| id == value) {
            let label = crate::models::label_of(&model_id).unwrap_or(model_id);
            return Err(format!("{value} is not an effort {label} offers."));
        }
    }

    let effort = if value.is_empty() {
        Value::Null
    } else {
        json!(value)
    };
    {
        let gate = room.policy_update_lock();
        let _held = gate.lock().await;
        update_persona(log, room, persona_id, &json!({ "effortId": effort }))?;
    }

    if room.info(persona_id).state != SessionState::Idle {
        return room.set_config(persona_id, config_id, value).await;
    }
    Ok(room.info(persona_id))
}

/// The model a Hotline Agent teammate most recently ran on. The wire is the
/// only writer of the room stream's settings, so this lives here rather
/// than on the session. A prompt on an idle teammate starts it, so start
/// and prompt both come through.
/// What a live session reports it runs on is remembered once the session
/// is up: for Hotline Agent as the room's last model, for a harness on the
/// teammate itself, so the band can name the model before the child is
/// started again. A harness picks its own default and only says so once
/// running; without this the teammate at rest has no model at all.
pub(in crate::wire) async fn remember_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
) -> Result<(), String> {
    // Every prompt comes through here, and nearly every one has nothing new to
    // remember. The answer is read again under the lock before anything is
    // written, so this look is only to keep the lock off the common path.
    if room
        .info(&persona.id)
        .current_model_id
        .is_none_or(|model_id| model_id.is_empty() || already_remembered(log, persona, &model_id))
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
    if persona.model_id.as_deref() == Some(model_id.as_str()) {
        return Ok(());
    }
    update_persona(log, room, &persona.id, &json!({ "modelId": model_id })).map(|_| ())
}

/// Whether `model_id` is already where [`remember_model`] would put it.
fn already_remembered(log: &Log, persona: &Persona, model_id: &str) -> bool {
    if persona.backend_id == HOTLINE_BACKEND_ID {
        room::settings(log)
            .get("lastModelId")
            .and_then(Value::as_str)
            == Some(model_id)
    } else {
        persona.model_id.as_deref() == Some(model_id)
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
