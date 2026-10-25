//! One card per held use, serialized with answers from either seat. A saved
//! card is history; only a live watch of this computer can answer its door.
use super::*;
use crate::computer::{
    Ready,
    uses::{self, HeldUse, Refusal},
};
use crate::contract::{CaptureKind, SecretUse, SecretUseAnswer, SecretUseStatus};

#[derive(Default)]
pub(super) struct UseWatch {
    pub(super) ready: Option<Ready>,
    pub(super) old: bool,
    cards: HashMap<String, UseCard>,
    pub(super) cookies: super::cookie_offers::CookieWatch,
}

struct UseCard {
    id: String,
    held: HeldUse,
    status: SecretUseStatus,
    capability: CapabilityLease,
}

impl Room {
    pub(super) fn use_watch(&self, persona: &str) -> Arc<TokioMutex<UseWatch>> {
        lock(&self.secret_uses)
            .entry(persona.into())
            .or_default()
            .clone()
    }

    fn ask_me_secrets(&self, persona: &str) -> Vec<SharedSecret> {
        let Ok(persona) = self.persona(persona) else {
            return Vec::new();
        };
        let Some(computer) = persona.computer.filter(|c| c.enabled) else {
            return Vec::new();
        };
        let granted = computer.secrets.unwrap_or_default();
        self.stored_secrets()
            .into_iter()
            .filter(|s| s.r#use == SecretUse::Ask && granted.contains(&s.name))
            .collect()
    }

    #[cfg(all(test, unix))]
    pub(crate) async fn poll_secret_uses(self: &Arc<Self>, persona: &str) {
        let watch = self.use_watch(persona);
        let mut watch = watch.lock().await;
        self.look_at_secret_uses(persona, &mut watch).await;
    }

    async fn look_at_secret_uses(self: &Arc<Self>, persona: &str, watch: &mut UseWatch) {
        self.look_at_cookie_offers(persona, &mut watch.cookies)
            .await;
        let granted = self.ask_me_secrets(persona);
        if granted.is_empty()
            && watch
                .cards
                .values()
                .all(|card| card.status != SecretUseStatus::Pending)
        {
            return;
        }
        let ready = self.computers.running(persona).await;
        if ready != watch.ready {
            for card in watch.cards.values_mut() {
                self.settle_secret_use(persona, card, SecretUseStatus::Expired);
            }
            watch.cards.clear();
            watch.ready.clone_from(&ready);
            watch.old = false;
        }
        for card in watch.cards.values_mut() {
            if now_ms() >= card.held.expires_at
                || !card.capability.is_current()
                || !granted.iter().any(|s| s.name == card.held.secret)
            {
                // Revocation must also release a page still waiting at the door.
                if card.status == SecretUseStatus::Pending
                    && let Some(ready) = &ready
                {
                    let _ = uses::answer(ready, &card.held.id, SecretUseAnswer::Deny).await;
                }
                self.settle_secret_use(persona, card, SecretUseStatus::Expired);
            }
        }
        let Some(ready) = ready else { return };
        if watch.old || granted.is_empty() {
            return;
        }
        let held = match uses::held(&ready).await {
            Ok(held) => held,
            Err(Refusal::TooOld) => {
                watch.old = true;
                eprintln!(
                    "{persona}'s computer predates secret-use approvals; every key is automatic on it. Update the computer to use Ask me."
                );
                for card in watch.cards.values_mut() {
                    self.settle_secret_use(persona, card, SecretUseStatus::Expired);
                }
                // Explicitly deliver the compatibility policy without changing
                // the person's stored choice for newer computers.
                let names: Vec<_> = self
                    .persona(persona)
                    .ok()
                    .and_then(|p| p.computer)
                    .and_then(|c| c.secrets)
                    .unwrap_or_default();
                if let Some(vault) = &self.vault
                    && let Ok((mut values, _)) = vault.shared_secret_values(&names)
                {
                    make_automatic(&mut values);
                    let _ = crate::computer::secrets::deliver(&ready, &values).await;
                }
                return;
            }
            Err(_) => return,
        };
        watch.cards.retain(|id, card| {
            if held.iter().any(|h| h.id == *id) {
                true
            } else {
                self.settle_secret_use(persona, card, SecretUseStatus::Expired);
                false
            }
        });
        for held in held {
            if watch.cards.contains_key(&held.id)
                || held.expires_at <= now_ms()
                || !granted.iter().any(|s| {
                    s.name == held.secret
                        && match held.kind {
                            CaptureKind::Login => {
                                s.kind == crate::contract::SharedSecretKind::Login
                            }
                            CaptureKind::Passkey => {
                                s.kind == crate::contract::SharedSecretKind::Passkey
                            }
                        }
                })
            {
                continue;
            }
            let card = UseCard {
                id: new_id(),
                held,
                status: SecretUseStatus::Pending,
                capability: self.capability_lease(persona),
            };
            self.write_secret_use(persona, &card);
            self.push.notify(
                &self.needs_you(persona),
                &format!(
                    "Approve use of {} on {} at the desk or phone.",
                    card.held.secret, card.held.site
                ),
                persona,
                Some(crate::push::Waiting::SecretUse {
                    use_id: card.held.id.clone(),
                }),
            );
            watch.cards.insert(card.held.id.clone(), card);
        }
    }

    pub async fn secrets_use_answer(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: SecretUseAnswer,
    ) -> Result<(), String> {
        // Like capture, an accepted answer finishes even if its socket goes away.
        let room = self.clone();
        let persona = persona.to_string();
        let id = id.to_string();
        tokio::spawn(async move { room.answer_secret_use(&persona, &id, answer).await })
            .await
            .map_err(|_| "The secret-use answer was interrupted.".to_string())?
    }

    async fn answer_secret_use(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: SecretUseAnswer,
    ) -> Result<(), String> {
        let updates = self.policy_update_lock();
        let _held = updates.lock().await;
        let watch = self.use_watch(persona);
        let mut watch = watch.lock().await;
        let ready = watch
            .ready
            .clone()
            .ok_or("That use is no longer waiting.")?;
        let card = watch
            .cards
            .get_mut(id)
            .ok_or("That use is no longer waiting.")?;
        if card.status != SecretUseStatus::Pending {
            return Err("That use was already settled.".into());
        }
        if !card.capability.is_current()
            || now_ms() >= card.held.expires_at
            || self.computers.running(persona).await.as_ref() != Some(&ready)
            || !self
                .ask_me_secrets(persona)
                .iter()
                .any(|s| s.name == card.held.secret)
        {
            self.settle_secret_use(persona, card, SecretUseStatus::Expired);
            return Err("That use has lapsed.".into());
        }
        match uses::answer(&ready, id, answer).await {
            Ok(()) => self.settle_secret_use(
                persona,
                card,
                match answer {
                    SecretUseAnswer::Approve => SecretUseStatus::Approved,
                    SecretUseAnswer::Allow => SecretUseStatus::Allowed,
                    SecretUseAnswer::Deny => SecretUseStatus::Denied,
                },
            ),
            Err(Refusal::Expired | Refusal::TooOld) => {
                self.settle_secret_use(persona, card, SecretUseStatus::Expired);
                return Err("That use has lapsed.".into());
            }
            Err(Refusal::Failed) => {
                return Err("The computer could not take the answer. Try again.".into());
            }
        }
        Ok(())
    }

    fn write_secret_use(&self, persona: &str, card: &UseCard) {
        self.write(
            persona,
            &TranscriptEvent::SecretUseAsk {
                id: card.id.clone(),
                ts: card.held.asked_at,
                use_id: card.held.id.clone(),
                secret: card.held.secret.clone(),
                secret_kind: card.held.kind,
                site: card.held.site.clone(),
                path: card.held.path.clone(),
                teammate: persona.into(),
                asked_at: card.held.asked_at,
                expires_at: card.held.expires_at,
                status: card.status,
            },
        );
    }

    fn settle_secret_use(
        self: &Arc<Self>,
        persona: &str,
        card: &mut UseCard,
        status: SecretUseStatus,
    ) {
        if card.status != SecretUseStatus::Pending {
            return;
        }
        card.status = status;
        self.write_secret_use(persona, card);
        let text = match status {
            SecretUseStatus::Approved => format!(
                "The person approved this use of {} on {}. Retry the login fill if it was waiting.",
                card.held.secret, card.held.site
            ),
            SecretUseStatus::Allowed => format!(
                "The person allowed {} on {} for this teammate for 10 minutes. Retry the login fill if it was waiting.",
                card.held.secret, card.held.site
            ),
            SecretUseStatus::Denied => format!(
                "The person declined this use of {} on {}. Stop this sign-in.",
                card.held.secret, card.held.site
            ),
            _ => format!(
                "The use of {} on {} lapsed before it was answered. The page is no longer waiting for approval.",
                card.held.secret, card.held.site
            ),
        };
        let status = match status {
            SecretUseStatus::Denied => HumanActionStatus::Dismissed,
            SecretUseStatus::Expired => HumanActionStatus::Expired,
            _ => HumanActionStatus::Done,
        };
        let room = self.clone();
        let persona = persona.to_string();
        let id = card.id.clone();
        tokio::spawn(async move {
            let _ = room
                .hand_over_answer(
                    &persona,
                    &id,
                    "Secret use",
                    status,
                    Some(text),
                    ThreadId::dm(&persona),
                )
                .await;
        });
    }
}

pub(super) fn watch(room: Weak<Room>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let Some(room) = room.upgrade() else { return };
            let mut personas: HashSet<String> = lock(&room.secret_uses).keys().cloned().collect();
            for persona in room::roster(&room.log) {
                if persona.computer.as_ref().is_some_and(|c| c.enabled) {
                    personas.insert(persona.id);
                }
            }
            // Each computer has its own lock and deadline; an unavailable one
            // cannot delay a different teammate's card.
            for persona in personas {
                let Ok(mut watch) = room.use_watch(&persona).try_lock_owned() else {
                    continue;
                };
                let room = room.clone();
                tokio::spawn(async move { room.look_at_secret_uses(&persona, &mut watch).await });
            }
        }
    });
}

/// Compatibility is per running computer, never a rewrite of the vault.
pub(super) fn make_automatic(
    values: &mut std::collections::BTreeMap<String, crate::vault::StoredSecret>,
) {
    for value in values.values_mut() {
        match value {
            crate::vault::StoredSecret::Login { r#use, .. }
            | crate::vault::StoredSecret::Passkey { r#use, .. } => *r#use = SecretUse::Auto,
            crate::vault::StoredSecret::Variable { .. } => {}
        }
    }
}
