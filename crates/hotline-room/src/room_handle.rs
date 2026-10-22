//! The seam between the wire and the live half of the room.

use async_trait::async_trait;
use hotline_contract::{
    AccountStatusState, AccountStatusView, LoginAttemptView, SessionAccount, SessionInfo,
    StreamDelta,
};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

/// A room that keeps no links answers a linked desk with this.
const NOT_LINKED: &str = "This desk is not linked with that one.";

/// How far a harness sign-in has got, as `agent.auth.poll` answers it: the
/// state, the sign-in's output so far, and why it failed when it did.
#[derive(Clone, Serialize)]
pub struct AuthStatus {
    pub state: &'static str,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The live half of the room, which the wire calls but does not own: the
/// sessions and the vault.
///
/// The log is the wire's own — a teammate and a setting are events it appends
/// itself — but a session is a running agent and a credential is a secret,
/// and neither is a thing to reimplement behind a door. This is the seam:
/// exactly the methods the commands and the roster view call, nothing more.
///
/// Every method answers promptly. `prompt` starts a turn and returns; what
/// the turn produces reaches the client as tape events and ephemeral deltas,
/// not as the answer to the command. The async ones wait on something short:
/// a driver coming up, or, for `prompt`, the agent whose chapter closed being
/// replaced before it hears the message. The read loop awaits them, so a
/// socket's commands are still answered one at a time, in order. Remote-seat
/// calls can be dropped at any await when the device is revoked or Remote is
/// disabled. Implementations must release guards on drop; dropping a call does
/// not roll back already dispatched side effects. Desk-seat mutations instead
/// run to completion.
#[async_trait]
pub trait RoomHandle: Send + Sync + 'static {
    fn images_status(&self) -> hotline_contract::ImagesStatus {
        hotline_contract::ImagesStatus {
            available: false,
            unavailable: Some("This room cannot make images.".into()),
            provider: None,
            model: None,
            spending: None,
            spending_unavailable: None,
        }
    }

    /// The providers the owner can pick for each job, the choice made and what
    /// automatic would pick now.
    async fn capability_options(&self) -> Result<hotline_contract::CapabilityOptions, String> {
        Err("This room cannot list what its providers can do.".into())
    }

    async fn generate_avatar(&self, _persona_id: &str) -> Result<(), String> {
        Err("This room cannot make images.".into())
    }

    /// This desk's teammates as a linked desk is shown them: the rows
    /// `list_teammates` gives a local colleague, and nothing more.
    fn federation_roster_entries(&self) -> Vec<hotline_contract::LinkedTeammate> {
        Vec::new()
    }
    /// A linked desk published its teammates.
    fn federation_roster(
        &self,
        _desk_id: &str,
        _teammates: Vec<hotline_contract::LinkedTeammate>,
    ) -> Result<(), String> {
        Err(NOT_LINKED.into())
    }
    /// A linked desk sent a message for one of this desk's teammates, or the
    /// answer to one of theirs.
    async fn federation_deliver(
        &self,
        _desk_id: &str,
        _delivery: hotline_contract::LinkedDelivery,
    ) -> Result<Value, String> {
        Err(NOT_LINKED.into())
    }
    /// Ends a link on this desk, whichever side ended it: the record goes,
    /// the channel closes, and every message waiting on it fails with a
    /// sentence.
    async fn unlinked(&self, _desk_id: &str) -> Result<(), String> {
        Err(NOT_LINKED.into())
    }

    /// Move confidential launch configuration into the credential vault before
    /// settings or subscriptions can retain it.
    fn protect_mcp_settings(&self, value: &Value) -> Result<Value, String> {
        if value.as_array().is_none_or(Vec::is_empty) {
            return Ok(value.clone());
        }
        Err("This room has no credential vault for saving tool sources.".into())
    }
    /// A teammate's computer settings changed without turning it on or off.
    /// Nothing restarts for it: limits, mounts and a pinned image take
    /// effect when the container is next made, and a running computer is
    /// handed the secrets it is granted now.
    async fn computer_settings_changed(&self, _persona_id: &str) {}
    /// Serializes policy persistence and reattachment across client sockets.
    fn policy_update_lock(&self) -> Arc<tokio::sync::Mutex<()>>;
    async fn agent_auth_start(
        &self,
        _persona_id: &str,
        _method_id: &str,
        _owner: tokio_util::sync::CancellationToken,
    ) -> Result<String, String> {
        Err("This room does not offer agent sign-in.".into())
    }
    fn agent_auth_poll(&self, _persona_id: &str, _id: &str) -> Result<AuthStatus, String> {
        Err("That sign-in is no longer available.".into())
    }
    fn agent_auth_input(&self, _persona_id: &str, _id: &str, _input: &str) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    fn agent_auth_cancel(&self, _persona_id: &str, _id: &str) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    /// What is already known of an account's sign-in, without asking its agent.
    fn account_live(&self, _account_id: &str) -> (AccountStatusView, Option<LoginAttemptView>) {
        let status = AccountStatusView {
            state: AccountStatusState::NotChecked,
            checked_at: None,
            email: None,
            plan: None,
        };
        (status, None)
    }
    /// Asks again about each of these accounts whose last answer is old. One
    /// never asked about stays unasked until a person asks.
    fn accounts_refresh_stale(&self, _account_ids: &[String]) {}
    async fn accounts_status(&self, _account_id: &str) -> Result<AccountStatusView, String> {
        Err("This room cannot check agent accounts.".into())
    }
    async fn accounts_login_start(
        &self,
        _account_id: &str,
        _email: Option<String>,
        _owner: tokio_util::sync::CancellationToken,
        _seat_is_desk: bool,
    ) -> Result<String, String> {
        Err("This room does not offer account sign-in.".into())
    }
    fn accounts_login_poll(&self, _account_id: &str, _id: &str) -> Result<AuthStatus, String> {
        Err("That sign-in is no longer available.".into())
    }
    fn accounts_login_input(
        &self,
        _account_id: &str,
        _id: &str,
        _input: &str,
    ) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    fn accounts_login_cancel(&self, _account_id: &str, _id: &str) -> Result<(), String> {
        Err("That sign-in is no longer available.".into())
    }
    async fn persona_account(
        &self,
        _persona_id: &str,
        _account_id: Option<String>,
    ) -> Result<SessionAccount, String> {
        Err("This room cannot assign agent accounts.".into())
    }
    async fn start(&self, persona_id: &str) -> Result<SessionInfo, String>;
    fn stop(&self, persona_id: &str) -> Result<(), String>;
    /// Stops one command a teammate left running (`Room::process_stop`).
    fn process_stop(&self, _persona_id: &str, _id: &str) -> Result<(), String> {
        Err("This desk runs no processes.".into())
    }
    /// One running command and its output tail (`Room::process_output`).
    fn process_output(
        &self,
        _persona_id: &str,
        _id: &str,
    ) -> Result<hotline_contract::ProcessOutput, String> {
        Err("This desk runs no processes.".into())
    }
    /// Runs the last turn again when it failed (`Room::retry`).
    async fn retry(&self, _persona_id: &str) -> Result<(), String> {
        Err("This desk cannot retry a turn.".into())
    }
    /// Revokes existing execution before a new policy is written to the log.
    fn invalidate(&self, persona_id: &str) -> Result<(), String>;
    /// A gateway change revokes every session, including cached peer sessions.
    fn invalidate_all(&self) -> Result<(), String>;
    /// Rebuilds a live session from the teammate's current record.
    async fn reattach(&self, persona_id: &str) -> Result<(), String>;
    /// Every live session: a change to the room's servers reaches all of them.
    async fn reattach_all(&self) -> Result<(), String>;
    async fn prompt(
        &self,
        persona_id: &str,
        text: &str,
        reply_to: Option<String>,
        attachments: Option<Vec<hotline_contract::Attachment>>,
    ) -> Result<(), String>;
    fn cancel(&self, persona_id: &str) -> Result<(), String>;
    async fn set_model(&self, persona_id: &str, model_id: &str) -> Result<SessionInfo, String>;
    async fn set_mode(&self, persona_id: &str, mode_id: &str) -> Result<SessionInfo, String>;
    async fn set_config(
        &self,
        persona_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<SessionInfo, String>;

    /// The effort levels a catalogue model offers, as picker choices, and
    /// the one a teammate with none stored runs at.
    fn models_efforts(&self, model_id: &str) -> hotline_contract::EffortChoices;

    /// Answers a permission the agent is waiting behind, refusing when there
    /// is nothing left to answer.
    async fn answer_permission(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String>;

    /// Stops a pair’s queued automatic exchange.
    fn stop_exchange(&self, a: &str, b: &str) -> Result<(), String>;

    /// Resumes a pair its message cap paused.
    fn resume_exchange(&self, a: &str, b: &str) -> Result<(), String>;

    /// Answers a `request_human` card, refusing when nothing is waiting.
    fn answer_human(
        &self,
        persona_id: &str,
        action_id: &str,
        status: hotline_contract::HumanAnswer,
        note: Option<String>,
    ) -> Result<(), String>;

    /// Closes the teammate's open chapter, answering with what it became.
    async fn start_fresh_chapter(
        &self,
        persona_id: &str,
    ) -> Result<hotline_contract::ChapterSummary, String>;

    /// Reopens the previous chapter's context in place of the current one.
    async fn resume_chapter(
        &self,
        persona_id: &str,
    ) -> Result<hotline_contract::ChapterSummary, String>;

    /// What this teammate's session is doing, idle when it has none.
    fn info(&self, persona_id: &str) -> SessionInfo;

    /// Every session's info as it changes, for the roster view.
    fn subscribe_info(&self) -> broadcast::Receiver<SessionInfo>;

    /// Whether this teammate's picture is being drawn, for the roster row.
    fn drawing(&self, _persona_id: &str) -> bool {
        false
    }

    /// The subagents this teammate has running, for the roster row.
    fn subagents(&self, _persona_id: &str) -> Vec<hotline_contract::RunningSubagent> {
        Vec::new()
    }

    /// Whether a work thread or pair has a turn in flight; no history is needed.
    fn thread_working(&self, _thread: &hotline_contract::ThreadId) -> bool {
        false
    }

    fn sides(&self, _persona_id: &str) -> Vec<hotline_contract::RunningSide> {
        Vec::new()
    }

    /// The jobs of this teammate with a turn running, by id, for the roster
    /// row.
    fn working_jobs(&self, _persona_id: &str) -> Vec<String> {
        Vec::new()
    }

    /// Call threads with a turn in flight, by id, for the roster row.
    fn working_calls(&self, _persona_id: &str) -> Vec<String> {
        Vec::new()
    }

    /// Whether one of this teammate's threads other than its DM has a card
    /// waiting on the person, for the roster row.
    fn threads_waiting(&self, _persona_id: &str) -> bool {
        false
    }

    /// Starts a side thread: a second conversation with this teammate.
    async fn side_start(
        &self,
        _persona_id: &str,
        _text: &str,
    ) -> Result<hotline_contract::SideThreadSummary, String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    /// Says something in a live side thread, in answer to `reply_to` when it
    /// names one of the thread's lines.
    async fn side_prompt(
        &self,
        _side_id: &str,
        _text: &str,
        _reply_to: Option<String>,
        _attachments: Option<Vec<hotline_contract::Attachment>>,
    ) -> Result<(), String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    fn side_cancel(&self, _side_id: &str) -> Result<(), String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    fn side_archive(&self, _side_id: &str) -> Result<(), String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    /// Says something in a job's thread: a turn on the job's agent.
    async fn job_prompt(
        &self,
        _job_id: &str,
        _text: &str,
        _attachments: Option<Vec<hotline_contract::Attachment>>,
    ) -> Result<(), String> {
        Err("Jobs are unavailable on this room.".to_string())
    }

    /// Stops the turn in flight in a job's thread; the job goes on firing.
    fn job_cancel(&self, _job_id: &str) -> Result<(), String> {
        Err("Jobs are unavailable on this room.".to_string())
    }

    /// Answers a permission card raised inside a job's thread.
    async fn job_answer_permission(
        &self,
        _job_id: &str,
        _request_id: &str,
        _option_id: &str,
    ) -> Result<(), String> {
        Err("Jobs are unavailable on this room.".to_string())
    }

    /// Lets go of a side thread's agent and keeps the thread open.
    fn side_park(&self, _side_id: &str) -> Result<(), String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    /// Brings a parked or archived side thread back.
    async fn side_continue(
        &self,
        _side_id: &str,
    ) -> Result<hotline_contract::SideThreadSummary, String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    fn side_list(&self, _persona_id: &str) -> Vec<hotline_contract::SideThreadSummary> {
        Vec::new()
    }

    async fn side_answer_permission(
        &self,
        _side_id: &str,
        _request_id: &str,
        _option_id: &str,
    ) -> Result<(), String> {
        Err("Side threads are unavailable on this room.".to_string())
    }

    async fn peers_answer_permission(
        &self,
        _key: &str,
        _request_id: &str,
        _option_id: &str,
    ) -> Result<(), String> {
        Err("Peer threads are unavailable on this room.".to_string())
    }

    /// Text as an agent writes it, for a tape subscription to forward. Never
    /// written to a tape: the durable line lands when the message is whole.
    fn subscribe_deltas(&self) -> broadcast::Receiver<StreamDelta>;

    fn credential_create(
        &self,
        provider_id: &str,
        label: &str,
        secret: &str,
    ) -> Result<hotline_contract::Credential, String>;
    fn credential_revoke(&self, id: &str) -> Result<(), String>;
    async fn credential_delete(&self, id: &str) -> Result<(), String>;
    /// Starts provider sign-in. Returns the prompt as soon as the provider
    /// issues a code; the login keeps running until the person signs in.
    async fn credential_login(
        &self,
        provider_id: &str,
    ) -> Result<hotline_contract::LoginPrompt, String>;
    fn credential_login_cancel(&self, _login_id: &str) -> Result<(), String> {
        Err("Login cancellation is unavailable.".into())
    }
    async fn credential_connect_local(
        &self,
        _base_url: &str,
    ) -> Result<hotline_contract::Credential, String> {
        Err("Local providers are unavailable.".into())
    }
    fn credential_custom_save(
        &self,
        _id: Option<&str>,
        _draft: hotline_contract::CustomProviderDraft,
    ) -> Result<hotline_contract::Credential, String> {
        Err("Custom connections are unavailable.".into())
    }
    async fn credential_custom_models(
        &self,
        _id: Option<&str>,
        _base_url: &str,
        _secret: Option<&str>,
    ) -> Result<Vec<String>, String> {
        Err("Custom model discovery is unavailable.".into())
    }
    /// How far a login started by [`Self::credential_login`] has got.
    fn login_status(&self, login_id: &str) -> Result<hotline_contract::LoginStatus, String>;
    /// What the room knows of every credential, the secrets left in the vault.
    fn credentials(&self) -> Vec<hotline_contract::Credential>;

    /// Every harness a teammate could run on here, the built-in one first.
    async fn backends(&self) -> Vec<hotline_contract::BackendChoice>;

    /// The skills catalog: built-ins, then the gateway folder and the
    /// person's own, then — for a named teammate — what is in its own
    /// workspace.
    fn skills(&self, persona_id: Option<&str>)
    -> Result<Vec<hotline_contract::SkillEntry>, String>;

    /// Every model the desk's keys can reach, for the model picker.
    fn models(&self) -> Vec<hotline_contract::ConfigChoice>;

    /// Every model the catalogue lists for this provider, flagged by the
    /// saved filter. An unwired provider is an error; a credential is not
    /// required, because a filter is about the catalogue.
    fn models_catalog(
        &self,
        provider_id: &str,
    ) -> Result<Vec<hotline_contract::CatalogModel>, String>;

    /// Copies an existing Hotline data directory into this room. The source is
    /// never written.
    fn import(&self, from: &std::path::Path) -> Result<hotline_contract::Report, String>;

    /// What tools this teammate was given the last time it started.
    fn teammate_tools(&self, persona_id: &str) -> Option<hotline_contract::TeammateToolLedger>;

    /// Writes a job onto the room stream and wakes the clock. Times are
    /// milliseconds; `when` is a one-shot's fire, `every` a loop's interval.
    /// A reminder posts its prompt when it fires and does no work.
    #[expect(
        clippy::too_many_arguments,
        reason = "one argument for each field of the wire's schedule.create"
    )]
    fn schedule_create(
        &self,
        persona_id: &str,
        kind: hotline_contract::ScheduleKind,
        when: Option<i64>,
        every: Option<i64>,
        prompt: &str,
        quiet: bool,
        reminder: bool,
    ) -> Result<hotline_contract::ScheduledJob, String>;

    fn schedule_list(&self) -> Vec<hotline_contract::ScheduledJob>;

    fn schedule_cancel(&self, id: &str) -> Result<(), String>;

    fn schedule_set_quiet(&self, id: &str, quiet: bool) -> Result<(), String>;

    /// Every thread this teammate has with another teammate, newest first.
    /// The events of one are a `{"thread": key}` subscription, which is a
    /// stream like any other.
    fn peer_threads(&self, persona_id: &str) -> Vec<hotline_contract::PeerThreadSummary>;

    /// Marks messages in a peer thread read, answering how many moved.
    fn mark_peer_read(&self, key: &str, event_ids: &[String]) -> usize;

    /// Whether the person is at the desk's window, which keeps the phone
    /// quiet while they are.
    fn desk_looking(&self, _looking: bool) {}

    /// Discovers models through the connection's native Rig client and answers
    /// with the provider's catalogue as [`Self::models_catalog`] would.
    async fn credential_refresh_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<hotline_contract::CatalogModel>, String>;

    fn models_manual_set(
        &self,
        _provider_id: &str,
        _model_ids: &[String],
    ) -> Result<Vec<hotline_contract::CatalogModel>, String> {
        Err("Manual model IDs are unavailable on this room.".into())
    }

    /// A teammate is gone: its agent is stopped, every peer session it was a
    /// side of is dropped, and nothing is kept for its id.
    fn forget(&self, persona_id: &str);

    /// Test harnesses without a runtime return conservative, explicit defaults.
    async fn computer_capacity(&self) -> hotline_contract::ComputerCapacity {
        hotline_contract::ComputerCapacity {
            runtime: None,
            cpus: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            source: hotline_contract::ComputerCapacitySource::Default,
        }
    }
    async fn computer_runtimes(&self) -> Vec<hotline_contract::RuntimeReport>;
    fn computer_releases(&self) -> hotline_contract::ComputerReleases;
    async fn computer_releases_check(&self) -> hotline_contract::ComputerReleases;
    async fn computer_status(
        &self,
        persona_id: &str,
    ) -> Result<hotline_contract::ComputerStatus, String>;
    async fn computer_stop(&self, persona_id: &str) -> Result<(), String>;
    async fn computer_remove(&self, persona_id: &str) -> Result<(), String>;
    async fn computer_update(&self, persona_id: &str) -> Result<(), String>;
    async fn computer_restart(&self, persona_id: &str) -> Result<(), String>;

    /// The host browsers the operator could bring cookies from. Names only.
    /// Defaults to none: a room without a desk under it offers no host.
    async fn computer_browsers(&self) -> Vec<hotline_contract::HostBrowser> {
        Vec::new()
    }
    /// The sites in one host browser profile and how many cookies each has.
    /// Domains and counts only; no value is read out.
    async fn computer_cookies_preview(
        &self,
        _browser_id: &str,
        _profile_id: &str,
    ) -> Result<Vec<hotline_contract::CookieSite>, String> {
        Err("Browser cookie import is unavailable on this room.".to_string())
    }
    /// Copies the ticked sites' cookies from a host browser into the
    /// teammate's computer, and answers with the sites actually imported.
    async fn computer_cookies_import(
        &self,
        _persona_id: &str,
        _browser_id: &str,
        _profile_id: &str,
        _domains: &[String],
    ) -> Result<Vec<hotline_contract::CookieSite>, String> {
        Err("Browser cookie import is unavailable on this room.".to_string())
    }
    async fn computer_cookies_push(
        &self,
        _persona_id: &str,
        _transfer: hotline_contract::CookieTransfer,
    ) -> Result<Vec<hotline_contract::CookieSite>, String> {
        Err("Cookie transfer is unavailable on this room.".into())
    }
    /// What has been brought over to a teammate's computer, by browser and
    /// profile, with the sites.
    async fn computer_cookies_list(
        &self,
        _persona_id: &str,
    ) -> Result<Vec<hotline_contract::CookieImport>, String> {
        Err("Browser cookie import is unavailable on this room.".to_string())
    }
    /// Takes brought-over cookies back out: one site, or the whole import.
    /// Answers the record as it stands afterwards.
    async fn computer_cookies_forget(
        &self,
        _persona_id: &str,
        _browser_id: &str,
        _profile_id: &str,
        _domain: Option<&str>,
    ) -> Result<Vec<hotline_contract::CookieImport>, String> {
        Err("Browser cookie import is unavailable on this room.".to_string())
    }

    /// The web search providers that have a key saved. Never a key.
    fn websearch_keyed(&self) -> std::collections::HashSet<hotline_contract::WebSearchProvider> {
        std::collections::HashSet::new()
    }
    /// Saves a web search provider's key in the vault, or forgets it.
    fn websearch_set_key(
        &self,
        _provider: hotline_contract::WebSearchProvider,
        _key: Option<&str>,
    ) -> Result<(), String> {
        Err("Web search keys are unavailable on this room.".to_string())
    }

    /// The secrets the operator keeps for teammates: names and when each
    /// changed, never a value. Defaults to none: a room without a vault
    /// under it stores nothing.
    fn secrets_list(&self) -> Result<Vec<hotline_contract::SharedSecret>, String> {
        Ok(Vec::new())
    }
    /// Stores or replaces a secret and hands the new value to every running
    /// computer granted its name. Answers the record, never the value.
    async fn secrets_set(
        &self,
        _name: &str,
        _value: &str,
    ) -> Result<hotline_contract::SharedSecret, String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Takes a secret away, and out of every running computer it was
    /// granted to.
    async fn secrets_delete(&self, _name: &str) -> Result<(), String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Stores or replaces a login: sites, username, password, and a TOTP
    /// seed when there is one. Answers the record, never a value.
    async fn secrets_login_set(
        &self,
        _name: &str,
        _sites: &[String],
        _username: &str,
        _password: &str,
        _totp: Option<&str>,
    ) -> Result<hotline_contract::SharedSecret, String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Arms a teammate's computer to make one passkey, to be stored under
    /// `name` and ticked for that teammate.
    async fn secrets_passkey_register(
        &self,
        _name: &str,
        _persona_id: &str,
        _rp_id: &str,
    ) -> Result<hotline_contract::PasskeyRegistration, String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Where the making of that passkey stands; the poll that finds it made
    /// stores it and answers `stored`.
    async fn secrets_passkey_registration(
        &self,
        _persona_id: &str,
    ) -> Result<hotline_contract::PasskeyRegistration, String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Answers the passkey card on a teammate's tape: approved, the
    /// browser makes it and the room stores it; denied, the arming ends.
    async fn secrets_passkey_answer(
        &self,
        _persona_id: &str,
        _ask_id: &str,
        _approved: bool,
    ) -> Result<hotline_contract::PasskeyRegistration, String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }
    /// Ends an arming without a passkey.
    async fn secrets_passkey_cancel(&self, _persona_id: &str) -> Result<(), String> {
        Err("Stored secrets are unavailable on this room.".to_string())
    }

    /// Starts OAuth discovery and a native browser callback for one HTTP MCP
    /// server. The result contains only a login id, URL and status.
    async fn mcp_auth_start(&self, _server_id: &str) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Delivers an OAuth callback to a pending native login. The callback URL
    /// is validated against the listener that created it.
    async fn mcp_auth_callback(
        &self,
        _login_id: &str,
        _callback_url: &str,
    ) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Answers a secret-free OAuth status snapshot.
    async fn mcp_auth_status(&self, _server_id: &str) -> Result<Value, String> {
        Err("MCP OAuth sign-in is unavailable on this room.".to_string())
    }

    /// Clears protected MCP OAuth registrations and tokens.
    fn mcp_secret_set(&self, _server_id: &str, _url: &str, _secret: &str) -> Result<(), String> {
        Err("This room keeps no MCP credentials.".to_string())
    }

    async fn mcp_auth_sign_out(&self, _server_id: &str) -> Result<(), String> {
        Err("MCP OAuth sign-out is unavailable on this room.".to_string())
    }
}
