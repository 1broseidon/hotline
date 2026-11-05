//! The person's cookies shared with a teammate's browser (epic-34): one list
//! in its pane for both browsers, each share a row with Keep fresh, Ends and
//! Remove, and the browser profiles Hotline may read at all.
//!
//! A share is a record on the room stream (`cookie_lend`), named by the
//! site, the source it came from and where its copy belongs: the teammate's
//! desk browser storage, or its computer's saved login. The record is saved
//! before any value leaves, so a share can always be taken back. Two older
//! shapes are read as shares too, without being rewritten until something
//! about them changes: a card answered Use (its `cookie_offer` on the tape)
//! and a site brought over by the import the pane had before (the room's
//! `cookie-imports` record).
//!
//! The copy follows the teammate: its computer turned on, a share is taken
//! out of the desk browser's storage and copied into the computer when it
//! is next up; turned off, it is taken out of the computer while it can be
//! reached and copied back into the desk browser. A copy is always read
//! again from its source, never carried between the two browsers, so a
//! source the person switched off in Settings stops it moving.
use super::*;
use crate::computer::{Ready, cookies::lending, login};
use crate::contract::{
    CookieHolder, CookieLend, CookieLendEnds, CookieOffer, CookieOfferSite, CookieOfferSites,
    CookieOfferSource, CookieOfferStatus, CookieSource, CookieSources,
};
use serde::{Deserialize, Serialize};

/// How long Ends' "1 day" lasts, counted from the press.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// How often a kept-fresh share looks at its source for a change.
const KEEP_FRESH_EVERY_MS: i64 = 60 * 1000;
/// How long a copy that could not be read is left before it is tried again,
/// so a browser that refuses or a keychain that asks is not hammered.
const RETRY_EVERY_MS: i64 = 5 * 60 * 1000;
/// How often the room looks at its shares: ends due, kept-fresh sources,
/// copies waiting on a computer.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// The room record of the switched-off sources.
const SOURCES_ID: &str = "cookie-sources";

/// What the room keeps for sharing, beside its stream.
#[derive(Default)]
pub(super) struct Lending {
    /// The profiles switched off, as every discovered source checks them.
    pub(super) gate: lending::Gate,
    /// One share change at a time per teammate.
    locks: Mutex<HashMap<String, Arc<TokioMutex<()>>>>,
    /// Each kept-fresh share's source, as it last read: its fingerprint
    /// and when. Kept in memory only, so a restart copies once again.
    fresh: Mutex<HashMap<String, (String, i64)>>,
    /// When a copy that could not be made was last tried.
    tried: Mutex<HashMap<String, i64>>,
}

/// One share, as the room records it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lend {
    id: String,
    teammate: String,
    site: String,
    /// The source's ids; absent for a card answered before they were kept,
    /// which is found again by its names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_id: Option<String>,
    browser_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    profile_name: String,
    /// The computer's saved login the copy goes into.
    login: String,
    lent_at: i64,
    #[serde(default)]
    keep_fresh: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ends_at: Option<i64>,
    /// The exact cookie domains an import named, said to the computer when
    /// it is taken back; a share names its site alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    /// Its copy belongs in the desk browser's storage, not the computer.
    desk_browser: bool,
    /// The copy is where it belongs.
    copied: bool,
    /// A copy may be in the computer's saved login and browser, to take
    /// out when the computer can be reached.
    in_computer: bool,
    #[serde(default)]
    removed: bool,
    /// Made by a card answered Use: removing it settles the card too.
    #[serde(default)]
    card: bool,
    /// Brought from a laptop to a desk on a server: no source here.
    #[serde(default)]
    from_laptop: bool,
}

impl Lend {
    fn from_card(card: &CookieOffer) -> Self {
        Self {
            id: card.id.clone(),
            teammate: card.teammate.clone(),
            site: card.site.clone(),
            browser_id: None,
            browser_name: card.browser_name.clone(),
            profile_id: None,
            profile_name: card.profile_name.clone(),
            login: card.login.clone(),
            lent_at: card.ts,
            keep_fresh: false,
            ends_at: None,
            domains: Vec::new(),
            desk_browser: card.desk_browser,
            // An unconfirmed card is mid-delivery or was cut off in it; its
            // answer is the card's to finish, not a copy to make again.
            copied: true,
            in_computer: !card.desk_browser,
            removed: false,
            card: true,
            from_laptop: false,
        }
    }

    fn wire(&self) -> CookieLend {
        CookieLend {
            id: self.id.clone(),
            teammate: self.teammate.clone(),
            site: self.site.clone(),
            browser_name: self.browser_name.clone(),
            profile_name: self.profile_name.clone(),
            lent_at: self.lent_at,
            keep_fresh: self.keep_fresh,
            ends_at: self.ends_at,
            waiting: !self.copied,
            from_laptop: self.from_laptop,
        }
    }

    /// What the computer is told to forget.
    fn forget_domains(&self) -> Vec<String> {
        if self.domains.is_empty() {
            vec![self.site.clone()]
        } else {
            self.domains.clone()
        }
    }

    fn saved_login(&self, cookies: Vec<Value>) -> Value {
        json!({
            "name": self.login,
            "browser": self.browser_name,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "cookies": cookies,
            "storage": {},
        })
    }
}

/// The sources the room's record switches off.
fn sources_off(log: &Log) -> HashSet<String> {
    log.load(&StreamId::Room)
        .into_iter()
        .rev()
        .find(|event| event["kind"] == "cookie_sources" && event["id"] == SOURCES_ID)
        .and_then(|event| serde_json::from_value(event["off"].clone()).ok())
        .unwrap_or_default()
}

/// The room's gate from its record, and the watch over its shares.
pub(super) fn start(room: &Arc<Room>) {
    room.cookie_lending.gate.replace(sources_off(&room.log));
    let room = Arc::downgrade(room);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(WATCH_EVERY).await;
            let Some(room) = room.upgrade() else { return };
            room.look_at_cookie_lends(false).await;
        }
    });
}

impl Room {
    /// The browsers this desk could read: the host's, or a test's fixtures,
    /// so no test ever opens the machine's own.
    #[cfg(not(test))]
    pub(super) fn host_candidates(&self) -> Vec<crate::computer::cookies::Candidate> {
        crate::computer::cookies::candidates()
    }

    #[cfg(test)]
    pub(super) fn host_candidates(&self) -> Vec<crate::computer::cookies::Candidate> {
        lock(&self.cookie_candidates).clone()
    }

    fn lend_lock(&self, persona: &str) -> Arc<TokioMutex<()>> {
        lock(&self.cookie_lending.locks)
            .entry(persona.into())
            .or_default()
            .clone()
    }

    /// Every share the room records, removed ones too.
    fn lend_records(&self) -> Vec<Lend> {
        self.log
            .load(&StreamId::Room)
            .into_iter()
            .filter(|event| event["kind"] == "cookie_lend")
            .filter_map(|event| serde_json::from_value(event["lend"].clone()).ok())
            .collect()
    }

    fn write_lend(&self, lend: &Lend) -> Result<(), String> {
        self.log
            .append(
                &StreamId::Room,
                &json!({
                    "id": format!("cookie-lend:{}", lend.id),
                    "kind": "cookie_lend",
                    "ts": now_ms(),
                    "lend": lend,
                }),
            )
            .map(|_| ())
            .map_err(|_| "Could not save the shared cookies' record.".into())
    }

    /// A teammate's shares, removed ones too: the room's records, then the
    /// cards answered Use and the sites an import brought over that no
    /// record has replaced yet.
    fn lends_of(&self, persona: &str) -> Vec<Lend> {
        let mut lends: Vec<Lend> = self
            .lend_records()
            .into_iter()
            .filter(|lend| lend.teammate == persona)
            .collect();
        let known: HashSet<String> = lends.iter().map(|lend| lend.id.clone()).collect();
        for card in self.computer_cookie_offers(persona) {
            if matches!(
                card.status,
                CookieOfferStatus::Lent | CookieOfferStatus::Unconfirmed
            ) && !known.contains(&card.id)
            {
                lends.push(Lend::from_card(&card));
            }
        }
        for import in room::cookie_imports(&self.log, persona) {
            let mut by_site: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for site in &import.sites {
                let key = lending::registrable(&site.domain).unwrap_or_else(|| site.domain.clone());
                by_site.entry(key).or_default().push(site.domain.clone());
            }
            for (site, domains) in by_site {
                let id = format!("import:{}:{}:{site}", import.browser_id, import.profile_id);
                if known.contains(&id) {
                    continue;
                }
                lends.push(Lend {
                    id,
                    teammate: persona.into(),
                    site,
                    browser_id: Some(import.browser_id.clone()),
                    browser_name: import.browser_name.clone(),
                    profile_id: Some(import.profile_id.clone()),
                    profile_name: import.profile_name.clone(),
                    login: login::name_for(&import.browser_id, &import.profile_id),
                    lent_at: import.imported_at,
                    keep_fresh: false,
                    ends_at: None,
                    domains,
                    desk_browser: false,
                    copied: true,
                    in_computer: true,
                    removed: false,
                    card: false,
                    from_laptop: import.browser_id.starts_with("laptop-"),
                });
            }
        }
        lends.sort_by_key(|lend| lend.lent_at);
        lends
    }

    fn active_lend(&self, persona: &str, id: &str) -> Result<Lend, String> {
        self.lends_of(persona)
            .into_iter()
            .find(|lend| lend.id == id && !lend.removed)
            .ok_or_else(|| "These cookies are no longer shared.".into())
    }

    /// The person's cookies shared with a teammate, a row a site.
    pub fn cookie_lends(&self, persona: &str) -> Vec<CookieLend> {
        self.lends_of(persona)
            .iter()
            .filter(|lend| !lend.removed)
            .map(Lend::wire)
            .collect()
    }

    /// Every teammate holding the person's cookies, with the sites.
    pub fn cookie_holders(&self) -> Vec<CookieHolder> {
        room::roster(&self.log)
            .into_iter()
            .filter_map(|persona| {
                let mut sites: Vec<String> = self
                    .lends_of(&persona.id)
                    .into_iter()
                    .filter(|lend| !lend.removed)
                    .map(|lend| lend.site)
                    .collect();
                sites.sort();
                sites.dedup();
                (!sites.is_empty()).then_some(CookieHolder {
                    teammate: persona.id,
                    name: persona.name,
                    sites,
                })
            })
            .collect()
    }

    /// The sites the person is signed into across the switched-on sources,
    /// with where each is from. Site names only; read when Add Cookies
    /// opens, never when the pane does.
    pub async fn cookie_lendable(&self) -> Result<CookieOfferSites, String> {
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

    /// Shares one site's cookies with the teammate's browser, from the
    /// first switched-on source signed into it. The record is saved before
    /// a value leaves; a computer that is not up has the copy made when it
    /// next starts.
    pub async fn cookie_lend(
        self: &Arc<Self>,
        persona: &str,
        site: &str,
    ) -> Result<CookieLend, String> {
        let room = self.clone();
        let (persona, site) = (persona.to_string(), site.to_string());
        tokio::spawn(async move { room.lend_site(&persona, &site).await })
            .await
            .map_err(|_| "Sharing the cookies was interrupted.".to_string())?
    }

    async fn lend_site(&self, persona: &str, site: &str) -> Result<CookieLend, String> {
        let teammate = self.persona(persona)?;
        let site = lending::registrable(site.trim()).ok_or("That is not a site.")?;
        let desk_browser = !teammate.computer.as_ref().is_some_and(|c| c.enabled);
        if desk_browser && self.lending_browser(persona).is_none() {
            return Err(format!(
                "{} has no browser to share cookies with.",
                teammate.name
            ));
        }
        let held = self.lend_lock(persona);
        let _held = held.lock().await;
        if self
            .lends_of(persona)
            .iter()
            .any(|lend| !lend.removed && lend.site == site)
        {
            return Err(format!("{site} is already shared with {}.", teammate.name));
        }
        let inventory = self.cookie_inventory().await?;
        let source = inventory
            .sources
            .into_iter()
            .find(|(_, sites)| sites.contains(&site))
            .map(|(source, _)| source)
            .ok_or_else(|| {
                format!("No browser switched on in Settings › Secrets is signed in to {site}.")
            })?;
        let cookies = source.read(&site).await?;
        if cookies.is_empty() {
            return Err("This browser no longer has live cookies for the site.".into());
        }
        let id = new_id();
        let mut lend = Lend {
            login: source.login(&id),
            id,
            teammate: persona.into(),
            site,
            browser_id: Some(source.browser_id().into()),
            browser_name: source.browser_name().into(),
            profile_id: Some(source.profile_id().into()),
            profile_name: source.profile_name().into(),
            lent_at: now_ms(),
            keep_fresh: false,
            ends_at: None,
            domains: Vec::new(),
            desk_browser,
            copied: false,
            in_computer: false,
            removed: false,
            card: false,
            from_laptop: false,
        };
        // The removal handle first: after a crash mid-copy, Remove still works.
        self.write_lend(&lend)?;
        let ready = self.computers.running(persona).await;
        if let Err(error) = self.put_copy(&mut lend, cookies, ready.as_ref()).await {
            // Part of it may have landed: take out whatever did.
            lend.copied = true;
            let _ = self.take_copy_out(&mut lend, ready.as_ref()).await;
            lend.removed = true;
            self.write_lend(&lend)?;
            return Err(error);
        }
        self.write_lend(&lend)?;
        if lend.desk_browser && lend.copied {
            self.browser_reload(persona).await;
        }
        Ok(lend.wire())
    }

    /// Puts a copy where the share belongs, when that browser can take it
    /// now: the desk browser's storage whenever the teammate has it, the
    /// computer only while it runs. Error bodies are not passed on: either
    /// end could echo a value it was handed.
    async fn put_copy(
        &self,
        lend: &mut Lend,
        cookies: Vec<Value>,
        ready: Option<&Ready>,
    ) -> Result<(), String> {
        if lend.desk_browser {
            let Some(browser) = self.lending_browser(&lend.teammate) else {
                return Ok(());
            };
            browser
                .lend_cookies(&lend.teammate, &cookies)
                .await
                .map_err(|_| "The teammate's browser could not take the cookies.".to_string())?;
        } else {
            let Some(ready) = ready else {
                return Ok(());
            };
            // In the computer from the moment it is asked, so a failure
            // still takes it out.
            lend.in_computer = true;
            login::deliver(ready, &lend.login, &lend.saved_login(cookies))
                .await
                .map_err(|_| "The computer could not take the cookies.".to_string())?;
        }
        lend.copied = true;
        Ok(())
    }

    /// Takes a share's copies out of the browsers that may hold one: the
    /// desk browser's storage, open or not, and the computer while it runs.
    /// A computer that is down keeps `in_computer` and is cleared when it
    /// next starts, before the teammate is handed it.
    async fn take_copy_out(&self, lend: &mut Lend, ready: Option<&Ready>) -> Result<(), String> {
        if lend.desk_browser && lend.copied {
            let browser = self.desk_browser().ok_or(
                "Open Hotline's window to remove these cookies from the teammate's browser.",
            )?;
            browser
                .forget_cookies(&lend.teammate, &lend.site)
                .await
                .map_err(|_| "The teammate's browser could not remove these cookies. Retry.")?;
            lend.copied = false;
        }
        if lend.in_computer
            && let Some(ready) = ready
        {
            login::forget(ready, &lend.login, &lend.forget_domains())
                .await
                .map_err(|_| "The computer could not remove these cookies. Check that it is up to date, then retry.")?;
            lend.in_computer = false;
            if !lend.desk_browser {
                lend.copied = false;
            }
        }
        Ok(())
    }

    /// Changes a share's options: Keep fresh, and when it ends.
    pub async fn cookie_lend_set(
        &self,
        persona: &str,
        id: &str,
        keep_fresh: Option<bool>,
        ends: Option<CookieLendEnds>,
    ) -> Result<CookieLend, String> {
        let held = self.lend_lock(persona);
        let _held = held.lock().await;
        let mut lend = self.active_lend(persona, id)?;
        if let Some(keep_fresh) = keep_fresh {
            if keep_fresh && lend.from_laptop {
                return Err("Keep fresh needs the browser to be on this desk.".into());
            }
            lend.keep_fresh = keep_fresh;
            // Looked at afresh on the next pass, which copies once.
            lock(&self.cookie_lending.fresh).remove(&lend.id);
        }
        match ends {
            Some(CookieLendEnds::Day) => lend.ends_at = Some(now_ms() + DAY_MS),
            Some(CookieLendEnds::Never) => lend.ends_at = None,
            None => {}
        }
        self.write_lend(&lend)?;
        Ok(lend.wire())
    }

    /// Takes a share back: out of the browser it is in, and off the list.
    pub async fn cookie_lend_remove(
        self: &Arc<Self>,
        persona: &str,
        id: &str,
    ) -> Result<(), String> {
        let room = self.clone();
        let (persona, id) = (persona.to_string(), id.to_string());
        tokio::spawn(async move { room.remove_lend(&persona, &id).await })
            .await
            .map_err(|_| "Removing the cookies was interrupted.".to_string())?
    }

    async fn remove_lend(self: &Arc<Self>, persona: &str, id: &str) -> Result<(), String> {
        self.persona(persona)?;
        let held = self.lend_lock(persona);
        let guard = held.lock().await;
        let mut lend = self.active_lend(persona, id)?;
        let reload = lend.desk_browser && lend.copied;
        let ready = self.computers.running(persona).await;
        self.take_copy_out(&mut lend, ready.as_ref()).await?;
        lend.removed = true;
        self.write_lend(&lend)?;
        lock(&self.cookie_lending.fresh).remove(&lend.id);
        lock(&self.cookie_lending.tried).remove(&lend.id);
        drop(guard);
        if reload {
            self.browser_reload(persona).await;
        }
        if lend.card
            && let Some(mut card) = self
                .computer_cookie_offers(persona)
                .into_iter()
                .find(|card| card.id == lend.id)
        {
            card.status = CookieOfferStatus::Removed;
            self.write_cookie_offer(&card)?;
            self.tell_cookie_outcome(&card, true).await;
        }
        Ok(())
    }

    /// The teammate's computer was turned on or off: its shares move to the
    /// browser it has now. Into the computer, the desk browser's copy goes
    /// at once and the computer's is made when it is up; out of it, the
    /// computer's copy goes while it can be reached and the desk browser's
    /// is made now. The rows stay as they are.
    pub(super) async fn cookie_lends_follow(&self, persona: &str) {
        let Ok(teammate) = self.persona(persona) else {
            return;
        };
        let desk_browser = !teammate.computer.as_ref().is_some_and(|c| c.enabled);
        let ready = self.computers.running(persona).await;
        {
            let held = self.lend_lock(persona);
            let _held = held.lock().await;
            for mut lend in self.lends_of(persona) {
                if lend.removed || lend.from_laptop || lend.desk_browser == desk_browser {
                    continue;
                }
                if lend.desk_browser {
                    if lend.copied {
                        let Some(browser) = self.desk_browser() else {
                            continue;
                        };
                        if browser.forget_cookies(persona, &lend.site).await.is_err() {
                            eprintln!(
                                "[cookies] a share stayed in the desk browser; it moves on the next change"
                            );
                            continue;
                        }
                    }
                } else if lend.in_computer
                    && let Some(ready) = &ready
                    && login::forget(ready, &lend.login, &lend.forget_domains())
                        .await
                        .is_ok()
                {
                    lend.in_computer = false;
                }
                lend.desk_browser = desk_browser;
                lend.copied = false;
                lock(&self.cookie_lending.tried).remove(&lend.id);
                if self.write_lend(&lend).is_err() {
                    eprintln!("[cookies] a share's move could not be recorded");
                }
            }
        }
        self.copy_waiting(persona, ready.as_ref(), true).await;
    }

    /// The computer is up and about to be handed to the teammate: copies
    /// taken back while it was down go, and shares that belong in it are
    /// copied in, before the teammate can use its browser.
    pub(super) async fn cookie_lends_at_computer(&self, persona: &str, ready: &Ready) {
        self.copy_waiting(persona, Some(ready), true).await;
    }

    /// Clears what a computer should no longer hold, and copies each share
    /// not yet where it belongs, reading its source again.
    async fn copy_waiting(&self, persona: &str, ready: Option<&Ready>, force: bool) {
        let held = self.lend_lock(persona);
        let _held = held.lock().await;
        let mut reload = false;
        for mut lend in self.lends_of(persona) {
            let mut changed = false;
            if lend.in_computer
                && (lend.removed || lend.desk_browser)
                && let Some(ready) = ready
                && login::forget(ready, &lend.login, &lend.forget_domains())
                    .await
                    .is_ok()
            {
                lend.in_computer = false;
                changed = true;
            }
            let reachable = if lend.desk_browser {
                self.lending_browser(persona).is_some()
            } else {
                ready.is_some()
            };
            let due = force
                || lock(&self.cookie_lending.tried)
                    .get(&lend.id)
                    .is_none_or(|at| now_ms() - at >= RETRY_EVERY_MS);
            if !lend.removed && !lend.copied && !lend.from_laptop && reachable && due {
                match self.read_source(&lend).await {
                    Ok(cookies) => match self.put_copy(&mut lend, cookies, ready).await {
                        Ok(()) => {
                            lock(&self.cookie_lending.tried).remove(&lend.id);
                            reload |= lend.desk_browser && lend.copied;
                        }
                        Err(_) => {
                            lock(&self.cookie_lending.tried).insert(lend.id.clone(), now_ms());
                        }
                    },
                    Err(_) => {
                        lock(&self.cookie_lending.tried).insert(lend.id.clone(), now_ms());
                    }
                }
                changed = true;
            }
            if changed && self.write_lend(&lend).is_err() {
                eprintln!("[cookies] a share's copy could not be recorded");
            }
        }
        if reload {
            self.browser_reload(persona).await;
        }
    }

    /// The share's source as this desk has it now, switched on: found by
    /// its ids, or by its names for a card from before they were kept.
    async fn source_of(&self, lend: &Lend) -> Option<lending::Source> {
        let candidates = self.host_candidates();
        let gate = self.cookie_lending.gate.clone();
        let sources =
            tokio::task::spawn_blocking(move || lending::discover(candidates, &gate).sources)
                .await
                .ok()?;
        sources
            .into_iter()
            .find(|source| match (&lend.browser_id, &lend.profile_id) {
                (Some(browser), Some(profile)) => {
                    source.browser_id() == browser && source.profile_id() == profile
                }
                _ => {
                    source.browser_name() == lend.browser_name
                        && source.profile_name() == lend.profile_name
                }
            })
    }

    async fn read_source(&self, lend: &Lend) -> Result<Vec<Value>, String> {
        let source = self
            .source_of(lend)
            .await
            .ok_or("This share's browser is gone or switched off.")?;
        let cookies = source.read(&lend.site).await?;
        if cookies.is_empty() {
            return Err("This browser no longer has live cookies for the site.".into());
        }
        Ok(cookies)
    }

    /// Copies a kept-fresh share again when its site's cookies in the source
    /// have changed since they were last read. A switched-off source is not
    /// found, so it is not read.
    async fn refresh_lend(&self, persona: &str, id: &str, force: bool) {
        let held = self.lend_lock(persona);
        let _held = held.lock().await;
        let Ok(mut lend) = self.active_lend(persona, id) else {
            return;
        };
        if !lend.keep_fresh || !lend.copied || lend.from_laptop {
            return;
        }
        let last = lock(&self.cookie_lending.fresh).get(&lend.id).cloned();
        if !force
            && last
                .as_ref()
                .is_some_and(|(_, at)| now_ms() - at < KEEP_FRESH_EVERY_MS)
        {
            return;
        }
        let ready = self.computers.running(persona).await;
        if (lend.desk_browser && self.lending_browser(persona).is_none())
            || (!lend.desk_browser && ready.is_none())
        {
            return;
        }
        let Some(source) = self.source_of(&lend).await else {
            return;
        };
        let site = lend.site.clone();
        let reading = source.clone();
        let Ok(Ok(fingerprint)) =
            tokio::task::spawn_blocking(move || reading.fingerprint(&site)).await
        else {
            return;
        };
        if last.as_ref().is_some_and(|(seen, _)| *seen == fingerprint) {
            lock(&self.cookie_lending.fresh).insert(lend.id.clone(), (fingerprint, now_ms()));
            return;
        }
        let Ok(cookies) = source.read(&lend.site).await else {
            return;
        };
        // The old copy goes first, so a cookie the source dropped goes too.
        if self.take_copy_out(&mut lend, ready.as_ref()).await.is_err() {
            return;
        }
        let copied = self
            .put_copy(&mut lend, cookies, ready.as_ref())
            .await
            .is_ok();
        if self.write_lend(&lend).is_err() {
            eprintln!("[cookies] a kept-fresh copy could not be recorded");
        }
        if copied {
            lock(&self.cookie_lending.fresh).insert(lend.id.clone(), (fingerprint, now_ms()));
        }
    }

    /// One pass over the room's shares: those whose end has come are taken
    /// back, kept-fresh ones look at their sources, and copies waiting on a
    /// browser are made. Only shares the room records need any of it.
    async fn look_at_cookie_lends(self: &Arc<Self>, force: bool) {
        let records = self.lend_records();
        let now = now_ms();
        for lend in records.iter().filter(|lend| !lend.removed) {
            if lend.ends_at.is_some_and(|ends| ends <= now)
                && let Err(error) = self.remove_lend(&lend.teammate, &lend.id).await
            {
                eprintln!("[cookies] a share that ended could not be removed yet: {error}");
            }
        }
        for lend in records
            .iter()
            .filter(|lend| !lend.removed && lend.keep_fresh)
        {
            self.refresh_lend(&lend.teammate, &lend.id, force).await;
        }
        let personas: HashSet<&str> = records
            .iter()
            .filter(|lend| {
                (!lend.copied && !lend.removed)
                    || (lend.in_computer && (lend.removed || lend.desk_browser))
            })
            .map(|lend| lend.teammate.as_str())
            .collect();
        for persona in personas {
            let ready = self.computers.running(persona).await;
            self.copy_waiting(persona, ready.as_ref(), force).await;
        }
    }

    /// The watch's pass, on demand, as every check were due.
    #[cfg(all(test, unix))]
    pub(crate) async fn poll_cookie_lends(self: &Arc<Self>) {
        self.look_at_cookie_lends(true).await;
    }

    /// The line a turn opens with when the sites the teammate holds the
    /// person's cookies for have changed since its agent was last told:
    /// the site names, never a value.
    pub(super) fn with_cookie_news(
        &self,
        told: &Mutex<Option<Vec<String>>>,
        persona: &str,
        text: String,
    ) -> String {
        let mut now: Vec<String> = self
            .lends_of(persona)
            .into_iter()
            .filter(|lend| !lend.removed)
            .map(|lend| lend.site)
            .collect();
        now.sort();
        now.dedup();
        let mut told = lock(told);
        if told.as_ref() == Some(&now) {
            return text;
        }
        let first = told.replace(now.clone()).is_none();
        if first && now.is_empty() {
            return text;
        }
        let news = if now.is_empty() {
            "(Cookies: the person no longer shares any site's cookies with your browser.)"
                .to_string()
        } else {
            format!(
                "(Cookies: the person shared their cookies for {} with your browser, so it is signed in to those sites as them. You never see the cookies themselves.)",
                now.join(", ")
            )
        };
        format!("{news}\n\n{text}")
    }

    /// Each browser profile this desk could read cookies from, and whether
    /// the person lets it be read. Reads no cookie store.
    pub async fn cookie_sources(&self) -> CookieSources {
        let candidates = self.host_candidates();
        let browsers =
            tokio::task::spawn_blocking(move || crate::computer::cookies::listed(&candidates))
                .await
                .unwrap_or_default();
        let gate = &self.cookie_lending.gate;
        let mut sources = CookieSources {
            sources: Vec::new(),
            blocked: Vec::new(),
        };
        for browser in browsers {
            if let Some(reason) = browser.blocked_reason.filter(|_| browser.blocked) {
                sources.blocked.push(reason);
                continue;
            }
            for profile in browser.profiles {
                sources.sources.push(CookieSource {
                    on: gate.allows(&browser.id, &profile.id),
                    browser_id: browser.id.clone(),
                    browser_name: browser.name.clone(),
                    profile_id: profile.id,
                    profile_name: profile.name,
                });
            }
        }
        sources
    }

    /// Switches a browser profile on or off as a cookie source. Off, every
    /// source already discovered from it refuses to be read as well.
    pub async fn cookie_source_set(
        &self,
        browser_id: &str,
        profile_id: &str,
        on: bool,
    ) -> Result<CookieSources, String> {
        if [browser_id, profile_id]
            .iter()
            .any(|id| id.is_empty() || id.len() > 256)
        {
            return Err("That is not a browser profile.".into());
        }
        let mut off = sources_off(&self.log);
        let key = lending::source_key(browser_id, profile_id);
        if on {
            off.remove(&key);
        } else {
            off.insert(key);
        }
        let mut listed: Vec<&String> = off.iter().collect();
        listed.sort();
        self.log
            .append(
                &StreamId::Room,
                &json!({"id": SOURCES_ID, "kind": "cookie_sources", "ts": now_ms(), "off": listed}),
            )
            .map_err(|_| "Could not save the cookie sources.")?;
        self.cookie_lending.gate.replace(off);
        Ok(self.cookie_sources().await)
    }
}

#[cfg(all(test, unix))]
mod tests;
