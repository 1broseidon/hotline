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
        if self.computer_links.begin_start(&persona.id) {
            // A download already under way joins when it lands; a second
            // one would race it for the same container.
            return ComputerAtStart::Downloading;
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
                    self.computer_links.end_start(&persona.id);
                    match self.finish_grant(persona, &ready, driving).await {
                        Ok(extra_mcp) => ComputerAtStart::Attached(extra_mcp),
                        Err(reason) => ComputerAtStart::Unavailable(reason),
                    }
                }
                Ok(Err(reason)) => {
                    self.computer_links.end_start(&persona.id);
                    ComputerAtStart::Unavailable(reason)
                }
                Err(error) => {
                    self.computer_links.end_start(&persona.id);
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
        self.computer_links.progress(
            persona_id,
            &report.image,
            report.layers_done,
            report.layers_total,
        );
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
                self.computer_links.downloaded(persona_id);
                self.attach_computer_when_idle(persona_id);
            }
            Err(reason) => {
                self.computer_links.failed(persona_id, reason.clone());
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
        if self.computer_links.phase(persona_id) != ComputerPhase::Ready {
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
            self.computer_links.end_start(persona_id);
            return;
        };
        if session.computer {
            self.computer_links.end_start(persona_id);
            return;
        }
        if lock(&session.turns).running {
            return;
        }
        let room = self.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            // Behind the start gate a message has either claimed its turn
            // already, which brings this back when it ends, or waits and
            // lands on the session this brings up.
            let Ok(_working) = room.working() else {
                return;
            };
            let gate = room.start_gate(&persona_id);
            let _held = gate.lock().await;
            let Some(session) = lock(&room.sessions).get(&persona_id).cloned() else {
                room.computer_links.end_start(&persona_id);
                return;
            };
            if session.computer {
                room.computer_links.end_start(&persona_id);
                return;
            }
            if lock(&session.turns).running {
                return;
            }
            room.computer_links.end_start(&persona_id);
            // A rebuild that fails says why on the teammate's band itself.
            let _ = room.rebuild_held(&persona_id).await;
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
        let mut watching = self.computer_links.watch(persona_id);
        if !wait.is_zero() && matches!(*watching.borrow(), ComputerPhase::Downloading { .. }) {
            let _ = tokio::time::timeout(wait, async {
                while matches!(
                    *watching.borrow_and_update(),
                    ComputerPhase::Downloading { .. }
                ) {
                    if watching.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
        let phase = self.computer_links.phase(persona_id);
        match phase {
            ComputerPhase::Downloading {
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
            ComputerPhase::Ready => json!({
                "state": "ready",
                "note": "Your computer has finished downloading. Its `computer__` tools arrive when this turn ends: finish or pause here, and they are yours from the next message.",
            }),
            ComputerPhase::Failed { reason, .. } => json!({
                "state": "failed",
                "reason": reason,
                "note": "Your computer could not start. The person has been told on your tape; work without it, and say so if the task needs it.",
            }),
            ComputerPhase::None
            | ComputerPhase::Preparing { .. }
            | ComputerPhase::Swap { .. }
            | ComputerPhase::Swapping { .. }
            | ComputerPhase::UpdateFailed(_) => json!({
                "state": "unavailable",
                "note": "Your computer did not start for this session; the notice on your tape says why. Work without it, and tell the person if the task needs it.",
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
        self.cookie_lends_at_computer(&persona.id, &ready).await;
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
        status.update_failed = match self.computer_links.phase(persona_id) {
            ComputerPhase::UpdateFailed(reason) => Some(reason),
            _ => None,
        };
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
        self.computer_links.forget_update_failure(persona_id);
        let old = self.computer_status(persona_id).await?.release;
        if !lock(&self.sessions).contains_key(persona_id) {
            self.computer_remove(persona_id).await?;
            self.forget_release(old).await;
            return Ok(());
        }
        if !self.computer_links.begin_update(persona_id, old) {
            // One update at a time; the one under way lands the same release.
            return Ok(());
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
                    room.computer_links.prepared(&persona.id);
                    room.swap_computer_when_idle(&persona.id);
                }
                Err(reason) => {
                    eprintln!(
                        "{}'s computer could not be updated, and keeps the one it has: {reason}",
                        persona.name
                    );
                    room.computer_links.update_failed(&persona.id, reason);
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
        self.computer_links.restart(persona_id);
        self.swap_computer_when_idle(persona_id);
        Ok(())
    }

    /// Swaps in a computer whose new release has downloaded, between turns.
    /// A turn in flight keeps the old computer to its end, which calls this
    /// again (see [`Room::run_queue`]). The swap is the old container going and the
    /// session rebuilt as a chapter's swap rebuilds it, which makes and grants
    /// the new one and leaves the teammate's threads and requests standing.
    ///
    /// The swap stays on the teammate's phase until it holds the start gate
    /// and finds no turn claimed, so an update pressed meanwhile is told one
    /// is under way, and a claimed turn leaves it standing for its own end.
    pub(super) fn swap_computer_when_idle(self: &Arc<Self>, persona_id: &str) {
        if !self.computer_links.swap_waiting(persona_id) {
            return;
        }
        if lock(&self.sessions)
            .get(persona_id)
            .is_some_and(|session| lock(&session.turns).running)
        {
            return;
        }
        let room = self.clone();
        let persona_id = persona_id.to_string();
        tokio::spawn(async move {
            // Behind the start gate a message either has claimed its turn
            // already, or waits and lands on the session this brings up.
            let gate = room.start_gate(&persona_id);
            let held = gate.lock().await;
            // The session as it is under the gate, not as it was before it.
            let session = lock(&room.sessions).get(&persona_id).cloned();
            if session
                .as_ref()
                .is_some_and(|session| lock(&session.turns).running)
            {
                // Admitted in the gap: that turn's end brings the swap back,
                // unless it has ended already, which this looks for once more.
                drop(held);
                room.swap_computer_when_idle(&persona_id);
                return;
            }
            let Some(old) = room.computer_links.claim_swap(&persona_id) else {
                return;
            };
            // A room that is closing or pausing for an update starts nothing
            // new: the swap waits, and the container is still the old one.
            let working = match room.working() {
                Ok(working) => working,
                Err(reason) => {
                    eprintln!("A computer swap waits: {reason}");
                    room.computer_links.unclaim_swap(&persona_id, old);
                    return;
                }
            };
            if let Err(reason) = room.computer_remove(&persona_id).await {
                eprintln!("A computer swap waits, its old computer would not go: {reason}");
                room.computer_links.unclaim_swap(&persona_id, old);
                return;
            }
            let wanted = room.persona(&persona_id).is_ok_and(|persona| {
                persona
                    .computer
                    .as_ref()
                    .is_some_and(|computer| computer.enabled)
            });
            if wanted && session.is_some() {
                // A rebuild that fails says why on the teammate's band itself.
                let _ = room.rebuild_held(&persona_id).await;
            }
            drop(working);
            drop(held);
            room.computer_links.finish_swap(&persona_id);
            room.forget_release(old).await;
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

    /// Brings a teammate's computer up from its pane. A computer serves its
    /// teammate's session, which is what makes and grants it: a teammate at
    /// rest starts, computer and all, and a running one whose computer was
    /// stopped from the pane is rebuilt with it, which starts it and leaves
    /// its threads and requests standing. Mid
    /// turn the session is left alone: the turn's tools are its own.
    pub async fn computer_start(self: &Arc<Self>, persona_id: &str) -> Result<(), String> {
        let persona = self.persona(persona_id)?;
        if !persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
        {
            return Err(format!("{} has no computer turned on.", persona.name));
        }
        let session = lock(&self.sessions).get(persona_id).cloned();
        match session {
            None => {
                self.start(persona_id).await?;
            }
            Some(session) if lock(&session.turns).running => {
                return Err(format!(
                    "{} is mid-turn; start its computer when the turn ends.",
                    persona.name
                ));
            }
            Some(_) => self.rebuild(persona_id).await?,
        }
        // A session goes ahead without a computer that cannot start; the key
        // that asked for the computer says why rather than nothing.
        if lock(&self.sessions)
            .get(persona_id)
            .is_some_and(|session| session.computer)
        {
            return Ok(());
        }
        match self.computer_links.phase(persona_id) {
            ComputerPhase::Failed { reason, .. } => Err(format!(
                "{}'s computer could not start: {}.",
                persona.name,
                reason.trim().trim_end_matches('.')
            )),
            // Still downloading: it joins the session when it lands.
            ComputerPhase::Downloading { .. } | ComputerPhase::Ready => Ok(()),
            ComputerPhase::None
            | ComputerPhase::Preparing { .. }
            | ComputerPhase::Swap { .. }
            | ComputerPhase::Swapping { .. }
            | ComputerPhase::UpdateFailed(_) => {
                Err(format!("{}'s computer did not start.", persona.name))
            }
        }
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

    /// A laptop's cookies for a teammate's computer on a desk on a server:
    /// the window read them from its own browser, and the selection is
    /// checked before anything starts. The import record it keeps lists
    /// each site as a row of the teammate's Cookies (`cookie_lends.rs`).
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
            &format!("{} ({})", transfer.browser_id, transfer.source_id),
        )
        .await
    }

    async fn deliver_cookies(
        &self,
        persona_id: &str,
        browser_id: &str,
        profile_id: &str,
        cookies: Vec<Value>,
        source_name: &str,
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

        // The record the pane lists afterwards: which source, when, and the
        // sites. The ids are what the record is keyed by.
        let import = crate::contract::CookieImport {
            browser_id: browser_id.to_owned(),
            browser_name: source_name.to_owned(),
            profile_id: profile_id.to_owned(),
            profile_name: profile_id.to_owned(),
            imported_at: now_ms(),
            sites: imported.clone(),
        };
        let mut imports = room::cookie_imports(&self.log, persona_id);
        room::merge_cookie_import(&mut imports, import);
        room::record_cookie_imports(&self.log, persona_id, &imports)?;
        Ok(imported)
    }

    pub(super) async fn sweep_computers(&self) {
        let settings = room::settings(&self.log);
        let ids: Vec<String> = room::personas(&self.log.load(&StreamId::Room))
            .into_iter()
            .map(|persona| persona.id)
            .collect();
        self.computers
            .adopt(
                &ids,
                crate::computer::preferred_runtime(&settings),
                now_ms(),
            )
            .await;
        // In use is a main session running, or any thread — a side thread, a
        // call, a job — holding the gate's lease.
        let mut in_use: HashSet<String> = lock(&self.sessions).keys().cloned().collect();
        in_use.extend(self.computer_leases.held_ids());
        self.computers.sweep(now_ms(), &in_use).await;
        // The newest release is looked up on this clock too: once when the
        // room opens, then every six hours.
        self.computers.refresh_releases(now_ms()).await;
    }
}

/// How long a failed set-up is remembered. The teammate's next start or
/// press of Start replaces it sooner; after this it is not news.
const FAILED_FOR_MS: i64 = 10 * 60 * 1000;

/// Where one teammate's computer set-up, update and swap stand: the one
/// answer `computer_status`, `computer_start` and the attach and swap steps
/// read. Whether the session has the computer is the session's own
/// (`Session.computer`); this is what is happening behind it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) enum ComputerPhase {
    /// Nothing under way.
    #[default]
    None,
    /// The image behind a session that started without it is downloading.
    Downloading {
        image: String,
        layers_done: u32,
        layers_total: u32,
    },
    /// Downloaded and up: joins the session when its turn ends.
    Ready,
    /// It could not start. Forgotten after [`FAILED_FOR_MS`], or by the
    /// next start.
    Failed { reason: String, at_ms: i64 },
    /// A new release is downloading while the old computer keeps working;
    /// `old` is the release whose image goes after the swap.
    Preparing { old: Option<String> },
    /// The new release is here, or only new limits are wanted: the old
    /// container goes between turns.
    Swap { old: Option<String> },
    /// The swap holds the start gate and is under way.
    Swapping { old: Option<String> },
    /// The last update did not download: why, for the pane, until the
    /// next press of update.
    UpdateFailed(String),
}

/// Every teammate's [`ComputerPhase`], changed only by the named moves below,
/// each of which decides and acts under the one lock.
#[derive(Default)]
pub(super) struct ComputerLinks {
    phases: Mutex<HashMap<String, watch::Sender<ComputerPhase>>>,
}

impl ComputerLinks {
    fn with<R>(&self, persona_id: &str, change: impl FnOnce(&mut ComputerPhase) -> R) -> R {
        let mut phases = lock(&self.phases);
        let phase = phases
            .entry(persona_id.to_string())
            .or_insert_with(|| watch::channel(ComputerPhase::None).0);
        let mut out = None;
        phase.send_if_modified(|phase| {
            let before = phase.clone();
            out = Some(change(phase));
            *phase != before
        });
        out.expect("the change ran")
    }

    /// The phase now. A failure past its time is forgotten here.
    pub(super) fn phase(&self, persona_id: &str) -> ComputerPhase {
        self.with(persona_id, |phase| {
            if matches!(phase, ComputerPhase::Failed { at_ms, .. } if now_ms().saturating_sub(*at_ms) >= FAILED_FOR_MS)
            {
                *phase = ComputerPhase::None;
            }
            phase.clone()
        })
    }

    fn watch(&self, persona_id: &str) -> watch::Receiver<ComputerPhase> {
        self.with(persona_id, |_| ());
        lock(&self.phases)[persona_id].subscribe()
    }

    /// A start begins. True when a download is already under way, which the
    /// start joins; a finished or failed set-up is forgotten.
    fn begin_start(&self, persona_id: &str) -> bool {
        self.with(persona_id, |phase| match phase {
            ComputerPhase::Downloading { .. } => true,
            ComputerPhase::Ready | ComputerPhase::Failed { .. } => {
                *phase = ComputerPhase::None;
                false
            }
            _ => false,
        })
    }

    /// A start's set-up is over, one way or the other. An update under way
    /// is not the start's to clear.
    pub(super) fn end_start(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if matches!(
                phase,
                ComputerPhase::Downloading { .. }
                    | ComputerPhase::Ready
                    | ComputerPhase::Failed { .. }
            ) {
                *phase = ComputerPhase::None;
            }
        });
    }

    fn progress(&self, persona_id: &str, image: &str, layers_done: u32, layers_total: u32) {
        self.with(persona_id, |phase| {
            if matches!(
                phase,
                ComputerPhase::None | ComputerPhase::Downloading { .. }
            ) {
                *phase = ComputerPhase::Downloading {
                    image: image.to_string(),
                    layers_done,
                    layers_total,
                };
            }
        });
    }

    /// The download behind a session has landed.
    pub(super) fn downloaded(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if matches!(
                phase,
                ComputerPhase::None | ComputerPhase::Downloading { .. }
            ) {
                *phase = ComputerPhase::Ready;
            }
        });
    }

    /// The computer could not start.
    pub(super) fn failed(&self, persona_id: &str, reason: String) {
        self.with(persona_id, |phase| {
            if matches!(
                phase,
                ComputerPhase::None
                    | ComputerPhase::Downloading { .. }
                    | ComputerPhase::Ready
                    | ComputerPhase::Failed { .. }
            ) {
                *phase = ComputerPhase::Failed {
                    reason,
                    at_ms: now_ms(),
                };
            }
        });
    }

    fn forget_update_failure(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if matches!(phase, ComputerPhase::UpdateFailed(_)) {
                *phase = ComputerPhase::None;
            }
        });
    }

    /// An update begins: false when something is already under way that
    /// lands the newest release anyway.
    fn begin_update(&self, persona_id: &str, old: Option<String>) -> bool {
        self.with(persona_id, |phase| match phase {
            ComputerPhase::None | ComputerPhase::Failed { .. } | ComputerPhase::UpdateFailed(_) => {
                *phase = ComputerPhase::Preparing { old };
                true
            }
            _ => false,
        })
    }

    /// The update's image is here.
    fn prepared(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if let ComputerPhase::Preparing { old } = phase {
                *phase = ComputerPhase::Swap { old: old.take() };
            }
        });
    }

    fn update_failed(&self, persona_id: &str, reason: String) {
        self.with(persona_id, |phase| {
            if matches!(phase, ComputerPhase::Preparing { .. }) {
                *phase = ComputerPhase::UpdateFailed(reason);
            }
        });
    }

    /// New limits are wanted: the computer is made again between turns.
    /// Anything already under way lands them.
    pub(super) fn restart(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if matches!(
                phase,
                ComputerPhase::None | ComputerPhase::Failed { .. } | ComputerPhase::UpdateFailed(_)
            ) {
                *phase = ComputerPhase::Swap { old: None };
            }
        });
    }

    pub(super) fn swap_waiting(&self, persona_id: &str) -> bool {
        matches!(self.phase(persona_id), ComputerPhase::Swap { .. })
    }

    /// The swap takes its turn. `None` when another already has.
    fn claim_swap(&self, persona_id: &str) -> Option<Option<String>> {
        self.with(persona_id, |phase| match phase {
            ComputerPhase::Swap { old } => {
                let old = old.take();
                *phase = ComputerPhase::Swapping { old: old.clone() };
                Some(old)
            }
            _ => None,
        })
    }

    /// A swap that could not go ahead waits for the next idle moment.
    fn unclaim_swap(&self, persona_id: &str, old: Option<String>) {
        self.with(persona_id, |phase| {
            if matches!(phase, ComputerPhase::Swapping { .. }) {
                *phase = ComputerPhase::Swap { old };
            }
        });
    }

    fn finish_swap(&self, persona_id: &str) {
        self.with(persona_id, |phase| {
            if matches!(phase, ComputerPhase::Swapping { .. }) {
                *phase = ComputerPhase::None;
            }
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure nobody is told about again is forgotten, and only a
    /// start's own set-up is a start's to clear.
    #[test]
    fn a_failed_setup_expires_and_a_start_leaves_an_update_alone() {
        let links = ComputerLinks::default();
        links.failed("ada", "no runtime".into());
        assert!(matches!(links.phase("ada"), ComputerPhase::Failed { .. }));
        links.with("ada", |phase| {
            *phase = ComputerPhase::Failed {
                reason: "no runtime".into(),
                at_ms: now_ms() - FAILED_FOR_MS,
            };
        });
        assert_eq!(links.phase("ada"), ComputerPhase::None);

        assert!(links.begin_update("ada", Some("0.1".into())));
        links.progress("ada", "image", 1, 2);
        links.end_start("ada");
        links.downloaded("ada");
        links.failed("ada", "late".into());
        assert_eq!(
            links.phase("ada"),
            ComputerPhase::Preparing {
                old: Some("0.1".into())
            },
            "a start's moves do not touch an update"
        );
        assert!(!links.begin_update("ada", None), "one at a time");
    }

    /// A claimed swap cannot be claimed twice, and one handed back can.
    #[test]
    fn a_swap_is_claimed_once() {
        let links = ComputerLinks::default();
        links.restart("ada");
        assert_eq!(links.claim_swap("ada"), Some(None));
        assert_eq!(links.claim_swap("ada"), None);
        links.unclaim_swap("ada", None);
        assert!(links.swap_waiting("ada"));
    }
}
