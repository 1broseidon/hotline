//! What the room knows of its accounts while it runs: each agent account's
//! last status, its sign-in attempt, and each teammate's switch still to
//! apply, in memory only.
//!
//! A switch applies like any policy change, on a harness or on Hotline Agent:
//! only when the teammate has no work in flight anywhere and the account it
//! wants is not held, and then the old lease dies, that backend's checkpoints
//! go, and the session is rebuilt. Until then every thread is built on the
//! account the switch is from.

use super::{Room, lock, now_ms};
use crate::accounts::login::AccountLogins;
use crate::accounts::status::AccountStatuses;
use crate::accounts::switch::{self, PendingAccount};
use crate::accounts::{account_of, check_choice, resolve};
use crate::contract::{AccountHold, AccountStatusView, LoginAttemptView, Persona, SessionAccount};
use crate::driver::acp::StartBounds;
use crate::log::Log;
use crate::room;
use hotline_room::provider_auth::ChosenAccount;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

pub(super) fn start_account(
    root: &Path,
    log: &Log,
    persona: &Persona,
) -> Result<Option<resolve::ResolvedAccount>, String> {
    resolve::resolve(root, log, persona).map_err(|refusal| {
        let agent = crate::driver::acp::registry::name_of(root, &persona.backend_id)
            .unwrap_or_else(|| persona.backend_id.clone());
        refusal.public_sentence(&persona.name, &agent)
    })
}

pub(super) fn hotline_account(
    log: &Log,
    persona: &Persona,
) -> Result<Option<ChosenAccount>, String> {
    resolve::hotline(log, persona)
        .map_err(|refusal| refusal.public_sentence(&persona.name, crate::driver::rig::AGENT_NAME))
}

pub(super) struct Accounts {
    statuses: Arc<AccountStatuses>,
    logins: Arc<AccountLogins>,
    /// By teammate. Read by cloning out: no other lock is taken under it.
    switches: Mutex<HashMap<String, PendingAccount>>,
}

impl Accounts {
    pub(super) fn new(bounds: StartBounds) -> Self {
        let statuses = Arc::new(AccountStatuses::new(bounds));
        Self {
            logins: Arc::new(AccountLogins::new(statuses.clone())),
            statuses,
            switches: Mutex::default(),
        }
    }
}

impl Room {
    pub(crate) fn account_statuses(&self) -> &Arc<AccountStatuses> {
        &self.accounts.statuses
    }

    pub(crate) fn account_logins(&self) -> &Arc<AccountLogins> {
        &self.accounts.logins
    }

    pub(crate) fn account_live(
        &self,
        account_id: &str,
    ) -> (AccountStatusView, Option<LoginAttemptView>) {
        (
            self.account_statuses().cached(account_id),
            self.account_logins().view(account_id),
        )
    }

    pub(crate) fn accounts_refresh_stale(&self, account_ids: &[String]) {
        let root = self.log.root();
        for account_id in account_ids {
            if self.account_statuses().stale(account_id, now_ms())
                && let Ok(account) = resolve::account(root, &self.log, account_id)
            {
                self.account_statuses().refresh(root.to_path_buf(), account);
            }
        }
    }

    pub(crate) async fn accounts_status(
        &self,
        account_id: &str,
    ) -> Result<AccountStatusView, String> {
        let account = crate::accounts::resolved(&self.log, account_id)?;
        Ok(self
            .account_statuses()
            .check(self.log.root(), &account)
            .await)
    }

    pub(crate) async fn accounts_login_start(
        self: &Arc<Self>,
        account_id: &str,
        email: Option<String>,
        owner: CancellationToken,
        seat_is_desk: bool,
    ) -> Result<String, String> {
        let account = crate::accounts::resolved(&self.log, account_id)?;
        self.account_logins()
            .start(self, &account, email, owner, seat_is_desk)
            .await
    }

    /// `persona.account`: records the choice, and applies it at once when
    /// nothing holds it back. Choosing the running account cancels a switch
    /// and rebuilds nothing, except the rebuild a newer sign-in still owes.
    pub(crate) async fn assign_account(
        self: &Arc<Self>,
        persona_id: &str,
        account_id: Option<String>,
    ) -> Result<SessionAccount, String> {
        let record = self.persona(persona_id)?;
        check_choice(
            &self.log,
            &record.backend_id,
            self.choice_model(&record).as_deref(),
            account_id.as_deref(),
        )?;
        let pending = {
            let policy = self.policy_update_lock();
            let _held = policy.lock().await;
            let (previous, backend_id) = self.record_account(persona_id, account_id.as_deref())?;
            let running = lock(&self.sessions)
                .get(persona_id)
                .map(|session| session.account_id.clone());
            let mut switches = lock(&self.accounts.switches);
            let current = running.unwrap_or_else(|| {
                switches
                    .get(persona_id)
                    .map_or(previous, |pending| pending.from.clone())
            });
            let relogin = switches
                .get(persona_id)
                .is_some_and(|pending| pending.relogin);
            if account_id != current || relogin {
                let switch = PendingAccount {
                    backend_id,
                    from: current,
                    wanted: account_id.clone(),
                    relogin,
                };
                switches.insert(persona_id.to_string(), switch);
            } else {
                switches.remove(persona_id);
            }
            switches.get(persona_id).cloned()
        };
        let _ = self.info_changes.send(self.info(persona_id));
        // Never under the policy lock: the apply takes the start gate, and a
        // message holds that gate while closing a chapter takes the lock.
        if pending.is_some_and(|pending| self.account_hold(pending.wanted.as_deref()).is_none()) {
            self.apply_pending_account(persona_id).await?;
        }
        Ok(self.info(persona_id).account.unwrap_or(SessionAccount {
            running: None,
            wanted: account_id,
            held: None,
        }))
    }

    /// The account the record ran on before the choice, and its agent.
    fn record_account(
        &self,
        persona_id: &str,
        account_id: Option<&str>,
    ) -> Result<(Option<String>, String), String> {
        let _writes = room::persona_write_lock();
        let mut persona = self.persona(persona_id)?;
        check_choice(
            &self.log,
            &persona.backend_id,
            self.choice_model(&persona).as_deref(),
            account_id,
        )?;
        let previous = account_of(&self.log, &persona);
        if persona.account_id.as_deref() != account_id {
            persona.account_id = account_id.map(str::to_string);
            persona.updated_at = now_ms();
            room::append_persona(&self.log, &persona)?;
        }
        Ok((previous, persona.backend_id))
    }

    /// The model a Hotline Agent choice is checked against: the live
    /// session's, else the record's. A session that came up with no provider
    /// reports an empty one.
    fn choice_model(&self, persona: &Persona) -> Option<String> {
        lock(&self.sessions)
            .get(&persona.id)
            .and_then(|session| lock(&session.info).current_model_id.clone())
            .filter(|model_id| !model_id.is_empty())
            .or_else(|| persona.model_id.clone())
    }

    /// From the last answer recorded: a switch the last answer held stays
    /// held while that account is checked again.
    fn account_hold(&self, wanted: Option<&str>) -> Option<AccountHold> {
        let wanted = wanted?;
        switch::hold(
            &self.account_statuses().last_answer(wanted),
            self.account_logins().view(wanted).as_ref(),
        )
    }

    pub(super) fn session_account(
        &self,
        persona_id: &str,
        running: Option<String>,
    ) -> Option<SessionAccount> {
        let pending = lock(&self.accounts.switches).get(persona_id).cloned();
        let held = pending
            .as_ref()
            .and_then(|pending| self.account_hold(pending.wanted.as_deref()));
        switch::session_account(running, pending.as_ref(), held)
    }

    fn account_idle(&self, persona_id: &str) -> bool {
        let dm = lock(&self.sessions).get(persona_id).cloned();
        !dm.is_some_and(|session| lock(&session.turns).running)
            && !self.sides(persona_id).iter().any(|side| side.working)
            && self.working_calls(persona_id).is_empty()
            && self.working_jobs(persona_id).is_empty()
            && self.subagents(persona_id).is_empty()
            && !self.peers.answering_for(persona_id)
    }

    fn ready_switch(&self, persona_id: &str) -> Option<PendingAccount> {
        let pending = lock(&self.accounts.switches).get(persona_id).cloned()?;
        (self.account_hold(pending.wanted.as_deref()).is_none() && self.account_idle(persona_id))
            .then_some(pending)
    }

    /// The switch that may apply now, with the backend whose checkpoints it
    /// clears. The caller holds the start gate, so no start or new DM turn is
    /// under way. A switch for a teammate that is gone or is now on another
    /// agent is dropped.
    fn switch_to_apply(&self, persona_id: &str) -> Option<(PendingAccount, String)> {
        let pending = self.ready_switch(persona_id)?;
        let backend_id = self
            .persona(persona_id)
            .ok()
            .map(|persona| persona.backend_id)
            .filter(|backend_id| *backend_id == pending.backend_id);
        if backend_id.is_none() {
            self.switch_applied(persona_id, &pending);
        }
        backend_id.map(|backend_id| (pending, backend_id))
    }

    /// One changed while it applied stays, for its own apply to settle.
    fn switch_applied(&self, persona_id: &str, pending: &PendingAccount) {
        let mut switches = lock(&self.accounts.switches);
        if switches.get(persona_id) == Some(pending) {
            switches.remove(persona_id);
        }
    }

    async fn apply_pending_account(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let _working = self.working()?;
        let gate = self.start_gate(persona_id);
        let _held = gate.lock().await;
        // A stopped DM has no lease to revoke, and revoking one now would
        // refuse a Start already waiting at this gate.
        if !lock(&self.sessions).contains_key(persona_id) {
            return self.apply_switch_without_session(persona_id);
        }
        let Some((pending, backend_id)) = self.switch_to_apply(persona_id) else {
            return Ok(());
        };
        // Before the clear: a late turn on the old lease writes no checkpoint.
        self.invalidate(persona_id)?;
        room::clear_backend_checkpoints(&self.log, persona_id, &backend_id)?;
        self.switch_applied(persona_id, &pending);
        self.reattach_held(persona_id).await?;
        let _ = self.info_changes.send(self.info(persona_id));
        Ok(())
    }

    /// The caller holds the start gate and no DM session runs, so nothing
    /// can write a checkpoint for this teammate meanwhile.
    fn apply_switch_without_session(&self, persona_id: &str) -> Result<(), String> {
        let Some((pending, backend_id)) = self.switch_to_apply(persona_id) else {
            return Ok(());
        };
        self.drop_sides(persona_id);
        self.let_go_of_jobs(Some(persona_id));
        self.drop_peer_sessions(persona_id);
        room::clear_backend_checkpoints(&self.log, persona_id, &backend_id)?;
        self.switch_applied(persona_id, &pending);
        let _ = self.info_changes.send(self.info(persona_id));
        Ok(())
    }

    /// Called as any of the teammate's work ends. The check is repeated
    /// under the start gate, because a turn can be admitted in between.
    pub(crate) fn apply_account_when_idle(&self, persona_id: &str) {
        if self.ready_switch(persona_id).is_none() {
            return;
        }
        let Some(room) = self.this() else {
            return;
        };
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            if let Err(error) = room.apply_pending_account(&persona_id).await {
                let mut info = room.info(&persona_id);
                info.error = Some(error);
                let _ = room.info_changes.send(info);
            }
        });
    }

    /// After a confirmed sign-in: each session that started on this account
    /// before it is rebuilt once its teammate is idle, even if a switch is
    /// pending for it meanwhile.
    pub(crate) fn account_signed_in(&self, account_id: &str) {
        let login_at = room::agent_accounts(&self.log)
            .into_iter()
            .find(|account| account.id == account_id)
            .and_then(|account| account.login_at);
        let started_before: Vec<(String, String)> = lock(&self.sessions)
            .values()
            .filter(|session| {
                session.account_id.as_deref() == Some(account_id)
                    && session.account_login_at < login_at
            })
            .map(|session| (session.persona_id.clone(), session.backend_id.clone()))
            .collect();
        {
            let mut switches = lock(&self.accounts.switches);
            for (persona_id, backend_id) in started_before {
                switches
                    .entry(persona_id)
                    .and_modify(|pending| pending.relogin = true)
                    .or_insert_with(|| PendingAccount {
                        backend_id,
                        from: Some(account_id.to_string()),
                        wanted: Some(account_id.to_string()),
                        relogin: true,
                    });
            }
        }
        self.retry_switches_to(account_id);
    }

    /// Whenever a hold on this account may have ended.
    pub(crate) fn retry_switches_to(&self, account_id: &str) {
        let waiting: Vec<String> = lock(&self.accounts.switches)
            .iter()
            .filter(|(_, pending)| pending.wanted.as_deref() == Some(account_id))
            .map(|(persona_id, _)| persona_id.clone())
            .collect();
        for persona_id in waiting {
            let _ = self.info_changes.send(self.info(&persona_id));
            self.apply_account_when_idle(&persona_id);
        }
    }

    pub(super) fn on_running_account(&self, mut persona: Persona) -> Persona {
        if let Some(pending) = lock(&self.accounts.switches).get(&persona.id) {
            persona.account_id.clone_from(&pending.from);
        }
        persona
    }

    /// A start applies a switch that may apply, so a start after a stop
    /// never reopens a checkpoint the switch clears. The caller holds the
    /// start gate.
    pub(super) fn apply_account_at_start(&self, persona_id: &str) -> Result<(), String> {
        if lock(&self.accounts.switches).contains_key(persona_id) {
            return self.apply_switch_without_session(persona_id);
        }
        self.clear_lost_switch(persona_id)
    }

    /// With no switch pending, a checkpoint on this backend for an account
    /// other than the record's is one a switch lost to a restart never
    /// cleared: it never heard what was said since.
    fn clear_lost_switch(&self, persona_id: &str) -> Result<(), String> {
        let persona = self.persona(persona_id)?;
        let account_id = account_of(&self.log, &persona);
        if persona.session_checkpoints.iter().any(|checkpoint| {
            checkpoint.backend_id == persona.backend_id && checkpoint.account_id != account_id
        }) {
            room::clear_backend_checkpoints(&self.log, persona_id, &persona.backend_id)?;
        }
        Ok(())
    }

    pub(super) fn forget_account_switch(&self, persona_id: &str) {
        lock(&self.accounts.switches).remove(persona_id);
    }

    pub(super) fn settle_account_switch(&self, persona: &Persona) {
        let mut switches = lock(&self.accounts.switches);
        if switches
            .get(&persona.id)
            .is_some_and(|pending| pending.backend_id != persona.backend_id)
        {
            switches.remove(&persona.id);
        }
    }
}
