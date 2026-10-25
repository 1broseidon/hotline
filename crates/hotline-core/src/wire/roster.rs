//! The roster joins the room fold, broadcast-fed tape projections and live
//! sessions. Disk seeds each projection once and recovers it after lag.

use super::socket::Outbox;
use super::{RoomHandle, send};
use crate::contract::{
    HumanActionStatus, PasskeyAskStatus, Persona, RosterEntry, SessionInfo, SessionState,
    ToolStatus, TranscriptEvent,
};
use crate::log::{Log, StreamId};
use crate::room;
use crate::store::previews::live::{Live, State};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// One pending entry per teammate and at most one queued wake, regardless of
/// how many events arrive before the view gets to read the current state.
struct Dirty {
    ids: Mutex<HashSet<String>>,
    wake: mpsc::Sender<()>,
}

impl Dirty {
    fn mark(&self, id: &str) -> bool {
        self.ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_string());
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => true,
            Err(mpsc::error::TrySendError::Closed(())) => false,
        }
    }

    fn take(&self) -> HashSet<String> {
        std::mem::take(&mut *self.ids.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

struct Watched {
    pump: JoinHandle<()>,
    state: Arc<Mutex<State>>,
    catalogs: Mutex<Option<[Value; 2]>>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

pub(super) async fn view(
    id: i64,
    log: Log,
    handle: Arc<dyn RoomHandle>,
    mut room_events: broadcast::Receiver<Value>,
    infos: broadcast::Receiver<SessionInfo>,
    sender: Outbox,
) {
    let mut infos = Some(infos);
    let (wake, mut spoken) = mpsc::channel(1);
    let dirty = Arc::new(Dirty {
        ids: Mutex::default(),
        wake,
    });
    let mut watched = HashMap::new();
    let mut personas = refresh(&log, &dirty, &mut watched);
    let mut pins = room::pinned_teammates(&log);
    let mut lead = room::team_lead(&log);
    if !send_snapshot(
        id,
        &watched,
        &handle,
        &sender,
        &personas,
        &pins,
        lead.as_deref(),
    ) {
        return;
    }

    loop {
        let info = async {
            match infos.as_mut() {
                Some(infos) => infos.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = room_events.recv() => match event {
                Ok(event) => {
                    // A pin lives in a setting, not on the teammate, so the
                    // rows whose slot moved are sent again.
                    if event.get("kind").and_then(Value::as_str) == Some("setting")
                        && event.get("id").and_then(Value::as_str) == Some("pinnedTeammates")
                    {
                        let now = room::settled_pins(
                            event.get("value").filter(|_| event.get("deleted").is_none()),
                            &personas,
                        );
                        for persona_id in pins.iter().chain(&now) {
                            if pins.iter().position(|pin| pin == persona_id)
                                != now.iter().position(|pin| pin == persona_id)
                            {
                                dirty.mark(persona_id);
                            }
                        }
                        pins = now;
                        continue;
                    }
                    // So does the lead: the row that lost it and the row
                    // that has it are sent again.
                    if event.get("kind").and_then(Value::as_str) == Some("setting")
                        && event.get("id").and_then(Value::as_str) == Some(room::TEAM_LEAD)
                    {
                        let now = room::settled_lead(
                            event.get("value").filter(|_| event.get("deleted").is_none()),
                            &personas,
                        );
                        if now != lead {
                            for persona_id in lead.iter().chain(&now) {
                                dirty.mark(persona_id);
                            }
                        }
                        lead = now;
                        continue;
                    }
                    if event.get("kind").and_then(Value::as_str) != Some("persona") {
                        continue;
                    }
                    let Some(persona_id) = event.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
                        continue;
                    };
                    if event.get("deleted").and_then(Value::as_bool) == Some(true) {
                        personas.retain(|persona| persona.id != persona_id);
                        if let Some(pump) = watched.remove(persona_id) {
                            drop(pump);
                        }
                        if !send(&sender, json!({ "sub": id, "removed": persona_id })) {
                            return;
                        }
                        continue;
                    }
                    let Ok(persona) = serde_json::from_value::<Persona>(event) else {
                        continue;
                    };
                    watch(&log, &persona.id, &dirty, &mut watched);
                    dirty.mark(&persona.id);
                    match personas.iter_mut().find(|current| current.id == persona.id) {
                        Some(current) => *current = persona,
                        None => personas.push(persona),
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    personas = refresh(&log, &dirty, &mut watched);
                    pins = room::pinned_teammates(&log);
                    lead = room::team_lead(&log);
                    dirty.take();
                    if !send_snapshot(id, &watched, &handle, &sender, &personas, &pins, lead.as_deref()) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            info = info => match info {
                Ok(info) => {
                    mark_session(&personas, &dirty, info);
                    // Coalesce the queued session burst without preferring
                    // this branch forever over tape or room updates.
                    let mut lagged = false;
                    if let Some(infos) = infos.as_mut() {
                        for _ in 0..infos.len() {
                            match infos.try_recv() {
                                Ok(info) => mark_session(&personas, &dirty, info),
                                Err(broadcast::error::TryRecvError::Lagged(_)) => lagged = true,
                                Err(_) => break,
                            }
                        }
                    }
                    if lagged {
                        dirty.take();
                        if !send_snapshot(id, &watched, &handle, &sender, &personas, &pins, lead.as_deref()) {
                            return;
                        }
                    }
                }
                // Session notifications are hints, not the state. Every row
                // must be refreshed when one may have lost its final hint.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    dirty.take();
                    if !send_snapshot(id, &watched, &handle, &sender, &personas, &pins, lead.as_deref()) {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => infos = None,
            },
            spoken = spoken.recv() => {
                if spoken.is_none() {
                    return;
                }
                for persona_id in dirty.take() {
                    // A late session/tape hint cannot restore a deleted row.
                    let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) else {
                        continue;
                    };
                    let Some(watch) = watched.get(&persona_id) else { continue; };
                    let row = shaped(&sender, roster_entry(watch, &handle, persona.clone(), &pins, lead.as_deref()));
                    let row = catalogs(&sender, watch, row, false);
                    if sender.send_roster(id, row).is_err() {
                        return;
                    }
                }
            },
        }
    }
}

fn mark_session(personas: &[Persona], dirty: &Dirty, info: SessionInfo) {
    if personas.iter().any(|persona| persona.id == info.persona_id) {
        dirty.mark(&info.persona_id);
    }
}

fn send_snapshot(
    id: i64,
    watched: &HashMap<String, Watched>,
    handle: &Arc<dyn RoomHandle>,
    sender: &Outbox,
    personas: &[Persona],
    pins: &[String],
    lead: Option<&str>,
) -> bool {
    let rows: Vec<_> = personas
        .iter()
        .map(|persona| {
            let watch = watched
                .get(&persona.id)
                .expect("every roster member is watched");
            let row = shaped(
                sender,
                roster_entry(watch, handle, persona.clone(), pins, lead),
            );
            catalogs(sender, watch, row, true)
        })
        .collect();
    send(sender, json!({ "sub": id, "snapshot": rows }))
}

/// The room is read once on opening and again only after room-stream lag.
fn refresh(log: &Log, dirty: &Arc<Dirty>, watched: &mut HashMap<String, Watched>) -> Vec<Persona> {
    let personas = room::roster(log);
    watched.retain(|id, _| personas.iter().any(|persona| persona.id == *id));
    for persona in &personas {
        watch(log, &persona.id, dirty, watched);
    }
    personas
}

fn watch(log: &Log, persona_id: &str, dirty: &Arc<Dirty>, watched: &mut HashMap<String, Watched>) {
    if watched.contains_key(persona_id) {
        return;
    }
    let mut live = Live::new(log, StreamId::Tape(persona_id.to_string()));
    let state = Arc::new(Mutex::new(live.state.clone()));
    let projected = state.clone();
    let dirty = dirty.clone();
    let persona_id = persona_id.to_string();
    let watched_id = persona_id.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = dirty.wake.closed() => return,
                changed = live.changed() => {
                    match changed {
                        Some(true) => {
                            *projected.lock().unwrap_or_else(PoisonError::into_inner) = live.state.clone();
                            if !dirty.mark(&persona_id) { return; }
                        }
                        Some(false) => {}
                        None => return,
                    }
                },
            }
        }
    });
    watched.insert(
        watched_id,
        Watched {
            pump,
            state,
            catalogs: Mutex::default(),
        },
    );
}

/// A snapshot always carries catalogs. A client that declared rosterOptions
/// retains them between rows; an explicit empty array clears a former list.
fn catalogs(sender: &Outbox, watch: &Watched, mut row: Value, snapshot: bool) -> Value {
    let current = [
        row["session"]["models"].clone(),
        row["session"]["slashCommands"].clone(),
    ];
    let mut previous = watch
        .catalogs
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if !snapshot
        && sender.roster_options()
        && let Some(previous) = previous.as_ref()
    {
        let session = row["session"]
            .as_object_mut()
            .expect("a session is an object");
        for (index, key) in ["models", "slashCommands"].into_iter().enumerate() {
            if previous[index] == current[index] {
                session.remove(key);
            }
        }
    }
    *previous = Some(current);
    row
}

/// A roster row as a phone that asked for `lean` is sent it, since every
/// tool call sends a teammate's whole row again. Hotline Agent's model list
/// is every model on the desk, which the phone already has from
/// `models.list`; slash commands and checkpoints are the desk's; the
/// preview is a line in a list.
fn shaped(sender: &Outbox, mut row: Value) -> Value {
    if !sender.lean() {
        return row;
    }
    let hotline = row["persona"]["backendId"] == crate::driver::HOTLINE_BACKEND_ID;
    if let Some(session) = row.get_mut("session").and_then(Value::as_object_mut) {
        if hotline {
            session.insert("models".into(), json!([]));
        }
        session.insert("slashCommands".into(), json!([]));
    }
    if let Some(persona) = row.get_mut("persona").and_then(Value::as_object_mut) {
        persona.insert("sessionCheckpoints".into(), json!([]));
    }
    if let Some(text) = row["preview"]["text"].as_str()
        && text.chars().count() > PREVIEW_CHARS
    {
        let short: String = text.chars().take(PREVIEW_CHARS).collect();
        row["preview"]["text"] = json!(short);
    }
    row
}

/// As much of the last message as a list row can show, with room to spare.
const PREVIEW_CHARS: usize = 280;

/// One roster row, joined from the room, tape projection and live session.
fn roster_entry(
    watch: &Watched,
    room: &Arc<dyn RoomHandle>,
    persona: crate::contract::Persona,
    pins: &[String],
    lead: Option<&str>,
) -> Value {
    let state = watch
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let preview = state.preview.clone();
    let latest = preview.as_ref().map(|preview| preview.at);
    let session = room.info(&persona.id);
    json!(RosterEntry {
        activity: (session.state == SessionState::Thinking)
            .then(|| state.activity.clone().map(|(_, title)| title))
            .flatten(),
        waiting: state.waiting() || room.threads_waiting(&persona.id),
        drawing: room.drawing(&persona.id),
        pin: pins
            .iter()
            .position(|pin| *pin == persona.id)
            .and_then(|slot| u8::try_from(slot).ok()),
        lead: lead == Some(persona.id.as_str()),
        subagents: room.subagents(&persona.id),
        sides: room.sides(&persona.id),
        working_jobs: room.working_jobs(&persona.id),
        working_calls: room.working_calls(&persona.id),
        session,
        preview,
        latest,
        persona,
    })
}

/// The title of the tool still running on this tape, if the session is
/// thinking.
///
/// Read from the tape rather than remembered on the view, so a lagged pump
/// or a second snapshot cannot disagree with what the tape says. A call
/// that is in progress sets the title; any other status for that call, a
/// turn ending, or the session leaving thinking clears it. Only the tail is
/// read: a tool still running is by definition near the end, and a tape is
/// only bounded by how much has been said.
///
/// `pub(crate)`, not private: `list_teammates` reads it the same way the
/// roster row does, so a colleague's activity is the one the desk shows and
/// not a second guess at it.
pub(crate) fn activity_on(tail: &[Value], session: &SessionInfo) -> Option<String> {
    if session.state != SessionState::Thinking {
        return None;
    }
    let mut activity: Option<(String, String)> = None;
    for event in tail {
        let Ok(event) = serde_json::from_value::<TranscriptEvent>(event.clone()) else {
            continue;
        };
        match event {
            TranscriptEvent::Tool {
                tool_call_id,
                title,
                status,
                ..
            } => {
                if status == ToolStatus::InProgress {
                    activity = Some((tool_call_id, title));
                } else if activity.as_ref().is_some_and(|(id, _)| *id == tool_call_id) {
                    activity = None;
                }
            }
            TranscriptEvent::Turn { .. } => activity = None,
            _ => {}
        }
    }
    activity.map(|(_, title)| title)
}

/// Whether a card on this tape is still waiting on the person.
///
/// A card is rewritten in place when it is answered, so the tail is folded
/// by id and only each card's latest line counts. Read from the tail, like
/// the running tool: a teammate waiting on the person has stopped to wait,
/// so what it waits on is near the end.
///
/// `pub(crate)`: `list_teammates` asks the same question of a colleague's
/// tape, so a teammate can tell a colleague is waiting on the person before
/// interrupting them.
pub(crate) fn waiting_on(tail: &[Value]) -> bool {
    let mut open: HashMap<String, bool> = HashMap::new();
    for event in tail {
        let Ok(event) = serde_json::from_value::<TranscriptEvent>(event.clone()) else {
            continue;
        };
        match event {
            TranscriptEvent::Permission { id, decision, .. } => {
                open.insert(id, decision.is_none());
            }
            TranscriptEvent::HumanAction { id, status, .. } => {
                open.insert(id, status == HumanActionStatus::Pending);
            }
            TranscriptEvent::CredentialAsk { id, status, .. } => {
                open.insert(id, status == crate::contract::CredentialStatus::Pending);
            }
            TranscriptEvent::CookieOffer { offer } => {
                open.insert(
                    offer.id,
                    offer.status == crate::contract::CookieOfferStatus::Pending,
                );
            }
            TranscriptEvent::SecretUseAsk { id, status, .. } => {
                open.insert(id, status == crate::contract::SecretUseStatus::Pending);
            }
            TranscriptEvent::PasskeyAsk { id, status, .. } => {
                open.insert(id, status == PasskeyAskStatus::Pending);
            }
            TranscriptEvent::ExchangePaused { id, status, .. } => {
                open.insert(id, status == crate::contract::ExchangePauseStatus::Pending);
            }
            _ => {}
        }
    }
    open.into_values().any(|waiting| waiting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_keeps_one_dirty_id_per_teammate_and_one_wake() {
        let (wake, mut receiver) = mpsc::channel(1);
        let dirty = Dirty {
            ids: Mutex::default(),
            wake,
        };
        for _ in 0..10_000 {
            assert!(dirty.mark("ada"));
            assert!(dirty.mark("bob"));
        }
        assert_eq!(receiver.len(), 1);
        receiver.try_recv().unwrap();
        assert_eq!(dirty.take(), HashSet::from(["ada".into(), "bob".into()]));
        assert!(dirty.take().is_empty());
        assert!(dirty.mark("ada"));
        // A lag snapshot can clear the IDs while their wake is still queued.
        assert_eq!(dirty.take(), HashSet::from(["ada".into()]));
        assert!(dirty.mark("bob"));
        assert_eq!(receiver.len(), 1);
        receiver.try_recv().unwrap();
        assert_eq!(dirty.take(), HashSet::from(["bob".into()]));
        drop(receiver);
        assert!(!dirty.mark("bob"));
    }
}
