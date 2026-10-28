//! A teammate's computer as the session sees it: whether a start waits for
//! it, attaching it when it lands, swapping it between turns, and the
//! computer commands the desk sends.

use super::*;

impl Room {
    /// What this start does about the teammate's computer.
    ///
    /// The computer is brought up on its own task. When it is up without a
    /// download — the image already here — the start waits for it, as it
    /// always has, and grants it. The moment a download begins, the start
    /// goes ahead without it: the pull fills its bar on the tape, and once
    /// it lands the session restarts with the computer after the turn in
    /// flight (see [`Room::attach_computer_when_idle`]).
    pub(super) async fn computer_at_start(
        self: &Arc<Self>,
        persona: &Persona,
        driving: &Driving,
    ) -> ComputerAtStart {
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return ComputerAtStart::NotWanted;
        }
        {
            let mut setups = lock(&self.computer_setups);
            if setups
                .get(&persona.id)
                .is_some_and(|setup| matches!(*setup.borrow(), ComputerSetup::Downloading { .. }))
            {
                // A download already under way joins when it lands; a second
                // one would race it for the same container.
                return ComputerAtStart::Downloading;
            }
            setups.remove(&persona.id);
        }
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let (began, downloading) = oneshot::channel::<()>();
        let room = self.clone();
        let wanted = persona.clone();
        let mut bringing_up = tokio::spawn(async move {
            let mut began = Some(began);
            let mut tape = room.pull_reporter(&wanted.id);
            let report = |report: crate::computer::PullReport| {
                room.computer_download_progress(&wanted.id, &report);
                if let Some(began) = began.take() {
                    let _ = began.send(());
                }
                tape(report);
            };
            room.computers
                .ensure_running(&wanted, &wanted.cwd, prefer, room_image.as_deref(), report)
                .await
        });
        tokio::select! {
            result = &mut bringing_up => match result {
                Ok(Ok(ready)) => {
                    lock(&self.computer_setups).remove(&persona.id);
                    match self.finish_grant(persona, &ready, driving).await {
                        Ok(extra_mcp) => ComputerAtStart::Attached(extra_mcp),
                        Err(reason) => ComputerAtStart::Unavailable(reason),
                    }
                }
                Ok(Err(reason)) => {
                    lock(&self.computer_setups).remove(&persona.id);
                    ComputerAtStart::Unavailable(reason)
                }
                Err(error) => {
                    lock(&self.computer_setups).remove(&persona.id);
                    ComputerAtStart::Unavailable(error.to_string())
                }
            },
            Ok(()) = downloading => {
                let room = self.clone();
                let persona_id = persona.id.clone();
                tokio::spawn(async move {
                    let result = match bringing_up.await {
                        Ok(result) => result,
                        Err(error) => Err(error.to_string()),
                    };
                    room.computer_downloaded(&persona_id, result);
                });
                ComputerAtStart::Downloading
            }
        }
    }

    /// Records a download's progress where `computer_status` can see it.
    fn computer_download_progress(&self, persona_id: &str, report: &crate::computer::PullReport) {
        let setup = ComputerSetup::Downloading {
            image: report.image.clone(),
            layers_done: report.layers_done,
            layers_total: report.layers_total,
        };
        let mut setups = lock(&self.computer_setups);
        match setups.get(persona_id) {
            Some(watching) => {
                watching.send_replace(setup);
            }
            None => {
                setups.insert(persona_id.to_string(), watch::channel(setup).0);
            }
        }
    }

    fn set_computer_setup(&self, persona_id: &str, setup: ComputerSetup) {
        let mut setups = lock(&self.computer_setups);
        match setups.get(persona_id) {
            Some(watching) => {
                watching.send_replace(setup);
            }
            None => {
                setups.insert(persona_id.to_string(), watch::channel(setup).0);
            }
        }
    }

    /// A download that ran behind a session has finished, one way or the
    /// other. Landed, the computer joins once the turn ends; failed, the
    /// tape says why exactly as a start that failed at once would.
    fn computer_downloaded(
        self: &Arc<Self>,
        persona_id: &str,
        result: Result<crate::computer::Ready, String>,
    ) {
        match result {
            Ok(_) => {
                self.set_computer_setup(persona_id, ComputerSetup::Ready);
                self.attach_computer_when_idle(persona_id);
            }
            Err(reason) => {
                self.set_computer_setup(persona_id, ComputerSetup::Failed(reason.clone()));
                if let Ok(persona) = self.persona(persona_id) {
                    eprintln!("{}", computer_unavailable(&persona.name, &reason));
                }
            }
        }
    }

    /// Brings a computer that finished downloading into its teammate's
    /// session, between turns. A turn in flight is never cut short for it:
    /// the end of the turn calls this again (see [`Room::run_queue`]). With no
    /// session running there is nothing to restart; the next start finds
    /// the computer up and grants it at once.
    pub(super) fn attach_computer_when_idle(self: &Arc<Self>, persona_id: &str) {
        if !lock(&self.computer_setups)
            .get(persona_id)
            .is_some_and(|setup| matches!(*setup.borrow(), ComputerSetup::Ready))
        {
            return;
        }
        let wanted = self.persona(persona_id).ok().filter(|persona| {
            persona
                .computer
                .as_ref()
                .is_some_and(|computer| computer.enabled)
        });
        let session = lock(&self.sessions).get(persona_id).cloned();
        let (Some(_), Some(session)) = (wanted, session) else {
            lock(&self.computer_setups).remove(persona_id);
            return;
        };
        if session.computer {
            lock(&self.computer_setups).remove(persona_id);
            return;
        }
        if lock(&session.turns).running {
            return;
        }
        lock(&self.computer_setups).remove(persona_id);
        let room = self.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            // A rebuild that fails says why on the teammate's band itself.
            let _ = room.rebuild(&persona_id).await;
        });
    }

    /// What `computer_status` answers in a work thread: the thread was built
    /// with the computer or without it, and a download that is still going is
    /// the DM's to watch.
    pub(crate) fn computer_in_a_thread(&self, persona_id: &str) -> String {
        let enabled = self.persona(persona_id).is_ok_and(|persona| {
            persona
                .computer
                .as_ref()
                .is_some_and(|computer| computer.enabled)
        });
        if enabled {
            json!({
                "state": "attached",
                "note": "Your computer is attached, shared with your other threads: one drives it at a time, and a call says who has it when it is busy. If it did not start for this thread, its `computer__` tools are absent; work without it and say so.",
            })
        } else {
            json!({
                "state": "none",
                "note": "You have no computer in this thread.",
            })
        }
        .to_string()
    }

    /// Whether this teammate's session is running with its computer now, as
    /// its colleagues are told.
    pub(crate) fn computer_in_use(&self, persona_id: &str) -> bool {
        lock(&self.sessions)
            .get(persona_id)
            .is_some_and(|session| session.computer)
    }

    /// What `computer_status` answers: where this teammate's computer is,
    /// waiting up to `wait` for a download in progress to finish.
    pub(crate) async fn computer_setup_status(&self, persona_id: &str, wait: Duration) -> Value {
        let enabled = self.persona(persona_id).is_ok_and(|persona| {
            persona
                .computer
                .as_ref()
                .is_some_and(|computer| computer.enabled)
        });
        if !enabled {
            return json!({
                "state": "none",
                "note": "You have no computer. The person can turn one on in your pane.",
            });
        }
        if lock(&self.sessions)
            .get(persona_id)
            .is_some_and(|session| session.computer)
        {
            return json!({
                "state": "attached",
                "note": "Your computer is attached: its `computer__` tools are yours to use.",
            });
        }
        let watching = lock(&self.computer_setups)
            .get(persona_id)
            .map(watch::Sender::subscribe);
        let Some(mut watching) = watching else {
            return json!({
                "state": "unavailable",
                "note": "Your computer did not start for this session; the notice on your tape says why. Work without it, and tell the person if the task needs it.",
            });
        };
        if !wait.is_zero() && matches!(*watching.borrow(), ComputerSetup::Downloading { .. }) {
            let _ = tokio::time::timeout(wait, async {
                while matches!(
                    *watching.borrow_and_update(),
                    ComputerSetup::Downloading { .. }
                ) {
                    if watching.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
        let setup = watching.borrow().clone();
        match setup {
            ComputerSetup::Downloading {
                image,
                layers_done,
                layers_total,
            } => json!({
                "state": "downloading",
                "image": image,
                "layersDone": layers_done,
                "layersTotal": layers_total,
                "note": "Your computer's image is still downloading. Keep working without it, or call again with `wait_seconds` to wait for it. When it lands, your session restarts with the computer once your current turn ends, and this conversation carries over.",
            }),
            ComputerSetup::Ready => json!({
                "state": "ready",
                "note": "Your computer has finished downloading. Its `computer__` tools arrive when this turn ends: finish or pause here, and they are yours from the next message.",
            }),
            ComputerSetup::Failed(reason) => json!({
                "state": "failed",
                "reason": reason,
                "note": "Your computer could not start. The person has been told on your tape; work without it, and say so if the task needs it.",
            }),
        }
    }

    /// Starts the teammate's computer, waiting for it however long it
    /// takes, and grants it to a thread that shares it: a work thread, a
    /// call, a run or a job, which goes on without it when this fails.
    pub(super) async fn grant_computer(
        &self,
        persona: &Persona,
        driving: &Driving,
    ) -> Result<Vec<mcp::McpServer>, String> {
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Ok(Vec::new());
        }
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let ready = self
            .computers
            .ensure_running(
                persona,
                &persona.cwd,
                prefer,
                room_image.as_deref(),
                self.pull_reporter(&persona.id),
            )
            .await?;
        self.finish_grant(persona, &ready, driving).await
    }

    /// The rest of the grant, once the computer is up: its guide as the
    /// teammate's skill, its secrets, and the MCP server to hand the session.
    async fn finish_grant(
        &self,
        persona: &Persona,
        ready: &crate::computer::Ready,
        driving: &Driving,
    ) -> Result<Vec<mcp::McpServer>, String> {
        let ready = ready.clone();
        // The guide the running release serves is the teammate's
        // `hotline-computer` skill: written here, after the container is up and
        // before the preamble is built, so the index lists it and the file it
        // names is the release actually running. A computer that will not
        // hand one over leaves no skill, and the preamble tells the teammate
        // to ask it directly.
        match crate::computer::guide::fetch(&ready).await {
            Ok(guide) => {
                crate::skills::write_computer(
                    Path::new(&persona.cwd),
                    &guide.version,
                    &guide.sha256,
                    &guide.skill,
                )
                .map_err(|error| {
                    format!(
                        "{}'s computer guide could not be written: {error}",
                        persona.name
                    )
                })?;
                self.computers.learned_release(&persona.id, &guide.version);
            }
            Err(reason) => eprintln!(
                "{}'s computer did not hand over its guide; it will ask it directly. {reason}",
                persona.name
            ),
        }
        self.hand_secrets(persona, &ready).await;
        // The agent is handed a gate in front of the computer, not the
        // computer: the teammate's threads take turns at it.
        let lease = driving.lease.clone();
        let gate = crate::computer::gate::serve(
            &ready,
            self.computer_leases.clone(),
            &persona.id,
            driving.holder.clone(),
            move || lease.is_current(),
        )
        .await
        .map_err(|error| format!("{}'s computer could not be shared: {error}", persona.name))?;
        Ok(vec![crate::computer::mcp_server(&crate::computer::Ready {
            url: gate.url,
            token: gate.token,
        })])
    }

    pub async fn computer_capacity(&self) -> crate::contract::ComputerCapacity {
        let settings = room::settings(&self.log);
        self.computers
            .capacity(crate::computer::preferred_runtime(&settings))
            .await
    }

    pub async fn computer_runtimes(&self) -> Vec<RuntimeReport> {
        self.computers.runtimes().await
    }

    pub async fn computer_status(&self, persona_id: &str) -> Result<ComputerStatus, String> {
        // The pane polls this, so the room is read once for both the settings
        // and the roster rather than once for each.
        let events = self.log.load(&StreamId::Room);
        let settings = room::settings_from_events(&events);
        let persona = room::personas(&events)
            .into_iter()
            .find(|persona| persona.id == persona_id);
        let mut status = self
            .computers
            .status(
                persona_id,
                persona.as_ref(),
                crate::computer::preferred_runtime(&settings),
            )
            .await?;
        // An existing container keeps the release it was made on, so the
        // pane is told when the one it would be made on now is different.
        if let Some(release) = &status.release
            && let Some(persona) = &persona
        {
            let image = self.computers.image_for(
                persona,
                crate::computer::preferred_image(&settings).as_deref(),
            );
            let wanted = crate::computer::release_of(&image);
            if wanted != *release {
                status.available = Some(wanted);
            }
        }
        status.update_failed = lock(&self.computer_update_failures)
            .get(persona_id)
            .cloned();
        Ok(status)
    }

    /// Recreates a teammate's computer on the release it would be created on
    /// now. The container goes; the workspace, scratch and home volumes stay.
    ///
    /// A running teammate is neither stopped nor held up for it: the new
    /// release downloads while the old computer keeps working, and the swap
    /// happens between turns (see [`Room::swap_computer_when_idle`]). With
    /// no session running the container simply goes, and the next start
    /// makes the new one.
    pub async fn computer_update(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let persona = self.persona(persona_id)?;
        lock(&self.computer_update_failures).remove(persona_id);
        let old = self.computer_status(persona_id).await?.release;
        if !lock(&self.sessions).contains_key(persona_id) {
            self.computer_remove(persona_id).await?;
            self.forget_release(old).await;
            return Ok(());
        }
        {
            let mut swaps = lock(&self.computer_swaps);
            if swaps.contains_key(persona_id) {
                // One update at a time; the one under way lands the same release.
                return Ok(());
            }
            swaps.insert(persona_id.to_string(), ComputerSwap { old, ready: false });
        }
        let room = self.clone();
        tokio::spawn(async move {
            let settings = room::settings(&room.log);
            let prepared = room
                .computers
                .prepare(
                    &persona,
                    crate::computer::preferred_runtime(&settings),
                    crate::computer::preferred_image(&settings).as_deref(),
                    room.pull_reporter(&persona.id),
                )
                .await;
            match prepared {
                Ok(()) => {
                    if let Some(swap) = lock(&room.computer_swaps).get_mut(&persona.id) {
                        swap.ready = true;
                    }
                    room.swap_computer_when_idle(&persona.id);
                }
                Err(reason) => {
                    lock(&room.computer_swaps).remove(&persona.id);
                    eprintln!(
                        "{}'s computer could not be updated, and keeps the one it has: {reason}",
                        persona.name
                    );
                    lock(&room.computer_update_failures).insert(persona.id.clone(), reason);
                }
            }
        });
        Ok(())
    }

    /// Makes a teammate's computer again with the memory and CPUs it has
    /// now, which a runtime only takes at create. A running teammate gets
    /// it between turns, the way an update lands, and an update already
    /// under way lands the new limits with it. With no session running the
    /// computer only stops: its next start sees the limits differ and
    /// makes it again.
    pub async fn computer_restart(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        self.persona(persona_id)?;
        if !lock(&self.sessions).contains_key(persona_id) {
            return self.computer_stop(persona_id).await;
        }
        lock(&self.computer_swaps)
            .entry(persona_id.to_string())
            .or_insert(ComputerSwap {
                old: None,
                ready: true,
            });
        self.swap_computer_when_idle(persona_id);
        Ok(())
    }

    /// Swaps in a computer whose new release has downloaded, between turns.
    /// A turn in flight keeps the old computer to its end, which calls this
    /// again (see [`Room::run_queue`]). The swap is the old container going and the
    /// session reattaching, which makes and grants the new one.
    pub(super) fn swap_computer_when_idle(self: &Arc<Self>, persona_id: &str) {
        if !lock(&self.computer_swaps)
            .get(persona_id)
            .is_some_and(|swap| swap.ready)
        {
            return;
        }
        let session = lock(&self.sessions).get(persona_id).cloned();
        if session
            .as_ref()
            .is_some_and(|session| lock(&session.turns).running)
        {
            return;
        }
        let Some(swap) = lock(&self.computer_swaps).remove(persona_id) else {
            return;
        };
        let wanted = self.persona(persona_id).ok().filter(|persona| {
            persona
                .computer
                .as_ref()
                .is_some_and(|computer| computer.enabled)
        });
        let room = self.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            // Behind the start gate a message either has claimed its turn
            // already, or waits and lands on the session this brings up.
            let gate = room.start_gate(&persona_id);
            let held = gate.lock().await;
            let busy = lock(&room.sessions)
                .get(&persona_id)
                .is_some_and(|session| lock(&session.turns).running);
            if busy {
                // Admitted in the gap: that turn's end brings the swap back,
                // unless it has ended already, which this looks for once more.
                drop(held);
                lock(&room.computer_swaps)
                    .entry(persona_id.clone())
                    .or_insert(swap);
                room.swap_computer_when_idle(&persona_id);
                return;
            }
            let _ = room.computer_remove(&persona_id).await;
            if wanted.is_some() && session.is_some() {
                // A rebuild that fails says why on the teammate's band itself.
                let _working = room.working();
                let _ = room.rebuild_held(&persona_id).await;
            }
            drop(held);
            room.forget_release(swap.old).await;
        });
    }

    /// The release that was running is not coming back; its image is removed
    /// unless another container is still on it.
    async fn forget_release(&self, old: Option<String>) {
        if let Some(old) = old {
            self.computers
                .forget_image(
                    &crate::computer::image_at(&old),
                    crate::computer::preferred_runtime(&room::settings(&self.log)),
                )
                .await;
        }
    }

    /// The release a new computer is created on, as the desk knows it now.
    pub fn computer_releases(&self) -> crate::contract::ComputerReleases {
        self.computers.releases_known()
    }

    /// The same, after asking the endpoint now.
    pub async fn computer_releases_check(&self) -> crate::contract::ComputerReleases {
        self.computers.check_releases(now_ms()).await
    }

    pub async fn computer_stop(&self, persona_id: &str) -> Result<(), String> {
        self.computers
            .stop(
                persona_id,
                crate::computer::preferred_runtime(&room::settings(&self.log)),
            )
            .await
    }

    pub async fn computer_remove(&self, persona_id: &str) -> Result<(), String> {
        self.computers
            .remove(
                persona_id,
                crate::computer::preferred_runtime(&room::settings(&self.log)),
            )
            .await
    }

    /// The host browsers the operator could bring cookies from. This reads
    /// the person's own machine; it is reachable only over the desk seat and
    /// no agent tool calls it.
    pub async fn computer_browsers(&self) -> Vec<HostBrowser> {
        crate::computer::cookies::detect()
    }

    /// The sites in one host browser profile, with counts, for the picker.
    /// Domains and counts only; no cookie value is read out.
    pub async fn computer_cookies_preview(
        &self,
        browser_id: &str,
        profile_id: &str,
    ) -> Result<Vec<CookieSite>, String> {
        crate::computer::cookies::preview(browser_id, profile_id).await
    }

    /// Copies the cookies for the ticked sites from a host browser into the
    /// teammate's computer, so its browser starts signed in to them, and
    /// answers with the sites actually imported. The values pass host → desk →
    /// container and are never written to the tape, the model, or a log.
    pub async fn computer_cookies_import(
        &self,
        persona_id: &str,
        browser_id: &str,
        profile_id: &str,
        domains: &[String],
    ) -> Result<Vec<CookieSite>, String> {
        let cookies = crate::computer::cookies::select(browser_id, profile_id, domains).await?;
        self.deliver_cookies(persona_id, browser_id, profile_id, cookies, None)
            .await
    }

    pub async fn computer_cookies_push(
        &self,
        persona_id: &str,
        transfer: crate::contract::CookieTransfer,
    ) -> Result<Vec<CookieSite>, String> {
        let cookies = crate::computer::cookies::validate_transfer(&transfer)?;
        // A collision-resistant source key avoids replacing server-local or
        // another laptop's browser record when names happen to match.
        use sha2::{Digest, Sha256};
        let source = serde_json::to_vec(&(&transfer.source_id, &transfer.browser_id))
            .map_err(|_| "Invalid cookie source.")?;
        let browser = format!("laptop-{:x}", Sha256::digest(source));
        self.deliver_cookies(
            persona_id,
            &browser,
            &transfer.profile_id,
            cookies,
            Some(&format!("{} ({})", transfer.browser_id, transfer.source_id)),
        )
        .await
    }

    async fn deliver_cookies(
        &self,
        persona_id: &str,
        browser_id: &str,
        profile_id: &str,
        cookies: Vec<Value>,
        source_name: Option<&str>,
    ) -> Result<Vec<CookieSite>, String> {
        let persona = room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == persona_id)
            .ok_or("No such teammate.")?;
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Err(format!("{} has no computer to import into.", persona.name));
        }
        if cookies.is_empty() {
            return Err("None of the chosen sites had cookies to import.".to_string());
        }
        let imported = crate::computer::cookies::summarize(&cookies);

        // The computer has to be up to receive the login; a stopped one is
        // started, the same as opening its screen would.
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let notice_id = persona.id.clone();
        let ready = self
            .computers
            .ensure_running(
                &persona,
                &persona.cwd,
                prefer,
                room_image.as_deref(),
                self.pull_reporter(&notice_id),
            )
            .await?;

        // The document the container's `login_load` reads. `storage` is empty:
        // v1 carries cookies, not localStorage. The name is stable per source,
        // so re-importing the same browser replaces the earlier file.
        let name = crate::computer::login::name_for(browser_id, profile_id);
        let saved = serde_json::json!({
            "name": name,
            "browser": browser_id,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "cookies": cookies,
            "storage": {},
        });
        crate::computer::login::deliver(&ready, &name, &saved).await?;

        // The record the pane lists afterwards: which browser and profile,
        // when, and the sites. Names are looked up now, while the browser
        // is still installed to say them; the ids are what the record is
        // keyed by.
        let browsers = if source_name.is_none() {
            self.computer_browsers().await
        } else {
            Vec::new()
        };
        let browser = browsers.iter().find(|browser| browser.id == browser_id);
        let import = crate::contract::CookieImport {
            browser_id: browser_id.to_owned(),
            browser_name: browser.map_or_else(
                || source_name.unwrap_or(browser_id).to_owned(),
                |browser| browser.name.clone(),
            ),
            profile_id: profile_id.to_owned(),
            profile_name: browser
                .and_then(|browser| {
                    browser
                        .profiles
                        .iter()
                        .find(|profile| profile.id == profile_id)
                })
                .map_or_else(|| profile_id.to_owned(), |profile| profile.name.clone()),
            imported_at: now_ms(),
            sites: imported.clone(),
        };
        let mut imports = room::cookie_imports(&self.log, persona_id);
        room::merge_cookie_import(&mut imports, import);
        room::record_cookie_imports(&self.log, persona_id, &imports)?;
        Ok(imported)
    }

    /// What has been brought over to `persona_id`'s computer, as recorded
    /// at each import and trimmed at each forget.
    pub fn computer_cookies_list(&self, persona_id: &str) -> Vec<crate::contract::CookieImport> {
        room::cookie_imports(&self.log, persona_id)
    }

    /// Takes brought-over cookies back out of the teammate's computer: one
    /// site when `domain` is given, the whole import when not. The domains
    /// are always named to the computer, so what the browser drops is what
    /// the record says, whatever the saved login there carries by now. The
    /// computer has to be up to drop them; a stopped one is started, the
    /// same as the import did.
    pub async fn computer_cookies_forget(
        &self,
        persona_id: &str,
        browser_id: &str,
        profile_id: &str,
        domain: Option<&str>,
    ) -> Result<Vec<crate::contract::CookieImport>, String> {
        let mut imports = room::cookie_imports(&self.log, persona_id);
        let Some(position) = imports
            .iter()
            .position(|import| import.browser_id == browser_id && import.profile_id == profile_id)
        else {
            return Err("Nothing from that browser is recorded for this teammate.".to_string());
        };
        let domains: Vec<String> = match domain {
            Some(domain) => {
                if !imports[position]
                    .sites
                    .iter()
                    .any(|site| site.domain == domain)
                {
                    return Err(format!(
                        "{domain} is not among the sites brought over from {}.",
                        imports[position].browser_name
                    ));
                }
                vec![domain.to_owned()]
            }
            None => imports[position]
                .sites
                .iter()
                .map(|site| site.domain.clone())
                .collect(),
        };
        let persona = self.persona(persona_id)?;
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Err(format!(
                "{} has no computer to take them out of.",
                persona.name
            ));
        }
        let settings = room::settings(&self.log);
        let prefer = crate::computer::preferred_runtime(&settings);
        let room_image = crate::computer::preferred_image(&settings);
        let notice_id = persona.id.clone();
        let ready = self
            .computers
            .ensure_running(
                &persona,
                &persona.cwd,
                prefer,
                room_image.as_deref(),
                self.pull_reporter(&notice_id),
            )
            .await?;
        let name = crate::computer::login::name_for(browser_id, profile_id);
        crate::computer::login::forget(&ready, &name, &domains).await?;
        match domain {
            Some(domain) => {
                imports[position].sites.retain(|site| site.domain != domain);
                if imports[position].sites.is_empty() {
                    imports.remove(position);
                }
            }
            None => {
                imports.remove(position);
            }
        }
        room::record_cookie_imports(&self.log, persona_id, &imports)?;
        Ok(imports)
    }

    pub(super) async fn sweep_computers(&self) {
        let live: HashSet<String> = lock(&self.sessions).keys().cloned().collect();
        self.computers.sweep(now_ms(), &live).await;
        // The newest release is looked up on this clock too: once when the
        // room opens, then every six hours.
        self.computers.refresh_releases(now_ms()).await;
    }
}

/// An update waiting to be swapped in: the release it replaces, whose image
/// goes once nothing is on it, and whether the new one has downloaded.
pub(super) struct ComputerSwap {
    old: Option<String>,
    pub(super) ready: bool,
}

/// Where a teammate's computer set-up stands, for `computer_status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ComputerSetup {
    Downloading {
        image: String,
        layers_done: u32,
        layers_total: u32,
    },
    /// Downloaded and up: joins the session when its turn ends.
    Ready,
    Failed(String),
}

/// Which agent is being handed the teammate's computer, and the authority it
/// holds it under. The computer is shared by the teammate's threads one at a
/// time ([`crate::computer::gate`]), and its gate ends with this lease.
pub(super) struct Driving {
    holder: crate::computer::gate::Holder,
    lease: CapabilityLease,
}

impl Driving {
    /// What this agent holds the computer under.
    pub(super) fn holder(&self) -> &crate::computer::gate::Holder {
        &self.holder
    }

    /// `key` is what the thread holds the computer under, and `title` is what
    /// another thread is told it is busy with.
    pub(super) fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        lease: &CapabilityLease,
    ) -> Self {
        Self {
            holder: crate::computer::gate::Holder::new(key, title),
            lease: lease.clone(),
        }
    }
}

impl Room {
    /// A thread's turn is over: it has no more use for the teammate's computer
    /// until it is next asked something.
    ///
    /// It is the agent's own hold that is let go of: a thread's replacement
    /// agent has the same key and a hold of its own.
    pub(super) fn let_go_of_computer(
        &self,
        persona_id: &str,
        holder: &crate::computer::gate::Holder,
    ) {
        self.computer_leases.release(persona_id, holder);
    }
}

/// What a start does about the teammate's computer.
pub(super) enum ComputerAtStart {
    NotWanted,
    Attached(Vec<mcp::McpServer>),
    Downloading,
    Unavailable(String),
}

/// What a teammate whose computer is still downloading is told at start.
pub(super) const COMPUTER_DOWNLOADING: &str = "Your computer is still downloading its image, so you are starting without it. Call `computer_status` to see how far it has got, or with `wait_seconds` to wait for it. When it lands, your session restarts with its `computer__` tools once your current turn ends, and this conversation carries over. Until then, do what you can without it.";

/// What a teammate whose computer could not start is told.
pub(super) fn computer_failed_note(reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    format!(
        "Your computer could not start for this session ({reason}), so you have none. The person has been told on your tape. Work without it, and say so if the task needs it."
    )
}

/// A note for the agent, ahead of the wake block.
pub(super) fn with_note(note: Option<String>, wake: Option<String>) -> Option<String> {
    match (note, wake) {
        (Some(note), Some(wake)) => Some(format!("{note}\n\n{wake}")),
        (note, wake) => note.or(wake),
    }
}

/// The tape's sentence when a teammate's computer could not be started and
/// the session went ahead without it.
pub(super) fn computer_unavailable(name: &str, reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    format!(
        "{name}'s computer could not start: {reason}. {name} is answering without it for now. \
         Once that is fixed, choose Stop the session from {name}'s menu; the next message \
         starts it again with its computer."
    )
}

/// This session's view of a teammate whose computer did not come up: the
/// grant is off for the preamble, the skills index and the driver, and the
/// teammate's record is untouched, so the next start tries again.
pub(super) fn without_computer(mut persona: Persona) -> Persona {
    if let Some(computer) = persona.computer.as_mut() {
        computer.enabled = false;
    }
    persona
}
