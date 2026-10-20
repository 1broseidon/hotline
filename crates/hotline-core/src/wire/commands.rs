//! What a command does.
//!
//! The room's own memory is the wire's to write: a teammate and a setting are
//! events on the room stream, and this is the only thing that appends them.
//! Everything else — a session, a secret, the models a key can reach — is
//! asked of the [`RoomHandle`], because a running agent and a vault are not
//! things to reimplement behind a door.

pub(super) mod models;
pub(super) mod persona;
mod settings;

use super::{Parts, RoomHandle, threads};
use crate::contract::{Command, ThreadAnswer, ThreadId};
use crate::log::{Log, StreamId};
use crate::store::{chapters, search};
use crate::{paths, room};
use models::{remember_model, set_config, set_model, welcome};
use persona::{
    MobileDraft, apply_persona_update, create_persona, delete_persona, lead_persona, living,
    mobile_persona_access, mobile_persona_computer, mobile_persona_create, mobile_persona_patch,
    pin_persona, set_mode,
};
use serde_json::{Map, Value, json};
use settings::{update_settings, web_search_status};
use std::sync::Arc;

// Only the dispatcher sets this scope; wire parameters cannot claim voice origin.
tokio::task_local! { pub(crate) static VOICE_COMMAND: (); }
tokio::task_local! { pub(crate) static CALL_ORIGIN: crate::voice::Origin; }

pub(crate) fn voice_origin() -> Option<crate::voice::Origin> {
    CALL_ORIGIN.try_with(Clone::clone).ok()
}

// Set by the door a message came in at, never by the message: whether this
// socket declared `threads2`, for the commands whose answer carries lines a
// thread's link is one of.
tokio::task_local! { pub(crate) static THREADS2: bool; }

pub(crate) fn threads2() -> bool {
    THREADS2.try_with(|declared| *declared).unwrap_or(false)
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

/// Runs work that reads or writes files or a database on the blocking pool,
/// so a search over a large index or a whole tape does not stop the other
/// sockets' frames from being read.
async fn off_the_reactor<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("The work did not finish: {error}"))?
}

pub(crate) async fn run(
    command: Command,
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    parts: &Parts,
) -> Result<Value, String> {
    match command {
        Command::ImagesStatus {} => Ok(json!(room.images_status())),
        Command::WebsearchStatus {} => web_search_status(log, room),
        Command::WebsearchSetEnabled { provider, enabled } => {
            settings::set_web_search_enabled(log, room, provider, enabled).await
        }
        Command::WebsearchSetKey { provider, key } => {
            room.websearch_set_key(provider, key.as_deref())?;
            web_search_status(log, room)
        }
        Command::Ping {} => Ok(Value::Null),
        Command::LinkPreview { url } => Ok(json!(crate::link_preview::preview(&url).await)),
        Command::CapabilitiesOptions {} => room
            .capability_options()
            .await
            .map(|options| json!(options)),
        Command::VoiceStatus { input_mode } => Ok(json!(
            voice(parts)?.status_for(input_mode.unwrap_or_default())
        )),
        Command::VoiceCallStart {
            call_id,
            persona_id,
            stream_audio,
            input_mode,
            mode,
            input_rate,
        } => {
            let request = crate::voice::CallRequest {
                target: persona_id,
                stream_audio: stream_audio.unwrap_or_default(),
                input_mode: input_mode.unwrap_or_default(),
                mode: mode.unwrap_or_default(),
                mic_rate: input_rate,
            };
            Ok(json!(voice(parts)?.start_call(
                &call_id,
                request,
                room.clone()
            )?))
        }
        Command::VoiceFrontAudio { call_id, seq, data } => {
            voice(parts)?.front_audio(&call_id, seq, &data)?;
            Ok(Value::Null)
        }
        Command::VoiceFrontBarge { call_id, played_ms } => {
            voice(parts)?.front_barge(&call_id, played_ms).await?;
            Ok(Value::Null)
        }
        Command::VoiceText { call_id, seq, text } => {
            voice(parts)?.text(&call_id, seq, &text)?;
            Ok(Value::Null)
        }
        Command::VoiceAudio {
            call_id,
            seq,
            index,
            data,
            r#final,
        } => {
            voice(parts)?.audio(&call_id, seq, index, &data, r#final)?;
            Ok(Value::Null)
        }
        Command::VoiceUtterance {
            call_id,
            seq,
            mime_type,
            data,
            duration_ms,
        } => {
            voice(parts)?.utterance(&call_id, seq, &mime_type, &data, duration_ms)?;
            Ok(Value::Null)
        }
        Command::VoiceInterrupt { call_id } => {
            voice(parts)?.interrupt(&call_id)?;
            Ok(Value::Null)
        }
        Command::VoiceHold { call_id, hold } => {
            voice(parts)?.hold(&call_id, hold)?;
            Ok(Value::Null)
        }
        Command::VoiceCallEnd { call_id } => {
            voice(parts)?.end(&call_id)?;
            Ok(Value::Null)
        }
        Command::VoiceModels {} => Ok(json!(voice(parts)?.speech_models())),
        Command::VoiceModelInstall { model_id } => {
            Ok(json!(voice(parts)?.install_speech_model(&model_id)?))
        }
        Command::VoiceModelCancel { model_id } => {
            Ok(json!(voice(parts)?.cancel_speech_model(&model_id)))
        }
        Command::VoiceModelRemove { model_id } => {
            Ok(json!(voice(parts)?.remove_speech_model(&model_id)?))
        }
        Command::VoiceTranscribe { mime_type, data } => {
            Ok(json!(crate::contract::VoiceTranscript {
                text: voice(parts)?.transcribe(&mime_type, &data).await?,
            }))
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
        Command::RemoteStatus {} => Ok(json!(remote(parts)?.status())),
        Command::RemoteConfigure { enabled, host } => {
            Ok(json!(remote(parts)?.configure(enabled, &host).await?))
        }
        Command::RemoteDevices {} => Ok(json!(remote(parts)?.devices())),
        Command::RemoteRevoke { device_id } => Ok(json!(remote(parts)?.revoke(&device_id)?)),
        Command::RemoteRelay { desk_id } => Ok(json!(remote(parts)?.relay_through(desk_id)?)),
        Command::RemoteRelays {} => Ok(json!(remote(parts)?.relay_servers())),
        Command::RemotePublicUrl { url } => {
            Ok(json!(remote(parts)?.set_public_url(url.as_deref())?))
        }
        Command::RemotePairing { role, id, cancel } => remote(parts)?.pairing(role, id, cancel),
        Command::DesksLinks {} => Ok(json!(remote(parts)?.desk_links()?)),
        Command::DesksLink { desk_id } => Ok(json!(remote(parts)?.link(&desk_id).await?)),
        Command::DesksLinkAccept { desk_id, name, key } => {
            remote(parts)?.accept_link(&desk_id, &name, &key).await
        }
        Command::DesksUnlink { desk_id } => {
            remote(parts)?.unlink(&desk_id).await?;
            Ok(Value::Null)
        }
        Command::FederationRoster { .. }
        | Command::FederationDeliver { .. }
        | Command::FederationUnlink {} => Err("Only a linked desk sends that.".into()),
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
        Command::RemoteBecomeRelay {} => {
            Err("Only a paired device changes its own pairing.".into())
        }
        Command::MobilePersonaCreate {
            request_id,
            name,
            goal,
            backend_id,
            model_id,
            effort_id,
        } => {
            let asked = MobileDraft {
                name,
                goal,
                backend_id,
                model_id,
                effort_id,
            };
            mobile_persona_create(log, room, &request_id, asked).await
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
        Command::PersonaPin { id, slot } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            pin_persona(log, &id, slot)
        }
        Command::PersonaLead { id } => {
            let gate = room.policy_update_lock();
            let _held = gate.lock().await;
            lead_persona(log, id.as_deref())
        }
        Command::SettingsUpdate { patch } => settings::apply_update(log, room, patch).await,

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
        Command::SessionRetry { persona_id } => room.retry(&persona_id).await.map(|()| Value::Null),
        Command::SessionPrompt {
            persona_id,
            text,
            reply_to,
            attachments,
        } => {
            threads::prompt(
                log,
                room,
                &ThreadId::dm(persona_id),
                &text,
                reply_to,
                attachments,
            )
            .await
        }
        Command::SessionCancel { persona_id } => threads::cancel(room, &ThreadId::dm(persona_id)),
        Command::ProcessStop { persona_id, id } => {
            room.process_stop(&persona_id, &id).map(|()| Value::Null)
        }
        Command::ProcessOutput { persona_id, id } => room
            .process_output(&persona_id, &id)
            .map(|output| json!(output)),
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
        } => {
            threads::answer(
                log,
                room,
                &ThreadId::dm(persona_id),
                ThreadAnswer::Permission {
                    request_id,
                    option_id,
                },
            )
            .await
        }
        Command::TeammatesExchangeStop { a, b } => room.stop_exchange(&a, &b).map(|()| Value::Null),
        Command::TeammatesExchangeResume { a, b } => {
            room.resume_exchange(&a, &b).map(|()| Value::Null)
        }
        Command::HumanAnswer {
            persona_id,
            action_id,
            status,
            note,
        } => {
            threads::answer(
                log,
                room,
                &ThreadId::dm(persona_id),
                ThreadAnswer::Human {
                    action_id,
                    status,
                    note,
                },
            )
            .await
        }

        Command::SearchThread {
            persona_id,
            query,
            limit,
        } => {
            let root = log.root().to_path_buf();
            off_the_reactor(move || search::search(&root, &persona_id, &query, limit)).await
        }
        Command::SearchAll { query, limit } => {
            let root = log.root().to_path_buf();
            off_the_reactor(move || search::search_all(&root, &query, limit)).await
        }
        Command::TapePage {
            persona_id,
            before,
            limit,
            through,
        } => threads::page(
            log,
            &ThreadId::dm(persona_id),
            &before,
            limit,
            through.as_deref(),
        ),
        Command::FileRead {
            persona_id,
            event_id,
            index,
            offset,
            size,
        } => {
            living(log, &persona_id)?;
            if let Some(edge) = size {
                crate::sent::read_message_thumb(
                    log,
                    &persona_id,
                    &event_id,
                    index.unwrap_or(0),
                    offset,
                    edge,
                )
                .await
                .map(|chunk| json!(chunk))
            } else {
                let log = log.clone();
                off_the_reactor(move || {
                    crate::sent::read_message(
                        &log,
                        &persona_id,
                        &event_id,
                        index.unwrap_or(0),
                        offset,
                    )
                    .map(|chunk| json!(chunk))
                })
                .await
            }
        }
        Command::AvatarRead {
            persona_id,
            hash,
            offset,
            size,
        } => {
            living(log, &persona_id)?;
            match size {
                Some(edge) => {
                    crate::session::avatar::read_thumb(log.root(), &persona_id, &hash, offset, edge)
                        .await
                        .map(|chunk| json!(chunk))
                }
                None => crate::session::avatar::read(log.root(), &persona_id, &hash, offset)
                    .map(|chunk| json!(chunk)),
            }
        }
        Command::AvatarGenerate { persona_id } => {
            living(log, &persona_id)?;
            room.generate_avatar(&persona_id)
                .await
                .map(|()| Value::Null)
        }
        Command::ChapterList { persona_id } => {
            let log = log.clone();
            off_the_reactor(move || Ok(json!(chapters::list(&log, &persona_id)))).await
        }
        Command::RoomImport { from } => {
            let room = room.clone();
            off_the_reactor(move || {
                room.import(&home_expanded(&from))
                    .map(|report| json!(report))
            })
            .await
        }
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
            reminder,
        } => room
            .schedule_create(
                &persona_id,
                kind,
                when,
                every,
                &prompt,
                quiet.unwrap_or(false),
                reminder.unwrap_or(false),
            )
            .map(|job| json!(job)),
        Command::ScheduleList {} => Ok(json!(room.schedule_list())),
        Command::ScheduleCancel { id } => room.schedule_cancel(&id).map(|()| Value::Null),
        Command::ScheduleSetQuiet { id, quiet } => {
            room.schedule_set_quiet(&id, quiet).map(|()| Value::Null)
        }

        Command::SideStart { persona_id, text } => room
            .side_start(&persona_id, &text)
            .await
            .map(|summary| json!(summary)),
        Command::SidePrompt {
            side_id,
            text,
            attachments,
        } => {
            threads::prompt(
                log,
                room,
                &ThreadId::side(side_id),
                &text,
                None,
                attachments,
            )
            .await
        }
        Command::SideCancel { side_id } => threads::cancel(room, &ThreadId::side(side_id)),
        Command::SideArchive { side_id } => threads::close(room, &ThreadId::side(side_id)),
        Command::SideContinue { side_id } => room
            .side_continue(&side_id)
            .await
            .map(|summary| json!(summary)),
        Command::SideList { persona_id } => Ok(json!(room.side_list(&persona_id))),
        Command::SideAnswerPermission {
            side_id,
            request_id,
            option_id,
        } => {
            threads::answer(
                log,
                room,
                &ThreadId::side(side_id),
                ThreadAnswer::Permission {
                    request_id,
                    option_id,
                },
            )
            .await
        }

        Command::PeersList { persona_id } => Ok(json!(room.peer_threads(&persona_id))),
        Command::PeersMarkRead { key, event_ids } => {
            Ok(json!(room.mark_peer_read(&key, &event_ids)))
        }
        Command::PeersAnswerPermission {
            key,
            request_id,
            option_id,
        } => {
            threads::answer(
                log,
                room,
                &ThreadId::pair(key),
                ThreadAnswer::Permission {
                    request_id,
                    option_id,
                },
            )
            .await
        }

        Command::ClientHello { .. } => {
            Err("A hello is read at the door, on the socket it names.".into())
        }
        Command::ThreadList { persona_id } => {
            threads::list(log, room, persona_id.as_deref(), false)
        }
        Command::ThreadOpen { persona_id, text } => {
            threads::open(log, room, &persona_id, &text).await
        }
        Command::ThreadPrompt {
            thread,
            text,
            reply_to,
            attachments,
        } => threads::prompt(log, room, &thread, &text, reply_to, attachments).await,
        Command::ThreadCancel { thread } => threads::cancel(room, &thread),
        Command::ThreadPark { thread } => threads::park(room, &thread),
        Command::ThreadClose { thread } => threads::close(room, &thread),
        Command::ThreadContinue { thread } => threads::resume(log, room, &thread).await,
        Command::ThreadAnswer { thread, answer } => {
            threads::answer(log, room, &thread, answer).await
        }
        Command::ThreadPage {
            thread,
            before,
            limit,
            through,
        } => threads::page(log, &thread, &before, limit, through.as_deref()),

        Command::ComputerCapacity {} => Ok(json!(room.computer_capacity().await)),
        Command::ComputerRuntimes {} => Ok(json!(room.computer_runtimes().await)),
        Command::ComputerReleases {} => {
            Ok(serde_json::to_value(room.computer_releases()).unwrap_or(Value::Null))
        }
        Command::ComputerReleasesCheck {} => {
            Ok(serde_json::to_value(room.computer_releases_check().await).unwrap_or(Value::Null))
        }
        Command::Welcome {} => {
            let events = log.load(&StreamId::Room);
            let welcome = welcome(
                &room::settings_from_events(&events),
                room::personas(&events).len(),
                room::has_had_a_teammate(&events),
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
        Command::ComputerRestart { persona_id } => {
            living(log, &persona_id)?;
            room.computer_restart(&persona_id)
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

fn voice(parts: &Parts) -> Result<&Arc<crate::voice::Calls>, String> {
    parts
        .voice
        .as_ref()
        .ok_or_else(|| "Voice is not available on this desk.".into())
}

fn remote(parts: &Parts) -> Result<Arc<crate::remote::Remote>, String> {
    parts
        .remote
        .upgrade()
        .ok_or_else(|| "Remote access is not available in this room.".into())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
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
