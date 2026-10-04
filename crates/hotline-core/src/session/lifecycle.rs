//! One lifecycle for the threads the room keeps.
//!
//! A thread is live, parked or closed (see [`crate::thread`]), and what moves
//! it between them is the same few things whatever its kind, bar what the
//! kind's [`Policy`] says: time passing with nobody speaking ([`Idle`]), the
//! process dying under it ([`Restart`]), and being closed, which writes a note.
//! This module is where they are, once:
//!
//! - **The sweep.** [`Room::sweep`] is run on the room's clock and applies
//!   every kind's `idle` policy. A side thread is parked there. A run is
//!   never idle-ended, and a call that nobody has spoken on for ten minutes is
//!   ended. The DM's chapters and a peer session's quiet clock are ported in
//!   later phases, so for now it calls the functions that own them.
//! - **The settle.** [`Room::settle`] is run once as the room opens. The last
//!   process's agents died with it, so a card it left open is expired and a
//!   thread it left live is moved by its kind's `restart` policy: a side
//!   thread is parked, a run is closed as cancelled, and a call is closed as
//!   stopped, its transcript kept. A peer exchange the
//!   process was answering is told to its sender by `reconcile_exchanges`,
//!   which the settle calls.
//! - **The closing note.** [`Room::queue_closing_note`] writes a thread's note
//!   through the summariser chapters use, once the thread has closed.
//! - **The link.** [`Room::write_link`] is the one line a thread leaves on its
//!   parent, written again under the same id as the thread goes.

use super::sides::{Ending, TITLE_CHARS, cut, outcome_line};
use super::{Room, lock, now_ms};
use crate::log::StreamId;
use crate::thread::{End, Idle, Link, Policy, Restart, ThreadId, ThreadKind, ThreadState};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// What a call is called until its closing note says better.
const CALL_TITLE: &str = "Call";

/// What a call says it came to when the desk restarted under it.
const ENDED_BY_RESTART: &str = "Ended when the desk restarted.";

/// A live thread, as the sweep sees it.
struct Quiet {
    thread: ThreadId,
    /// When anyone last spoke in it, or it started.
    since: i64,
    /// A turn is running in it.
    working: bool,
}

impl Room {
    /// The room, for work that outlives the call that began it: a closing
    /// note is a model call, and the person has long since been told the
    /// thread is closed.
    pub(super) fn this(&self) -> Option<Arc<Room>> {
        lock(&self.me).upgrade()
    }

    // The sweep.

    /// Applies every kind's idle policy once. See the module.
    pub(crate) async fn sweep(self: &Arc<Self>, now: i64, looked_again: &mut HashMap<String, i64>) {
        // The DM: its chapters close when the room's idle setting says so.
        self.sweep_chapters(looked_again).await;
        // A pair: a peer session that has gone quiet is the same kind of
        // fact as a chapter that has: nothing to arm when a message lands and
        // nothing to cancel when a teammate is deleted.
        self.sweep_peers(now);
        self.sweep_threads(now);
    }

    /// The idle policy of the kinds whose live threads the room holds in
    /// memory. A thread with a turn running is left alone.
    pub(super) fn sweep_threads(&self, now: i64) {
        for kind in [ThreadKind::Side, ThreadKind::Run, ThreadKind::Call] {
            for quiet in self.quiet(kind) {
                if quiet.working {
                    continue;
                }
                match Policy::of(kind).idle {
                    Idle::Park(after) if now - quiet.since >= after => {
                        self.end_thread(&quiet.thread, Ending::Park);
                    }
                    Idle::Close(after, end) if now - quiet.since >= after => {
                        self.end_thread(&quiet.thread, Ending::Close(end, None));
                    }
                    Idle::Park(_) | Idle::Close(..) | Idle::Chapters | Idle::Never => {}
                }
            }
        }
    }

    fn quiet(&self, kind: ThreadKind) -> Vec<Quiet> {
        match kind {
            ThreadKind::Side => self
                .sides
                .all()
                .iter()
                .map(|side| Quiet {
                    thread: ThreadId::side(&side.id),
                    since: side.last_used(),
                    working: side.working(),
                })
                .collect(),
            ThreadKind::Run => lock(&self.subagents)
                .values()
                .flatten()
                .map(|run| Quiet {
                    thread: ThreadId::run(&run.run_id),
                    since: run.started_at,
                    working: true,
                })
                .collect(),
            // A call that nobody has spoken on for a while: the voice keeps the
            // time, and the room's clock reads it. A desk call is here too,
            // with no thread behind it.
            ThreadKind::Call => lock(&self.voice)
                .upgrade()
                .map(|voice| {
                    let now = now_ms();
                    voice
                        .quiet()
                        .into_iter()
                        .map(|(call_id, quiet_ms)| Quiet {
                            thread: ThreadId::call(call_id),
                            since: now - quiet_ms,
                            working: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ThreadKind::Dm | ThreadKind::Pair => Vec::new(),
        }
    }

    /// Lets go of a live thread's agent, or closes it. A kind with no live
    /// handle in the room has nothing to end here.
    pub(super) fn end_thread(&self, thread: &ThreadId, ending: Ending) {
        match thread.kind {
            ThreadKind::Side => {
                if let Some(side) = self.sides.get(&thread.key) {
                    self.end_side(&side, ending);
                }
            }
            // Ending the call writes its link and takes its note.
            ThreadKind::Call => {
                if let Some(voice) = lock(&self.voice).upgrade() {
                    voice.end_quiet(&thread.key);
                }
            }
            ThreadKind::Run | ThreadKind::Dm | ThreadKind::Pair => {}
        }
    }

    // The settle.

    /// The startup fold and the index, brought in line with the files before
    /// anything is served from them.
    ///
    /// A permission or human-action card left open by the last process is a
    /// button nobody is behind, so it is expired, a thread it left live is
    /// moved by its kind's restart policy, and the stream compacted; then the
    /// index is synced, because the fold just rewrote files and a tape written
    /// by the importer or the previous edition has never been indexed here at
    /// all.
    ///
    /// Threads are settled with the tapes. A card raised inside a peer turn is
    /// written to the thread and nowhere else, and the resolver behind it only
    /// ever existed in the process that received the request — so a thread
    /// left unfolded draws a live button forever, on a stream nothing else
    /// revisits.
    pub(super) fn settle(&self) {
        let now = now_ms();
        let teammates: Vec<String> = crate::room::roster(&self.log)
            .into_iter()
            .map(|persona| persona.id)
            .chain(std::iter::once(crate::voice::TAPE_ID.to_string()))
            .collect();
        let mut closed = Vec::new();
        let streams = teammates.iter().cloned().map(StreamId::Tape).chain(
            crate::log::thread::list_all_keys(self.log.root())
                .into_iter()
                .map(StreamId::Pair),
        );
        for stream in streams {
            let events = self.log.load(&stream);
            let mut settled = crate::log::expire_orphaned_permissions(&events, now);
            if matches!(stream, StreamId::Tape(_)) {
                settled.extend(self.settle_links(&events, now, &mut closed));
            }
            for event in settled {
                if let Err(error) = self.log.append(&stream, &event) {
                    eprintln!("could not settle a line left open by the last process: {error}");
                }
            }
            if let Err(error) = self.log.compact(&stream) {
                eprintln!("could not compact a stream the startup fold rewrote: {error}");
            }
        }
        if let Some(indexer) = lock(&self.indexer).as_mut()
            && let Err(error) = indexer.sync(&teammates)
        {
            eprintln!("the search index could not be synced: {error}");
        }
        // What a pair was answering when the process died is told to the
        // teammate that asked.
        self.reconcile_exchanges();
        // A thread the restart closed is told as one that closed any other way
        // is: with the note its kind takes.
        for thread in closed {
            self.queue_closing_note(&thread, false);
        }
    }

    /// The threads a tape holds a link for that the last process left live:
    /// each is moved as its kind's restart policy says, on the thread's own
    /// stream (after expiring a card it left open) and, returned, on the tape.
    fn settle_links(&self, events: &[Value], now: i64, closed: &mut Vec<ThreadId>) -> Vec<Value> {
        let mut settled = Vec::new();
        for link in events.iter().filter_map(Link::read) {
            if link.state != ThreadState::Live {
                continue;
            }
            let state = match Policy::of(link.thread.kind).restart {
                Restart::Resume => continue,
                Restart::Park => ThreadState::Parked,
                Restart::Close(end) => ThreadState::Closed(end),
            };
            let mut link = Link {
                state,
                at: matches!(state, ThreadState::Closed(_)).then_some(now),
                ..link
            };
            if link.thread.kind == ThreadKind::Call {
                // The call stopped being spoken on when its last line was said,
                // not when the desk next opened.
                let last = link.thread.stream().and_then(|stream| {
                    self.log
                        .load(&stream)
                        .iter()
                        .filter_map(|event| event.get("ts").and_then(Value::as_i64))
                        .max()
                });
                link.at = Some(last.unwrap_or(link.ts).max(link.ts));
                link.outcome.get_or_insert_with(|| ENDED_BY_RESTART.into());
            }
            if matches!(state, ThreadState::Closed(_)) {
                closed.push(link.thread.clone());
            }
            if let Some(stream) = link.thread.stream() {
                let mut lines =
                    crate::log::expire_orphaned_permissions(&self.log.load(&stream), now);
                lines.push(link.event());
                for line in lines {
                    if let Err(error) = self.log.append(&stream, &line) {
                        eprintln!("could not settle the {}: {error}", link.thread);
                    }
                }
            }
            settled.push(link.event());
        }
        settled
    }

    // The link.

    /// The link a thread's own stream holds, as last written.
    pub(super) fn link_of(&self, thread: &ThreadId) -> Option<Link> {
        Link::find(&self.log.load(&thread.stream()?), thread)
    }

    /// The id a thread's link is written under: the one it already has, so a
    /// marker written before links is replaced and not joined by a second line.
    pub(super) fn link_id(&self, thread: &ThreadId) -> String {
        self.link_of(thread)
            .map_or_else(|| Link::fresh_id(thread), |link| link.id)
    }

    /// Writes the link on the thread's parent, when its kind has one, and at
    /// the head of its own stream, through the shared write path.
    pub(super) fn write_link(&self, link: &Link) {
        let Some(persona_id) = &link.persona_id else {
            return;
        };
        let event = link.event();
        let threads = self.threads();
        if Policy::of(link.thread.kind).surface.link {
            threads.write(&ThreadId::dm(persona_id), persona_id, &event);
        }
        threads.write(&link.thread, persona_id, &event);
    }

    // Calls.

    /// A call with a teammate has begun, or begun again under an id it
    /// had: its link goes on the teammate's DM and heads its own stream.
    pub(crate) fn call_began(&self, call_id: &str, persona_id: &str, ts: i64) {
        let thread = ThreadId::call(call_id);
        let earlier = self.link_of(&thread);
        self.write_link(&Link {
            id: earlier
                .as_ref()
                .map_or_else(|| Link::fresh_id(&thread), |link| link.id.clone()),
            ts: earlier.as_ref().map_or(ts, |link| link.ts),
            thread,
            persona_id: Some(persona_id.to_string()),
            title: CALL_TITLE.into(),
            state: ThreadState::Live,
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms: None,
        });
    }

    /// One line said on a call, to its thread.
    pub(crate) fn call_said(&self, call_id: &str, persona_id: &str, line: &Value) {
        self.threads()
            .write(&ThreadId::call(call_id), persona_id, line);
    }

    /// A call has ended: its link says how, and the note a closed thread of its
    /// kind takes is queued.
    pub(crate) fn call_ended(
        &self,
        call_id: &str,
        persona_id: &str,
        end: End,
        outcome: &str,
        ts: i64,
    ) {
        let thread = ThreadId::call(call_id);
        let began = self.link_of(&thread);
        self.write_link(&Link {
            id: began
                .as_ref()
                .map_or_else(|| Link::fresh_id(&thread), |link| link.id.clone()),
            ts: began.as_ref().map_or(ts, |link| link.ts),
            thread: thread.clone(),
            persona_id: Some(persona_id.to_string()),
            title: CALL_TITLE.into(),
            state: ThreadState::Closed(end),
            outcome: Some(outcome.to_string()),
            at: Some(ts),
            note: None,
            binding: None,
            elapsed_ms: None,
        });
        self.queue_closing_note(&thread, false);
    }

    // The closing note.

    /// Writes the closing note of a thread that has just closed, once a model
    /// has written it, in the background: the person was told the thread is
    /// closed the moment they pressed the button, and the note is a model
    /// call. Only a kind whose policy has one writes it.
    ///
    /// The note is produced the way a chapter's is ([`Room::note`]): the same
    /// summariser over the thread's stream, so it reads goal, what got done,
    /// what is still open and the key files. It replaces the link's title with
    /// the note's, and its outcome becomes the one-line outcome unless the
    /// teammate wrote one itself. A link that has moved on since — the thread
    /// was continued, or closed again — is left alone.
    pub(super) fn queue_closing_note(&self, thread: &ThreadId, said_so: bool) {
        if !Policy::of(thread.kind).closing_note {
            return;
        }
        let Some(room) = self.this() else {
            return;
        };
        let Some(link) = self.link_of(thread) else {
            return;
        };
        let (Some(persona_id), Some(closed_at), Some(stream)) =
            (link.persona_id.clone(), link.at, thread.stream())
        else {
            return;
        };
        let (Ok(persona), Ok(runtime)) = (
            self.persona(&persona_id),
            tokio::runtime::Handle::try_current(),
        ) else {
            return;
        };
        let thread = thread.clone();
        runtime.spawn(async move {
            let Ok(_working) = room.working() else {
                return;
            };
            let slice = room.log.load(&stream);
            if !slice.iter().any(crate::store::chapters::is_message) {
                return;
            }
            let Some(note) = room.note(&persona, &slice).await else {
                eprintln!("the {thread} was closed without a closing note: no model answered");
                return;
            };
            let Some(mut link) = room.link_of(&thread) else {
                return;
            };
            if !matches!(link.state, ThreadState::Closed(_)) || link.at != Some(closed_at) {
                return;
            }
            link.title = cut(&note.title, TITLE_CHARS);
            if !said_so && let Some(outcome) = outcome_line(&note.note) {
                link.outcome = Some(outcome);
            }
            link.note = Some(note.note);
            room.write_link(&link);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{RunningSubagent, SideEnd};
    use crate::driver::{MessageKind, Update};
    use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
    use crate::thread::{End, SIDE_IDLE_MS, ThreadStore};
    use serde_json::json;
    use std::time::Duration;

    fn room_on(log: crate::log::Log, agents: Arc<Fake>) -> Arc<Room> {
        Room::with_agents_and_computers(
            log,
            Arc::new(DeskKeys),
            agents,
            crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
        )
    }

    fn quiet_agents() -> Arc<Fake> {
        Fake::new(Scripted::new(vec![
            Update::Message {
                kind: MessageKind::Agent,
                id: "m1".into(),
                text: "On it.".into(),
            },
            Update::Turn {
                stop_reason: "end_turn".into(),
                usage: None,
            },
        ]))
    }

    fn stored(room: &Room, stream: StreamId) -> Vec<Value> {
        room.log.load(&stream)
    }

    async fn idle(room: &Arc<Room>, thread: &ThreadId) {
        for _ in 0..500 {
            if room
                .sides
                .get(&thread.key)
                .is_none_or(|side| !side.working())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the {thread} never finished its turn");
    }

    #[tokio::test]
    async fn one_sweep_parks_a_side_at_its_idle_and_leaves_the_other_kinds() {
        let log = scratch("sweep-one");
        enrol(&log, &persona("ada"));
        let room = room_on(log, quiet_agents());
        let summary = room.start_side("ada", "Look at the winch").await.unwrap();
        let side = ThreadId::side(&summary.side_id);
        idle(&room, &side).await;
        // A run is on the roster, and has been for longer than any idle.
        room.list_subagent(
            "ada",
            RunningSubagent {
                run_id: "r1".into(),
                title: "Check the crane".into(),
                started_at: 1,
            },
            true,
        );

        let mut looked_again = HashMap::new();
        room.sweep(now_ms() + SIDE_IDLE_MS - 60_000, &mut looked_again)
            .await;
        assert!(
            room.sides.get(&side.key).is_some(),
            "not idle for long enough"
        );

        room.sweep(now_ms() + SIDE_IDLE_MS + 60_000, &mut looked_again)
            .await;
        assert!(
            room.sides.get(&side.key).is_none(),
            "its agent is let go of"
        );
        assert_eq!(
            room.link_of(&side).unwrap().state,
            ThreadState::Parked,
            "and the thread stays open"
        );
        assert_eq!(
            room.subagents("ada").len(),
            1,
            "a run has no idle: it ends with its work"
        );
        assert_eq!(
            ThreadStore::new(&room.log)
                .load(&ThreadId::dm("ada"))
                .unwrap()
                .state,
            ThreadState::Live,
            "and the DM is not a thread the sweep parks"
        );
    }

    #[tokio::test]
    async fn one_settle_moves_every_kind_the_last_process_left_open() {
        let log = scratch("settle-kinds");
        enrol(&log, &persona("ada"));
        let tape = StreamId::Tape("ada".into());
        let card = |id: &str| json!({"kind": "permission", "id": id, "ts": 2, "requestId": id, "title": "read a file", "options": []});
        // A side thread and a run the last process left going, in the shape
        // a link has, and a pair of each in the shape that came before it.
        let side = ThreadId::side("s1");
        let run = ThreadId::run("r1");
        let live = |thread: &ThreadId, title: &str| Link {
            id: Link::fresh_id(thread),
            ts: 5,
            thread: thread.clone(),
            persona_id: Some("ada".into()),
            title: title.into(),
            state: ThreadState::Live,
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms: None,
        };
        for link in [live(&side, "A side"), live(&run, "A run")] {
            log.append(&tape, &link.event()).unwrap();
            log.append(&link.thread.stream().unwrap(), &link.event())
                .unwrap();
        }
        log.append(&StreamId::Side("s1".into()), &card("side-card"))
            .unwrap();
        let old_side = json!({
            "kind": "side", "id": "side:s2", "ts": 6, "sideId": "s2",
            "personaId": "ada", "title": "An old side", "status": "live"
        });
        let old_run = json!({
            "kind": "subagent", "id": "subagent:r2", "ts": 7, "runId": "r2",
            "title": "An old run", "status": "running"
        });
        for (stream, marker) in [
            (StreamId::Side("s2".into()), &old_side),
            (StreamId::Run("r2".into()), &old_run),
        ] {
            log.append(&tape, marker).unwrap();
            log.append(&stream, marker).unwrap();
        }
        // A card on the tape, on a pair, and on the voice front's own tape.
        log.append(&tape, &card("tape-card")).unwrap();
        enrol(&log, &persona("bob"));
        let key = crate::paths::thread_key("ada", "bob").unwrap();
        crate::log::thread::ensure(log.root(), &key).unwrap();
        log.append(&StreamId::Pair(key.clone()), &card("pair-card"))
            .unwrap();
        let voice = StreamId::Tape(crate::voice::TAPE_ID.into());
        log.append(&voice, &card("call-card")).unwrap();

        let room = room_on(log, quiet_agents());

        let link = |thread: &ThreadId| room.link_of(thread).unwrap();
        assert_eq!(link(&side).state, ThreadState::Parked);
        assert_eq!(link(&ThreadId::side("s2")).state, ThreadState::Parked);
        assert_eq!(link(&run).state, ThreadState::Closed(End::Cancelled));
        assert_eq!(
            link(&ThreadId::run("r2")).state,
            ThreadState::Closed(End::Cancelled)
        );
        // The tape's copy of each is the one that moved, under its own id.
        let on_tape: Vec<Link> = stored(&room, tape.clone())
            .iter()
            .filter_map(Link::read)
            .collect();
        assert_eq!(on_tape.len(), 4, "none of them is doubled");
        assert!(on_tape.iter().all(|link| link.state != ThreadState::Live));
        assert!(on_tape.iter().any(|link| link.id == "side:s2"));
        // Every card a dead process was behind is expired.
        let decision = |events: Vec<Value>, id: &str| {
            events
                .into_iter()
                .find(|event| event["id"] == id)
                .map(|event| event["decision"].clone())
        };
        assert_eq!(
            decision(stored(&room, StreamId::Side("s1".into())), "side-card"),
            Some(json!("expired"))
        );
        assert_eq!(
            decision(stored(&room, tape), "tape-card"),
            Some(json!("expired"))
        );
        assert_eq!(
            decision(stored(&room, StreamId::Pair(key)), "pair-card"),
            Some(json!("expired"))
        );
        assert_eq!(
            decision(stored(&room, voice), "call-card"),
            Some(json!("expired"))
        );
    }

    #[tokio::test]
    async fn a_link_is_rewritten_by_id_as_the_thread_goes() {
        let log = scratch("link-rewrite");
        enrol(&log, &persona("ada"));
        let room = room_on(log, quiet_agents());
        let summary = room.start_side("ada", "Look at the winch").await.unwrap();
        let thread = ThreadId::side(&summary.side_id);
        idle(&room, &thread).await;
        room.archive_side(&summary.side_id, SideEnd::Person, None)
            .unwrap();
        room.continue_side(&summary.side_id).await.unwrap();
        idle(&room, &thread).await;
        room.archive_side(&summary.side_id, SideEnd::Agent, Some("Done.".into()))
            .unwrap();

        // Started, closed, brought back, closed again: still one line on the
        // tape and one at the head of the thread, under one id.
        let id = Link::fresh_id(&thread);
        for stream in [StreamId::Tape("ada".into()), thread.stream().unwrap()] {
            let links: Vec<Value> = stored(&room, stream)
                .into_iter()
                .filter(|event| event["kind"] == "link")
                .collect();
            assert_eq!(links.len(), 1, "{links:?}");
            assert_eq!(links[0]["id"], id);
            assert_eq!(links[0]["ts"], room.link_of(&thread).unwrap().ts);
            assert_eq!(links[0]["state"], "closed");
            assert_eq!(links[0]["end"], "agent");
            assert_eq!(links[0]["outcome"], "Done.");
        }
    }

    #[tokio::test]
    async fn a_marker_from_before_links_still_loads_renders_resumes_and_is_found() {
        let log = scratch("link-old");
        enrol(&log, &persona("ada"));
        let tape = StreamId::Tape("ada".into());
        let marker = json!({
            "kind": "side", "id": "side:old", "ts": 5, "sideId": "old",
            "personaId": "ada", "title": "Mend the crane", "status": "archived",
            "result": "Welded.", "archivedBy": "person", "archivedAt": 9,
            "note": "Goal: the crane. Outcome: welded the jib."
        });
        log.append(&tape, &marker).unwrap();
        let stream = StreamId::Side("old".into());
        log.append(&stream, &marker).unwrap();
        log.append(
            &stream,
            &json!({"kind": "user", "id": "u1", "ts": 6, "text": "mind the jib"}),
        )
        .unwrap();
        log.append(
            &stream,
            &json!({"kind": "agent", "id": "a1", "ts": 7, "text": "Welded the jib."}),
        )
        .unwrap();
        let room = room_on(log, quiet_agents());

        // It loads.
        let loaded = ThreadStore::new(&room.log)
            .load(&ThreadId::side("old"))
            .unwrap();
        assert_eq!(loaded.state, ThreadState::Closed(End::Person));
        assert_eq!(loaded.title.as_deref(), Some("Mend the crane"));
        assert_eq!(loaded.parent.unwrap().event.as_deref(), Some("side:old"));
        // It renders: a client is sent the very line it always was.
        assert_eq!(Link::wire(stored(&room, tape.clone())[0].clone()), marker);
        let listed = room.side_threads("ada");
        assert_eq!(listed[0].status, crate::contract::SideStatus::Archived);
        assert_eq!(listed[0].result.as_deref(), Some("Welded."));
        // It is found by search, as the line it left behind.
        let hits = crate::store::search::search(room.log.root(), "ada", "jib", Some(5)).unwrap();
        assert!(serde_json::to_string(&hits).unwrap().contains("side:old"));
        // It resumes, and is rewritten as a link under the id it had.
        room.continue_side("old").await.unwrap();
        idle(&room, &ThreadId::side("old")).await;
        let lines = stored(&room, tape);
        let on_tape: Vec<&Value> = lines
            .iter()
            .filter(|event| event["kind"] == "link" || event["kind"] == "side")
            .collect();
        assert_eq!(on_tape.len(), 1, "replaced in place: {on_tape:?}");
        assert_eq!(on_tape[0]["kind"], "link");
        assert_eq!(on_tape[0]["id"], "side:old");
        assert_eq!(on_tape[0]["state"], "live");
        assert_eq!(on_tape[0]["ts"], 5);
        assert_eq!(Link::wire(on_tape[0].clone())["status"], "live");
    }
    #[tokio::test]
    async fn a_call_the_last_process_left_going_is_closed_with_its_transcript_kept() {
        let log = scratch("settle-call");
        enrol(&log, &persona("ada"));
        let call = ThreadId::call("c1");
        let live = Link {
            id: Link::fresh_id(&call),
            ts: 1_000,
            thread: call.clone(),
            persona_id: Some("ada".into()),
            title: "Call".into(),
            state: ThreadState::Live,
            outcome: None,
            at: None,
            note: None,
            binding: None,
            elapsed_ms: None,
        };
        let tape = StreamId::Tape("ada".into());
        let stream = call.stream().unwrap();
        log.append(&tape, &live.event()).unwrap();
        log.append(&stream, &live.event()).unwrap();
        for (kind, id, ts, text) in [
            ("user", "u1", 61_000, "Is the winch jammed?"),
            ("agent", "a1", 241_000, "It was the grease."),
        ] {
            log.append(
                &stream,
                &json!({"kind": kind, "id": id, "ts": ts, "text": text}),
            )
            .unwrap();
        }

        let room = room_on(log, quiet_agents());

        let closed = room.link_of(&call).unwrap();
        assert_eq!(closed.state, ThreadState::Closed(End::Stopped));
        assert_eq!(closed.at, Some(241_000), "when it was last spoken on");
        assert_eq!(closed.outcome.as_deref(), Some(ENDED_BY_RESTART));
        let on_tape: Vec<Link> = stored(&room, tape).iter().filter_map(Link::read).collect();
        assert_eq!(on_tape.len(), 1);
        assert_eq!(on_tape[0].state, ThreadState::Closed(End::Stopped));
        // The client is sent a quiet line about it: how long, and how it ended.
        let sent = Link::wire(closed.event());
        assert_eq!(sent["kind"], "call");
        assert_eq!(sent["callId"], "c1");
        assert_eq!(sent["status"], "ended");
        assert_eq!(sent["durationMs"], 240_000);
        assert_eq!(sent["outcome"], ENDED_BY_RESTART);

        // What was said is still there, loads as a closed thread, and is found.
        assert_eq!(stored(&room, stream).len(), 3, "its link and what was said");
        let loaded = ThreadStore::new(&room.log).load(&call).unwrap();
        assert_eq!(loaded.state, ThreadState::Closed(End::Stopped));
        assert_eq!(loaded.parent.unwrap().thread, ThreadId::dm("ada"));
        let found =
            crate::store::search::search_teammate(room.log.root(), "ada", "grease", None).unwrap();
        assert_eq!(found["hits"][0]["thread"], "call:c1");
    }
}
