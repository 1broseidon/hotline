//! Passkeys in a teammate's desk browser: a page's `navigator.credentials`
//! call, handed over by the app with the origin WebKit reports for the
//! frame, answered by the core with the teammate's own passkey from the
//! vault (docs/security.md, "A teammate's browser"). Signing in uses a
//! granted passkey for the site, after the person's approval on the card
//! when it is an ask-me one; making one needs the arming from the
//! teammate's pane and the person's approval on the passkey card, as on a
//! computer, and the record goes straight to the vault. The private key is
//! made, kept and used in this process: the page gets the signature and the
//! public key, and the agent gets nothing at all.

use super::secrets::PasskeyArming;
use super::*;
use crate::browser::authenticator::{self, Key, Refusal, Request, refuse};
use crate::contract::{PasskeyAsk, PasskeyRegistration, PasskeyRegistrationState};

/// How long the desk browser is armed to make one passkey, as a computer is.
const ARMED_MS: i64 = 10 * 60_000;

impl Room {
    /// Answers a page's passkey call in `persona_id`'s desk browser, or
    /// says what the page hears instead.
    pub async fn browser_passkey(
        self: &Arc<Self>,
        persona_id: &str,
        request: &Request,
    ) -> Result<Value, Refusal> {
        if !self.offers_browser(persona_id) {
            return Err(refuse(
                "NotAllowedError",
                "this teammate has no desk browser",
            ));
        }
        // A frame of another site gets no passkey: a page is answered for
        // its own origin only, as a browser's permissions policy does by
        // default.
        if request.origin != request.top_origin {
            return Err(refuse(
                "NotAllowedError",
                format!(
                    "{} is a frame inside {}, and passkeys are used by the page itself only",
                    request.origin, request.top_origin
                ),
            ));
        }
        if request.create {
            self.make_browser_passkey(persona_id, &request.origin, &request.options)
                .await
        } else {
            self.sign_in_with_passkey(persona_id, &request.origin, &request.options)
                .await
        }
    }

    /// The passkeys `persona_id` is granted, as the vault keeps them, read
    /// off the async threads, since a keychain read blocks.
    async fn granted_passkeys(&self, persona_id: &str) -> Vec<Key> {
        let names = self
            .persona(persona_id)
            .ok()
            .and_then(|persona| persona.secrets)
            .unwrap_or_default();
        let Some(vault) = self.vault.clone() else {
            return Vec::new();
        };
        if names.is_empty() {
            return Vec::new();
        }
        tokio::task::spawn_blocking(move || vault.shared_secret_values(&names))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|(values, _)| values)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, stored)| Key::of(name, stored))
            .collect()
    }

    async fn sign_in_with_passkey(
        &self,
        persona_id: &str,
        origin: &str,
        options: &Value,
    ) -> Result<Value, Refusal> {
        let rp_id = authenticator::relying_party(origin, options["rpId"].as_str())?;
        let keys = self.granted_passkeys(persona_id).await;
        let key = authenticator::chosen(&rp_id, options, &keys)?;
        if key.r#use == crate::contract::SecretUse::Ask {
            let persona = self
                .persona(persona_id)
                .map_err(|error| refuse("NotAllowedError", error))?;
            let page = url::Url::parse(origin)
                .map_err(|_| refuse("SecurityError", format!("{origin} is not an origin")))?;
            let doing = format!("sign in to {rp_id} with it on {origin}");
            self.ask_for_key(persona_id, &persona.name, &key.name, &doing, &page)
                .await
                .map_err(|error| refuse("NotAllowedError", error))?;
        }
        authenticator::assert(origin, options, key)
    }

    /// Arms `persona_id`'s desk browser to make one passkey for `rp_id`,
    /// stored under `name`, for ten minutes: the arming a computer gets
    /// from the same row in the teammate's pane.
    pub(super) fn arm_desk_browser(
        &self,
        name: &str,
        persona_id: &str,
        rp_id: &str,
    ) -> PasskeyRegistration {
        self.drop_passkey_arming(persona_id);
        let expires_at = now_ms() + ARMED_MS;
        lock(&self.passkey_armings).insert(
            persona_id.to_owned(),
            PasskeyArming {
                capture: None,
                desk: true,
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
        PasskeyRegistration {
            state: PasskeyRegistrationState::Armed,
            name: Some(name.to_owned()),
            rp_id: Some(rp_id.to_owned()),
            expires_at: Some(expires_at),
            ask: None,
            secret: None,
        }
    }

    /// A page's `create()`: only under an arming for its relying party,
    /// only once the person approves the card it raises, and the passkey
    /// is in the vault, ticked for the teammate, before the page hears of
    /// it.
    async fn make_browser_passkey(
        self: &Arc<Self>,
        persona_id: &str,
        origin: &str,
        options: &Value,
    ) -> Result<Value, Refusal> {
        let creation = authenticator::creation(origin, options)?;
        let not_armed = || {
            refuse(
                "NotAllowedError",
                format!(
                    "making a passkey for {} is not armed: the person arms it from the teammate's pane",
                    creation.rp_id
                ),
            )
        };
        let arming = lock(&self.passkey_armings)
            .get(persona_id)
            .filter(|arming| {
                arming.desk
                    && arming.rp_id == creation.rp_id
                    && arming.expires_at > now_ms()
                    && !arming.storing
                    && arming.stored.is_none()
            })
            .cloned()
            .ok_or_else(not_armed)?;
        if authenticator::excluded(&creation, &self.granted_passkeys(persona_id).await) {
            return Err(refuse(
                "InvalidStateError",
                "this teammate already has that passkey",
            ));
        }
        let ask = PasskeyAsk {
            id: new_id(),
            rp_id: creation.rp_id.clone(),
            origin: creation.origin.clone(),
            rp_name: creation.rp_name.clone(),
            user_name: creation.user_name.clone(),
            user_display_name: creation.user_display_name.clone(),
            asked_at: now_ms(),
        };
        let (sender, answered) = tokio::sync::oneshot::channel();
        {
            let mut asks = lock(&self.browser_passkey_asks);
            if asks.contains_key(persona_id) {
                return Err(refuse(
                    "NotAllowedError",
                    "another request to make a passkey is waiting for the person",
                ));
            }
            asks.insert(persona_id.to_owned(), sender);
        }
        if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
            live.asked = Some(ask.clone());
            live.answered = false;
            live.operator_approved = false;
        }
        self.raise_passkey_card(persona_id, &arming, &ask);
        let left = u64::try_from(arming.expires_at - now_ms()).unwrap_or_default();
        match tokio::time::timeout(std::time::Duration::from_millis(left), answered).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                return Err(refuse(
                    "NotAllowedError",
                    "the person declined making this passkey",
                ));
            }
            // The arming ran out, or was cancelled, before an answer.
            _ => {
                self.drop_passkey_arming(persona_id);
                return Err(refuse(
                    "NotAllowedError",
                    "the arming ended before the person approved this passkey",
                ));
            }
        }
        let (record, response) = authenticator::mint(&creation)?;
        match lock(&self.passkey_armings).get_mut(persona_id) {
            Some(live) if live.desk && !live.storing && live.stored.is_none() => {
                live.storing = true;
            }
            _ => return Err(not_armed()),
        }
        match self
            .store_minted_passkey(persona_id, &arming, None, record)
            .await
        {
            Ok(secret) => {
                if let Some(live) = lock(&self.passkey_armings).get_mut(persona_id) {
                    live.storing = false;
                    live.stored = Some(secret);
                }
                Ok(response)
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
                            "The passkey for {} could not be stored, so the site was not given one: {error} Arm it again from the teammate's pane.",
                            arming.rp_id
                        ),
                    },
                );
                Err(refuse("UnknownError", "the passkey could not be stored"))
            }
        }
    }
}
