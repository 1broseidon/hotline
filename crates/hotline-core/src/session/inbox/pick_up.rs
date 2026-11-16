//! The pick-up: what a teammate set aside in its inbox is offered again,
//! once, when its main conversation goes idle.
//!
//! A teammate busy on the person's work sorts what arrives meanwhile, and
//! leaves some of it `later` ("finishing the research first"). Nothing new
//! lands to wake it once that work is done, so the items it set aside would
//! wait for some unrelated turn. The pick-up closes that gap:
//!
//! - **What it offers.** The items heard in the main conversation that the
//!   teammate has not finished sorting ([`Room::set_aside`]): those `later`,
//!   and those it was shown (offered at a turn's start, or read) and left
//!   unsorted. Not `taken`, which is being worked, and not `closed`. Items no
//!   turn has been offered yet are not its: they wake the conversation by
//!   their own rule (`heard.rs`), and that wake wins.
//! - **When.** A turn of the main conversation ends and the conversation is
//!   idle with items set aside that no pick-up has offered yet; the gather
//!   delay later, if it is still idle with nothing queued and the turn that
//!   armed it is still the last one, one quiet turn starts whose line lists
//!   those items, each `later` one with its reason ([`PICK_UP_LINE`]).
//!   Anything that starts a turn in between (the person's line, a new
//!   item's wake, a post) wins, and that turn's end arms its own pick-up.
//! - **Once an item.** A turn's end offers an item once: the first time the
//!   conversation goes idle after it was set aside or left unsorted. From
//!   then on only its own clock offers it, however many turns the person
//!   starts, so a busy conversation never buys a pick-up a turn.
//! - **Never a loop.** A turn the pick-up started arms nothing when it ends
//!   ([`PickUp::ended`]), whatever it leaves open: the next pick-up can only
//!   follow a turn something else started. A turn the person stopped or
//!   interrupted, a cancelled or failed one, arms nothing either, and the
//!   person's Stop while one waits disarms it.
//! - **Again, on its own time.** Each item a pick-up offers is marked with
//!   when it is offered next (`offer_again`: an hour after its first
//!   pick-up, then a day after each); the clock (`clock.rs`) offers what has
//!   come due in the same quiet turn ([`Due::Timed`]), and expires what stays
//!   open three days. A turn's end never offers a marked item.
//! - **Nothing fires** for a teammate that is not running, was removed, or
//!   whose room is closing; work threads and calls have no pick-up.
//! - **When the desk opens** ([`Room::inbox_at_start`]), once the room has
//!   handed on what it resumes, each teammate with items set aside is offered
//!   them in the same quiet turn, started for it as the new-item wake starts
//!   one: a gather delay apart, one teammate after another, so a launch does
//!   not start them all at once. It is offered once per launch, only while the
//!   conversation is idle with nothing queued and has had no turn yet (a turn
//!   it had offers by its own end), and like every pick-up its turn arms
//!   nothing. Items no turn has been offered are left to their own wake at
//!   start (`Room::recover_inbox`).

use super::clock;
use super::heard::{gather_delay, nobody_reads, since};
use super::{Room, kept, lock, now_ms, short};
use crate::contract::{InboxItem, InboxState, ThreadId};
use crate::fence::{fenced, quoted};
use crate::session::turns::Occupant;
use crate::session::{Wired, timed};
use std::sync::Arc;
use std::time::Duration;

/// What the pick-up's line says before the items and after them. The turn
/// is quiet, as the inbox's own wake is, and says so in the same words.
pub(super) const PICK_UP_LINE: (&str, &str) = (
    "quiet inbox · You are idle now. These items in your inbox are still open: while you worked you set them aside for later, or were shown them and left them unsorted.",
    concat!(
        "Pick up what is ready now: read, take, answer or close it with `update_inbox`, and leave for later, with why, what still waits on something. What you leave open now is offered again an hour after its first offer like this, then daily, and three days after it arrived it closes as expired and its sender is told. ",
        nobody_reads!()
    ),
);

/// Which pick-up is due: the one a turn's end armed, or the one a desk that
/// opens offers.
#[derive(Clone, Copy, Debug)]
enum Due {
    /// Armed by the end of this turn, by its number.
    After(u64),
    /// The desk opened: due while the conversation has had no turn.
    AtStart,
    /// Items whose own time to be offered again (`offer_again`) has come by
    /// this time (`clock.rs`): due while the conversation is idle.
    Timed(i64),
}

/// Whether the main conversation's last turn may be followed by a pick-up,
/// kept on its live session.
#[derive(Debug, Default)]
pub(in crate::session) struct PickUp {
    /// How many turns have ended, so a pick-up armed by one turn is known
    /// stale once another has begun or ended.
    ended: u64,
    /// The turn whose end armed a pick-up, while it waits.
    armed: Option<u64>,
}

impl PickUp {
    /// A turn began: it hears the inbox in its index, so what the last one
    /// armed is spent.
    pub(in crate::session) fn began(&mut self) {
        self.armed = None;
    }

    /// A turn ended. `may` says whether a pick-up may follow it: never after
    /// a pick-up's own turn, nor after one the person stopped.
    pub(in crate::session) fn ended(&mut self, may: bool) {
        self.ended += 1;
        self.armed = may.then_some(self.ended);
    }

    /// The person said stop: nothing waits to be offered.
    pub(in crate::session) fn disarm(&mut self) {
        self.armed = None;
    }

    /// The turn a waiting pick-up follows.
    pub(in crate::session) fn armed(&self) -> Option<u64> {
        self.armed
    }

    /// Whether `due` may still be offered: the turn that armed it is still
    /// the last one, or, at start, no turn has ended.
    fn holds(&self, due: Due) -> bool {
        match due {
            Due::After(turn) => self.armed == Some(turn),
            Due::AtStart => self.ended == 0,
            Due::Timed(_) => true,
        }
    }
}

impl Room {
    /// The items heard in `persona_id`'s main conversation that it has not
    /// finished sorting, oldest first: `later`, or shown to it and unsorted.
    pub(in crate::session) fn set_aside(&self, persona_id: &str) -> Vec<InboxItem> {
        let main = ThreadId::dm(persona_id);
        kept::received(&self.log, persona_id)
            .into_iter()
            .filter(|item| {
                let open = match item.state {
                    InboxState::Later => true,
                    InboxState::New | InboxState::Noticed | InboxState::Seen => {
                        item.offered.is_some()
                    }
                    InboxState::Taken | InboxState::Closed => false,
                };
                open && self.inbox_heard_in(item) == main
            })
            .collect()
    }

    /// The main conversation is idle after a turn: when that turn armed a
    /// pick-up and something is set aside, it is offered once the gather
    /// delay is up.
    pub(super) fn arm_pick_up(self: &Arc<Self>, persona_id: &str) {
        let Ok(session) = self.session(persona_id) else {
            return;
        };
        let Some(turn) = lock(&session.pick_up).armed() else {
            return;
        };
        if self.offered_by(persona_id, Due::After(turn)).is_empty() {
            return;
        }
        let room = Arc::downgrade(self);
        let persona_id = persona_id.to_string();
        let delay = self.pick_up_delay();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(room) = room.upgrade() {
                room.pick_up(&persona_id, turn).await;
            }
        });
    }

    /// How long a pick-up waits: the gather delay.
    fn pick_up_delay(&self) -> Duration {
        #[cfg(test)]
        if let Some(delay) = *lock(&self.pick_up_delay) {
            return delay;
        }
        gather_delay()
    }

    /// The desk has opened and handed on what it resumes: each teammate
    /// whose main conversation has items set aside is offered them once, a
    /// gather delay apart, one after another ([`Room::inbox_at_start`], once
    /// per room).
    pub(super) fn pick_up_at_start(self: &Arc<Self>) {
        let mut after = Duration::ZERO;
        for persona in crate::room::roster(&self.log) {
            if self.set_aside(&persona.id).is_empty() {
                continue;
            }
            after += self.pick_up_delay();
            let room = Arc::downgrade(self);
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                if let Some(room) = room.upgrade() {
                    room.offer_set_aside(&persona.id, Due::AtStart).await;
                }
            });
        }
    }

    /// The pick-up armed by `persona_id`'s turn number `turn` is due.
    pub(in crate::session) async fn pick_up(self: &Arc<Self>, persona_id: &str, turn: u64) {
        self.offer_set_aside(persona_id, Due::After(turn)).await;
    }

    /// The items a pick-up that is `due` offers: after a turn, what is set
    /// aside that no pick-up has offered yet, so a turn's end offers an item
    /// once and its clock the rest; for a timed one, what has come due; at
    /// start, all of it.
    fn offered_by(&self, persona_id: &str, due: Due) -> Vec<InboxItem> {
        let items = self.set_aside(persona_id);
        match due {
            Due::After(_) => items
                .into_iter()
                .filter(|item| item.offer_again.is_none())
                .collect(),
            Due::Timed(now) => items
                .into_iter()
                .filter(|item| item.offer_again.is_some_and(|at| at <= now))
                .collect(),
            Due::AtStart => items,
        }
    }

    /// What set aside has come due to be offered again by `now`
    /// (`clock.rs`): one quiet turn offers it all.
    pub(in crate::session) async fn offer_again(self: &Arc<Self>, persona_id: &str, now: i64) {
        self.offer_set_aside(persona_id, Due::Timed(now)).await;
    }

    /// One quiet turn offers what `persona_id` set aside, if the
    /// conversation is idle, nothing is queued and `due` still holds. Only
    /// the pick-up at start starts a teammate that is not running. Each item
    /// it offers is due to be offered again an hour after its first pick-up,
    /// then a day after each.
    async fn offer_set_aside(self: &Arc<Self>, persona_id: &str, due: Due) {
        // A teammate held at its hourly limit is offered nothing quietly.
        if self.closing()
            || self.persona(persona_id).is_err()
            || self.quiet_held(persona_id).is_some()
        {
            return;
        }
        // Items no turn has been offered wake it by their own rule.
        let unoffered = |room: &Self| {
            !room
                .unoffered(persona_id, &ThreadId::dm(persona_id))
                .is_empty()
        };
        if unoffered(self) || self.offered_by(persona_id, due).is_empty() {
            return;
        }
        if matches!(due, Due::AtStart)
            && self.session(persona_id).is_err()
            && self.start(persona_id).await.is_err()
        {
            return;
        }
        let Ok(session) = self.session(persona_id) else {
            return;
        };
        if !session.answering(self) || unoffered(self) {
            return;
        }
        let items = self.offered_by(persona_id, due);
        if items.is_empty() {
            return;
        }
        let Ok(working) = self.lease() else {
            return;
        };
        let line = Wired {
            quiet: true,
            pick_up: true,
            woke: items.iter().map(|item| item.id.clone()).collect(),
            ..Wired::words(timed(now_ms(), &self.pick_up_line(&items)))
        };
        let claimed = {
            let mut turns = lock(&session.turns);
            let mut pick_up = lock(&session.pick_up);
            let idle = !turns.running && turns.waiting.is_empty();
            if idle && pick_up.holds(due) && lock(&session.cut).is_none() {
                pick_up.disarm();
                turns.claim(line)
            } else {
                None
            }
        };
        if let Some(line) = claimed {
            let offered_at = match due {
                Due::Timed(now) => now,
                Due::After(_) | Due::AtStart => now_ms(),
            };
            for item in &items {
                self.change_item(persona_id, &item.id, |kept| {
                    kept.offer_again = Some(
                        offered_at
                            + if kept.offer_again.is_none() {
                                clock::again_after_first()
                            } else {
                                clock::again_every()
                            },
                    );
                    true
                });
            }
            let room = self.clone();
            tokio::spawn(async move {
                let _working = working;
                room.run_queue(session, line).await;
            });
        }
    }

    /// The pick-up's line: what this is, the items as the index lists them
    /// with where each stands (a `later` one with its reason), and who reads
    /// the turn.
    pub(super) fn pick_up_line(&self, items: &[InboxItem]) -> String {
        let now = now_ms();
        let lines: Vec<String> = items
            .iter()
            .map(|item| self.set_aside_line(item, now))
            .collect();
        let (before, after) = PICK_UP_LINE;
        format!(
            "{before}\n{}\n{after}",
            fenced("hotline_inbox", &lines.join("\n"))
        )
    }

    /// One item set aside: title, sender, whether the sender waits, what it
    /// wants, its age, where it stands and its id.
    fn set_aside_line(&self, item: &InboxItem, now: i64) -> String {
        let waiting = if item.waiting {
            " · waiting on you"
        } else {
            ""
        };
        let stands = match item.state {
            InboxState::Later => format!(
                "for later{}",
                item.reason
                    .as_deref()
                    .map_or_else(String::new, |why| format!(": {}", quoted(why)))
            ),
            InboxState::Seen => "read, not sorted".to_string(),
            _ => "not read".to_string(),
        };
        let answers = item
            .reply_to
            .as_deref()
            .map_or_else(String::new, |id| format!(" · answers {}", short(id)));
        format!(
            "{} · from {}{waiting} · wants {} · {}{answers} · {stands} · {}",
            quoted(&item.title),
            self.sender_of(item),
            item.wants.name(),
            since(now - item.ts),
            short(&item.id)
        )
    }
}
