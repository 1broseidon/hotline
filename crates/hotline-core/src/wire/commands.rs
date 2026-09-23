//! What a command does.
//!
//! The room's own memory is the wire's to write: a teammate and a setting are
//! events on the room stream, and this is the only thing that appends them.
//! Everything else — a session, a secret, the models a key can reach — is
//! asked of the [`RoomHandle`], because a running agent and a vault are not
//! things to reimplement behind a door.

use super::RoomHandle;
use crate::contract::{
    Command, McpPolicy, Persona, PersonaDraft, PolicyMode, Reach, SessionInfo, SessionState,
};
use crate::driver::HOTLINE_BACKEND_ID;
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
        Command::MobilePrompt { .. }
        | Command::MobileAttachment { .. }
        | Command::MobilePushRegister { .. } => Err("This command requires a paired phone.".into()),
        Command::PersonaCreate { draft } => create_persona(log, draft),
        Command::PersonaUpdate { id, patch } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            let (updated, reattaches) = update_persona(log, room, &id, &patch)?;
            if reattaches {
                room.reattach(&id).await?;
            } else if patch.get("computer").is_some() {
                room.computer_settings_changed(&id).await;
            }
            Ok(updated)
        }
        Command::PersonaDelete { id } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            delete_persona(log, room, &id)
        }
        Command::SettingsUpdate { mut patch } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            if let Some(value) = patch.get_mut("mcpServers")
                && !value.is_null()
            {
                *value = room.protect_mcp_settings(value)?;
            }
            let servers = patch.contains_key("mcpServers");
            if servers {
                room.invalidate_all()?;
            }
            let updated = update_settings(log, patch)?;
            if servers {
                forget_removed_servers(log)?;
                // Replacing or deleting legacy sources also removes their
                // superseded plaintext values from the room's history.
                log.migrate_mcp_settings(|value| {
                    room.protect_mcp_settings(value)
                        .map_err(std::io::Error::other)
                })
                .map_err(|error| error.to_string())?;
                room.reattach_all().await?;
            }
            Ok(updated)
        }

        Command::CredentialCreate {
            provider_id,
            label,
            secret,
        } => room
            .credential_create(&provider_id, &label, &secret)
            .map(|credential| json!(credential)),
        Command::CredentialRevoke { id } => room.credential_revoke(&id).map(|()| Value::Null),
        Command::CredentialDelete { id } => room.credential_delete(&id).map(|()| Value::Null),
        Command::CredentialLogin { provider_id } => room
            .credential_login(&provider_id)
            .await
            .map(|prompt| json!(prompt)),
        Command::CredentialLoginCancel { login_id } => room
            .credential_login_cancel(&login_id)
            .map(|()| Value::Null),
        Command::CredentialConnectLocal { base_url } => room
            .credential_connect_local(&base_url)
            .await
            .map(|credential| json!(credential)),
        Command::CredentialCustomSave { id, draft } => room
            .credential_custom_save(id.as_deref(), draft)
            .map(|credential| json!(credential)),
        Command::CredentialCustomModels {
            id,
            base_url,
            secret,
        } => room
            .credential_custom_models(id.as_deref(), &base_url, secret.as_deref())
            .await
            .map(|models| json!(models)),
        Command::CredentialLoginStatus { login_id } => {
            room.login_status(&login_id).map(|status| json!(status))
        }
        Command::CredentialRefreshModels { provider_id } => room
            .credential_refresh_models(&provider_id)
            .await
            .map(|models| json!(models)),
        Command::CredentialList {} => Ok(json!(room.credentials())),
        Command::McpAuthStart { server_id } | Command::McpAuthReconnect { server_id } => {
            room.mcp_auth_start(&server_id).await
        }
        Command::McpAuthCallback {
            login_id,
            callback_url,
        } => room.mcp_auth_callback(&login_id, &callback_url).await,
        Command::McpAuthStatus { server_id } => room.mcp_auth_status(&server_id).await,
        Command::McpAuthSignOut { server_id } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            room.invalidate_all()?;
            // Failed credential deletion must leave the old capabilities
            // revoked; reattaching would restore access the operator removed.
            room.mcp_auth_sign_out(&server_id).await?;
            room.reattach_all().await?;
            Ok(Value::Null)
        }
        Command::McpSecretSet {
            server_id,
            url,
            secret,
        } => room
            .mcp_secret_set(&server_id, &url, &secret)
            .map(|()| Value::Null),
        Command::BackendsList {} => Ok(json!(room.backends().await)),
        Command::SkillsList { persona_id } => room
            .skills(persona_id.as_deref())
            .map(|skills| json!(skills)),
        // The gateway is a folder in the data directory, which the log owns;
        // no session is touched, so the room is not asked.
        Command::SkillsAdd { path } => {
            let gateway = paths::skills_path(log.root());
            std::fs::create_dir_all(&gateway)
                .map_err(|error| format!("{} could not be made: {error}", gateway.display()))?;
            crate::skills::add_to_gateway(&gateway, std::path::Path::new(&path))
                .map(|entry| json!(entry))
        }
        Command::SkillsRemove { name } => {
            crate::skills::remove_from_gateway(&paths::skills_path(log.root()), &name)
                .map(|()| Value::Null)
        }
        // The switch is a room setting naming what is offered; the person's
        // folder itself is never written.
        Command::SkillsOffer { name, offered } => {
            let offering = crate::skills::Offering::from_settings(log.root(), &room::settings(log));
            let entry = offering.home_entry(&name)?;
            let mut patch = Map::new();
            patch.insert(
                crate::skills::OFFERED_SETTING.to_owned(),
                json!(offering.switched(&name, offered)),
            );
            update_settings(log, patch)?;
            Ok(json!(crate::contract::SkillEntry {
                offered: Some(offered),
                ..entry
            }))
        }
        Command::ProvidersList {} => Ok(json!(crate::models::providers())),
        Command::ModelsList {} => Ok(json!(room.models())),
        Command::ModelsCatalog { provider_id } => room
            .models_catalog(&provider_id)
            .map(|models| json!(models)),
        Command::ModelsManualSet {
            provider_id,
            model_ids,
        } => room
            .models_manual_set(&provider_id, &model_ids)
            .map(|models| json!(models)),
        Command::ModelsEfforts { model_id } => Ok(json!(room.models_efforts(&model_id))),

        Command::SessionStart { persona_id } => {
            let info = room.start(&persona_id).await?;
            if let Ok(persona) = living(log, &persona_id) {
                remember_model(log, room, &persona).await?;
            }
            Ok(json!(info))
        }
        Command::SessionStop { persona_id } => room.stop(&persona_id).map(|()| Value::Null),
        Command::SessionPrompt {
            persona_id,
            text,
            reply_to,
            attachments,
        } => {
            room.prompt(&persona_id, &text, reply_to, attachments)
                .await?;
            if let Ok(persona) = living(log, &persona_id) {
                remember_model(log, room, &persona).await?;
            }
            Ok(Value::Null)
        }
        Command::SessionCancel { persona_id } => room.cancel(&persona_id).map(|()| Value::Null),
        Command::SessionSetModel {
            persona_id,
            model_id,
        } => set_model(log, room, &persona_id, &model_id)
            .await
            .map(|info| json!(info)),

        Command::SessionSetMode {
            persona_id,
            mode_id,
        } => room
            .set_mode(&persona_id, &mode_id)
            .await
            .map(|info| json!(info)),
        Command::SessionSetConfig {
            persona_id,
            config_id,
            value,
        } => set_config(log, room, &persona_id, &config_id, &value)
            .await
            .map(|info| json!(info)),
        Command::SessionAnswerPermission {
            persona_id,
            request_id,
            option_id,
        } => room
            .answer_permission(&persona_id, &request_id, &option_id)
            .await
            .map(|()| Value::Null),
        Command::HumanAnswer {
            persona_id,
            action_id,
            status,
            note,
        } => room
            .answer_human(&persona_id, &action_id, status, note)
            .map(|()| Value::Null),

        Command::SearchThread {
            persona_id,
            query,
            limit,
        } => Ok(search::search(log.root(), &persona_id, &query, limit)),
        Command::SearchAll { query, limit } => Ok(search::search_all(log.root(), &query, limit)),
        Command::ChapterList { persona_id } => Ok(json!(chapters::list(log, &persona_id))),
        Command::RoomImport { from } => room
            .import(&home_expanded(&from))
            .map(|report| json!(report)),
        Command::ChapterStartFresh { persona_id } => room
            .start_fresh_chapter(&persona_id)
            .await
            .map(|chapter| json!(chapter)),
        Command::ChapterResume { persona_id } => room
            .resume_chapter(&persona_id)
            .await
            .map(|chapter| json!(chapter)),
        Command::TeammateTools { persona_id } => Ok(json!(room.teammate_tools(&persona_id))),

        Command::ScheduleCreate {
            persona_id,
            kind,
            when,
            every,
            prompt,
            quiet,
        } => room
            .schedule_create(
                &persona_id,
                kind,
                when,
                every,
                &prompt,
                quiet.unwrap_or(false),
            )
            .map(|job| json!(job)),
        Command::ScheduleList {} => Ok(json!(room.schedule_list())),
        Command::ScheduleCancel { id } => room.schedule_cancel(&id).map(|()| Value::Null),
        Command::ScheduleSetQuiet { id, quiet } => {
            room.schedule_set_quiet(&id, quiet).map(|()| Value::Null)
        }

        Command::PeersList { persona_id } => Ok(json!(room.peer_threads(&persona_id))),
        Command::PeersMarkRead { key, event_ids } => {
            Ok(json!(room.mark_peer_read(&key, &event_ids)))
        }

        Command::ComputerRuntimes {} => Ok(json!(room.computer_runtimes().await)),
        Command::ComputerReleases {} => {
            Ok(serde_json::to_value(room.computer_releases()).unwrap_or(Value::Null))
        }
        Command::ComputerReleasesCheck {} => {
            Ok(serde_json::to_value(room.computer_releases_check().await).unwrap_or(Value::Null))
        }
        Command::Welcome {} => {
            let settings = room::settings(log);
            let welcome = welcome(
                &settings,
                room::roster(log).len(),
                &room.credentials(),
                room.backends().await,
            );
            Ok(serde_json::to_value(welcome).unwrap_or(Value::Null))
        }
        Command::ComputerStatus { persona_id } => {
            living(log, &persona_id)?;
            room.computer_status(&persona_id)
                .await
                .map(|status| json!(status))
        }
        Command::ComputerStop { persona_id } => {
            living(log, &persona_id)?;
            room.computer_stop(&persona_id).await.map(|()| Value::Null)
        }
        Command::ComputerRemove { persona_id } => {
            living(log, &persona_id)?;
            room.computer_remove(&persona_id)
                .await
                .map(|()| Value::Null)
        }
        Command::ComputerUpdate { persona_id } => {
            living(log, &persona_id)?;
            room.computer_update(&persona_id)
                .await
                .map(|()| Value::Null)
        }
        Command::ComputerBrowsersList {} => Ok(json!(room.computer_browsers().await)),
        Command::ComputerCookiesPreview {
            browser_id,
            profile_id,
        } => room
            .computer_cookies_preview(&browser_id, &profile_id)
            .await
            .map(|sites| json!(sites)),
        Command::ComputerCookiesImport {
            persona_id,
            browser_id,
            profile_id,
            domains,
        } => {
            living(log, &persona_id)?;
            room.computer_cookies_import(&persona_id, &browser_id, &profile_id, &domains)
                .await
                .map(|sites| json!(sites))
        }
        Command::ComputerCookiesList { persona_id } => room
            .computer_cookies_list(&persona_id)
            .await
            .map(|imports| json!(imports)),
        Command::ComputerCookiesForget {
            persona_id,
            browser_id,
            profile_id,
            domain,
        } => {
            living(log, &persona_id)?;
            room.computer_cookies_forget(&persona_id, &browser_id, &profile_id, domain.as_deref())
                .await
                .map(|imports| json!(imports))
        }
        Command::SecretsList {} => room.secrets_list().map(|secrets| json!(secrets)),
        Command::SecretsSet { name, value } => room
            .secrets_set(&name, &value)
            .await
            .map(|secret| json!(secret)),
        Command::SecretsDelete { name } => room.secrets_delete(&name).await.map(|()| Value::Null),
        Command::SecretsLoginSet {
            name,
            sites,
            username,
            password,
            totp,
        } => room
            .secrets_login_set(&name, &sites, &username, &password, totp.as_deref())
            .await
            .map(|secret| json!(secret)),
        Command::SecretsPasskeyRegister {
            name,
            persona_id,
            rp_id,
        } => {
            living(log, &persona_id)?;
            room.secrets_passkey_register(&name, &persona_id, &rp_id)
                .await
                .map(|registration| json!(registration))
        }
        Command::SecretsPasskeyRegistration { persona_id } => room
            .secrets_passkey_registration(&persona_id)
            .await
            .map(|registration| json!(registration)),
        Command::SecretsPasskeyAnswer {
            persona_id,
            ask_id,
            approved,
        } => room
            .secrets_passkey_answer(&persona_id, &ask_id, approved)
            .await
            .map(|registration| json!(registration)),
        Command::SecretsPasskeyCancel { persona_id } => room
            .secrets_passkey_cancel(&persona_id)
            .await
            .map(|()| Value::Null),
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

/// A teammate as the previous edition's `createPersona` made one: a fresh uuid,
/// the room's default backend, a workspace under the data directory, and
/// every capability its policy can give.
fn create_persona(log: &Log, draft: PersonaDraft) -> Result<Value, String> {
    let id = Uuid::new_v4().to_string();
    let settings = room::settings(log);
    let default_backend = settings
        .get("defaultBackendId")
        .and_then(Value::as_str)
        .unwrap_or("hotline")
        .to_string();
    let stamped = now();
    let backend_id = given(draft.backend_id).unwrap_or(default_backend);
    // A Hotline Agent draft that leaves the model blank takes the room's
    // standing choice, so the teammate starts on what Settings named
    // rather than whichever model happens to lead the catalogue.
    let model_id = given(draft.model_id).or_else(|| {
        if backend_id != HOTLINE_BACKEND_ID {
            return None;
        }
        crate::models::preferred_model(&settings)
    });
    let persona = Persona {
        node: None,
        id: id.clone(),
        name: given(Some(draft.name)).unwrap_or_else(|| "Untitled".to_string()),
        goal: given(draft.goal).unwrap_or_default(),
        face: None,
        team: given(draft.team),
        backend_id,
        cwd: given(draft.cwd).unwrap_or_else(|| {
            paths::default_workspace(log.root(), &id)
                .to_string_lossy()
                .into_owned()
        }),
        // The workspace is the wall unless the draft asked for the machine,
        // and an absent reach is the workspace, so only the wider one is
        // written down.
        reach: draft.reach.filter(|reach| *reach == Reach::Machine),
        model_id,
        mode_id: None,
        effort_id: given(draft.effort_id),
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::None,
            server_ids: Vec::new(),
        },
        skill_policy: Default::default(),
        background_work: draft.background_work.unwrap_or(false),
        allowed_senders: Vec::new(),
        web_search_policy: None,
        computer: draft.computer,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: stamped,
        updated_at: stamped,
    };
    room::append_persona(log, &persona)?;
    Ok(json!(persona))
}

/// The patch over the record, and the whole record written again: a stream
/// folds by id, so a line carrying only what changed would leave the fold
/// holding only what changed.
fn update_persona(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<(Value, bool), String> {
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
    let reattaches = persona_update_reattaches(patch, &previous, &updated);
    if reattaches {
        room.invalidate(id)?;
    }
    room::append_persona(log, &updated)?;
    Ok((json!(updated), reattaches))
}

/// Whether this update restarts a live session. The computer's own
/// settings do not, short of turning it on or off: its limits, mounts and
/// image take effect when the container is next made, and its secrets are
/// handed to it where it runs. A turn in flight is not cut short for them.
fn persona_update_reattaches(patch: &Value, previous: &Persona, updated: &Persona) -> bool {
    let enabled = |persona: &Persona| {
        persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
    };
    let mut rest = patch.clone();
    if let Some(fields) = rest.as_object_mut() {
        fields.remove("computer");
    }
    persona_patch_reattaches(&rest) || enabled(previous) != enabled(updated)
}

/// A patch of these fields rebuilds the driver, so a live session has to
/// restart for the new tools to take effect. `name`, `team`, `face`,
/// `modelId`, `modeId` and `effortId` do not: model, mode and effort already
/// switch live.
fn persona_patch_reattaches(patch: &Value) -> bool {
    const KEYS: [&str; 10] = [
        "cwd",
        "reach",
        "goal",
        "mcpPolicy",
        "skillPolicy",
        "computer",
        "backendId",
        "harnessOverride",
        "backgroundWork",
        "allowedSenders",
    ];
    patch
        .as_object()
        .is_some_and(|fields| KEYS.iter().any(|key| fields.contains_key(*key)))
}

/// The tombstone is the same kind and id again, so the fold finds it instead
/// of the teammate and a mirror has a line to ship rather than an absence to
/// notice.
fn delete_persona(log: &Log, room: &Arc<dyn RoomHandle>, id: &str) -> Result<Value, String> {
    living(log, id)?;
    room.invalidate(id)?;
    append(
        log,
        &json!({ "kind": "persona", "id": id, "deleted": true }),
    )?;
    crate::session::ledger::forget(id);
    room.forget(id);
    Ok(Value::Null)
}

/// One event per key, and `null` clears a key rather than setting it to
/// nothing — which puts the room's default back, because a deleted setting is
/// not an override.
fn update_settings(log: &Log, patch: Map<String, Value>) -> Result<Value, String> {
    for (key, value) in patch {
        // MCP entries are a settings boundary. Canonicalise them before the
        // room event is written so a caller cannot persist an OAuth client
        // secret alongside public server configuration.
        let value = if key == "mcpServers" && !value.is_null() {
            Value::Array(crate::mcp::normalize_servers(&value))
        } else {
            value
        };
        let event = if value.is_null() {
            json!({ "kind": "setting", "id": key, "deleted": true })
        } else {
            json!({ "kind": "setting", "id": key, "value": value })
        };
        append(log, &event)?;
    }
    let mut settings = room::settings(log);
    if let Some(servers) = settings.get_mut("mcpServers") {
        *servers = crate::mcp::public_servers(servers);
    }
    Ok(Value::Object(settings))
}

/// The ids of the room's tool sources, as settings hold them now.
fn server_ids(log: &Log) -> Vec<String> {
    room::settings(log)
        .get("mcpServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|server| server["id"].as_str().map(String::from))
        .collect()
}

/// A tool source removed from settings leaves every teammate's grant with
/// it. Removing it was the decision; a policy still naming it would only be
/// a warning that it is gone, on every teammate that ever had it. Each save
/// of the list settles every grant against it, so one left behind by an
/// older build goes too.
fn forget_removed_servers(log: &Log) -> Result<(), String> {
    let servers = server_ids(log);
    for mut persona in room::roster(log) {
        let granted = persona.mcp_policy.server_ids.len();
        persona
            .mcp_policy
            .server_ids
            .retain(|id| servers.contains(id));
        if persona.mcp_policy.server_ids.len() != granted {
            persona.updated_at = now();
            room::append_persona(log, &persona)?;
        }
    }
    Ok(())
}

/// Where a fresh room stands, from what it already knows. A live credential
/// is a way to run Hotline Agent; a startable harness is a way to run only once
/// it is the room's default, because that is what the first teammate lands
/// on. Hotline Agent is not a harness here: it is what the providers are for.
pub(crate) fn welcome(
    settings: &Map<String, Value>,
    teammates: usize,
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
    }
}

fn living(log: &Log, id: &str) -> Result<Persona, String> {
    room::roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
        .ok_or_else(|| format!("There is no teammate {id}."))
}

/// Write the teammate's model first, then switch a live session if there is
/// one. The persona is the truth; the live switch is a courtesy to the turn
/// already running. Writing first means an idle teammate keeps the choice,
/// and a live switch that fails still lands on the next start.
async fn set_model(
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
async fn set_config(
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
async fn remember_model(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
) -> Result<(), String> {
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

fn append(log: &Log, event: &Value) -> Result<(), String> {
    log.append(&StreamId::Room, event)
        .map(|_| ())
        .map_err(|error| format!("The room's stream could not be written: {error}."))
}

/// A path as a person types one: a leading `~/` is their home directory,
/// because the window offers the previous edition's data directory spelled that
/// way and does not know where home is.
fn home_expanded(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => std::path::PathBuf::from(home).join(rest),
            None => std::path::PathBuf::from(path),
        },
        None => std::path::PathBuf::from(path),
    }
}

#[cfg(test)]
mod path_tests {
    use super::home_expanded;

    #[test]
    fn a_tilde_is_the_home_directory_and_anything_else_is_itself() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            home_expanded("~/.local/share/hotline"),
            std::path::Path::new(&home).join(".local/share/hotline")
        );
        assert_eq!(
            home_expanded("/var/hotline"),
            std::path::PathBuf::from("/var/hotline")
        );
        assert_eq!(
            home_expanded("~hotline"),
            std::path::PathBuf::from("~hotline")
        );
    }
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
        let fresh = welcome(&settings("hotline"), 0, &[], vec![backend("hotline", None)]);
        assert!(!fresh.can_run);
        assert!(fresh.providers.is_empty());
        assert!(fresh.harnesses.is_empty(), "Hotline Agent is not a harness");
        assert_eq!(fresh.teammates, 0);

        let revoked = welcome(
            &settings("hotline"),
            0,
            &[credential("anthropic", true)],
            vec![backend("hotline", None)],
        );
        assert!(!revoked.can_run, "a revoked key runs nothing");

        let keyed = welcome(
            &settings("hotline"),
            0,
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
        let installed = welcome(&settings("hotline"), 0, &[], backends());
        assert!(
            !installed.can_run,
            "a harness on the machine is not yet the room's"
        );
        assert_eq!(installed.harnesses.len(), 1);
        assert_eq!(installed.harnesses[0].id, "cursor");

        let chosen = welcome(&settings("cursor"), 0, &[], backends());
        assert!(chosen.can_run);
        assert_eq!(chosen.default_backend_id, "cursor");

        let missing = welcome(&settings("gemini"), 2, &[], backends());
        assert!(
            !missing.can_run,
            "a default this machine cannot start is no way in"
        );
        assert_eq!(missing.teammates, 2);
    }
}
