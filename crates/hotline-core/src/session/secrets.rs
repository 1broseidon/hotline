//! Secrets on a teammate's computer: handing the stored ones over when it is
//! ready, and arming, watching and storing a passkey the browser mints.

use super::*;

impl Room {
    /// Hands a running computer the secrets its teammate is granted — the
    /// whole set, so what was unticked or deleted since is gone from it too
    /// — and says on the tape what could not be handed: a name no longer
    /// stored, a release too old to take any, a machine that would not
    /// answer. The values pass vault → this process → container and are
    /// written nowhere on the way.
    pub(super) async fn hand_secrets(&self, persona: &Persona, ready: &crate::computer::Ready) {
        let notice = |text: String| eprintln!("{}: {text}", persona.name);
        let granted: Vec<String> = persona
            .computer
            .as_ref()
            .and_then(|computer| computer.secrets.clone())
            .unwrap_or_default();
        let (values, missing) = match &self.vault {
            Some(vault) => match vault.shared_secret_values(&granted) {
                Ok(read) => read,
                Err(error) => {
                    notice(format!(
                        "The secrets {} was granted could not be read from the vault, so its computer has none: {error}",
                        persona.name
                    ));
                    return;
                }
            },
            None if granted.is_empty() => (std::collections::BTreeMap::new(), Vec::new()),
            None => (std::collections::BTreeMap::new(), granted.clone()),
        };
        if !missing.is_empty() {
            notice(format!(
                "{} was granted secrets that are not stored any more: {}. Store them again under Settings → Secrets, or untick them.",
                persona.name,
                missing.join(", ")
            ));
        }
        let names: Vec<&str> = values.keys().map(String::as_str).collect();
        match crate::computer::secrets::deliver(ready, &values).await {
            Ok(()) => {}
            // A release from before secrets, granted none: nothing to say.
            Err(crate::computer::secrets::Refusal::TooOld) if names.is_empty() => {}
            Err(crate::computer::secrets::Refusal::TooOld) => notice(format!(
                "{}'s computer is on a release that cannot take secrets, so {} not in its shell. Update the computer from the teammate's pane.",
                persona.name,
                if names.len() == 1 {
                    format!("{} is", names[0])
                } else {
                    format!("{} are", names.join(", "))
                }
            )),
            Err(crate::computer::secrets::Refusal::Failed(reason)) => notice(format!(
                "{}'s computer could not be handed its secrets. {reason} Stop and start the computer to try again.",
                persona.name
            )),
        }
    }

    /// What the vault lists, for the preamble to say what each granted name
    /// is. A room without a vault, or one whose vault will not answer, lists
    /// nothing, and the preamble says so per name.
    pub(super) fn stored_secrets(&self) -> Vec<SharedSecret> {
        self.vault
            .as_ref()
            .and_then(|vault| vault.shared_secrets().ok())
            .unwrap_or_default()
    }

    /// Arms `persona_id`'s computer to make one passkey for `rp_id`, to be
    /// stored under `name` and ticked for that teammate. The computer has
    /// to be up for its browser to make one; a stopped one is started, the
    /// same as opening its screen would.
    pub async fn secrets_passkey_register(
        self: &Arc<Self>,
        name: &str,
        persona_id: &str,
        rp_id: &str,
    ) -> Result<PasskeyRegistration, String> {
        self.register_passkey(name, persona_id, rp_id, None).await
    }

    pub(super) async fn arm_captured_passkey(
        self: &Arc<Self>,
        name: &str,
        persona_id: &str,
        rp_id: &str,
        capture: PasskeyCapture,
    ) -> Result<PasskeyRegistration, String> {
        self.register_passkey(name, persona_id, rp_id, Some(capture))
            .await
    }

    async fn register_passkey(
        self: &Arc<Self>,
        name: &str,
        persona_id: &str,
        rp_id: &str,
        capture: Option<PasskeyCapture>,
    ) -> Result<PasskeyRegistration, String> {
        if lock(&self.passkey_armings)
            .get(persona_id)
            .is_some_and(|arming| arming.capture.is_some() && arming.stored.is_none())
        {
            return Err("A credential card already has this computer armed.".into());
        }
        crate::vault::check_secret_name(name).map_err(|error| error.to_string())?;
        crate::vault::check_rp_id(rp_id).map_err(|error| error.to_string())?;
        if self.vault.is_none() {
            return Err("Stored secrets are unavailable on this room.".to_string());
        }
        let persona = self.persona(persona_id)?;
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Err(format!(
                "{} has no computer to make a passkey in. Turn one on in the teammate's pane first.",
                persona.name
            ));
        }
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let ready = self
            .computers
            .ensure_running(
                &persona,
                &persona.cwd,
                prefer,
                room_image.as_deref(),
                self.pull_reporter(&persona.id),
            )
            .await?;
        let expires_at = match crate::computer::passkeys::arm(&ready, rp_id).await {
            Ok(expires_at) => expires_at,
            Err(crate::computer::secrets::Refusal::TooOld) => {
                return Err(format!(
                    "{}'s computer is on a release that cannot make passkeys. Update it from the teammate's pane, then try again.",
                    persona.name
                ));
            }
            Err(crate::computer::secrets::Refusal::Failed(reason)) => {
                return Err(format!("The computer could not be armed. {reason}"));
            }
        };
        lock(&self.passkey_armings).insert(
            persona.id.clone(),
            PasskeyArming {
                capture,
                name: name.to_owned(),
                rp_id: rp_id.to_owned(),
                expires_at,
                asked: None,
                answered: false,
                operator_approved: false,
                storing: false,
                stored: None,
            },
        );
        watch_passkey_arming(Arc::downgrade(self), persona.id.clone());
        Ok(PasskeyRegistration {
            state: PasskeyRegistrationState::Armed,
            name: Some(name.to_owned()),
            rp_id: Some(rp_id.to_owned()),
            expires_at: Some(expires_at),
            ask: None,
            secret: None,
        })
    }

    /// Where the making stands, for the pane's poll: `armed` while the site
    /// has not asked, `asked` with the request while the card on the tape
    /// waits for the person, `approved` while the browser makes it,
    /// `stored` with the record once it is — answered once, whether this
    /// poll or the room's own watch found it — and `idle` when there is
    /// nothing pending.
    pub async fn secrets_passkey_registration(
        self: &Arc<Self>,
        persona_id: &str,
    ) -> Result<PasskeyRegistration, String> {
        let (answer, _) = self.look_at_passkey_arming(persona_id).await?;
        if answer.state == PasskeyRegistrationState::Stored {
            lock(&self.passkey_armings).remove(persona_id);
        }
        Ok(answer)
    }

    /// One look at a teammate's arming: where the making stands, and
    /// whether there is anything left to watch. The look that finds the
    /// site's request waiting raises the card on the teammate's tape and
    /// tells the phones; the look that finds the passkey made stores it
    /// under the name the arming was for, ticks it for the teammate, hands
    /// the computer its set — which is what keeps the credential in the
    /// browser — ends the arming, and says so on the tape; a look that
    /// arrives while that is under way answers where it stood. A request
    /// that went away with its page, an arming that ran out, and a computer
    /// that stopped each leave the card expired rather than answerable.
    async fn look_at_passkey_arming(
        self: &Arc<Self>,
        persona_id: &str,
    ) -> Result<(PasskeyRegistration, bool), String> {
        use crate::computer::passkeys::Registration;
        use crate::contract::PasskeyAsk;
        let idle = PasskeyRegistration {
            state: PasskeyRegistrationState::Idle,
            name: None,
            rp_id: None,
            expires_at: None,
            ask: None,
            secret: None,
        };
        let standing = |arming: &PasskeyArming,
                        state: PasskeyRegistrationState,
                        ask: Option<PasskeyAsk>| PasskeyRegistration {
            state,
            name: Some(arming.name.clone()),
            rp_id: Some(arming.rp_id.clone()),
            expires_at: Some(arming.expires_at),
            ask,
            secret: None,
        };
        let stored =
            |arming: &PasskeyArming, secret: crate::contract::SharedSecret| PasskeyRegistration {
                state: PasskeyRegistrationState::Stored,
                name: Some(arming.name.clone()),
                rp_id: Some(arming.rp_id.clone()),
                expires_at: None,
                ask: None,
                secret: Some(secret),
            };
        let Some(arming) = lock(&self.passkey_armings).get(persona_id).cloned() else {
            return Ok((idle, false));
        };
        if let Some(secret) = arming.stored.clone() {
            return Ok((stored(&arming, secret), false));
        }
        if arming.capture.is_some() && arming.expires_at <= now_ms() {
            self.secrets_passkey_cancel(persona_id).await?;
            return Ok((idle, false));
        }
        if arming.storing {
            let state = if arming.asked.is_some() {
                PasskeyRegistrationState::Approved
            } else {
                PasskeyRegistrationState::Armed
            };
            return Ok((standing(&arming, state, arming.asked.clone()), true));
        }
        let Some(ready) = self.computers.running(persona_id).await else {
            self.drop_passkey_arming(persona_id);
            return Err("The computer stopped before a passkey was made. Start again.".to_string());
        };
        let polled = match crate::computer::passkeys::poll(&ready).await {
            Ok(polled) => polled,
            Err(crate::computer::secrets::Refusal::TooOld) => {
                self.drop_passkey_arming(persona_id);
                return Err("The computer is on a release that cannot make passkeys.".to_string());
            }
            Err(crate::computer::secrets::Refusal::Failed(reason)) => {
                return Err(format!("The computer could not be asked. {reason}"));
            }
        };
        if let Some(capture) = &arming.capture {
            let valid = capture.capability.is_current()
                && match &polled {
                    Registration::Asked { ask, .. } => {
                        ask.origin == capture.origin && ask.rp_id == arming.rp_id
                    }
                    Registration::Approved { ask, .. } => {
                        arming.operator_approved
                            && arming
                                .asked
                                .as_ref()
                                .is_some_and(|known| known.id == ask.id)
                            && ask.origin == capture.origin
                            && ask.rp_id == arming.rp_id
                    }
                    Registration::Registered {
                        credential, ask, ..
                    } => {
                        arming.operator_approved
                            && ask.as_ref().is_some_and(|ask| {
                                arming
                                    .asked
                                    .as_ref()
                                    .is_some_and(|known| known.id == ask.id)
                                    && ask.origin == capture.origin
                                    && ask.rp_id == arming.rp_id
                            })
                            && matches!(credential, crate::vault::StoredSecret::Passkey { rp_id, .. } if *rp_id == arming.rp_id)
                    }
                    _ => true,
                };
            if !valid {
                self.secrets_passkey_cancel(persona_id).await?;
                return Err("The captured passkey requires its exact origin, a live session and the person's approval.".into());
            }
        }
        match polled {
            Registration::Idle => {
                self.drop_passkey_arming(persona_id);
                Ok((idle, false))
            }
            Registration::Armed { .. } => {
                // The request the card was raised for is gone: its page went
                // away before anyone answered.
                if arming.asked.is_some() {
                    self.expire_passkey_card(persona_id, &arming);
                    if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
                        live.asked = None;
                        live.answered = false;
                        live.operator_approved = false;
                    }
                }
                Ok((
                    standing(&arming, PasskeyRegistrationState::Armed, None),
                    true,
                ))
            }
            Registration::Asked { ask, .. } => {
                let known = arming
                    .asked
                    .as_ref()
                    .is_some_and(|known| known.id == ask.id);
                if !known {
                    self.expire_passkey_card(persona_id, &arming);
                    if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
                        live.asked = Some(ask.clone());
                        live.answered = false;
                        live.operator_approved = false;
                    }
                    self.raise_passkey_card(persona_id, &arming, &ask);
                }
                Ok((
                    standing(&arming, PasskeyRegistrationState::Asked, Some(ask)),
                    true,
                ))
            }
            Registration::Approved { ask, .. } => {
                self.note_passkey_answer(persona_id, &arming, &ask, true);
                Ok((
                    standing(&arming, PasskeyRegistrationState::Approved, Some(ask)),
                    true,
                ))
            }
            Registration::Registered {
                credential, ask, ..
            } => {
                if let Some(ask) = &ask {
                    self.note_passkey_answer(persona_id, &arming, ask, true);
                }
                {
                    let mut armings = lock(&self.passkey_armings);
                    match armings.get_mut(persona_id) {
                        Some(live) if !live.storing && live.stored.is_none() => live.storing = true,
                        // Another look has it, or had it.
                        Some(live) => {
                            let answer = match live.stored.clone() {
                                Some(secret) => stored(&arming, secret),
                                None => standing(&arming, PasskeyRegistrationState::Approved, ask),
                            };
                            let more = live.stored.is_none();
                            return Ok((answer, more));
                        }
                        // Cancelled meanwhile.
                        None => return Ok((idle, false)),
                    }
                }
                match self
                    .store_minted_passkey(persona_id, &arming, &ready, credential)
                    .await
                {
                    Ok(secret) => {
                        if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
                            live.storing = false;
                            live.stored = Some(secret.clone());
                        }
                        Ok((stored(&arming, secret), false))
                    }
                    Err(error) => {
                        lock(&self.passkey_armings).remove(persona_id);
                        self.write(
                            persona_id,
                            &TranscriptEvent::Notice {
                                id: new_id(),
                                ts: now_ms(),
                                level: NoticeLevel::Warn,
                                text: format!(
                                    "The passkey for {} was made in the browser but could not be stored, so it is not kept: {error} Arm it again from the teammate's pane.",
                                    arming.rp_id
                                ),
                            },
                        );
                        Err(error)
                    }
                }
            }
        }
    }

    /// The person's answer to the passkey card on `persona_id`'s tape: the
    /// site's request, parked in the browser, is approved — the browser
    /// makes the passkey and the room's watch stores it — or denied, which
    /// ends the arming. One answer to one request: a request that is not
    /// waiting any more is refused, so a stale card cannot let one through.
    pub async fn secrets_passkey_answer(
        self: &Arc<Self>,
        persona_id: &str,
        ask_id: &str,
        approved: bool,
    ) -> Result<PasskeyRegistration, String> {
        let persona = self.persona(persona_id)?;
        let arming = lock(&self.passkey_armings)
            .get(persona_id)
            .cloned()
            .ok_or_else(|| format!("No passkey request is waiting for {}.", persona.name))?;
        let Some(ask) = arming.asked.clone().filter(|ask| ask.id == ask_id) else {
            return Err("That passkey request is not waiting any more.".to_string());
        };
        if arming.answered {
            return Err("That passkey request was already answered.".to_string());
        }
        if let Some(capture) = &arming.capture {
            capture.capability.check()?;
            if ask.origin != capture.origin
                || ask.rp_id != arming.rp_id
                || now_ms() >= arming.expires_at
            {
                return Err(
                    "That passkey request is outside the credential card's origin or lifetime."
                        .into(),
                );
            }
        }
        let Some(ready) = self.computers.running(persona_id).await else {
            self.drop_passkey_arming(persona_id);
            return Err("The computer stopped before the request was answered.".to_string());
        };
        match crate::computer::passkeys::answer(&ready, ask_id, approved).await {
            Ok(_) => {}
            Err(crate::computer::secrets::Refusal::TooOld) => {
                return Err(
                    "The computer is on a release that does not ask before making a passkey."
                        .to_string(),
                );
            }
            Err(crate::computer::secrets::Refusal::Failed(reason)) => {
                return Err(format!("The computer could not take the answer. {reason}"));
            }
        }
        if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
            live.operator_approved = approved;
        }
        self.note_passkey_answer(persona_id, &arming, &ask, approved);
        if approved {
            let (answer, _) = self.look_at_passkey_arming(persona_id).await?;
            Ok(answer)
        } else {
            lock(&self.passkey_armings).remove(persona_id);
            Ok(PasskeyRegistration {
                state: PasskeyRegistrationState::Idle,
                name: None,
                rp_id: None,
                expires_at: None,
                ask: None,
                secret: None,
            })
        }
    }

    /// Raises the card for a site's request on the teammate's tape, and
    /// tells the phones the teammate needs someone.
    fn raise_passkey_card(
        &self,
        persona_id: &str,
        arming: &PasskeyArming,
        ask: &crate::contract::PasskeyAsk,
    ) {
        self.write_passkey_card(
            persona_id,
            &arming.name,
            ask,
            crate::contract::PasskeyAskStatus::Pending,
        );
        let account = match (&ask.user_name, &ask.user_display_name) {
            (Some(name), Some(display)) if display != name => format!(" for {name} ({display})"),
            (Some(name), _) => format!(" for {name}"),
            (None, Some(display)) => format!(" for {display}"),
            (None, None) => String::new(),
        };
        self.push.notify(
            &self.needs_you(persona_id),
            &format!(
                "{} asks to make a passkey{account}. Approve or deny it on the tape.",
                ask.rp_id
            ),
            persona_id,
            Some(crate::push::Waiting::Passkey {
                ask_id: ask.id.clone(),
            }),
        );
    }

    /// The card's afterlife once the request is answered — by the person
    /// here, or found answered at the computer — written once.
    fn note_passkey_answer(
        &self,
        persona_id: &str,
        arming: &PasskeyArming,
        ask: &crate::contract::PasskeyAsk,
        approved: bool,
    ) {
        {
            let mut armings = lock(&self.passkey_armings);
            let Some(live) = armings.get_mut(persona_id) else {
                return;
            };
            let known = live.asked.as_ref().is_some_and(|known| known.id == ask.id);
            if known && live.answered {
                return;
            }
            live.asked = Some(ask.clone());
            live.answered = true;
        }
        self.write_passkey_card(
            persona_id,
            &arming.name,
            ask,
            if approved {
                crate::contract::PasskeyAskStatus::Approved
            } else {
                crate::contract::PasskeyAskStatus::Denied
            },
        );
    }

    /// A card nobody answered, for a request that is gone.
    fn expire_passkey_card(&self, persona_id: &str, arming: &PasskeyArming) {
        if arming.answered {
            return;
        }
        if let Some(ask) = &arming.asked {
            self.write_passkey_card(
                persona_id,
                &arming.name,
                ask,
                crate::contract::PasskeyAskStatus::Expired,
            );
        }
    }

    /// Lets an arming go, expiring the card it raised if nobody answered.
    fn drop_passkey_arming(&self, persona_id: &str) {
        let arming = lock(&self.passkey_armings).remove(persona_id);
        if let Some(arming) = arming {
            self.expire_passkey_card(persona_id, &arming);
        }
    }

    fn write_passkey_card(
        &self,
        persona_id: &str,
        name: &str,
        ask: &crate::contract::PasskeyAsk,
        status: crate::contract::PasskeyAskStatus,
    ) {
        let home = lock(&self.passkey_armings)
            .get(persona_id)
            .and_then(|a| a.capture.as_ref().map(|c| c.home.clone()))
            .unwrap_or_else(|| ThreadId::dm(persona_id));
        self.write_home(
            persona_id,
            &home,
            &TranscriptEvent::PasskeyAsk {
                from: None,
                id: format!("passkey-ask:{}", ask.id),
                ts: now_ms(),
                ask_id: ask.id.clone(),
                name: name.to_owned(),
                rp_id: ask.rp_id.clone(),
                origin: ask.origin.clone(),
                rp_name: ask.rp_name.clone(),
                user_name: ask.user_name.clone(),
                user_display_name: ask.user_display_name.clone(),
                status,
            },
        );
    }

    /// Keeps what the browser minted: stored in the vault under the
    /// arming's name, ticked for the teammate — the tick the person would
    /// otherwise make in the teammate's pane, made for them since they
    /// asked for this passkey for this teammate — handed to the computer
    /// with the rest of its set, the arming ended, and a line on the tape.
    /// Answers the record as listed.
    async fn store_minted_passkey(
        self: &Arc<Self>,
        persona_id: &str,
        arming: &PasskeyArming,
        ready: &crate::computer::Ready,
        mut credential: crate::vault::StoredSecret,
    ) -> Result<crate::contract::SharedSecret, String> {
        let pending = arming
            .capture
            .as_ref()
            .and_then(|capture| lock(&self.captures).get(&capture.action).cloned());
        let mut captured = if arming.capture.is_some() {
            Some(
                pending
                    .as_ref()
                    .ok_or("The credential request ended before the passkey was stored.")?
                    .lock()
                    .await,
            )
        } else {
            None
        };
        if let Some(capture) = captured.as_ref() {
            capture.check_passkey(&arming.name)?;
        }
        let vault = self
            .vault
            .as_ref()
            .ok_or("Stored secrets are unavailable on this room.")?;
        let updates = self.policy_update_lock();
        let held = updates.lock().await;
        if let Some(capture) = &arming.capture {
            capture.capability.check()?;
            if vault
                .shared_secrets()
                .map_err(|_| "The vault could not be listed.")?
                .iter()
                .any(|secret| secret.name == arming.name)
            {
                return Err("That passkey name was stored meanwhile. Choose another name.".into());
            }
            if let crate::vault::StoredSecret::Passkey { ask, .. } = &mut credential {
                *ask = capture.ask;
            }
        }
        vault
            .set_shared(&arming.name, credential)
            .map_err(|error| error.to_string())?;
        let mut persona = self.persona(persona_id)?;
        if let Some(computer) = persona.computer.as_mut() {
            let mut granted = computer.secrets.take().unwrap_or_default();
            if !granted.contains(&arming.name) {
                granted.push(arming.name.clone());
                granted.sort();
            }
            computer.secrets = Some(granted);
        }
        persona.updated_at = now_ms();
        room::append_persona(&self.log, &persona)?;
        drop(held);
        self.hand_secrets(&persona, ready).await;
        if let Err(error) = crate::computer::passkeys::disarm(ready).await {
            self.write(
                persona_id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Info,
                    text: format!(
                        "The passkey {} was stored, but the computer's arming could not be ended; it ends by itself within ten minutes. {error:?}",
                        arming.name
                    ),
                },
            );
        }
        let secret = vault
            .shared_secrets()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|secret| secret.name == arming.name)
            .ok_or_else(|| format!("{} was stored but is not listed.", arming.name))?;
        self.write(
            persona_id,
            &TranscriptEvent::Notice {
                id: new_id(),
                ts: now_ms(),
                level: NoticeLevel::Info,
                text: format!(
                    "The passkey {} for {} is stored and ticked for {}: its browser signs in there with it from now on. Take it back by unticking it here, removing it under Settings → Secrets, or deleting it in the site's security settings.",
                    arming.name, arming.rp_id, persona.name
                ),
            },
        );
        if let Some(capture) = captured.as_mut() {
            self.settle_capture(
                capture,
                crate::contract::CredentialStatus::Stored,
                Some(arming.name.clone()),
            );
        }
        Ok(secret)
    }

    /// Ends an arming without a passkey; a card nobody answered expires
    /// with it. A computer that is not running has nothing to end.
    pub async fn secrets_passkey_cancel(&self, persona_id: &str) -> Result<(), String> {
        self.drop_passkey_arming(persona_id);
        if let Some(ready) = self.computers.running(persona_id).await
            && let Err(crate::computer::secrets::Refusal::Failed(reason)) =
                crate::computer::passkeys::disarm(&ready).await
        {
            return Err(format!("The computer could not be told. {reason}"));
        }
        Ok(())
    }

    /// After a secret was stored, replaced or deleted: every running computer
    /// is handed the set its teammate is granted now, so a rotation or a
    /// revocation reaches the machine without a restart. A stopped computer
    /// gets the current set at its next start, as every start does.
    pub async fn secrets_changed(&self) {
        for persona in room::roster(&self.log) {
            if !persona
                .computer
                .as_ref()
                .is_some_and(|computer| computer.enabled)
            {
                continue;
            }
            if let Some(ready) = self.computers.running(&persona.id).await {
                self.hand_secrets(&persona, &ready).await;
            }
        }
    }
}

/// What a teammate's computer is armed to make: the name the passkey will
/// be stored under, the site, and when the computer's arming ends. The
/// room watches it by itself (see `watch_passkey_arming`); once a look has
/// stored what the browser minted, the record as listed waits here for the
/// next `registration` poll, which answers it once.
#[derive(Clone)]
pub(super) struct PasskeyCapture {
    pub action: String,
    pub origin: String,
    pub ask: bool,
    pub capability: CapabilityLease,
    pub home: ThreadId,
}

#[derive(Clone)]
pub(super) struct PasskeyArming {
    pub(super) capture: Option<PasskeyCapture>,
    name: String,
    rp_id: String,
    pub(super) expires_at: i64,
    /// The site's request the card on the tape was raised for, once one
    /// was, and whether that card has been answered — by the person, or
    /// by a look that found the computer already past it.
    asked: Option<crate::contract::PasskeyAsk>,
    answered: bool,
    /// Only the authenticated answer handler sets this; a poll is not consent.
    operator_approved: bool,
    /// A look found it made and is storing it; another look meanwhile
    /// answers `armed` rather than storing it twice.
    storing: bool,
    /// Stored, ticked and handed: answered to the next poll, then let go.
    stored: Option<crate::contract::SharedSecret>,
}

/// How often the room looks at an arming it holds.
const PASSKEY_LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// The room's own look at a teammate's arming, every couple of seconds
/// until the passkey is made or the arming ends. A person makes the passkey
/// from the teammate's screen, or asks the teammate to, and neither keeps
/// Settings → Secrets open, so no pane's poll can be relied on to be running
/// at the moment the browser mints it: the room watches whatever is open,
/// and the pane's poll only reads where it stands.
fn watch_passkey_arming(room: Weak<Room>, persona_id: String) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(PASSKEY_LOOK_EVERY).await;
            let Some(room) = room.upgrade() else {
                return;
            };
            match room.look_at_passkey_arming(&persona_id).await {
                Ok((_, true)) => {}
                Ok((_, false)) => return,
                // The computer could not be asked this time; the arming is
                // still there to look at unless the look let it go.
                Err(_) => {
                    if !lock(&room.passkey_armings).contains_key(&persona_id) {
                        return;
                    }
                }
            }
        }
    });
}

/// What the agent is told before it is told anything else: who it is, where it
/// stands, how far it can reach, what day it is, how to talk in this room, and
/// — when it is joining a conversation that already has chapters behind it —
/// what happened in the one that closed. Everything here is something it would
/// otherwise have to ask for or guess. Both kinds of agent hear this, so the
/// house style is not a second briefing an ACP child gets and Hotline Agent does
/// not.
/// One stored secret as the preamble names it: its kind, and for a login
/// or a passkey the sites it is for, so the teammate knows where it applies.
pub(super) fn describe_secret(secret: &SharedSecret) -> String {
    use crate::contract::SharedSecretKind;
    match secret.kind {
        SharedSecretKind::Variable => "a variable".to_string(),
        SharedSecretKind::Login => match secret.sites.as_deref() {
            Some(sites) if !sites.is_empty() => format!("a login for {}", sites.join(", ")),
            _ => "a login".to_string(),
        },
        SharedSecretKind::Passkey => match secret.rp_id.as_deref() {
            Some(rp_id) => format!("a passkey for {rp_id}"),
            None => "a passkey".to_string(),
        },
    }
}
