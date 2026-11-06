//! A lend: one site's cookies copied from a browser of the person's into a
//! teammate's browser, only because the person answered the teammate's ask
//! (`cookie_asks.rs`). The record is written before any bytes leave, so a
//! lend can always be listed and removed; it never carries a cookie value.
//! A teammate with a computer borrows into the computer; one with the desk
//! browser instead borrows into that browser's own web storage.
use super::*;
use crate::browser::DeskBrowser;
use crate::computer::{Ready, cookies::lending, login};
use crate::contract::{CookieOffer, CookieOfferSites, CookieOfferStatus};

/// Where a lend goes, and is taken back from.
enum Sink {
    Computer(Ready),
    Desk(Arc<dyn DeskBrowser>),
}

impl Sink {
    async fn put(
        &self,
        persona: &str,
        card: &CookieOffer,
        cookies: Vec<Value>,
    ) -> Result<(), String> {
        match self {
            Self::Computer(ready) => {
                let saved = serde_json::json!({"name":card.login,"browser":card.browser_name,"created_at":chrono::Utc::now().to_rfc3339(),"cookies":cookies,"storage":{}});
                login::deliver(ready, &card.login, &saved).await
            }
            Self::Desk(browser) => browser.lend_cookies(persona, &cookies).await,
        }
    }

    async fn take_back(&self, persona: &str, card: &CookieOffer) -> Result<(), String> {
        match self {
            Self::Computer(ready) => {
                login::forget(ready, &card.login, std::slice::from_ref(&card.site))
                    .await
                    .map(|_| ())
            }
            Self::Desk(browser) => browser.forget_cookies(persona, &card.site).await,
        }
    }

    /// Where the copy is, as the person is told it was removed.
    fn place(&self) -> &'static str {
        match self {
            Self::Computer(_) => "container",
            Self::Desk(_) => "browser",
        }
    }
}

impl Room {
    /// The site names in every switched-on source: a source the person
    /// switched off in Settings › Secrets is not discovered, nor read.
    pub(super) async fn cookie_inventory(&self) -> Result<lending::Inventory, String> {
        let candidates = self.host_candidates();
        let gate = self.cookie_lending.gate.clone();
        #[cfg(test)]
        let calls = self.cookie_discovery_calls.clone();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            lending::inventory(candidates, &gate)
        })
        .await
        .map_err(|_| "Could not discover browser cookie sites.".into())
    }

    pub async fn computer_cookie_sites(&self) -> Result<CookieOfferSites, String> {
        self.cookie_lendable().await
    }

    pub(super) fn write_cookie_offer(&self, offer: &CookieOffer) -> Result<(), String> {
        self.log
            .append(
                &StreamId::Tape(offer.teammate.clone()),
                &serde_json::to_value(TranscriptEvent::CookieOffer {
                    offer: offer.clone(),
                })
                .map_err(|_| "Could not record the shared cookies.")?,
            )
            .map(|_| ())
            .map_err(|_| "Could not record the shared cookies.".into())
    }

    pub fn computer_cookie_offers(&self, persona: &str) -> Vec<CookieOffer> {
        self.tape(persona)
            .into_iter()
            .filter_map(|event| {
                if event["kind"] != "cookie_offer" {
                    return None;
                }
                serde_json::from_value(event).ok()
            })
            .collect()
    }

    /// Copies `site`'s cookies from `source` into the teammate's browser as
    /// it is now: its running computer, or the desk browser it has with no
    /// computer. The record is saved unconfirmed before any bytes leave, so
    /// a desk that stops mid-copy still leaves the person a Remove; a copy
    /// that fails is taken back. Answers the record, lent.
    pub(super) async fn lend_site(
        self: &Arc<Self>,
        persona: &str,
        site: &str,
        source: &lending::Source,
        capability: &CapabilityLease,
    ) -> Result<CookieOffer, String> {
        let updates = self.policy_update_lock();
        let policy_guard = updates.lock().await;
        let watch = self.use_watch(persona);
        let held_watch = watch.lock().await;
        capability.check()?;
        let ready = self.computers.running(persona).await;
        let sink = match &ready {
            Some(ready) => Sink::Computer(ready.clone()),
            None => Sink::Desk(
                self.lending_browser(persona)
                    .ok_or("The teammate's computer is off. Start it, then answer again.")?,
            ),
        };
        let cookies = source.read(site).await?;
        capability.check()?;
        if cookies.is_empty() {
            return Err("This browser no longer has live cookies for the site.".into());
        }
        let id = new_id();
        let mut card = CookieOffer {
            login: source.login(&id),
            id,
            ts: now_ms(),
            teammate: persona.into(),
            site: site.into(),
            browser_name: source.browser_name().into(),
            profile_name: source.profile_name().into(),
            status: CookieOfferStatus::Unconfirmed,
            desk_browser: ready.is_none(),
        };
        self.write_cookie_offer(&card)?;
        // Neither browser nor computer error bodies are trusted sinks:
        // either endpoint could echo a value it was handed.
        let delivered = sink.put(persona, &card, cookies).await;
        let place = sink.place();
        if delivered.is_err()
            || !capability.is_current()
            || self.computers.running(persona).await != ready
        {
            if sink.take_back(persona, &card).await.is_ok() {
                card.status = CookieOfferStatus::Expired;
                self.write_cookie_offer(&card)?;
                return Err(format!(
                    "Could not lend the cookies. The {place} copy was removed."
                ));
            }
            return Err(format!(
                "Sign-in could not be confirmed. Use Remove to take the {place} copy back."
            ));
        }
        card.status = CookieOfferStatus::Lent;
        if let Err(error) = self.write_cookie_offer(&card) {
            // The durable record is still unconfirmed and removable.
            let _ = sink.take_back(persona, &card).await;
            return Err(error);
        }
        drop(held_watch);
        drop(policy_guard);
        if card.desk_browser {
            self.browser_reload(persona).await;
        }
        Ok(card)
    }

    pub(super) async fn tell_cookie_outcome(self: &Arc<Self>, card: &CookieOffer, removed: bool) {
        let text = cookie_outcome(card, removed);
        let id = format!("{}:{}", card.id, if removed { "removed" } else { "lent" });
        let _ = self
            .hand_over_answer(
                &card.teammate,
                &id,
                "Browser session",
                HumanActionStatus::Done,
                Some(text),
                ThreadId::dm(&card.teammate),
            )
            .await;
    }
}

/// What the teammate is told of a lend or its removal: the site and the
/// browser, never a cookie.
pub(super) fn cookie_outcome(card: &CookieOffer, removed: bool) -> String {
    match (removed, card.desk_browser) {
        (true, false) => format!(
            "The person took back the session on {} from {}. Its cookies were removed from your computer. Stop using that sign-in.",
            card.site, card.browser_name
        ),
        (true, true) => format!(
            "The person took back the session on {} from {}. Its cookies were removed from your browser, and the page reloaded. Stop using that sign-in.",
            card.site, card.browser_name
        ),
        (false, false) => format!(
            "The person shared their {} cookies for {}. Your computer is signed in as them; you may reload the page.",
            card.browser_name, card.site
        ),
        (false, true) => format!(
            "The person shared their {} cookies for {}. Your browser is signed in as them, and the page reloaded: read it again.",
            card.browser_name, card.site
        ),
    }
}
