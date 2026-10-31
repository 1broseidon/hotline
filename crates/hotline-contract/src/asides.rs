//! Asides: the short lines one thread leaves in a teammate's peripheral
//! context (`docs/design/thread-awareness.md` §2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

/// Where an aside came from: one of a teammate's threads, or, once inbound
/// sources exist, a source the person added to the desk (a webhook, a mailbox,
/// a repository), by its id. Nothing makes an inbound aside yet; the variant
/// is here so the record and the wire already carry it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub enum AsideSource {
    Thread(ThreadId),
    Inbound(String),
}

/// How much an aside asks of the teammate it is for, lowest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum AsideLevel {
    /// Context only: it waits for the teammate's next turn.
    #[default]
    Note,
    /// Wakes an idle teammate once, counted on the pair's brake.
    Nudge,
    /// Needs the person: a card on the teammate's main conversation, and a
    /// note as well.
    Person,
}

impl AsideLevel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Nudge => "nudge",
            Self::Person => "person",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Note, Self::Nudge, Self::Person]
            .into_iter()
            .find(|level| level.name() == name)
    }
}

/// One aside, as it is kept on its teammate's own stream (`asides/<id>`) and,
/// as a `TranscriptEvent::Aside`, on the thread that left it. It lands in the
/// teammate's context at the start of its next turn in the thread it is
/// `about` (else its main conversation), and never in the person's chat.
/// Rewritten under the same id once it is heard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct Aside {
    pub id: String,
    pub ts: i64,
    /// The teammate it is for.
    pub to: String,
    pub from: AsideSource,
    /// The teammate it is from: whose words it is, or, for one Hotline says
    /// of the person's act (Loop in…, End thread), whose thread the person
    /// acted in. Absent for an inbound source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The sender's own words, or Hotline's, never the person's.
    pub text: String,
    /// The thread of the teammate's it is about, which is where it is heard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<ThreadId>,
    pub level: AsideLevel,
    /// When a turn of the teammate's took it in. Absent until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heard: Option<i64>,
    /// When the person last opened the teammate's Noted page with it there
    /// (`aside.seen`): the person's reading, apart from the teammate's
    /// hearing. Absent until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<i64>,
    /// The person looped the teammate into the work thread it came from
    /// (`thread.loop_in`), whose answer is written there once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub looped_in: bool,
}
