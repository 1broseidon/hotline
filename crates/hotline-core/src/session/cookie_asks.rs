//! A teammate asks to be signed in to one site, in its own words, and the
//! person answers on one card. Nothing is offered unless the teammate asks:
//! only it knows a page is a wall, since a private page often answers 404
//! rather than a sign-in form.
//!
//! The ask is a credential capture (`capture.rs`) with cookie parts, so Sign
//! In is the capture's own answer: values go only to the vault, under the
//! same lock, lease and lifetime. Use Cookies lends through `lend_site`, and
//! Open Browser hands the person the teammate's own browser, which the
//! window opens. The card is the same for a desk browser and a computer;
//! only where the cookies land and what opens differ.
use super::capture::Capture;
use super::*;
use crate::computer::cookies::lending;
use crate::contract::{
    CaptureKind, CookieAskStatus, CookieOffer, CookieOfferAnswer, CookieOfferSource,
    CookieOfferStatus, CredentialStatus, ThreadKind,
};

/// How long a card waits for the person, as a `request_human` card does.
const LIFETIME: i64 = 2 * 60 * 60_000;
/// How long a login given without Save to Secrets stays in the vault: long
/// enough for the sign-in it was given for.
pub(super) const UNSAVED_LOGIN_MS: i64 = 60 * 60_000;
const GONE: &str = "This sign-in ask is no longer waiting for an answer.";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CookieAskRequest {
    site: String,
    reason: String,
}

/// What a capture that is a cookie ask holds besides the login's parts.
pub(super) struct Parts {
    /// The registrable domain, as the cookies are lent by.
    pub(super) site: String,
    /// The person's browsers signed in to the site, the last lent from
    /// first, each as the card names it and as it is read.
    sources: Vec<(CookieOfferSource, lending::Source)>,
    desk_browser: bool,
    status: CookieAskStatus,
    lent: Option<CookieOffer>,
    /// When a login given without Save to Secrets leaves the vault.
    pub(super) kept_until: Option<i64>,
}

impl Parts {
    pub(super) fn event(&self, capture: &Capture, stored_as: Option<String>) -> TranscriptEvent {
        TranscriptEvent::CookieAsk {
            from: None,
            id: capture.id.clone(),
            ts: now_ms(),
            site: self.site.clone(),
            origin: capture.site.clone(),
            reason: capture.reason.clone(),
            status: self.status,
            sources: self
                .sources
                .iter()
                .map(|(named, _)| named.clone())
                .collect(),
            name: capture.name.clone(),
            browser_name: self.lent.as_ref().map(|lent| lent.browser_name.clone()),
            profile_name: self.lent.as_ref().map(|lent| lent.profile_name.clone()),
            stored_as,
            kept_until: self.kept_until,
            desk_browser: self.desk_browser,
            expires_at: capture.expires_at,
        }
    }

    /// What the teammate hears of the outcome: how, never a value.
    fn told(&self, capture: &Capture, stored_as: Option<&str>) -> String {
        let site = &self.site;
        match self.status {
            CookieAskStatus::Cookies => self.lent.as_ref().map_or_else(
                || format!("The person shared their cookies for {site}."),
                |lent| super::cookie_offers::cookie_outcome(lent, false),
            ),
            CookieAskStatus::SignedIn => {
                let name = stored_as.unwrap_or(&capture.name);
                let kept = if self.kept_until.is_some() {
                    " It was not saved to the person's Secrets: you have it for the next hour, for this sign-in only."
                } else {
                    ""
                };
                format!(
                    "The person gave you a login for {site}, stored as {name}. Sign in with it: use browser fill with secret {name}.username and secret {name}.password (or {name}.code for a one-time code) on {} only. You never see the values.{kept}",
                    capture.site
                )
            }
            CookieAskStatus::Opened if self.desk_browser => format!(
                "The person opened your browser to sign in to {site} themselves. Leave your browser to them until they write to you, then read the page again."
            ),
            CookieAskStatus::Opened => format!(
                "The person opened your computer's screen to sign in to {site} themselves. Leave the screen to them until they write to you, then look again."
            ),
            CookieAskStatus::NotNow => format!(
                "The person said no to signing you in to {site}. Do not ask for {site} again until they next write to you; carry on without it, or tell them what you could not reach."
            ),
            CookieAskStatus::Expired => format!(
                "No answer came before your sign-in ask for {site} expired, or its session ended. Nothing was shared or stored."
            ),
            CookieAskStatus::Withdrawn => {
                format!("Your sign-in ask for {site} was withdrawn. Nothing was shared or stored.")
            }
            CookieAskStatus::Pending => unreachable!("only a settled ask is told"),
        }
    }
}

impl Room {
    /// `ask_cookies`: raises the card, one per site while it is up.
    pub(crate) async fn ask_cookies(
        self: &Arc<Self>,
        persona_id: &str,
        home: &ThreadId,
        capability: CapabilityLease,
        request: CookieAskRequest,
    ) -> Result<String, String> {
        capability.check()?;
        let persona = self.persona(persona_id)?;
        let has_computer = persona.computer.as_ref().is_some_and(|c| c.enabled);
        if !has_computer && !self.offers_browser(persona_id) {
            return Err(
                "You have no browser to be signed in to: no computer and no desk browser.".into(),
            );
        }
        if matches!(home.kind, ThreadKind::Run | ThreadKind::Pair) {
            return Err("Ask to be signed in from your own chat or work thread.".into());
        }
        if home.kind == ThreadKind::Job
            && has_computer
            && room::all_jobs(&self.log).into_iter().any(|job| {
                job.id == home.key
                    && job.without.as_ref().is_some_and(|without| {
                        without.contains(&crate::contract::Ability::Computer)
                    })
            })
        {
            return Err("This job was started without the computer.".into());
        }
        let (site, origin) = site_and_origin(&request.site)
            .ok_or("Name the site as github.com or a full https:// address.")?;
        let reason = request.reason.trim();
        if !(3..=500).contains(&reason.chars().count()) {
            return Err("Say why in 3 to 500 characters.".into());
        }
        if let Some(id) = self.ask_up(persona_id, &site).await {
            return Err(format!(
                "Your ask for {site} is already up ({id}). Wait for the person's answer; it arrives as its own message."
            ));
        }
        if self.said_no(persona_id, &site) {
            return Err(format!(
                "The person said no to signing you in to {site}. Do not ask again until they next write to you."
            ));
        }
        let sources = self.signed_in_sources(&site).await;
        capability.check()?;
        let name = self.free_login_name(&site);
        let id = format!("cookie-ask:{}", new_id());
        let capture = Capture {
            persona: persona_id.into(),
            home: home.clone(),
            capability: capability.scoped(),
            id: id.clone(),
            site: origin,
            reason: reason.into(),
            name,
            kind: CaptureKind::Login,
            expires_at: now_ms() + LIFETIME,
            existing: Vec::new(),
            armed: false,
            status: CredentialStatus::Pending,
            ask: Some(Parts {
                site: site.clone(),
                sources,
                desk_browser: !has_computer,
                status: CookieAskStatus::Pending,
                lent: None,
                kept_until: None,
            }),
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
            &format!("Sign in to {site}?"),
            persona_id,
            Some(crate::push::Waiting::CookieAsk { id: id.clone() }),
        );
        super::capture::watch(Arc::downgrade(self), id.clone());
        Ok(format!(
            "Asked: {id} waits on the desk and phone for two hours. The person may share their cookies for {site}, give you a login, or open your browser to sign in themselves. The outcome arrives as its own message, never with a value; carry on with what does not need {site}. Use withdraw_request with this id if you stop needing it."
        ))
    }

    /// The ask for this site still up, if there is one.
    async fn ask_up(&self, persona_id: &str, site: &str) -> Option<String> {
        let captures: Vec<_> = lock(&self.captures).values().cloned().collect();
        for capture in captures {
            let capture = capture.lock().await;
            if capture.persona == persona_id
                && capture.status == CredentialStatus::Pending
                && capture.ask.as_ref().is_some_and(|ask| ask.site == site)
            {
                return Some(capture.id.clone());
            }
        }
        None
    }

    /// Whether the person closed an ask for this site and has not written
    /// to the teammate since, in its conversation or the thread it asked in.
    fn said_no(&self, persona_id: &str, site: &str) -> bool {
        self.cards_of(persona_id).into_iter().any(|(event, home)| {
            event["kind"] == "cookie_ask"
                && event["site"] == site
                && event["status"] == "not_now"
                && event["ts"]
                    .as_i64()
                    .is_some_and(|ts| !self.person_wrote_since(persona_id, &home, ts))
        })
    }

    fn person_wrote_since(&self, persona_id: &str, home: &ThreadId, since: i64) -> bool {
        let wrote = |events: Vec<Value>| {
            events.iter().any(|event| {
                event["kind"] == "user"
                    && event.get("scheduled").is_none()
                    && event["ts"].as_i64().is_some_and(|ts| ts > since)
            })
        };
        wrote(self.tape(persona_id))
            || (home.kind != ThreadKind::Dm
                && StreamId::of_thread(home).is_some_and(|stream| wrote(self.log.load(&stream))))
    }

    /// The person's browsers signed in to `site`, read now that the teammate
    /// asked: the one last lent from first, then the rest in a steady order.
    /// A browser Hotline may not read is left out rather than failing the ask.
    async fn signed_in_sources(&self, site: &str) -> Vec<(CookieOfferSource, lending::Source)> {
        let Ok(inventory) = self.cookie_inventory().await else {
            return Vec::new();
        };
        let mut last_lent: HashMap<(String, String), i64> = HashMap::new();
        for persona in room::roster(&self.log) {
            for lend in self.computer_cookie_offers(&persona.id) {
                if lend.status == CookieOfferStatus::Pending {
                    continue;
                }
                let at = last_lent
                    .entry((lend.browser_name, lend.profile_name))
                    .or_default();
                *at = (*at).max(lend.ts);
            }
        }
        let mut sources: Vec<_> = inventory
            .sources
            .into_iter()
            .filter(|(_, sites)| sites.contains(site))
            .map(|(source, _)| {
                let named = CookieOfferSource {
                    browser_name: source.browser_name().into(),
                    profile_name: source.profile_name().into(),
                };
                (named, source)
            })
            .collect();
        sources.sort_by_key(|(named, _)| {
            std::cmp::Reverse(
                last_lent
                    .get(&(named.browser_name.clone(), named.profile_name.clone()))
                    .copied()
                    .unwrap_or(i64::MIN),
            )
        });
        sources
    }

    /// The key a login for `site` is saved under unless the person renames
    /// it: `GITHUB_COM`, or the first `GITHUB_COM_2`, `_3` … not yet taken.
    fn free_login_name(&self, site: &str) -> String {
        let base: String = site
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
        let base = if base.starts_with(|c: char| c.is_ascii_alphabetic()) {
            base
        } else {
            format!("SITE_{base}")
        };
        let taken: HashSet<String> = self
            .stored_secrets()
            .into_iter()
            .map(|secret| secret.name)
            .collect();
        std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base}_{n}")))
            .find(|name| !taken.contains(name))
            .expect("an unbounded run of names has a free one")
    }

    /// Use Cookies, Open Browser and Not now, from the desk or the phone.
    /// Sign In is `answer_credential` with the same id.
    pub async fn computer_cookie_answer(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: CookieOfferAnswer,
        source: Option<CookieOfferSource>,
    ) -> Result<(), String> {
        // A phone disconnect cancels its wire future. Once accepted, the
        // answer must still settle the card after a lend has begun.
        let room = self.clone();
        let (persona, id) = (persona.to_string(), id.to_string());
        tokio::spawn(async move { room.answer_cookie_ask(&persona, &id, answer, source).await })
            .await
            .map_err(|_| "The answer was interrupted.".to_string())?
    }

    async fn answer_cookie_ask(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: CookieOfferAnswer,
        source: Option<CookieOfferSource>,
    ) -> Result<(), String> {
        let capture = lock(&self.captures).get(id).cloned().ok_or(GONE)?;
        let mut capture = capture.lock().await;
        if capture.persona != persona
            || capture.ask.is_none()
            || capture.status != CredentialStatus::Pending
        {
            return Err(GONE.into());
        }
        if capture.expires_at <= now_ms() || !capture.capability.is_current() {
            self.settle_ask(&mut capture, CookieAskStatus::Expired, None);
            return Err(GONE.into());
        }
        let outcome = match answer {
            CookieOfferAnswer::NotNow => CookieAskStatus::NotNow,
            CookieOfferAnswer::OpenBrowser => {
                // The window opens the pane or the screen; the core only
                // checks the teammate still has the browser it would open.
                let has_computer = self
                    .persona(persona)?
                    .computer
                    .is_some_and(|computer| computer.enabled);
                if !has_computer && !self.offers_browser(persona) {
                    return Err("The teammate has no browser to open now.".into());
                }
                CookieAskStatus::Opened
            }
            CookieOfferAnswer::Use => {
                let (site, chosen) = {
                    let ask = capture.ask.as_ref().expect("checked above");
                    let chosen = match &source {
                        Some(source) => ask.sources.iter().find(|(named, _)| named == source),
                        None => ask.sources.first(),
                    };
                    let chosen = chosen
                        .map(|(_, source)| source.clone())
                        .ok_or("That browser is not signed in to this site.")?;
                    (ask.site.clone(), chosen)
                };
                let capability = capture.capability.clone();
                let lent = self.lend_site(persona, &site, &chosen, &capability).await?;
                capture.ask.as_mut().expect("checked above").lent = Some(lent);
                CookieAskStatus::Cookies
            }
        };
        self.settle_ask(&mut capture, outcome, None);
        Ok(())
    }

    /// Settles a cookie ask: the card folds to its line, the capture is let
    /// go of, and the teammate hears how it came out.
    pub(super) fn settle_ask(
        self: &Arc<Self>,
        capture: &mut Capture,
        outcome: CookieAskStatus,
        stored_as: Option<String>,
    ) {
        let notify_agent = capture.capability.is_current();
        capture.status = match outcome {
            CookieAskStatus::NotNow => CredentialStatus::Declined,
            CookieAskStatus::Expired => CredentialStatus::Expired,
            CookieAskStatus::Withdrawn => CredentialStatus::Withdrawn,
            _ => CredentialStatus::Stored,
        };
        capture.capability.revoke();
        capture
            .ask
            .as_mut()
            .expect("a cookie ask has its parts")
            .status = outcome;
        let ask = capture.ask.as_ref().expect("a cookie ask has its parts");
        let event = ask.event(capture, stored_as.clone());
        let text = ask.told(capture, stored_as.as_deref());
        self.write_home(&capture.persona, &capture.home, &event);
        lock(&self.captures).remove(&capture.id);
        if notify_agent {
            let status = match outcome {
                CookieAskStatus::NotNow => HumanActionStatus::Dismissed,
                CookieAskStatus::Expired => HumanActionStatus::Expired,
                CookieAskStatus::Withdrawn => HumanActionStatus::Withdrawn,
                _ => HumanActionStatus::Done,
            };
            let room = self.clone();
            let (persona, id, home) = (
                capture.persona.clone(),
                capture.id.clone(),
                capture.home.clone(),
            );
            tokio::spawn(async move {
                let _ = room
                    .hand_over_answer(&persona, &id, "Sign-in ask", status, Some(text), home)
                    .await;
            });
        }
    }

    /// Takes back the logins given on a cookie ask without Save to Secrets
    /// once their hour is up: out of the vault and out of the teammate's
    /// grant. The card says when, so a desk that was closed meanwhile does
    /// it at its first sweep.
    pub(crate) async fn forget_unsaved_logins(self: &Arc<Self>, now: i64) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let mut changed = false;
        for persona in room::roster(&self.log) {
            for (mut event, home) in self.cards_of(&persona.id) {
                if event["kind"] != "cookie_ask"
                    || event["keptUntil"].as_i64().is_none_or(|until| until > now)
                {
                    continue;
                }
                let (Some(name), Some(origin)) = (
                    event["storedAs"].as_str().map(str::to_owned),
                    event["origin"].as_str().map(str::to_owned),
                ) else {
                    continue;
                };
                let updates = self.policy_update_lock();
                let held = updates.lock().await;
                // Only the login this card stored: a key of the same name
                // saved since for another site is the person's.
                let stored = vault.shared_secrets().is_ok_and(|secrets| {
                    secrets.iter().any(|secret| {
                        secret.name == name
                            && secret.sites.as_deref() == Some(std::slice::from_ref(&origin))
                    })
                });
                if stored && vault.delete_shared_secret(&name).is_err() {
                    continue;
                }
                if let Ok(mut teammate) = self.persona(&persona.id)
                    && let Some(granted) = teammate.secrets.as_mut()
                    && granted.contains(&name)
                {
                    granted.retain(|granted| *granted != name);
                    teammate.updated_at = now_ms();
                    let _ = room::append_persona(&self.log, &teammate);
                }
                drop(held);
                changed = true;
                event
                    .as_object_mut()
                    .expect("a card is an object")
                    .remove("keptUntil");
                event["ts"] = json!(now_ms());
                if home.kind == ThreadKind::Dm {
                    self.write_value(&persona.id, &event);
                } else {
                    self.threads().write(&home, &persona.id, &event);
                }
            }
        }
        if changed {
            self.secrets_changed().await;
        }
    }
}

/// `github.com`, `https://github.com/pulls` or `gist.github.com` as the
/// registrable domain the cookies are lent by, and the HTTPS origin a login
/// given for it is typed on.
fn site_and_origin(named: &str) -> Option<(String, String)> {
    let named = named.trim();
    let url = if named.contains("://") {
        url::Url::parse(named).ok()?
    } else {
        url::Url::parse(&format!("https://{named}")).ok()?
    };
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    let site = lending::registrable(host)?;
    Some((site, format!("https://{host}")))
}

#[cfg(test)]
impl Room {
    /// The tools the teammate's live session holds, under its own lease, so
    /// a stop ends what they raised.
    pub(crate) fn session_tools(
        self: &Arc<Self>,
        persona_id: &str,
    ) -> crate::mcp::server::TeammateTools {
        crate::mcp::server::TeammateTools::new(self, persona_id)
            .with_capability(self.capability_lease(persona_id))
    }

    pub(crate) fn vault_for_tests(&self) -> Arc<crate::vault::Vault> {
        self.vault.clone().expect("the test gave the room a vault")
    }
}

#[cfg(test)]
mod tests {
    use super::site_and_origin;

    #[test]
    fn a_site_is_named_by_its_registrable_domain_and_a_login_by_its_https_origin() {
        assert_eq!(
            site_and_origin("github.com"),
            Some(("github.com".into(), "https://github.com".into()))
        );
        assert_eq!(
            site_and_origin("https://gist.GitHub.com/me/1"),
            Some(("github.com".into(), "https://gist.github.com".into()))
        );
        for refused in [
            "http://github.com",
            "co.uk",
            "127.0.0.1",
            "",
            "chrome://settings",
        ] {
            assert_eq!(site_and_origin(refused), None, "{refused}");
        }
    }
}
