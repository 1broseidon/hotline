//! A credential card binds operator input to one origin and one live lease.
//! Values exist only in the answer handler, the vault and computer delivery.
use super::*;
use crate::contract::{
    CaptureKind, CredentialAnswer, CredentialStatus, SharedSecretKind, ThreadKind,
};

const LIFETIME: i64 = 30 * 60_000;
const GONE: &str = "That credential card is no longer waiting for an answer.";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRequest {
    site: String,
    reason: String,
    name: Option<String>,
    kind: CaptureKind,
}

pub(super) struct Capture {
    persona: String,
    home: ThreadId,
    capability: CapabilityLease,
    id: String,
    site: String,
    reason: String,
    name: String,
    kind: CaptureKind,
    expires_at: i64,
    existing: Vec<String>,
    armed: bool,
    status: CredentialStatus,
}

impl Capture {
    pub(super) fn check_passkey(&self, name: &str) -> Result<(), String> {
        self.capability.check()?;
        if self.status != CredentialStatus::Pending
            || !self.armed
            || self.name != name
            || self.expires_at <= now_ms()
        {
            return Err(GONE.into());
        }
        Ok(())
    }

    fn event(&self, stored_as: Option<String>) -> TranscriptEvent {
        TranscriptEvent::CredentialAsk {
            from: None,
            id: self.id.clone(),
            action_id: self.id.clone(),
            ts: now_ms(),
            site: self.site.clone(),
            reason: self.reason.clone(),
            credential_kind: self.kind,
            name: self.name.clone(),
            status: self.status,
            stored_as,
            expires_at: self.expires_at,
            existing: self.existing.clone(),
            armed: self.armed,
        }
    }
}

impl Room {
    pub(crate) fn request_credential(
        self: &Arc<Self>,
        persona_id: &str,
        home: &ThreadId,
        capability: CapabilityLease,
        request: CredentialRequest,
    ) -> Result<String, String> {
        capability.check()?;
        let persona = self.persona(persona_id)?;
        if !persona.computer.as_ref().is_some_and(|c| c.enabled) {
            return Err("A credential needs this teammate's enabled computer.".into());
        }
        if matches!(home.kind, ThreadKind::Run | ThreadKind::Pair) {
            return Err("Ask for a credential in your own chat or work thread.".into());
        }
        if home.kind == ThreadKind::Job {
            let job = room::all_jobs(&self.log)
                .into_iter()
                .find(|job| job.id == home.key && job.persona_id == persona_id)
                .ok_or("This job no longer has a computer grant.")?;
            if job
                .without
                .as_ref()
                .is_some_and(|without| without.contains(&crate::contract::Ability::Computer))
            {
                return Err("This job was started without the computer.".into());
            }
        }
        let site = crate::vault::check_site(&request.site)
            .map_err(|_| "Use a full HTTPS origin, such as https://github.com.".to_string())?;
        let host = url::Url::parse(&site)
            .map_err(|_| "Invalid origin.")?
            .host_str()
            .ok_or("Invalid origin.")?
            .to_string();
        if request.kind == CaptureKind::Passkey {
            crate::vault::check_rp_id(&host).map_err(|_| "This origin cannot make a passkey.")?;
        }
        let reason = request.reason.trim();
        if !(3..=500).contains(&reason.chars().count()) {
            return Err("A credential request needs a reason of 3 to 500 characters.".into());
        }
        let name = request.name.unwrap_or_else(|| {
            let host: String = host
                .chars()
                .take(40)
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .collect();
            format!(
                "KEY_{host}_{}",
                if request.kind == CaptureKind::Login {
                    "LOGIN"
                } else {
                    "PASSKEY"
                }
            )
        });
        crate::vault::check_secret_name(&name).map_err(|e| e.to_string())?;
        let vault = self
            .vault
            .as_ref()
            .ok_or("Stored secrets are unavailable on this room.")?;
        let existing = vault
            .shared_secrets()
            .map_err(|_| "The vault could not be listed.")?
            .into_iter()
            .filter(|secret| matches_site(secret, &site, request.kind))
            .map(|secret| secret.name)
            .collect();
        {
            let mut posting = lock(&self.posting);
            let turn = posting
                .get_mut(home)
                .ok_or("A credential request needs a turn in progress.")?;
            if !turn.credential_sites.insert(site.clone()) {
                return Err(format!(
                    "Already asked for {site} in this turn. If the person declined, stop: do not ask again."
                ));
            }
        }
        let id = format!("credential:{}", new_id());
        let capture = Capture {
            persona: persona_id.into(),
            home: home.clone(),
            capability: capability.scoped(),
            id: id.clone(),
            site,
            reason: reason.into(),
            name,
            kind: request.kind,
            expires_at: now_ms() + LIFETIME,
            existing,
            armed: false,
            status: CredentialStatus::Pending,
        };
        let event = capture.event(None);
        lock(&self.captures).insert(id.clone(), Arc::new(tokio::sync::Mutex::new(capture)));
        self.write_home(persona_id, home, &event);
        self.push.notify_in(
            Some(&if home.kind == ThreadKind::Call {
                ThreadId::dm(persona_id)
            } else {
                home.clone()
            }),
            &self.needs_you(persona_id),
            "A credential card waits on the desk and phone.",
            persona_id,
            Some(crate::push::Waiting::Credential {
                action_id: id.clone(),
            }),
        );
        watch(Arc::downgrade(self), id.clone());
        Ok(format!(
            "Credential card {id} waits on the desk and phone for 30 minutes. Never ask for or repeat values in chat or voice. Its outcome will arrive here; continue only work that does not need it. Use withdraw_request with this id if it is no longer needed."
        ))
    }

    /// Both seats enter here. The request lock covers validation, the vault
    /// write and settlement, so a losing seat can never store a second value.
    pub(crate) async fn answer_credential(
        self: &Arc<Self>,
        answer: CredentialAnswer,
    ) -> Result<(), String> {
        // A phone disconnect cancels its wire future. Once accepted, this
        // command must still settle the card after storing and delivering it.
        let room = self.clone();
        tokio::spawn(async move { room.capture_answer(answer).await })
            .await
            .map_err(|_| "The credential answer could not finish.".to_string())?
    }

    async fn capture_answer(self: &Arc<Self>, answer: CredentialAnswer) -> Result<(), String> {
        let capture = lock(&self.captures)
            .get(&answer.action_id)
            .cloned()
            .ok_or(GONE)?;
        let mut capture = capture.lock().await;
        if capture.status != CredentialStatus::Pending || capture.armed {
            return Err(GONE.into());
        }
        if capture.expires_at <= now_ms() || !capture.capability.is_current() {
            self.settle_capture(&mut capture, CredentialStatus::Expired, None);
            return Err(GONE.into());
        }
        let Some(name) = answer.name else {
            if answer.values.is_some() {
                return Err("A decline must carry no values.".into());
            }
            self.settle_capture(&mut capture, CredentialStatus::Declined, None);
            return Ok(());
        };
        crate::vault::check_secret_name(&name).map_err(|e| e.to_string())?;
        let vault = self
            .vault
            .as_ref()
            .ok_or("Stored secrets are unavailable.")?;
        let listed = vault
            .shared_secrets()
            .map_err(|_| "The vault could not be listed.")?;
        let existing = listed.iter().find(|secret| secret.name == name);
        if let Some(secret) = existing {
            if answer.values.is_some()
                || !capture.existing.contains(&name)
                || !matches_site(secret, &capture.site, capture.kind)
            {
                return Err(
                    "That name is already stored. Grant a matching key or choose a new name."
                        .into(),
                );
            }
        } else {
            match (capture.kind, answer.values) {
                (CaptureKind::Login, Some(values)) => {
                    // The person's grant is serialized with policy changes;
                    // a revoked lease cannot be used to grant a different generation.
                    let updates = self.policy_update_lock();
                    let held = updates.lock().await;
                    capture.capability.check()?;
                    if vault
                        .shared_secrets()
                        .map_err(|_| "The vault could not be listed.")?
                        .iter()
                        .any(|secret| secret.name == name)
                    {
                        return Err("That name was just stored. Choose another name.".into());
                    }
                    let login = crate::vault::StoredSecret::Login {
                        sites: vec![capture.site.clone()],
                        username: values.username,
                        password: values.password,
                        totp: values.totp,
                        ask: answer.ask,
                    };
                    vault.set_shared(&name, login).map_err(
                        |_| "The login could not be stored. Check the fields and unlock the vault.",
                    )?;
                    self.grant_captured_secret(&capture.persona, &name)?;
                    drop(held);
                    self.secrets_changed().await;
                    self.settle_capture(&mut capture, CredentialStatus::Stored, Some(name));
                    return Ok(());
                }
                (CaptureKind::Passkey, None) => {
                    capture.capability.check()?;
                    let rp_id = url::Url::parse(&capture.site)
                        .map_err(|_| "Invalid origin.")?
                        .host_str()
                        .ok_or("Invalid origin.")?
                        .to_string();
                    let arming = self
                        .arm_captured_passkey(
                            &name,
                            &capture.persona,
                            &rp_id,
                            super::secrets::PasskeyCapture {
                                action: capture.id.clone(),
                                origin: capture.site.clone(),
                                ask: answer.ask,
                                capability: capture.capability.clone(),
                                home: capture.home.clone(),
                            },
                        )
                        .await?;
                    capture.name = name;
                    if let Some(expires_at) = arming.expires_at {
                        capture.expires_at = capture.expires_at.min(expires_at);
                    }
                    capture.armed = true;
                    self.write_home(&capture.persona, &capture.home, &capture.event(None));
                    // Arming is not completion: the agent must be able to navigate
                    // the site to trigger navigator.credentials.create().
                    self.deliver_capture(&capture, format!("Ready to make a passkey named {} for {}. Choose Add a passkey on that site; the person must approve its separate request on the desk or phone.", capture.name, capture.site));
                    return Ok(());
                }
                _ => return Err("A login needs values; a passkey must carry no values.".into()),
            }
        }
        let updates = self.policy_update_lock();
        let held = updates.lock().await;
        capture.capability.check()?;
        if !vault
            .shared_secrets()
            .map_err(|_| "The vault could not be listed.")?
            .iter()
            .any(|secret| secret.name == name && matches_site(secret, &capture.site, capture.kind))
        {
            return Err("That key changed before it could be granted. Ask for a new card.".into());
        }
        self.grant_captured_secret(&capture.persona, &name)?;
        drop(held);
        self.secrets_changed().await;
        self.settle_capture(&mut capture, CredentialStatus::Stored, Some(name));
        Ok(())
    }

    pub(crate) async fn expire_credential(self: &Arc<Self>, id: &str, now: i64) -> bool {
        let capture = lock(&self.captures).get(id).cloned();
        let Some(capture) = capture else {
            return false;
        };
        let mut capture = capture.lock().await;
        if capture.status != CredentialStatus::Pending {
            return false;
        }
        let arming_gone = capture.armed
            && lock(&self.passkey_armings)
                .get(&capture.persona)
                .is_none_or(|arming| {
                    arming.expires_at <= now
                        || arming
                            .capture
                            .as_ref()
                            .is_none_or(|source| source.action != capture.id)
                });
        if capture.expires_at <= now || !capture.capability.is_current() || arming_gone {
            if capture.armed {
                let _ = self.secrets_passkey_cancel(&capture.persona).await;
            }
            self.settle_capture(&mut capture, CredentialStatus::Expired, None);
            return false;
        }
        true
    }

    /// Saved cards cannot recreate a session lease after a restart.
    pub(super) fn expire_orphaned_credentials(self: &Arc<Self>) {
        for persona in room::roster(&self.log) {
            for (mut event, home) in self.cards_of(&persona.id) {
                if event["kind"] != "credential_ask" || event["status"] != "pending" {
                    continue;
                }
                let Some(id) = event["actionId"].as_str().map(str::to_owned) else {
                    continue;
                };
                if lock(&self.captures).contains_key(&id) {
                    continue;
                }
                event["status"] = json!("expired");
                event["ts"] = json!(now_ms());
                if home.kind == ThreadKind::Dm {
                    self.write_value(&persona.id, &event);
                } else {
                    self.threads().write(&home, &persona.id, &event);
                }
            }
        }
    }

    fn grant_captured_secret(&self, persona_id: &str, name: &str) -> Result<(), String> {
        let mut persona = self.persona(persona_id)?;
        let computer = persona
            .computer
            .as_mut()
            .filter(|c| c.enabled)
            .ok_or("The computer grant ended.")?;
        let granted = computer.secrets.get_or_insert_default();
        if !granted.iter().any(|n| n == name) {
            granted.push(name.into());
            granted.sort();
        }
        persona.updated_at = now_ms();
        room::append_persona(&self.log, &persona)
    }

    pub(crate) async fn withdraw_credential(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
    ) -> Result<(), String> {
        let capture = lock(&self.captures).get(id).cloned().ok_or(GONE)?;
        let mut capture = capture.lock().await;
        if capture.persona != persona || capture.status != CredentialStatus::Pending {
            return Err(GONE.into());
        }
        if capture.armed {
            self.secrets_passkey_cancel(persona).await?;
        }
        self.settle_capture(&mut capture, CredentialStatus::Withdrawn, None);
        Ok(())
    }

    pub(super) fn settle_capture(
        self: &Arc<Self>,
        capture: &mut Capture,
        status: CredentialStatus,
        name: Option<String>,
    ) {
        let notify_agent = capture.capability.is_current();
        capture.status = status;
        capture.capability.revoke();
        self.write_home(
            &capture.persona,
            &capture.home,
            &capture.event(name.clone()),
        );
        lock(&self.captures).remove(&capture.id);
        let text = match status {
            CredentialStatus::Stored => {
                let name = name.expect("a stored capture names its key");
                if capture.home.kind == ThreadKind::Call {
                    format!("Stored as {name}")
                } else if capture.kind == CaptureKind::Passkey {
                    format!("Stored as {name}: a passkey for {}. The browser signs in there with it; you never see its private key.", capture.site)
                } else {
                    format!("Stored as {name}: a login for {}. Use browser fill secret {name}.username and browser fill secret {name}.password (or {name}.code for a one-time code) on that origin only. You never see the values.", capture.site)
                }
            }
            CredentialStatus::Declined => "The person declined the credential request. Nothing was stored or granted. Stop this sign-in; do not ask again for this site in this turn.".into(),
            CredentialStatus::Expired if capture.armed => "The passkey arming expired before a passkey was made. Nothing was stored. Stop this sign-in.".into(),
            CredentialStatus::Expired => "No answer came before the credential request expired, or its session ended. Nothing was stored. Stop this sign-in.".into(),
            CredentialStatus::Withdrawn => "The credential request was withdrawn. Nothing was stored. Stop this sign-in.".into(),
            CredentialStatus::Pending => unreachable!("settlement is terminal"),
        };
        // Revoking a session must not restart it merely to report expiry.
        if notify_agent {
            self.deliver_capture(capture, text);
        }
    }

    fn deliver_capture(self: &Arc<Self>, capture: &Capture, text: String) {
        let room = self.clone();
        let status = match capture.status {
            CredentialStatus::Declined => HumanActionStatus::Dismissed,
            CredentialStatus::Expired => HumanActionStatus::Expired,
            CredentialStatus::Withdrawn => HumanActionStatus::Withdrawn,
            _ => HumanActionStatus::Done,
        };
        let persona = capture.persona.clone();
        let home = capture.home.clone();
        let id = if capture.armed && capture.status == CredentialStatus::Pending {
            format!("{}:armed", capture.id)
        } else {
            capture.id.clone()
        };
        tokio::spawn(async move {
            let _ = room
                .hand_over_answer(
                    &persona,
                    &id,
                    "Credential request",
                    status,
                    Some(text),
                    home,
                )
                .await;
        });
    }
}

fn matches_site(secret: &SharedSecret, site: &str, kind: CaptureKind) -> bool {
    match kind {
        CaptureKind::Login => {
            secret.kind == SharedSecretKind::Login
                && secret
                    .sites
                    .as_ref()
                    .is_some_and(|sites| sites.iter().any(|s| s == site))
        }
        CaptureKind::Passkey => {
            secret.kind == SharedSecretKind::Passkey
                && url::Url::parse(site).is_ok_and(|url| url.host_str() == secret.rp_id.as_deref())
        }
    }
}

fn watch(room: Weak<Room>, id: String) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(room) = room.upgrade() else {
                return;
            };
            if !room.expire_credential(&id, now_ms()).await {
                return;
            }
        }
    });
}
