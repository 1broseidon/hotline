//! The inbox: the one way a teammate reaches another
//! (`docs/design/agent-inbox.md`).
//!
//! A teammate `send`s a colleague an item; the item waits in the colleague's
//! inbox, which the colleague reads and sorts itself. Each item keeps the
//! history of what became of it, so the sender, and the window, can see where
//! it stands without anyone writing a reply.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::*;

/// What the sender of an item wants from it: facts about the sender, not an
/// urgency. `answer`: words back. `work`: something done. `nothing`: only to
/// be known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum InboxWants {
    #[default]
    Answer,
    Work,
    Nothing,
}

impl InboxWants {
    pub fn name(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Work => "work",
            Self::Nothing => "nothing",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [Self::Answer, Self::Work, Self::Nothing]
            .into_iter()
            .find(|wants| wants.name() == name)
    }
}

/// Where an item stands, in the order it can move: `new` until its title is
/// put in front of the receiver (`noticed`), `seen` once its body is read,
/// and then `taken`, `later` (with a reason) or `closed` (with an outcome).
/// A `later` item can be taken or closed afterwards; a closed one is over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum InboxState {
    #[default]
    New,
    Noticed,
    Seen,
    Taken,
    Later,
    Closed,
}

impl InboxState {
    pub fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Noticed => "noticed",
            Self::Seen => "seen",
            Self::Taken => "taken",
            Self::Later => "later",
            Self::Closed => "closed",
        }
    }

    /// Whether the receiver has not read its body yet.
    pub fn unread(self) -> bool {
        matches!(self, Self::New | Self::Noticed)
    }
}

/// How a closed item ended: `answered` (a reply went back with
/// `update_inbox`'s `answer`), `done` (its work finished), `noted` (read,
/// nothing to do), `dropped` (not done, with a reason: the receiver's, or the
/// person's Stop of its work), `combined` (folded into another item, `into`,
/// and done with it), `expired` (left open or for later too long after it
/// arrived: Hotline closed it, and told its sender; never a teammate's sort).
/// `answered` and `expired` are never a teammate's `outcome`: an answer is
/// its own field, and expiry is the desk's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum InboxOutcome {
    Answered,
    Done,
    Noted,
    Dropped,
    Combined,
    Expired,
}

impl InboxOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Done => "done",
            Self::Noted => "noted",
            Self::Dropped => "dropped",
            Self::Combined => "combined",
            Self::Expired => "expired",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Answered,
            Self::Done,
            Self::Noted,
            Self::Dropped,
            Self::Combined,
            Self::Expired,
        ]
        .into_iter()
        .find(|outcome| outcome.name() == name)
    }
}

/// What Hotline's own notice in a teammate's inbox is about: never a
/// teammate's words, and never set by a teammate's tool. `expired`: an item
/// the teammate sent was closed as expired in its receiver's inbox.
/// `unanswered`: a question the teammate asked a colleague before the inbox
/// was ended unanswered when Hotline was updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum InboxNotice {
    Expired,
    Unanswered,
}

/// One change of an item's state, when it happened, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct InboxReceipt {
    pub state: InboxState,
    pub at: i64,
    /// For `later`, and for a dropped item: why, in the words given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InboxOutcome>,
    /// For a combined item: the item it was folded into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// What a newer Hotline wrote on the receipt that this build does not
    /// read, as on [`InboxItem::newer`].
    #[serde(flatten)]
    #[ts(skip)]
    pub newer: serde_json::Map<String, serde_json::Value>,
}

/// One item, as it is kept on its receiver's own stream (`inbox/<id>`),
/// written again under the same id each time it changes: the fold is the
/// latest line of each id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct InboxItem {
    pub id: String,
    /// When it was sent.
    pub ts: i64,
    /// The teammate it is from.
    pub from: String,
    /// The sender's thread it was sent from: where its reply is heard while
    /// that thread is live, and where the window draws the sent row.
    pub from_thread: ThreadId,
    /// The teammate it is for.
    pub to: String,
    /// The other teammates the same `send` made an item for, each its own:
    /// who else got it, by id. Absent on an item sent to one teammate, and on
    /// one kept before it was recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub also_to: Vec<String>,
    /// The sender's short title: what the receiver is shown before reading.
    pub title: String,
    /// The sender's words, and where any files it carried landed.
    pub body: String,
    pub wants: InboxWants,
    /// The sender has stopped, and will not go on until this is answered.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub waiting: bool,
    /// The item this one answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// A thread of the receiver's it is about, where it is heard while that
    /// thread is live; else the receiver's main conversation hears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<ThreadId>,
    pub state: InboxState,
    /// Why it is `later`, or why it was dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InboxOutcome>,
    /// For a combined item: the item it was folded into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combined_into: Option<String>,
    /// Every state it has been in, oldest first, with its time.
    #[serde(default)]
    pub receipts: Vec<InboxReceipt>,
    /// The receiver's work thread that taking it started, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<ThreadId>,
    /// The exchange request that work runs as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Kept, but not yet the receiver's: it waits for the person's
    /// first-contact card between these two teammates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub held: bool,
    /// The files a held item carries, by name, kept on the desk until the
    /// card lets it in; then they land in the receiver's working directory
    /// and `body` says where.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub files: Vec<String>,
    /// When a turn of the receiver's first opened with its title, at the
    /// turn's start: it wakes an idle receiver at most once, and only until
    /// then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered: Option<i64>,
    /// How many of `receipts` its sender has been told at a turn's start.
    #[serde(default)]
    pub sender_told: u32,
    /// The person looped the receiver into the sender's work thread
    /// (`thread.loop_in`): their act is its grant, and its answer is heard
    /// in that thread.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub looped_in: bool,
    /// Sent from a turn of the sender's main conversation that the person
    /// read: something the person asked for, so its expiry is theirs to hear
    /// from the sender. Absent on one sent from a quiet turn, a thread, or
    /// before it was recorded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub for_person: bool,
    /// Hotline's own notice, not a teammate's words: what it is about. Its
    /// `from` is the teammate it is about; only the desk sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<InboxNotice>,
    /// When a pick-up offers it again while it is still set aside: an hour
    /// after a pick-up first offered it, then a day after each offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer_again: Option<i64>,
    /// What a newer Hotline wrote on the item that this build does not
    /// read, written back exactly as it was found each time the item moves
    /// here, so going back to that build finds it as it left it. Nothing
    /// here reads or sets it, and `contract.ts` does not name it. An item
    /// line this build cannot read at all (a state, an outcome or another
    /// value it does not know) is no `InboxItem` here: the store hands it
    /// over as it was found (`hotline_store::inbox::newer_of`) and nothing
    /// moves it (`docs/log.md`, The room stream).
    #[serde(flatten)]
    #[ts(skip)]
    pub newer: serde_json::Map<String, serde_json::Value>,
}

/// A teammate's inbox as the window draws it (`inbox.list`, and the
/// `{"inbox": "<personaId>"}` subscription): what is open for it, newest
/// first; what closed, newest first; what it sent others, newest first; and
/// how many open items it has not read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct InboxView {
    pub persona_id: String,
    pub open: Vec<InboxItem>,
    pub closed: Vec<InboxItem>,
    pub sent: Vec<InboxItem>,
    pub unread: u32,
}

/// One item with what came of it (`inbox.item`): the item, its receipts
/// and the work thread it started among them, and the items that answer it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct InboxItemDetail {
    pub item: InboxItem,
    pub replies: Vec<InboxItem>,
}

/// One item as an `inbox_sorted` line lists it: in the order the receiver
/// means to work them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts", optional_fields)]
pub struct InboxSortedEntry {
    pub item: String,
    pub title: String,
    /// The teammate it is from.
    pub from: String,
    pub state: InboxState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InboxOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// Hotline's own notice, whose sender is Hotline, not `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<InboxNotice>,
}
