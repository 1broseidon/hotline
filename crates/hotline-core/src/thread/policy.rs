//! What a kind of thread decides, as values.
//!
//! A kind is not a type with its own methods. It is a row in this table, and
//! the write path and the agent builder read the row. Only the decisions they
//! make are here; each later phase adds the ones it needs.

use super::ThreadKind;

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
}

/// What a fresh agent is told about the conversation it joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seed {
    /// The parent's last chapter note and the last lines said in it.
    pub parent_tail: bool,
    /// What the thread itself has already said, so an agent that cannot
    /// reopen its own session picks the thread up where it stood.
    pub own_history: bool,
}

/// Which of the teammate's tools the thread's agent is served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tools {
    /// All of them, with subagents.
    Teammate,
    /// A side thread's own set, addressed by its id.
    Side,
    /// A subagent run's: its teammate's conversation to read, no subagents.
    Run,
    /// What a peer session answers a colleague with.
    Peer,
    /// None: the thread has no agent of its own.
    None,
}

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
    /// Whether the agent gets the teammate's computer. Two agents driving one
    /// desktop is a fight nobody wins, so only the main conversation does.
    pub computer: bool,
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
                    own_history: true,
                },
                computer: true,
                ..Self::asked_of_the_person()
            },
            ThreadKind::Side => Self {
                tools: Tools::Side,
                lease: Lease::Independent,
                seed: Seed {
                    parent_tail: true,
                    own_history: true,
                },
                computer: false,
                ..Self::asked_of_the_person()
            },
            ThreadKind::Pair => Self {
                tools: Tools::Peer,
                lease: Lease::Scoped,
                seed: Seed {
                    parent_tail: false,
                    own_history: true,
                },
                computer: true,
                ..Self::asked_of_the_person()
            },
            // A call signals a card and never answers it, and a run has no
            // one to answer: both expire what they raise. A call has no agent
            // of its own yet, and a run starts fresh and is never reopened.
            ThreadKind::Call => Self::unattended(Tools::None, Lease::Same),
            ThreadKind::Run => Self::unattended(Tools::Run, Lease::Scoped),
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
            },
            seed: Seed {
                parent_tail: false,
                own_history: false,
            },
            tools: Tools::None,
            lease: Lease::Same,
            computer: false,
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
            },
            seed: Seed {
                parent_tail: false,
                own_history: false,
            },
            tools,
            lease,
            computer: false,
        }
    }
}
