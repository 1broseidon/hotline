//! What a request's turn came to, in the words its sender hears, and the
//! brake every inbox item counts on.

use super::*;
use crate::session::runner::Driven;

/// The most of a turn's answer a short line carries: a line, not the work.
/// The whole of it is in the thread it came from.
pub(super) const GIST_CHARS: usize = 200;

/// What a turn said once its work was done: its words after the last tool
/// card, else its last line.
pub(super) fn said_last(driven: &Driven) -> String {
    if driven.last_words.is_empty() {
        driven.replies.last().cloned().unwrap_or_default()
    } else {
        driven.last_words.join("\n\n")
    }
}

/// A turn's last message: the answer it came to, after whatever it said on
/// the way there ("On it, let me check the notes first.").
pub(super) fn last_message(driven: &Driven) -> String {
    driven
        .replies
        .iter()
        .map(|reply| reply.trim())
        .rfind(|reply| !reply.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// What a turn answered a question with: of what it said once its last tool
/// card was done, the message that answers (`text::answer`: the first more
/// than an opener, since a remark it adds after the answer is not the
/// answer); else its last message.
pub(super) fn answer_of(driven: &Driven) -> String {
    let said = if driven.last_words.is_empty() {
        &driven.replies
    } else {
        &driven.last_words
    };
    hotline_room::text::answer(said.iter().map(String::as_str))
        .unwrap_or_else(|| last_message(driven))
}

impl Request {
    /// What came of a request as an aside or a quiet line says it: the
    /// answer its turn ended on, else its whole reply, in plain words and
    /// short.
    pub(super) fn gist(&self) -> String {
        let said = if self.answer.is_empty() {
            &self.reply
        } else {
            &self.answer
        };
        hotline_room::text::gist(said, GIST_CHARS)
    }
}

/// Why nothing may pass between two teammates now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::session) enum Brake {
    /// At the cap, until the person answers the card: Keep going or Stop.
    Paused,
    /// The person's Stop, until the person next writes to one of the two.
    Stopped,
}

impl Brake {
    /// The plain reason a teammate is refused, with `other` the colleague
    /// and `nothing` what did not happen ("Nothing was sent.").
    pub(in crate::session) fn refusal(self, other: &str, nothing: &str) -> String {
        match self {
            Self::Paused => format!(
                "You and {other} have paused at {EXCHANGE_CAP} messages: the person has a card to let you keep going. {nothing}"
            ),
            Self::Stopped => format!(
                "The person stopped the exchange between you and {other}: nothing passes between you until the person writes to one of you. {nothing}"
            ),
        }
    }
}

impl Room {
    /// Whether two teammates are at their brake, and which: paused until the
    /// person answers its card, or stopped until the person next writes to
    /// one of them.
    pub(in crate::session) fn brake(&self, key: &str) -> Option<Brake> {
        let pair = self.exchange_pair(key)?;
        if pair.paused {
            Some(Brake::Paused)
        } else if pair.stopped {
            Some(Brake::Stopped)
        } else {
            None
        }
    }

    /// Counts a nudge's wake on two teammates' brake, as a reply is counted,
    /// and pauses them at the cap.
    pub(in crate::session) fn count_wake(&self, key: &str, by: &str, to: &str) {
        let _guard = lock(&self.exchange_lock);
        let mut pair = self.exchange_pair(key).unwrap_or(Pair {
            id: key.to_string(),
            a: by.to_string(),
            b: to.to_string(),
            exchanges: 0,
            paused: false,
            stopped: false,
            requests: vec![],
        });
        if pair.paused {
            return;
        }
        pair.exchanges += 1;
        pair.paused = pair.exchanges >= EXCHANGE_CAP;
        if let Err(error) = self.save_pair(&pair) {
            eprintln!("count a nudge's wake: {error}");
            return;
        }
        if pair.paused {
            self.exchange_card(&pair, ExchangePauseStatus::Pending);
        }
    }
}
