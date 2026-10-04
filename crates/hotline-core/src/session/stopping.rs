//! Stopping for a restart without losing what was asked.
//!
//! A line waiting behind a teammate's turn lives only in memory (`Turns`):
//! the tape has the words, but nothing would hand them to the agent again.
//! So a desk that is told to stop does it in this order:
//!
//! 1. It closes admission. New prompts, schedule firings and phone sends are
//!    refused with a sentence that says the desk is restarting; the phone's
//!    outbox and the scheduler both try again later.
//! 2. It takes every person's line still waiting behind a turn out of the
//!    queue and writes it to `pending.json`, before anything else can go
//!    wrong, so a desk killed during the drain still has them.
//! 3. It lets the turns already running finish, for up to the drain.
//! 4. It stops whatever is still running and says so on that teammate's
//!    tape. The turn is not retried: what it had already done may have had
//!    effects, and doing it twice is worse than asking.
//! 5. It syncs every stream and holds the room shut until the process ends.
//!
//! On the next start the room hands each pending line to its teammate once,
//! in the order they were said, and removes the file. A line the agent has
//! meanwhile read is skipped, and a scheduled line whose teammate lost
//! background work is dropped, exactly as the queue would have dropped it.
//!
//! Only the person's lines are kept. A delivery from another teammate is
//! already durable on the tape and reconciled at start; a nudge is Hotline's
//! own words for a moment that has passed. None of this is exactly-once: a
//! process killed outright keeps what was written and nothing more.

use super::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

pub(crate) const PENDING_FILE: &str = "pending.json";

/// One of the person's lines that was waiting when the desk stopped.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PendingLine {
    persona_id: String,
    /// The user event on the tape this line was written as.
    said: String,
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scheduled: Option<ScheduledRun>,
    #[serde(default)]
    steer: bool,
}

/// What a stop did, for the log line the service writes.
pub struct Stopped {
    /// Lines kept for the next start.
    pub kept: usize,
    /// Teammates whose turn was cut off.
    pub interrupted: Vec<String>,
    /// Held until the process ends, so nothing starts after the final sync.
    _shut: Option<tokio::sync::OwnedRwLockWriteGuard<()>>,
}

impl Room {
    pub(super) fn closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    /// Stops the room for a restart: see the module comment.
    pub async fn stop_for_restart(&self, drain: Duration) -> Stopped {
        self.closing.store(true, Ordering::SeqCst);

        let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        let mut kept = Vec::new();
        for session in &sessions {
            let waiting: Vec<Wired> = lock(&session.turns).waiting.drain(..).collect();
            let tape = self.tape(&session.persona_id);
            for wire in waiting {
                let Some(said) = wire.said.clone() else {
                    continue;
                };
                let persons = tape
                    .iter()
                    .rev()
                    .find(|event| event["id"] == said.as_str())
                    .is_some_and(|event| event["kind"] == "user");
                if persons {
                    kept.push(PendingLine {
                        persona_id: session.persona_id.clone(),
                        said,
                        text: wire.text,
                        attachments: wire.attachments,
                        scheduled: wire.scheduled,
                        steer: wire.steer,
                    });
                }
            }
        }
        let count = kept.len();
        if let Err(error) = self.keep_pending(kept) {
            eprintln!("[stop] the lines waiting behind turns could not be kept: {error}");
        }

        let mut shut = self.shut_within(drain).await;
        let mut interrupted = Vec::new();
        if shut.is_none() {
            self.interrupt_sides();
            for session in &sessions {
                if !lock(&session.turns).running {
                    continue;
                }
                let _ = self.cancel(&session.persona_id);
                self.write(
                    &session.persona_id,
                    &TranscriptEvent::Notice {
                        id: new_id(),
                        ts: now_ms(),
                        level: NoticeLevel::Warn,
                        text: "Hotline restarted while this turn was running, so it was stopped. It was not run again: send it again if it still matters.".to_string(),
                    },
                );
                interrupted.push(session.persona_id.clone());
            }
            shut = self.shut_within(Duration::from_secs(10)).await;
        }
        if shut.is_none() {
            eprintln!(
                "[stop] work was still holding the room after it was stopped; syncing anyway"
            );
        }
        self.sync_all();
        Stopped {
            kept: count,
            interrupted,
            _shut: shut,
        }
    }

    /// The room's write lease, once every turn has let go of it.
    async fn shut_within(&self, wait: Duration) -> Option<tokio::sync::OwnedRwLockWriteGuard<()>> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Ok(held) = self.activity.clone().try_write_owned() {
                return Some(held);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn sync_all(&self) {
        let _ = self.log.sync(&StreamId::Room);
        for persona in room::roster(&self.log) {
            let _ = self.log.sync(&StreamId::Tape(persona.id));
        }
    }

    fn pending_path(&self) -> std::path::PathBuf {
        self.log.root().join(PENDING_FILE)
    }

    fn read_pending(&self) -> Vec<PendingLine> {
        match std::fs::read(self.pending_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                eprintln!("[restart] {PENDING_FILE} could not be read, so it is ignored: {error}");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        }
    }

    /// Adds to what is already kept, so lines a previous start could not hand
    /// on are not overwritten by this stop's.
    fn keep_pending(&self, lines: Vec<PendingLine>) -> std::io::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut all = self.read_pending();
        for line in lines {
            if !all.iter().any(|kept| kept.said == line.said) {
                all.push(line);
            }
        }
        self.write_pending(&all)
    }

    fn write_pending(&self, lines: &[PendingLine]) -> std::io::Result<()> {
        let path = self.pending_path();
        if lines.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let staged = path.with_extension(format!("json.{}", std::process::id()));
        let mut file = std::fs::File::create(&staged)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec(lines)?)?;
        file.sync_all()?;
        std::fs::rename(&staged, &path)
    }

    /// Hands each kept line to its teammate once, in order.
    pub(super) async fn resume_pending(self: &Arc<Self>) {
        let lines = self.read_pending();
        if lines.is_empty() {
            return;
        }
        let mut left = Vec::new();
        for line in lines {
            match self.resume_line(&line).await {
                Ok(()) => {}
                // A teammate that cannot start now keeps its line for the
                // next start rather than losing it; one that is gone, or a
                // line already read, is settled.
                Err(Some(error)) => {
                    eprintln!(
                        "[restart] a line for {} is kept for later: {error}",
                        line.persona_id
                    );
                    left.push(line);
                }
                Err(None) => {}
            }
        }
        if let Err(error) = self.write_pending(&left) {
            eprintln!("[restart] {PENDING_FILE} could not be updated: {error}");
        }
    }

    /// `Err(None)` is a line that no longer needs handing on.
    async fn resume_line(self: &Arc<Self>, line: &PendingLine) -> Result<(), Option<String>> {
        if !room::roster(&self.log)
            .iter()
            .any(|persona| persona.id == line.persona_id)
        {
            return Err(None);
        }
        let read = self
            .tape(&line.persona_id)
            .iter()
            .rev()
            .find(|event| event["id"] == line.said.as_str())
            .is_none_or(|event| event["receipt"] == "read");
        if read {
            return Err(None);
        }
        if let Some(run) = &line.scheduled
            && !schedule::scheduled_run_allowed(&self.log, &line.persona_id, run)
        {
            return Err(None);
        }
        let _working = self.working().map_err(Some)?;
        self.start(&line.persona_id).await.map_err(Some)?;
        let (session, _held) = self.in_this_chapter(&line.persona_id).await.map_err(Some)?;
        self.dispatch(
            session,
            Wired {
                text: line.text.clone(),
                attachments: line.attachments.clone(),
                scheduled: line.scheduled.clone(),
                steer: line.steer,
                said: Some(line.said.clone()),
                from: None,
                // A line kept across a stop is known by the id it was written
                // under: the one place the id of a spoken line is still read.
                voice: crate::voice::Origin::from_event_id(&line.said),
                spoken: line.said.starts_with("voice:"),
                unprompted: None,
            },
        );
        Ok(())
    }
}
