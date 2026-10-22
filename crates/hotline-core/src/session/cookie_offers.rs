//! The core binds the source, site and live lease before publishing an offer.
//! A saved lend can be removed, but a saved pending card cannot grant access.
use super::*;
use crate::computer::{Ready, cookies::lending, login};
use crate::contract::{
    CookieOffer, CookieOfferAnswer, CookieOfferBlock, CookieOfferSettings, CookieOfferSite,
    CookieOfferSites, CookieOfferSource, CookieOfferStatus,
};

#[derive(Default)]
pub(super) struct CookieWatch {
    ready: Option<Ready>,
    capability: Option<CapabilityLease>,
    seen: HashSet<String>,
    cards: HashMap<String, (CookieOffer, lending::Source)>,
}

impl Room {
    pub(super) async fn look_at_cookie_offers(
        self: &Arc<Self>,
        persona: &str,
        watch: &mut CookieWatch,
    ) {
        // Answers and settings take policy then watch. A poll already owns
        // the watch, so it must skip rather than wait in the opposite order.
        let updates = self.policy_update_lock();
        let Ok(_policy) = updates.try_lock() else {
            return;
        };
        let Ok(settings) = room::cookie_offer_settings(&self.log) else {
            return;
        };
        let _ = self.expire_disabled_cookie_offers(persona, watch, &settings);
        if !settings.enabled {
            return;
        }
        let ready = self.computers.running(persona).await;
        let live = lock(&self.sessions).contains_key(persona);
        if watch.ready != ready
            || watch.capability.as_ref().is_none_or(|c| !c.is_current())
            || !live
        {
            for (card, _) in watch.cards.values_mut() {
                if card.status == CookieOfferStatus::Pending {
                    card.status = CookieOfferStatus::Expired;
                    let _ = self.write_cookie_offer(card);
                }
            }
            watch.cards.clear();
            watch.seen.clear();
            watch.ready.clone_from(&ready);
            watch.capability = live.then(|| self.capability_lease(persona));
        }
        let Some(capability) = watch.capability.clone().filter(CapabilityLease::is_current) else {
            return;
        };
        let Some(ready) = ready else { return };
        let Ok(sites) = lending::pages(&ready).await else {
            return;
        };
        let sites: Vec<_> = sites
            .into_iter()
            .filter(|site| settings.allows(site) && !watch.seen.contains(site))
            .collect();
        if sites.is_empty() {
            return;
        }
        let blocked = self.blocked_cookie_sites(persona);
        let lent = self.computer_cookie_offers(persona);
        let mut sites: Vec<_> = sites
            .into_iter()
            .filter(|site| {
                !blocked.contains(site)
                    && !lent.iter().any(|c| {
                        c.site == *site
                            && matches!(
                                c.status,
                                CookieOfferStatus::Lent | CookieOfferStatus::Unconfirmed
                            )
                    })
            })
            .collect();
        if sites.is_empty() {
            return;
        }
        let mut unique = HashSet::new();
        sites.retain(|site| unique.insert(site.clone()));
        let Ok(inventory) = self.cookie_inventory().await else {
            return;
        };
        let found = sites
            .iter()
            .filter_map(|site| {
                inventory
                    .sources
                    .iter()
                    .find(|(_, sites)| sites.contains(site))
                    .map(|(source, _)| (site.clone(), source.clone()))
            })
            .collect::<Vec<_>>();
        if !capability.is_current() {
            return;
        }
        // A refused discovery says nothing about whether this site has a
        // session. Leave it eligible so the same open page is retried.
        if inventory.refused.is_empty() && inventory.unavailable.is_empty() {
            watch.seen.extend(sites.iter().cloned());
        }
        for browser in inventory.refused {
            if lock(&self.cookie_access_notices).insert(browser.id) {
                self.write(
                    persona,
                    &TranscriptEvent::Notice {
                        id: new_id(),
                        ts: now_ms(),
                        level: NoticeLevel::Warn,
                        text: crate::computer::cookies::access_notice(
                            &browser.name,
                            Some(&sites[0]),
                        ),
                    },
                );
            }
        }
        for (site, source) in found {
            let id = new_id();
            let card = CookieOffer {
                login: source.login(&id),
                id,
                ts: now_ms(),
                teammate: persona.into(),
                site,
                browser_name: source.browser_name().into(),
                profile_name: source.profile_name().into(),
                status: CookieOfferStatus::Pending,
            };
            if self.write_cookie_offer(&card).is_err() {
                continue;
            }
            watch.seen.insert(card.site.clone());
            self.push.notify(&self.needs_you(persona), &format!("Use your existing cookie from {} for {}? The teammate will be signed in as you.", card.browser_name, card.site), persona,
                Some(crate::push::Waiting::CookieOffer { id: card.id.clone() }));
            watch.cards.insert(card.id.clone(), (card, source));
        }
    }

    async fn cookie_inventory(&self) -> Result<lending::Inventory, String> {
        #[cfg(test)]
        let candidates = lock(&self.cookie_candidates).clone();
        #[cfg(test)]
        let calls = self.cookie_discovery_calls.clone();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            #[cfg(not(test))]
            let candidates = crate::computer::cookies::candidates();
            lending::inventory(candidates)
        })
        .await
        .map_err(|_| "Could not discover browser cookie sites.".into())
    }

    pub async fn computer_cookie_sites(&self) -> Result<CookieOfferSites, String> {
        let inventory = self.cookie_inventory().await?;
        let mut sites: std::collections::BTreeMap<String, Vec<CookieOfferSource>> =
            std::collections::BTreeMap::new();
        for (source, found) in inventory.sources {
            for site in found {
                sites.entry(site).or_default().push(CookieOfferSource {
                    browser_name: source.browser_name().into(),
                    profile_name: source.profile_name().into(),
                });
            }
        }
        let mut unavailable = inventory.unavailable;
        unavailable.extend(
            inventory
                .refused
                .iter()
                .map(|browser| crate::computer::cookies::access_notice(&browser.name, None)),
        );
        Ok(CookieOfferSites {
            sites: sites
                .into_iter()
                .map(|(site, sources)| CookieOfferSite { site, sources })
                .collect(),
            unavailable,
        })
    }

    pub async fn cookie_offer_settings_changed(&self) -> Result<(), String> {
        let settings = room::cookie_offer_settings(&self.log)?;
        let watches: Vec<_> = lock(&self.secret_uses)
            .iter()
            .map(|(persona, watch)| (persona.clone(), watch.clone()))
            .collect();
        for (persona, watch) in watches {
            self.expire_disabled_cookie_offers(
                &persona,
                &mut watch.lock().await.cookies,
                &settings,
            )?;
        }
        Ok(())
    }

    fn expire_disabled_cookie_offers(
        &self,
        persona: &str,
        watch: &mut CookieWatch,
        settings: &CookieOfferSettings,
    ) -> Result<(), String> {
        let mut expired = false;
        for (card, _) in watch.cards.values_mut() {
            if card.status == CookieOfferStatus::Pending && !settings.allows(&card.site) {
                card.status = CookieOfferStatus::Expired;
                self.write_cookie_offer(card)?;
                watch.seen.remove(&card.site);
                expired = true;
            }
        }
        if expired {
            let text = if settings.enabled {
                "Browser cookie offers for these sites are off. Pending offers have expired."
            } else {
                "Browser cookie offers are off. Pending offers have expired."
            };
            self.write(
                persona,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Info,
                    text: text.into(),
                },
            );
        }
        Ok(())
    }

    fn write_cookie_offer(&self, offer: &CookieOffer) -> Result<(), String> {
        self.log
            .append(
                &StreamId::Tape(offer.teammate.clone()),
                &serde_json::to_value(TranscriptEvent::CookieOffer {
                    offer: offer.clone(),
                })
                .map_err(|_| "Could not record cookie offer.")?,
            )
            .map(|_| ())
            .map_err(|_| "Could not record cookie offer.".into())
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

    fn blocked_cookie_sites(&self, persona: &str) -> Vec<String> {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .find(|e| e["kind"] == "cookie_offer_never" && e["teammate"] == persona)
            .and_then(|e| serde_json::from_value(e["sites"].clone()).ok())
            .unwrap_or_default()
    }

    pub fn computer_cookie_blocked(&self) -> Vec<CookieOfferBlock> {
        room::roster(&self.log)
            .into_iter()
            .flat_map(|p| {
                self.blocked_cookie_sites(&p.id)
                    .into_iter()
                    .map(move |site| CookieOfferBlock {
                        teammate: p.id.clone(),
                        name: p.name.clone(),
                        site,
                    })
            })
            .collect()
    }

    fn record_cookie_blocks(&self, persona: &str, sites: &[String]) -> Result<(), String> {
        self.log.append(&StreamId::Room, &serde_json::json!({"id":format!("cookie-offer-never:{persona}"),"kind":"cookie_offer_never","ts":now_ms(),"teammate":persona,"sites":sites}))
            .map(|_| ()).map_err(|_| "Could not save cookie offer preference.".into())
    }

    pub async fn computer_cookie_reset(&self, persona: &str, site: &str) -> Result<(), String> {
        self.persona(persona)?;
        let watch = self.use_watch(persona);
        let mut watch = watch.lock().await;
        let mut sites = self.blocked_cookie_sites(persona);
        sites.retain(|s| s != site);
        self.record_cookie_blocks(persona, &sites)?;
        watch.cookies.seen.remove(site);
        Ok(())
    }

    pub async fn computer_cookie_answer(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: CookieOfferAnswer,
    ) -> Result<(), String> {
        let room = self.clone();
        let (persona, id) = (persona.to_string(), id.to_string());
        tokio::spawn(async move { room.answer_cookie_offer(&persona, &id, answer).await })
            .await
            .map_err(|_| "The cookie answer was interrupted.".to_string())?
    }

    async fn answer_cookie_offer(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
        answer: CookieOfferAnswer,
    ) -> Result<(), String> {
        let updates = self.policy_update_lock();
        let policy_guard = updates.lock().await;
        let watch = self.use_watch(persona);
        let mut held_watch = watch.lock().await;
        let watch = &mut held_watch.cookies;
        let settings = room::cookie_offer_settings(&self.log)?;
        self.expire_disabled_cookie_offers(persona, watch, &settings)?;
        let capability = watch
            .capability
            .clone()
            .ok_or("This cookie offer expired.")?;
        let ready = watch.ready.clone().ok_or("This cookie offer expired.")?;
        let (card, source) = watch
            .cards
            .get_mut(id)
            .ok_or("This cookie offer expired.")?;
        if card.status != CookieOfferStatus::Pending {
            return Err("This cookie offer was already answered.".into());
        }
        if !capability.is_current()
            || self.computers.running(persona).await.as_ref() != Some(&ready)
        {
            card.status = CookieOfferStatus::Expired;
            self.write_cookie_offer(card)?;
            return Err("This cookie offer expired.".into());
        }
        match answer {
            CookieOfferAnswer::NotNow => card.status = CookieOfferStatus::Declined,
            CookieOfferAnswer::Never => {
                let mut blocked = self.blocked_cookie_sites(persona);
                if !blocked.contains(&card.site) {
                    blocked.push(card.site.clone());
                }
                self.record_cookie_blocks(persona, &blocked)?;
                card.status = CookieOfferStatus::Never;
            }
            CookieOfferAnswer::Use => {
                let cookies = source.read(&card.site).await?;
                capability.check()?;
                if self.computers.running(persona).await.as_ref() != Some(&ready) {
                    return Err("This computer changed; the cookie offer expired.".into());
                }
                if cookies.is_empty() {
                    return Err("This browser no longer has live cookies for the site.".into());
                }
                let saved = serde_json::json!({"name":card.login,"browser":card.browser_name,"created_at":chrono::Utc::now().to_rfc3339(),"cookies":cookies,"storage":{}});
                // Neither browser nor computer error bodies are trusted sinks:
                // either endpoint could echo a value it was handed.
                // Save the removal handle before any bytes leave: after a
                // crash the person must still be able to take the lend back.
                card.status = CookieOfferStatus::Unconfirmed;
                if let Err(error) = self.write_cookie_offer(card) {
                    card.status = CookieOfferStatus::Pending;
                    return Err(error);
                }
                let delivered = login::deliver(&ready, &card.login, &saved).await;
                if delivered.is_err() || !capability.is_current() {
                    if login::forget(&ready, &card.login, std::slice::from_ref(&card.site))
                        .await
                        .is_ok()
                    {
                        card.status = if capability.is_current() {
                            CookieOfferStatus::Pending
                        } else {
                            CookieOfferStatus::Expired
                        };
                        self.write_cookie_offer(card)?;
                        return Err(
                            "Could not lend the cookies. The container copy was removed.".into(),
                        );
                    }
                    return Err("Sign-in could not be confirmed. Use Remove to take the container copy back.".into());
                }
                card.status = CookieOfferStatus::Lent;
            }
        }
        if let Err(error) = self.write_cookie_offer(card) {
            if card.status == CookieOfferStatus::Lent {
                let _ = login::forget(&ready, &card.login, std::slice::from_ref(&card.site)).await;
                // The durable record is still unconfirmed and removable.
                card.status = CookieOfferStatus::Unconfirmed;
            }
            return Err(error);
        }
        let card = card.clone();
        drop(held_watch);
        drop(policy_guard);
        if card.status == CookieOfferStatus::Lent {
            self.tell_cookie_outcome(&card, false).await;
        }
        Ok(())
    }

    pub async fn computer_cookie_remove(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
    ) -> Result<(), String> {
        let room = self.clone();
        let (persona, id) = (persona.to_string(), id.to_string());
        tokio::spawn(async move { room.remove_cookie_offer(&persona, &id).await })
            .await
            .map_err(|_| "The cookie removal was interrupted.".to_string())?
    }

    async fn remove_cookie_offer(self: &Arc<Self>, persona: &str, id: &str) -> Result<(), String> {
        let updates = self.policy_update_lock();
        let policy_guard = updates.lock().await;
        let watch = self.use_watch(persona);
        let mut watch = watch.lock().await;
        let mut card = self
            .computer_cookie_offers(persona)
            .into_iter()
            .find(|c| {
                c.id == id
                    && matches!(
                        c.status,
                        CookieOfferStatus::Lent | CookieOfferStatus::Unconfirmed
                    )
            })
            .ok_or("This cookie lend is no longer present.")?;
        let teammate = self.persona(persona)?;
        if !teammate.computer.as_ref().is_some_and(|c| c.enabled) {
            return Err("Enable the teammate's computer to remove this lend.".into());
        }
        let settings = room::settings(&self.log);
        let ready = self
            .computers
            .ensure_running(
                &teammate,
                &teammate.cwd,
                crate::computer::preferred_runtime(&settings),
                crate::computer::preferred_image(&settings).as_deref(),
                self.pull_reporter(persona),
            )
            .await?;
        login::forget(&ready, &card.login, std::slice::from_ref(&card.site)).await.map_err(|_| "The computer could not remove this lend. Check that it is up to date, then retry.")?;
        card.status = CookieOfferStatus::Removed;
        self.write_cookie_offer(&card)?;
        if let Some((live, _)) = watch.cookies.cards.get_mut(id) {
            *live = card.clone();
        }
        drop(watch);
        drop(policy_guard);
        self.tell_cookie_outcome(&card, true).await;
        Ok(())
    }

    async fn tell_cookie_outcome(self: &Arc<Self>, card: &CookieOffer, removed: bool) {
        let text = if removed {
            format!(
                "The person took back the session on {} from {}. Its cookies were removed from your computer. Stop using that sign-in.",
                card.site, card.browser_name
            )
        } else {
            format!(
                "The person lent their {} cookies for {}. Your computer is signed in as them; you may reload the page.",
                card.browser_name, card.site
            )
        };
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
