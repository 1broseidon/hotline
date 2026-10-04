//! What a kind of thread decides, as values.
//!
//! A kind is not a type with its own methods. It is a row in this table, and
//! the write path reads the row. Only the decisions the write path makes are
//! here; each later phase adds the ones it needs.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub answer: Answer,
    pub surface: Surface,
}

impl Policy {
    pub fn of(kind: ThreadKind) -> Self {
        match kind {
            // A pair is here for the table's sake: nothing writes it through
            // the shared path yet, and until it does a card raised in one has
            // no answer path (docs/threads.md, phase 6).
            ThreadKind::Dm | ThreadKind::Side | ThreadKind::Pair => Self {
                answer: Answer::Person,
                surface: Surface {
                    push_cards: true,
                    mirror_to_call: true,
                    index: true,
                },
            },
            // A call signals a card and never answers it, and a run has no
            // one to answer: both expire what they raise.
            ThreadKind::Call | ThreadKind::Run => Self {
                answer: Answer::Nobody,
                surface: Surface {
                    push_cards: false,
                    mirror_to_call: false,
                    index: true,
                },
            },
        }
    }
}
