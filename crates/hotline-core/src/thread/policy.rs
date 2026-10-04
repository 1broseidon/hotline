//! What a kind of thread decides, as values.
//!
//! A kind is not a type with its own methods. It is a row in this table, and
//! the write path and the agent builder read the row. Only the decisions they
//! make are here; each later phase adds the ones it needs.

use super::{End, ThreadKind};

/// Who may answer a card raised in the thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The person, from whatever surface they are on.
    Person,
    /// Nobody. A card raised here is expired at once, so the agent that raised
    /// it is refused rather than left waiting for a button nobody can press.
    Nobody,
}

/// What a thread shows beyond itself when a card is raised in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// Push the card to the person's phone.
    pub push_cards: bool,
    /// Tell a live call on the parent DM that a card is waiting.
    pub mirror_to_call: bool,
    /// Index the thread's messages, so `search_thread` finds them.
    pub index: bool,
    /// Write one `link` line on the parent that stands for the thread.
    pub link: bool,
}

/// What a fresh agent is told about the conversation it joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seed {
    /// The parent's last chapter note and the last lines said in it.
    pub parent_tail: bool,
    /// What the thread itself has already said, so an agent that cannot
    /// reopen its own session picks the thread up where it stood.
    pub own_history: bool,
    /// The DM's own seed: the open chapter's lines as the agent's history, and
    /// the wake block, which says how the chapter before it closed.
    pub chapters: bool,
}

/// How a thread's agent picks its conversation up after it was let go of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// The teammate's own session: the checkpoint on its record, which the
    /// agent is started with. Only the DM has one, and it is the only thread
    /// that writes it.
    Checkpoint,
    /// The session the thread saved on its own link.
    Binding,
    /// Nothing is reopened: the agent starts fresh.
    Never,
}

/// What a thread's agent is handed of the teammate's computer, and how long
/// the start waits for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Computer {
    /// None.
    No,
    /// The teammate's computer, shared through a lease. The start waits for it
    /// however long it takes, and goes on without it if it cannot come up.
    Lease,
    /// The same, but a start does not wait for a download: it goes ahead
    /// without the computer, which joins once the turn in flight ends.
    Download,
}

/// Which of the teammate's tools the thread's agent is served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tools {
    /// All of them, with subagents.
    Teammate,
    /// A work thread's: the teammate's tools, but the chapter moves, which
    /// belong to the DM, and `archive_thread`, addressed by the thread's id.
    Work,
    /// A subagent run's: its teammate's conversation to read, no subagents.
    Run,
    /// What a peer session answers a colleague with.
    Peer,
    /// None: the thread has no agent of its own.
    None,
}

/// What an idle thread comes to, and when. The one sweep reads this for every
/// kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// The DM is not idle-swept as a thread: its chapters close when the
    /// room's idle setting says so.
    Chapters,
    /// The agent is let go of after this many milliseconds with nobody
    /// speaking, and the thread stays open.
    Park(i64),
    /// The thread ends this long after anyone last spoke.
    Close(i64, End),
    /// Idle does not end it: it lasts as long as the work it is for.
    Never,
}

/// What a restart does to a thread the last process left live. Its agent died
/// with that process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// Nothing is done: the thread is reopened from its record.
    Resume,
    /// It is parked, and the next line said in it starts an agent again.
    Park,
    /// It is closed, with this end, and its transcript kept.
    Close(End),
}

/// How long a side thread may sit with nobody speaking in it before it is
/// parked. Hours, not minutes: the person may leave a thread to think and come
/// back after lunch.
pub const SIDE_IDLE_MS: i64 = 3 * 60 * 60_000;

/// How long a peer session or a call may go quiet.
pub const QUIET_MS: i64 = 10 * 60_000;

/// How a thread's authority derives from the one it hangs off. Never wider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lease {
    /// The parent session's own.
    Same,
    /// A child of the parent's, revoked with it.
    Scoped,
    /// Its own, which the parent session ending does not end. It is revoked
    /// with the teammate's authority, not with a session of it.
    Independent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub answer: Answer,
    pub surface: Surface,
    pub seed: Seed,
    pub tools: Tools,
    pub lease: Lease,
    pub idle: Idle,
    pub restart: Restart,
    /// Whether closing the thread writes a note through the chapter
    /// summariser. A run's report is a job result, so it has none.
    pub closing_note: bool,
    /// Whether the agent is handed the teammate's computer. The teammate has
    /// one, so threads share it through a lease: one drives it at a time
    /// (`computer/gate.rs`), and the thread's lease ending ends its hold.
    pub computer: Computer,
    /// How a fresh agent picks the conversation up again.
    pub resume: Resume,
}

impl Policy {
    /// Whether the agent is handed the teammate's computer at all.
    pub fn has_computer(self) -> bool {
        self.computer != Computer::No
    }
}

impl Policy {
    pub fn of(kind: ThreadKind) -> Self {
        match kind {
            // A pair is here for the table's sake: nothing writes it through
            // the shared path yet, and until it does a card raised in one has
            // no answer path (docs/threads.md, phase 6).
            ThreadKind::Dm => Self {
                tools: Tools::Teammate,
                lease: Lease::Same,
                seed: Seed {
                    parent_tail: false,
                    own_history: false,
                    chapters: true,
                },
                computer: Computer::Download,
                resume: Resume::Checkpoint,
                idle: Idle::Chapters,
                restart: Restart::Resume,
                ..Self::asked_of_the_person()
            },
            ThreadKind::Side => Self {
                tools: Tools::Work,
                lease: Lease::Independent,
                idle: Idle::Park(SIDE_IDLE_MS),
                restart: Restart::Park,
                closing_note: true,
                surface: Surface {
                    link: true,
                    ..Self::asked_of_the_person().surface
                },
                seed: Seed {
                    parent_tail: true,
                    own_history: true,
                    chapters: false,
                },
                computer: Computer::Lease,
                resume: Resume::Binding,
                ..Self::asked_of_the_person()
            },
            ThreadKind::Pair => Self {
                tools: Tools::Peer,
                lease: Lease::Scoped,
                seed: Seed {
                    parent_tail: false,
                    own_history: true,
                    chapters: false,
                },
                computer: Computer::Lease,
                resume: Resume::Binding,
                idle: Idle::Park(QUIET_MS),
                restart: Restart::Park,
                ..Self::asked_of_the_person()
            },
            // A call signals a card and never answers it, and a run has no
            // one to answer: both expire what they raise. A call has no agent
            // of its own yet, and a run starts fresh and is never reopened.
            ThreadKind::Call => Self {
                idle: Idle::Close(QUIET_MS, End::Idle),
                restart: Restart::Close(End::Stopped),
                closing_note: true,
                ..Self::unattended(Tools::None, Lease::Same)
            },
            ThreadKind::Run => Self {
                idle: Idle::Never,
                restart: Restart::Close(End::Cancelled),
                ..Self::unattended(Tools::Run, Lease::Scoped)
            },
        }
    }
}

impl Policy {
    /// A thread whose cards the person answers and whose agent starts with
    /// nothing: the base the kinds that have the person in them build on.
    fn asked_of_the_person() -> Self {
        Self {
            answer: Answer::Person,
            surface: Surface {
                push_cards: true,
                mirror_to_call: true,
                index: true,
                link: false,
            },
            seed: Seed {
                parent_tail: false,
                own_history: false,
                chapters: false,
            },
            tools: Tools::None,
            lease: Lease::Same,
            idle: Idle::Never,
            restart: Restart::Resume,
            closing_note: false,
            computer: Computer::No,
            resume: Resume::Never,
        }
    }

    /// A thread nobody answers cards in, and whose agent is told nothing of
    /// any conversation.
    fn unattended(tools: Tools, lease: Lease) -> Self {
        Self {
            answer: Answer::Nobody,
            surface: Surface {
                push_cards: false,
                mirror_to_call: false,
                index: true,
                link: true,
            },
            seed: Seed {
                parent_tail: false,
                own_history: false,
                chapters: false,
            },
            tools,
            lease,
            idle: Idle::Never,
            restart: Restart::Resume,
            closing_note: false,
            computer: Computer::No,
            resume: Resume::Never,
        }
    }
}
