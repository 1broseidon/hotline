//! Linked desks (ADR 0003): two desks that trust each other, so that their
//! teammates can see and message each other as if they shared one room.
//!
//! A link is a `desk_link` record on each desk's room stream, written by the
//! operator's action and removed by either side. The desk that made the link
//! dials the other one's sealed `/v2/link` with its own desk key, which the
//! other side's record names; the channel then carries traffic both ways
//! (`remote/link.rs`). Nothing crosses it but the `federation.*` commands:
//! each side's roster metadata and messages for the exchange queue.
//!
//! A teammate on a linked desk is known here by a node-qualified id,
//! `deskId/personaId`. It is never a persona in this room's roster: what
//! this desk knows of it is the live mirror below, refreshed over the link
//! and gone when the link is, plus the last names it had, kept on the record
//! so a message can be addressed and queued while the other desk is away.

use crate::log::{Log, StreamId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

/// The room stream's kind for a link. Not `link`: thread markers are.
const KIND: &str = "desk_link";

/// The most teammates one linked desk may publish, or be known by.
pub(crate) const TEAMMATES_MAX: usize = 200;

/// One linked desk, as the room stream keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    /// The other desk's id.
    pub desk_id: String,
    /// What the other desk calls itself.
    pub name: String,
    /// Whether this desk made the link, and so is the one that dials.
    pub dials: bool,
    /// The other desk's Noise public key, on the desk that is dialled: the
    /// only key that may open `/v2/link` for this link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The other desk's teammates as they were last heard of, by id and
    /// name, so a message to one can be addressed and wait while the other
    /// desk is away. Nothing else about them is kept.
    #[serde(default)]
    pub teammates: Vec<Known>,
}

/// A linked teammate's id and name, and nothing else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Known {
    pub persona_id: String,
    pub name: String,
}

fn record_id(desk_id: &str) -> String {
    format!("{KIND}:{desk_id}")
}

/// Every desk this one is linked with.
pub(crate) fn records(log: &Log) -> Vec<Record> {
    log.load(&StreamId::Room)
        .into_iter()
        .filter(|event| event["kind"] == KIND && event["deleted"] != true)
        .filter_map(|event| serde_json::from_value(event).ok())
        .collect()
}

pub(crate) fn record(log: &Log, desk_id: &str) -> Option<Record> {
    records(log)
        .into_iter()
        .find(|record| record.desk_id == desk_id)
}

pub(crate) fn save(log: &Log, record: &Record) -> Result<(), String> {
    let mut event = serde_json::to_value(record).map_err(|error| error.to_string())?;
    event["kind"] = json!(KIND);
    event["id"] = json!(record_id(&record.desk_id));
    event["ts"] = json!(chrono::Utc::now().timestamp_millis());
    log.append(&StreamId::Room, &event)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The link's end on this desk, as a room event: the record is gone.
pub(crate) fn remove(log: &Log, desk_id: &str) -> Result<(), String> {
    log.append(
        &StreamId::Room,
        &json!({
            "kind": KIND,
            "id": record_id(desk_id),
            "deskId": desk_id,
            "ts": chrono::Utc::now().timestamp_millis(),
            "deleted": true,
        }),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// A desk id as desks give them out: a UUID, or anything as plain.
pub(crate) fn valid_desk_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A teammate id as a room mints them.
pub(crate) fn valid_persona_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A linked teammate's id here: its desk's id, a slash, its own id there.
pub(crate) fn qualified(desk_id: &str, persona_id: &str) -> String {
    format!("{desk_id}/{persona_id}")
}

/// The desk and the teammate a node-qualified id names, when it is one.
pub(crate) fn split(id: &str) -> Option<(&str, &str)> {
    let (desk, persona) = id.split_once('/')?;
    (valid_desk_id(desk) && valid_persona_id(persona)).then_some((desk, persona))
}

/// What a linked teammate is called here, "Name on Desk", from what its
/// desk's record last heard; none for an id that is not a linked one.
pub(crate) fn shown_name(log: &Log, id: &str) -> Option<String> {
    let (desk_id, persona_id) = split(id)?;
    let record = record(log, desk_id)?;
    let known = record
        .teammates
        .iter()
        .find(|known| known.persona_id == persona_id)?;
    Some(format!("{} on {}", known.name, record.name))
}

/// Whether this id names a teammate on another desk.
pub(crate) fn is_linked(id: &str) -> bool {
    split(id).is_some()
}

/// What a linked desk publishes about one of its teammates: exactly what
/// `list_teammates` shows of a local one, and nothing more. No workspace,
/// tools, model, message or secret has a field here to travel in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct LinkedTeammate {
    pub persona_id: String,
    pub name: String,
    /// idle, working, waiting or stopped.
    pub state: String,
    /// The tool or task it is doing right now, while it is working.
    #[serde(default)]
    pub activity: Option<String>,
    pub working_on: WorkingOn,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct WorkingOn {
    pub title: String,
    #[serde(default)]
    pub last_turn_at: Option<String>,
}

fn clipped(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

/// A roster another desk sent, kept only as far as it is plausible: known
/// states, ids a room could have minted, bounded text. A desk that sends
/// more than that is not refused; what it sent is cut down to it.
pub(crate) fn plausible(teammates: Vec<LinkedTeammate>) -> Vec<LinkedTeammate> {
    teammates
        .into_iter()
        .filter(|teammate| valid_persona_id(&teammate.persona_id))
        .filter(|teammate| !teammate.name.trim().is_empty())
        .take(TEAMMATES_MAX)
        .map(|teammate| LinkedTeammate {
            name: clipped(teammate.name.trim(), 100),
            state: match teammate.state.as_str() {
                "idle" | "working" | "waiting" | "stopped" => teammate.state,
                _ => "idle".into(),
            },
            activity: teammate.activity.map(|activity| clipped(&activity, 200)),
            working_on: WorkingOn {
                title: clipped(&teammate.working_on.title, 200),
                last_turn_at: teammate
                    .working_on
                    .last_turn_at
                    .filter(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok()),
            },
            persona_id: teammate.persona_id,
        })
        .collect()
}

/// What crosses a link for the exchange queue: a teammate's message for one
/// of the other desk's, or the answer to one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "contract.ts")]
pub enum LinkedDelivery {
    /// `fromPersonaId` and `fromName` are the sender on its own desk; `to`
    /// is the recipient's id on the desk receiving this. The request id is
    /// the sender desk's, and the same id sent again is the same request.
    Request {
        request_id: String,
        from_persona_id: String,
        from_name: String,
        to: String,
        message: String,
        /// ask or handoff.
        intent: String,
    },
    /// The answer to a request this desk sent, by its id.
    Result {
        request_id: String,
        reply: String,
        failed: bool,
    },
}

/// How a link is doing, as the Desks section shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "contract.ts")]
pub enum DeskLinkState {
    /// The channel is open; messages cross now.
    Connected,
    /// Not open: the other desk is away, or this one is still reaching it.
    /// Messages wait in the exchange queue meanwhile.
    Offline,
}

/// One linked desk, for `desks.links`.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct DeskLink {
    pub desk_id: String,
    pub name: String,
    /// Whether this desk made the link and dials the other.
    pub dials: bool,
    pub state: DeskLinkState,
    /// Why the channel is not open, when this desk knows.
    #[ts(optional)]
    pub error: Option<String>,
    /// The other desk's teammates, live while connected and as last heard
    /// of while not.
    pub teammates: Vec<LinkedTeammate>,
}

/// Why a call over a link got no answer.
#[derive(Debug, PartialEq)]
pub(crate) enum CallError {
    /// The channel is not open, or closed before the answer came. A request
    /// may or may not have arrived, so whatever is sent must be safe to send
    /// again.
    Offline,
    /// The other desk answered, and said no.
    Refused(String),
}

/// One command for the other desk, and where its answer goes.
pub(crate) struct Call {
    pub cmd: &'static str,
    pub params: Value,
    pub answer: oneshot::Sender<Result<Value, CallError>>,
}

/// Every linked desk's live half: whether its channel is open, the calls
/// waiting to cross it, and its teammates as last published.
#[derive(Default)]
pub struct Links {
    peers: Mutex<HashMap<String, Peer>>,
    /// This desk's own roster rows as last made for linked desks, each with
    /// what it was made from. Making one reads its teammate's whole tape for
    /// the open chapter's title, so a row is made again only when that has
    /// moved, and every link shares what was made.
    rows: Mutex<HashMap<String, (String, LinkedTeammate)>>,
}

#[derive(Default)]
struct Peer {
    /// Ends this link's dialing and its channel. A new link with the same
    /// desk gets a new one.
    cancel: CancellationToken,
    live: Option<Live>,
    roster: Option<Vec<LinkedTeammate>>,
    error: Option<String>,
    /// When its teammates' names were last written to the link's record.
    names_saved: Option<std::time::Instant>,
}

/// How often a linked desk's teammates' names may be written down. The
/// live roster is always current; the record is what routing falls back on
/// while the desk is away, and another desk must not be able to make this
/// one write to its room stream as fast as it can send.
const NAMES_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

struct Live {
    generation: u64,
    calls: mpsc::Sender<Call>,
}

impl Links {
    fn peers(&self) -> std::sync::MutexGuard<'_, HashMap<String, Peer>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The token that ends everything this desk runs for a link: its dialing
    /// and its open channel.
    pub(crate) fn token(&self, desk_id: &str) -> CancellationToken {
        self.peers()
            .entry(desk_id.to_string())
            .or_default()
            .cancel
            .clone()
    }

    /// A channel opened: calls for this desk go to it from now on. Answers
    /// the generation to hand back when it closes, so a channel that closes
    /// late never unregisters the one that replaced it.
    pub(crate) fn opened(&self, desk_id: &str, calls: mpsc::Sender<Call>) -> u64 {
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        {
            let mut peers = self.peers();
            let peer = peers.entry(desk_id.to_string()).or_default();
            peer.live = Some(Live { generation, calls });
            peer.error = None;
        }
        generation
    }

    /// A channel closed, or a dial failed, with why when there is a reason
    /// worth showing.
    pub(crate) fn closed(&self, desk_id: &str, generation: Option<u64>, error: Option<String>) {
        let mut peers = self.peers();
        let Some(peer) = peers.get_mut(desk_id) else {
            return;
        };
        if generation.is_some() && peer.live.as_ref().map(|live| live.generation) != generation {
            return;
        }
        if generation.is_some() {
            // What it published was live; while it is away it is offline.
            peer.live = None;
            peer.roster = None;
        }
        peer.error = error;
    }

    pub(crate) fn is_open(&self, desk_id: &str) -> bool {
        self.peers()
            .get(desk_id)
            .is_some_and(|peer| peer.live.is_some())
    }

    /// The link's state and why, for the Desks section.
    pub(crate) fn state(&self, desk_id: &str) -> (DeskLinkState, Option<String>) {
        match self.peers().get(desk_id) {
            Some(peer) if peer.live.is_some() => (DeskLinkState::Connected, None),
            Some(peer) => (DeskLinkState::Offline, peer.error.clone()),
            None => (DeskLinkState::Offline, None),
        }
    }

    /// The other desk's teammates as it published them over the channel
    /// that is open now, if it has.
    pub(crate) fn roster(&self, desk_id: &str) -> Option<Vec<LinkedTeammate>> {
        self.peers().get(desk_id)?.roster.clone()
    }

    /// Whether the names a linked desk just published may be written down
    /// now: the first time, and then at most every [`NAMES_EVERY`].
    pub(crate) fn may_save_names(&self, desk_id: &str) -> bool {
        let mut peers = self.peers();
        let peer = peers.entry(desk_id.to_string()).or_default();
        let due = peer
            .names_saved
            .is_none_or(|saved| saved.elapsed() >= NAMES_EVERY);
        if due {
            peer.names_saved = Some(std::time::Instant::now());
        }
        due
    }

    pub(crate) fn set_roster(&self, desk_id: &str, roster: Vec<LinkedTeammate>) {
        self.peers().entry(desk_id.to_string()).or_default().roster = Some(roster);
    }

    /// This desk's rows, each made again only when what it is made from
    /// (`made_from`, by teammate) has changed since it was last made.
    pub(crate) fn rows(
        &self,
        teammates: Vec<(String, String)>,
        make: impl Fn(&str) -> Option<LinkedTeammate>,
    ) -> Vec<LinkedTeammate> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        rows.retain(|id, _| teammates.iter().any(|(kept, _)| kept == id));
        teammates
            .into_iter()
            .filter_map(|(id, made_from)| match rows.get(&id) {
                Some((was, row)) if *was == made_from => Some(row.clone()),
                _ => {
                    let row = make(&id)?;
                    rows.insert(id, (made_from, row.clone()));
                    Some(row)
                }
            })
            .collect()
    }

    /// Ends this link here: its dialing, its channel and every call waiting
    /// on it stop, and what was known of its teammates goes.
    pub(crate) fn forget(&self, desk_id: &str) {
        let peer = self.peers().remove(desk_id);
        if let Some(peer) = peer {
            peer.cancel.cancel();
        }
    }

    /// Asks the other desk something and waits for its answer, or for the
    /// channel to close first.
    pub(crate) async fn call(
        &self,
        desk_id: &str,
        cmd: &'static str,
        params: Value,
    ) -> Result<Value, CallError> {
        let calls = self
            .peers()
            .get(desk_id)
            .and_then(|peer| peer.live.as_ref())
            .map(|live| live.calls.clone())
            .ok_or(CallError::Offline)?;
        let (answer, answered) = oneshot::channel();
        calls
            .send(Call {
                cmd,
                params,
                answer,
            })
            .await
            .map_err(|_| CallError::Offline)?;
        let answered = tokio::time::timeout(std::time::Duration::from_secs(30), answered).await;
        match answered {
            Ok(Ok(result)) => result,
            _ => Err(CallError::Offline),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qualified_id_names_a_desk_and_a_teammate_and_nothing_else() {
        assert_eq!(
            split("2f1c0a9e-desk/lou_1"),
            Some(("2f1c0a9e-desk", "lou_1"))
        );
        assert_eq!(qualified("desk", "lou"), "desk/lou");
        for not in [
            "lou",
            "/lou",
            "desk/",
            "desk/lou/again",
            "de.sk/lou",
            "desk/l~ou",
            "../lou",
            "desk/..",
        ] {
            assert_eq!(split(not), None, "{not}");
        }
    }

    #[test]
    fn a_row_is_made_again_only_when_what_it_is_made_from_moves() {
        let links = Links::default();
        let made = std::cell::Cell::new(0);
        let make = |id: &str| {
            made.set(made.get() + 1);
            Some(LinkedTeammate {
                persona_id: id.into(),
                name: format!("{id} #{}", made.get()),
                state: "idle".into(),
                activity: None,
                working_on: WorkingOn {
                    title: String::new(),
                    last_turn_at: None,
                },
            })
        };
        let lou = |made_from: &str| vec![("lou".to_string(), made_from.to_string())];
        assert_eq!(links.rows(lou("1:Ready:10"), make)[0].name, "lou #1");
        assert_eq!(links.rows(lou("1:Ready:10"), make)[0].name, "lou #1");
        assert_eq!(links.rows(lou("1:Thinking:12"), make)[0].name, "lou #2");
        // A teammate that has left is forgotten, and made afresh if it returns.
        assert!(links.rows(Vec::new(), make).is_empty());
        assert_eq!(links.rows(lou("1:Thinking:12"), make)[0].name, "lou #3");
    }

    #[test]
    fn a_published_roster_is_cut_down_to_what_a_roster_row_holds() {
        let sent: Vec<LinkedTeammate> = serde_json::from_value(json!([
            {"personaId":"lou","name":"  Lou  ","state":"working","activity":"x".repeat(500),
             "workingOn":{"title":"CI runners","lastTurnAt":"2026-10-13T06:00:00Z"},
             "cwd":"/home/george/secret","modelId":"gpt","tools":["shell"]},
            {"personaId":"../etc","name":"Mallory","state":"idle","workingOn":{"title":""}},
            {"personaId":"m","name":"M","state":"root","workingOn":{"title":"","lastTurnAt":"yesterday"}},
        ]))
        .unwrap();
        let kept = plausible(sent);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].name, "Lou");
        assert_eq!(kept[0].activity.as_ref().unwrap().chars().count(), 200);
        assert_eq!(kept[1].state, "idle");
        assert_eq!(kept[1].working_on.last_turn_at, None);
        // What is kept serializes to the roster row's fields alone.
        let row = serde_json::to_value(&kept[0]).unwrap();
        let mut fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort();
        assert_eq!(
            fields,
            ["activity", "name", "personaId", "state", "workingOn"]
        );
    }
}
