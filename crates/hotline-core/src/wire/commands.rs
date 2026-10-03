//! What a command does.
//!
//! The room's own memory is the wire's to write: a teammate and a setting are
//! events on the room stream, and this is the only thing that appends them.
//! Everything else — a session, a secret, the models a key can reach — is
//! asked of the [`RoomHandle`], because a running agent and a vault are not
//! things to reimplement behind a door.

use super::RoomHandle;
use crate::contract::{
    Command, McpPolicy, Persona, PersonaComputer, PersonaDraft, PolicyMode, Reach, SessionInfo,
    SessionState,
};
use crate::driver::HOTLINE_BACKEND_ID;
use crate::log::{Log, StreamId};
use crate::store::{chapters, search};
use crate::{paths, room};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

// Only the dispatcher sets this scope; wire parameters cannot claim voice origin.
tokio::task_local! { pub(crate) static VOICE_COMMAND: (); }
tokio::task_local! { pub(crate) static CALL_ORIGIN: crate::voice::Origin; }

pub(crate) fn voice_origin() -> Option<crate::voice::Origin> {
    CALL_ORIGIN.try_with(Clone::clone).ok()
}

// What was said on a direct call that the teammate's session has not heard,
// to go ahead of the person's words. The tape shows only the words.
tokio::task_local! { pub(crate) static CALL_HEARD: Option<String>; }

pub(crate) fn call_heard() -> Option<String> {
    CALL_HEARD.try_with(Clone::clone).ok().flatten()
}

// Set by the door a message came in at, never by the message: which of the
// person's apps wrote it.
tokio::task_local! { pub(crate) static PROMPT_CLIENT: crate::contract::Client; }

pub(crate) fn prompt_client() -> Option<crate::contract::Client> {
    PROMPT_CLIENT.try_with(|client| *client).ok()
}

pub(crate) fn from_voice() -> bool {
    VOICE_COMMAND.try_with(|()| ()).is_ok()
}

pub(crate) async fn run(
    command: Command,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
) -> Result<Value, String> {
    match command {
        Command::ImagesStatus {} => Ok(json!(room.images_status())),
        Command::CapabilitiesOptions {} => room
            .capability_options()
            .await
            .map(|options| json!(options)),
        Command::VoiceStatus { input_mode } => Ok(json!(
            voice(room)?.status_for(input_mode.unwrap_or_default())
        )),
        Command::VoiceCallStart {
            call_id,
            persona_id,
            stream_audio,
            input_mode,
        } => Ok(json!(voice(room)?.start_with_input(
            &call_id,
            persona_id,
            stream_audio.unwrap_or_default(),
            input_mode.unwrap_or_default(),
            room.clone()
        )?)),
        Command::VoiceText { call_id, seq, text } => {
            voice(room)?.text(&call_id, seq, &text)?;
            Ok(Value::Null)
        }
        Command::VoiceAudio {
            call_id,
            seq,
            index,
            data,
            r#final,
        } => {
            voice(room)?.audio(&call_id, seq, index, &data, r#final)?;
            Ok(Value::Null)
        }
        Command::VoiceUtterance {
            call_id,
            seq,
            mime_type,
            data,
            duration_ms,
        } => {
            voice(room)?.utterance(&call_id, seq, &mime_type, &data, duration_ms)?;
            Ok(Value::Null)
        }
        Command::VoiceInterrupt { call_id } => {
            voice(room)?.interrupt(&call_id)?;
            Ok(Value::Null)
        }
        Command::VoiceHold { call_id, hold } => {
            voice(room)?.hold(&call_id, hold)?;
            Ok(Value::Null)
        }
        Command::VoiceCallEnd { call_id } => {
            voice(room)?.end(&call_id)?;
            Ok(Value::Null)
        }
        Command::FilesBrowse { path } => {
            tokio::task::spawn_blocking(move || super::files::browse(&path))
                .await
                .map_err(|_| "The file browser stopped.".to_string())?
        }
        Command::FilesMkdir { path } => {
            tokio::task::spawn_blocking(move || super::files::mkdir(&path))
                .await
                .map_err(|_| "The file browser stopped.".to_string())?
        }
        Command::FilesDownload { path, offset } => {
            tokio::task::spawn_blocking(move || super::files::download(&path, offset))
                .await
                .map_err(|_| "The download stopped.".to_string())?
        }
        Command::FilesUploadStart(_)
        | Command::FilesUploadChunk { .. }
        | Command::FilesUploadFinish { .. }
        | Command::FilesUploadCancel { .. } => Err("Uploads need a live desk connection.".into()),
        Command::RemoteStatus {} => Ok(json!(remote(room)?.status())),
        Command::RemoteConfigure { enabled, host } => {
            Ok(json!(remote(room)?.configure(enabled, &host).await?))
        }
        Command::RemoteDevices {} => Ok(json!(remote(room)?.devices())),
        Command::RemoteRevoke { device_id } => Ok(json!(remote(room)?.revoke(&device_id)?)),
        Command::RemotePairing {
            role,
            id,
            cancel,
            legacy,
        } => {
            let remote = remote(room)?;
            if legacy {
                if role.is_some() || id.is_some() || cancel {
                    return Err("Legacy pairing cannot name a role or a v2 invitation.".into());
                }
                return Ok(json!(remote.pairing()?));
            }
            if let Some(id) = id {
                if role.is_some() {
                    return Err("A pairing role is chosen when the invitation is created.".into());
                }
                if cancel {
                    remote.cancel_pairing(&id)?;
                    Ok(Value::Null)
                } else {
                    Ok(json!(remote.pairing_result(&id)?))
                }
            } else if cancel {
                Err("Name the pairing invitation to cancel.".into())
            } else {
                Ok(json!(remote.pairing_v2(role.unwrap_or_default())?))
            }
        }
        // All authentication traffic needs socket ownership checked by the wire.
        Command::AgentAuthStart { .. }
        | Command::AgentAuthPoll { .. }
        | Command::AgentAuthInput { .. }
        | Command::AgentAuthCancel { .. } => {
            Err("Sign-in requires its owning desktop connection.".into())
        }
        Command::MobilePrompt { .. }
        | Command::MobileAttachment { .. }
        | Command::MobilePushRegister { .. } => Err("This command requires a paired phone.".into()),
        Command::MobilePersonaCreate {
            request_id,
            name,
            goal,
            backend_id,
            model_id,
            effort_id,
        } => {
            mobile_persona_create(
                log,
                room,
                &request_id,
                name,
                goal,
                backend_id,
                model_id,
                effort_id,
            )
            .await
        }
        Command::MobilePersonaUpdate { id, name, goal } => {
            let patch = mobile_persona_patch(name, goal)?;
            apply_persona_update(log, room, &id, &patch).await
        }
        Command::MobilePersonaAccess {
            id,
            reach,
            mode_id,
            background_work,
        } => mobile_persona_access(log, room, &id, reach, mode_id, background_work).await,
        Command::MobilePersonaComputer {
            id,
            enabled,
            memory,
            cpus,
        } => mobile_persona_computer(log, room, &id, enabled, memory, cpus).await,
        Command::PersonaCreate { draft } => create_persona(log, draft),
        Command::PersonaUpdate { id, patch } => apply_persona_update(log, room, &id, &patch).await,
        Command::PersonaDelete { id } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            delete_persona(log, room, &id)
        }
        Command::SettingsUpdate { mut patch } => {
            for (key, value) in &mut patch {
                if !value.is_null() {
                    *value = room::normalize_setting(key, value)?;
                }
            }
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
        } => set_mode(log, room, &persona_id, &mode_id)
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
        Command::TeammatesExchangeStop { a, b } => room.stop_exchange(&a, &b).map(|()| Value::Null),
        Command::TeammatesExchangeResume { a, b } => {
            room.resume_exchange(&a, &b).map(|()| Value::Null)
        }
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
        } => search::search(log.root(), &persona_id, &query, limit),
        Command::SearchAll { query, limit } => search::search_all(log.root(), &query, limit),
        Command::TapePage {
            persona_id,
            before,
            limit,
            through,
        } => {
            living(log, &persona_id)?;
            Ok(super::tape_page(
                log,
                &persona_id,
                &before,
                limit,
                through.as_deref(),
            ))
        }
        Command::FileRead {
            persona_id,
            event_id,
            index,
            offset,
        } => {
            living(log, &persona_id)?;
            crate::sent::read_message(log, &persona_id, &event_id, index.unwrap_or(0), offset)
                .map(|chunk| json!(chunk))
        }
        Command::AvatarRead {
            persona_id,
            hash,
            offset,
        } => {
            living(log, &persona_id)?;
            crate::session::avatar::read(log.root(), &persona_id, &hash, offset)
                .map(|chunk| json!(chunk))
        }
        Command::AvatarGenerate { persona_id } => {
            living(log, &persona_id)?;
            room.generate_avatar(&persona_id)
                .await
                .map(|()| Value::Null)
        }
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

        Command::ComputerCapacity {} => Ok(json!(room.computer_capacity().await)),
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
        Command::DeskLooking { looking } => {
            room.desk_looking(looking);
            Ok(Value::Null)
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
        Command::ComputerCookiesPush {
            persona_id,
            transfer,
        } => room
            .computer_cookies_push(&persona_id, transfer)
            .await
            .map(|sites| json!(sites)),
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

fn voice(room: &Arc<dyn RoomHandle>) -> Result<Arc<crate::voice::Calls>, String> {
    room.voice()
        .ok_or_else(|| "Voice is not available on this desk.".into())
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

/// The backend a draft takes when it names none: the room's chosen default,
/// or Hotline Agent itself.
fn default_backend_id(log: &Log) -> String {
    room::settings(log)
        .get("defaultBackendId")
        .and_then(Value::as_str)
        .unwrap_or(HOTLINE_BACKEND_ID)
        .to_string()
}

/// A teammate as the previous edition's `createPersona` made one: a fresh uuid,
/// the room's default backend, a workspace under the data directory, and
/// every capability its policy can give.
fn create_persona(log: &Log, draft: PersonaDraft) -> Result<Value, String> {
    build_persona(log, Uuid::new_v4().to_string(), draft)
}

/// The write behind both `persona.create` and `mobile.persona_create`: the
/// id is the caller's, a fresh uuid for the desk and a phone's own
/// `requestId` for a mobile create, so a retried create finds the teammate
/// already made instead of a second one.
fn build_persona(log: &Log, id: String, draft: PersonaDraft) -> Result<Value, String> {
    let stamped = now();
    let backend_id = given(draft.backend_id).unwrap_or_else(|| default_backend_id(log));
    // A Hotline Agent draft that leaves the model blank takes the room's
    // standing choice, so the teammate starts on what Settings named
    // rather than whichever model happens to lead the catalogue.
    let model_id = given(draft.model_id).or_else(|| {
        if backend_id != HOTLINE_BACKEND_ID {
            return None;
        }
        crate::models::preferred_model(&room::settings(log))
    });
    let persona = Persona {
        node: None,
        id: id.clone(),
        name: given(Some(draft.name)).unwrap_or_else(|| "Untitled".to_string()),
        goal: given(draft.goal).unwrap_or_default(),
        avatar: None,
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
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: stamped,
        updated_at: stamped,
    };
    room::append_persona(log, &persona)?;
    Ok(json!(persona))
}

/// `mobile.persona_create`: the phone's own narrow create. `requestId`
/// becomes the id, so a lost acknowledgement's retry finds the teammate
/// already made rather than making a second one. Everything the phone
/// cannot express — reach, cwd, computer, background work — is left at
/// [`build_persona`]'s safest default by never being named in the draft.
#[allow(clippy::too_many_arguments)]
async fn mobile_persona_create(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    request_id: &str,
    name: String,
    goal: Option<String>,
    backend_id: Option<String>,
    model_id: Option<String>,
    effort_id: Option<String>,
) -> Result<Value, String> {
    let id = Uuid::parse_str(request_id)
        .map_err(|_| "requestId must be a uuid.".to_string())?
        .to_string();
    if let Some(existing) = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
    {
        return Ok(json!(existing));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A teammate needs a name.".to_string());
    }
    let backend_id = given(backend_id).unwrap_or_else(|| default_backend_id(log));
    match room
        .backends()
        .await
        .into_iter()
        .find(|backend| backend.id == backend_id)
    {
        None => return Err(format!("There is no harness {backend_id} on this desk.")),
        Some(backend) if backend.unavailable.is_some() => {
            return Err(format!(
                "{} is not ready: {}",
                backend.name,
                backend.unavailable.unwrap_or_default()
            ));
        }
        Some(_) => {}
    }
    let draft = PersonaDraft {
        name,
        goal,
        team: None,
        backend_id: Some(backend_id),
        cwd: None,
        reach: None,
        model_id,
        effort_id,
        computer: None,
        background_work: None,
    };
    build_persona(log, id, draft)
}

/// `mobile.persona_update`'s whole patch: only `name` and `goal` can be in
/// it, because the command has nowhere to carry anything else. A name is
/// trimmed and must not be blank; a goal is trimmed and may be, which
/// clears it.
fn mobile_persona_patch(name: Option<String>, goal: Option<String>) -> Result<Value, String> {
    let mut patch = Map::new();
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() {
            return Err("A teammate needs a name.".to_string());
        }
        patch.insert("name".into(), Value::from(name));
    }
    if let Some(goal) = goal {
        patch.insert("goal".into(), Value::from(goal.trim()));
    }
    if patch.is_empty() {
        return Err("Nothing to change: name a new name or goal.".to_string());
    }
    Ok(Value::Object(patch))
}

/// `mobile.persona_access`: the owner phone's access controls. Reach is
/// Hotline Agent's and a mode a harness's, so each is refused for the other
/// kind of teammate rather than stored where nothing reads it.
async fn mobile_persona_access(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    reach: Option<Reach>,
    mode_id: Option<String>,
    background_work: Option<bool>,
) -> Result<Value, String> {
    let persona = living(log, id)?;
    let hotline = persona.backend_id == HOTLINE_BACKEND_ID;
    if reach.is_none() && mode_id.is_none() && background_work.is_none() {
        return Err("Nothing to change: name a reach, a mode or background work.".to_string());
    }
    if reach.is_some() && !hotline {
        return Err(
            "Only a Hotline Agent teammate has a reach; a harness's access is its mode."
                .to_string(),
        );
    }
    if mode_id.is_some() && hotline {
        return Err("A Hotline Agent teammate has no mode; its access is its reach.".to_string());
    }
    let mut patch = Map::new();
    if let Some(reach) = reach {
        patch.insert("reach".into(), json!(reach));
    }
    if let Some(background_work) = background_work {
        patch.insert("backgroundWork".into(), json!(background_work));
    }
    if !patch.is_empty() {
        apply_persona_update(log, room, id, &Value::Object(patch)).await?;
    }
    if let Some(mode_id) = mode_id {
        set_mode(log, room, id, &mode_id).await?;
    }
    living(log, id).map(|persona| json!(persona))
}

/// Merge only the owner's resource controls while holding the same gate as
/// desktop patches, so a concurrent image/mount/secret edit cannot be lost.
async fn mobile_persona_computer(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    enabled: Option<bool>,
    memory: Option<String>,
    cpus: Option<Option<f64>>,
) -> Result<Value, String> {
    if enabled.is_none() && memory.is_none() && cpus.is_none() {
        return Err("Nothing to change: name enabled, memory or CPUs.".into());
    }
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let persona = living(log, id)?;
    // Enabling, disabling and clearing a CPU limit do not need a probe.
    if memory.is_some() || cpus.flatten().is_some() {
        let capacity = room.computer_capacity().await;
        if let Some(Some(cpus)) = cpus {
            validate_computer_cpus(cpus, capacity.cpus)?;
        }
        if let Some(memory) = &memory {
            validate_computer_memory(memory, capacity.memory_bytes)?;
        }
    }
    let mut computer = persona.computer.unwrap_or(PersonaComputer {
        enabled: false,
        image: None,
        memory: None,
        cpus: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    if let Some(enabled) = enabled {
        computer.enabled = enabled;
    }
    if let Some(memory) = memory {
        computer.memory = Some(memory);
    }
    if let Some(cpus) = cpus {
        computer.cpus = cpus;
    }
    apply_persona_update_locked(log, room, id, &json!({ "computer": computer })).await
}

fn validate_computer_cpus(cpus: f64, capacity: u32) -> Result<(), String> {
    if !cpus.is_finite() || cpus <= 0.0 || cpus > f64::from(capacity) || (cpus * 2.0).fract() != 0.0
    {
        return Err(format!(
            "CPUs must be positive half-core increments, at most {capacity}."
        ));
    }
    Ok(())
}

/// Whole m/M (MiB) and g/G (GiB) runtime spellings only. In particular,
/// decimal/fractional values are not rounded into an unintended grant.
fn validate_computer_memory(memory: &str, capacity: u64) -> Result<(), String> {
    const STEP: u64 = 512 * 1024 * 1024;
    let invalid = || {
        "Memory must be whole MiB or GiB (such as 4608m or 4g), at least 512 MiB and in 512 MiB increments.".to_string()
    };
    let (digits, scale) = match memory.as_bytes().last() {
        Some(b'm' | b'M') => (&memory[..memory.len() - 1], 1024_u64 * 1024),
        Some(b'g' | b'G') => (&memory[..memory.len() - 1], 1024_u64 * 1024 * 1024),
        _ => return Err(invalid()),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let bytes = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(invalid)?;
    if bytes < STEP || !bytes.is_multiple_of(STEP) {
        return Err(invalid());
    }
    if bytes > capacity {
        return Err(format!(
            "Memory exceeds this computer's capacity of {} MiB.",
            capacity / (1024 * 1024)
        ));
    }
    Ok(())
}

/// Switches a harness teammate's mode and keeps it, so the teammate comes
/// back in it after a restart. A resting teammate advertises no modes, so its
/// choice is kept unchecked and offered to the harness at its next start,
/// which refuses one it does not have.
async fn set_mode(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    mode_id: &str,
) -> Result<SessionInfo, String> {
    living(log, persona_id)?;
    if mode_id.trim().is_empty() {
        return Err("A mode needs an id.".to_string());
    }
    let offered = room.info(persona_id);
    let info = if offered.modes.is_empty() {
        offered
    } else {
        if !offered.modes.iter().any(|mode| mode.id == mode_id) {
            return Err(format!("{mode_id} is not a mode this teammate offers."));
        }
        room.set_mode(persona_id, mode_id).await?
    };
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    if living(log, persona_id)?.mode_id.as_deref() != Some(mode_id) {
        update_persona(log, room, persona_id, &json!({ "modeId": mode_id }))?;
    }
    Ok(info)
}

/// `persona.update` and `mobile.persona_update` both land here, under the
/// policy gate, restarting a live session when the patch asks for it.
async fn apply_persona_update(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    apply_persona_update_locked(log, room, id, patch).await
}

/// The caller holds `policy_update_lock` for read/merge/write and effects.
async fn apply_persona_update_locked(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<Value, String> {
    let (updated, reattaches) = update_persona(log, room, id, patch)?;
    if reattaches {
        room.reattach(id).await?;
    } else if patch.get("computer").is_some() {
        room.computer_settings_changed(id).await;
    }
    Ok(updated)
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
    if patch.get("avatar").is_some_and(|avatar| !avatar.is_null()) {
        return Err("A picture can only be cleared here, with `avatar: null`.".into());
    }
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
    if updated.avatar.is_none() {
        crate::session::avatar::remove_except(log.root(), id, None);
    }
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
/// restart for the new tools to take effect. `name`, `team`, `avatar`,
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

fn remote(room: &Arc<dyn RoomHandle>) -> Result<Arc<crate::remote::Remote>, String> {
    room.remote()
        .ok_or_else(|| "Remote access is not available in this room.".into())
}
