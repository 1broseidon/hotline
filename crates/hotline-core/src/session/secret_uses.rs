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
}

struct UseCard {
    id: String,
    held: HeldUse,
    status: SecretUseStatus,
    capability: CapabilityLease,
}

impl UseCard {
    /// A sign-in waiting for the person to finish it, not a use to answer.
    fn finishing(&self) -> bool {
        is_finish(&self.held)
    }
}

fn is_finish(held: &HeldUse) -> bool {
    held.operation
        .as_ref()
        .is_some_and(|operation| operation.kind == crate::contract::SecretOperationKind::Finish)
}

/// How long a "finish signing in" card may stand while the computer still
/// lists it: a day, as a delivering card's longest wait.
const FINISH_MS: i64 = 24 * 60 * 60 * 1000;

/// A sign-in the computer could not finish, as the card the person sees: the
/// login it is, on the site it is for, opened on the site's screen.
fn finish_card(finish: crate::computer::uses::Finish) -> HeldUse {
    let site = url::Url::parse(&finish.origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| finish.origin.clone());
    HeldUse {
        id: finish.id(),
        secret: finish.name,
        kind: CaptureKind::Login,
        site,
        path: String::new(),
        asked_at: finish.since,
        expires_at: finish.since.saturating_add(FINISH_MS),
        operation: Some(crate::contract::SecretOperation {
            kind: crate::contract::SecretOperationKind::Finish,
            origin: finish.origin,
            account: finish.account,
            rp_id: None,
        }),
        attested: false,
    }
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
        if !persona.computer.is_some_and(|c| c.enabled) {
            return Vec::new();
        }
        let granted = persona.secrets.unwrap_or_default();
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
        let granted = self.ask_me_secrets(persona);
        // A protected computer may hand the person a sign-in to finish
        // whatever the login's policy, so it is watched while it holds one.
        let names: Vec<String> = self
            .persona(persona)
            .ok()
            .and_then(|teammate| teammate.secrets)
            .unwrap_or_default();
        let finishes = !names.is_empty()
            && self.computers.protection(persona)
                == Some(crate::contract::ComputerProtection::Protected);
        if granted.is_empty()
            && !finishes
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
            let still_granted = if card.finishing() {
                names.contains(&card.held.secret)
            } else {
                granted.iter().any(|s| s.name == card.held.secret)
            };
            if now_ms() >= card.held.expires_at || !card.capability.is_current() || !still_granted {
                // Revocation must also release a page still waiting at the door.
                if card.status == SecretUseStatus::Pending
                    && !card.finishing()
                    && let Some(ready) = &ready
                {
                    let _ = uses::answer(ready, &card.held.id, SecretUseAnswer::Deny).await;
                }
                self.settle_secret_use(persona, card, SecretUseStatus::Expired);
            }
        }
        let Some(ready) = ready else { return };
        let asks = !watch.old && !granted.is_empty();
        if !asks && !finishes {
            return;
        }
        // Sign-ins to finish are their own door, so a desk that reads the
        // held uses strictly never meets a kind it does not know there.
        let finishing = if finishes {
            match uses::pending_signins(&ready).await {
                Ok(finishing) => finishing,
                Err(_) => return,
            }
        } else {
            Vec::new()
        };
        let held = if asks {
            uses::held(&ready).await
        } else {
            Ok(Vec::new())
        };
        let mut held = match held {
            Ok(held) => {
                let mut held = held;
                held.extend(finishing.into_iter().map(finish_card));
                held
            }
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
                if let Ok(teammate) = self.persona(persona) {
                    self.put_secrets(&teammate, &ready, true).await;
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
        held.retain(|held| !is_finish(held) || names.contains(&held.secret));
        for held in held {
            let finishing = is_finish(&held);
            if watch.cards.contains_key(&held.id)
                || held.expires_at <= now_ms()
                || (!finishing
                    && !granted.iter().any(|s| {
                        s.name == held.secret
                            && match held.kind {
                                CaptureKind::Login => {
                                    s.kind == crate::contract::SharedSecretKind::Login
                                }
                                CaptureKind::Passkey => {
                                    s.kind == crate::contract::SharedSecretKind::Passkey
                                }
                            }
                    }))
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
            let said = if finishing {
                format!("Finish signing in to {} on its screen.", card.held.site)
            } else {
                format!(
                    "Approve use of {} on {} at the desk or phone.",
                    card.held.secret, card.held.site
                )
            };
            self.push.notify(
                &self.needs_you(persona),
                &said,
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
        if card.finishing() {
            return Err(
                "There is nothing to answer: Open shows the site, to finish signing in there."
                    .into(),
            );
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
        // On a split computer, which says what each use is, ten minutes may
        // cover only a sign-in its broker itself is doing: from anything else
        // there, one answer is one use. An older computer's Allow is as it was.
        if answer == SecretUseAnswer::Allow && card.held.operation.is_some() && !card.held.attested
        {
            return Err("Allow 10 min is only for a sign-in Hotline's own browser is doing. Approve this one use, or deny it.".into());
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
                operation: card.held.operation.clone(),
                attested: card.held.attested,
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
            _ if card.finishing() => format!(
                "{} is no longer waiting for the person to finish signing in. Look at the page again.",
                card.held.site
            ),
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
            // A teammate without a computer has the desk browser, when the
            // desk has one, and is offered cookies for the site it is on.
            let desk_browser = room.desk_browser().is_some();
            for persona in room::roster(&room.log) {
                if desk_browser || persona.computer.as_ref().is_some_and(|c| c.enabled) {
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

/// What an Ask-me card asks, bound to its one operation: a sign-in as an
/// account on a site, or a passkey sign-in. The window's card for a
/// computer's use asks the same words (`SecretUseAsk.tsx`), and so does the
/// desk browser's.
pub(super) fn use_question(secret: &str, operation: &crate::contract::SecretOperation) -> String {
    let host = |origin: &str| {
        url::Url::parse(origin)
            .ok()
            .and_then(|url| {
                url.host_str().map(|host| match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                })
            })
            .unwrap_or_else(|| origin.to_owned())
    };
    let (site, passkey) = match operation.kind {
        crate::contract::SecretOperationKind::Finish => {
            return format!("Finish signing in to {}", host(&operation.origin));
        }
        crate::contract::SecretOperationKind::Signin => (host(&operation.origin), ""),
        crate::contract::SecretOperationKind::Passkey => (
            operation
                .rp_id
                .clone()
                .unwrap_or_else(|| host(&operation.origin)),
            "the passkey ",
        ),
    };
    let account = if operation.account.is_empty() {
        String::new()
    } else {
        format!(" as {}", operation.account)
    };
    format!("Sign in to {site}{account} with {passkey}{secret}?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{SecretOperation, SecretOperationKind};

    #[test]
    fn the_card_asks_about_one_sign_in_in_the_windows_words() {
        let signin = SecretOperation {
            kind: SecretOperationKind::Signin,
            origin: "https://github.com".into(),
            account: "george".into(),
            rp_id: None,
        };
        assert_eq!(
            use_question("GITHUB", &signin),
            "Sign in to github.com as george with GITHUB?"
        );
        let passkey = SecretOperation {
            kind: SecretOperationKind::Passkey,
            origin: "https://github.com".into(),
            account: String::new(),
            rp_id: Some("github.com".into()),
        };
        assert_eq!(
            use_question("GITHUB_PASSKEY", &passkey),
            "Sign in to github.com with the passkey GITHUB_PASSKEY?"
        );
    }
}
