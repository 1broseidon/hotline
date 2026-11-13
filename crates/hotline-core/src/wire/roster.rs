//! The roster joins the room fold, broadcast-fed tape projections and live
//! sessions. Disk seeds each projection once and recovers it after lag.

use super::socket::Outbox;
use super::{RoomHandle, send};
use crate::contract::{
    HumanActionStatus, PasskeyAskStatus, Persona, RosterEntry, RunningSubagent, SessionInfo,
    SessionState, TranscriptEvent,
};
use crate::driver::acp::catalog;
use crate::log::{Log, StreamId};
use crate::room;
use crate::store::previews::live::{Live, State, Turn};
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
                    // A pair has no agent of its own: its record moving is
                    // what moves the rows of the two teammates in it.
                    if event.get("kind").and_then(Value::as_str) == Some("exchange_pair") {
                        for side in ["a", "b"] {
                            if let Some(persona_id) = event.get(side).and_then(Value::as_str) {
                                dirty.mark(persona_id);
                            }
                        }
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
                    let Ok(persona) = Persona::from_record(&event) else {
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
    let mut session = offered(room, &persona, room.info(&persona.id));
    // The plan is the login's: a teammate not running shows it too.
    if session.plan_usage.is_none() {
        session.plan_usage = room.plan_of(&persona.backend_id);
    }
    let subagents = room.subagents(&persona.id);
    json!(RosterEntry {
        activity: activity_line(&state.turn, &session, &subagents),
        waiting: state.waiting() || room.threads_waiting(&persona.id),
        drawing: room.drawing(&persona.id),
        pin: pins
            .iter()
            .position(|pin| *pin == persona.id)
            .and_then(|slot| u8::try_from(slot).ok()),
        lead: lead == Some(persona.id.as_str()),
        subagents,
        sides: room.sides(&persona.id),
        working_jobs: room.working_jobs(&persona.id),
        working_calls: room.working_calls(&persona.id),
        working_pairs: room.working_pairs(&persona.id),
        asides: u32::try_from(room.asides_unheard(&persona.id))
            .ok()
            .filter(|unheard| *unheard > 0),
        asides_new: u32::try_from(room.asides_unseen(&persona.id))
            .ok()
            .filter(|unseen| *unseen > 0),
        session,
        preview,
        latest,
        persona,
    })
}

/// A harness teammate whose session has not opened offers what its agent
/// offered when one last did — its models, modes and effort — with the
/// teammate's own choices as the current ones, each only while the agent
/// still offers it, the way the next session will open. So the menu lists
/// them before the teammate runs, and a choice made then is shown at once.
/// Its effort picker is its own model's levels, from the agent's catalogue
/// when no session reported them. A session that is up says its own, each
/// other model with the levels the agent offers for it. Hotline Agent's
/// choices are the desk's, which the window reads from the room.
pub(super) fn offered(
    room: &Arc<dyn RoomHandle>,
    persona: &Persona,
    mut info: SessionInfo,
) -> SessionInfo {
    if persona.backend_id == crate::driver::HOTLINE_BACKEND_ID {
        return info;
    }
    let offer = room.backend_offer(&persona.backend_id).unwrap_or_default();
    if info.session_id.is_some() {
        catalog::with_levels(
            &mut info.models,
            info.current_model_id.as_deref().unwrap_or_default(),
            &info.configs,
            &offer.models,
        );
        return info;
    }
    // The teammate's choice where the list has it, or where there is no list.
    let chosen = |choice: Option<&String>, list: &[crate::contract::ConfigChoice]| {
        choice
            .filter(|id| list.is_empty() || list.iter().any(|one| one.id == **id))
            .cloned()
    };
    if info.models.is_empty() {
        info.models = offer.models;
        info.model_label = info.model_label.or(offer.model_label);
    }
    let model_id = persona
        .model_id
        .as_deref()
        .map(|id| catalog::listed_as(id, &info.models).to_string());
    info.current_model_id = chosen(model_id.as_ref(), &info.models)
        .or(info.current_model_id.filter(|id| !id.is_empty()))
        .or(offer.current_model_id);
    if info.modes.is_empty() {
        info.modes = offer.modes;
        info.mode_label = info.mode_label.or(offer.mode_label);
    }
    info.current_mode_id = chosen(persona.mode_id.as_ref(), &info.modes)
        .or(info.current_mode_id)
        .or(offer.current_mode_id);
    if info.configs.is_empty() {
        info.configs = offer.configs;
    }
    for config in &mut info.configs {
        let choice = if config.category == Some(crate::contract::SessionConfigCategory::Effort) {
            persona.effort_id.as_ref()
        } else {
            persona.config_choices.get(&config.id)
        };
        if let Some(value) =
            choice.filter(|value| config.options.iter().any(|one| one.id == **value))
        {
            config.current_id = Some(value.clone());
        }
    }
    catalog::fit_effort(
        &mut info.configs,
        &info.models,
        info.current_model_id.as_deref(),
        persona.effort_id.as_ref(),
    );
    catalog::fit_fast(
        &mut info.configs,
        &info.models,
        info.current_model_id.as_deref(),
    );
    info
}

/// The teammate's session as [`offered`] draws it, read afresh: what a
/// choice made while it rests answers with.
pub(super) fn offered_info(log: &Log, room: &Arc<dyn RoomHandle>, persona_id: &str) -> SessionInfo {
    let info = room.info(persona_id);
    match room::roster(log)
        .into_iter()
        .find(|persona| persona.id == persona_id)
    {
        Some(persona) => offered(room, &persona, info),
        None => info,
    }
}

/// Whether a teammate is working: a session starting, or a turn running that
/// is not held open only for its helpers. The one reading the window's
/// `isWorking` (ui/src/activity.ts) makes of the same two fields, so the
/// header's beat and the words beside the mark cannot disagree.
fn working(session: &SessionInfo) -> bool {
    session.state == SessionState::Starting
        || (session.state == SessionState::Thinking && !session.awaiting_subagents)
}

/// How many helpers are running: subagents, not the commands a harness
/// sent to the background.
fn helpers(runs: &[RunningSubagent]) -> usize {
    runs.iter().filter(|run| run.kind.is_subagent()).count()
}

/// The words beside a teammate's mark, whenever it is working or waiting on
/// its helpers, and none otherwise.
///
/// The teammate's own words, from `activity`, always win. Without them the
/// desk says what it already knows of the turn, never a phrase on a timer:
/// "Getting started" until the agent has given anything back, "Working on
/// it" while a step is in flight, and "Thinking" between steps, while it
/// reasons or writes. With the turn over, or held open only for its
/// helpers, the words are what it last said it is on in the turn, or how
/// many helpers are working; they go when the last helper does.
///
/// The roster row, `list_teammates` and a linked desk all read this, so a
/// colleague, the rail and the phone say what the composer says.
pub(crate) fn activity_line(
    turn: &Turn,
    session: &SessionInfo,
    runs: &[RunningSubagent],
) -> Option<String> {
    // A session coming up has nothing of this turn to tell yet.
    if session.state == SessionState::Starting {
        return Some("Getting started".to_string());
    }
    if working(session) {
        let ask = &turn.ask;
        return Some(ask.declared.clone().unwrap_or_else(|| {
            let known = if !ask.heard {
                "Getting started"
            } else if ask.stepping() {
                "Working on it"
            } else {
                "Thinking"
            };
            known.to_string()
        }));
    }
    match helpers(runs) {
        0 => None,
        count => Some(turn.declared.clone().unwrap_or_else(|| {
            if count == 1 {
                "1 helper working".to_string()
            } else {
                format!("{count} helpers working")
            }
        })),
    }
}

/// [`activity_line`] for a stretch of tape read afresh, as `list_teammates`
/// reads a colleague's: only the tail, because what a turn in flight says
/// is near the end, and a tape is only bounded by how much has been said.
pub(crate) fn activity_on(
    tail: &[Value],
    session: &SessionInfo,
    runs: &[RunningSubagent],
) -> Option<String> {
    activity_line(&State::of(tail).turn, session, runs)
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
            TranscriptEvent::CookieAsk { id, status, .. } => {
                open.insert(id, status == crate::contract::CookieAskStatus::Pending);
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

    /// The cases the harness (`tests/activity_line.rs`) does not reach: a
    /// session coming up, more than one helper, and a background command,
    /// which is no helper.
    #[test]
    fn a_session_starting_and_the_helpers_are_said_plainly() {
        let capabilities = crate::contract::SessionCapabilities::default();
        let mut session: SessionInfo = serde_json::from_value(json!({
            "personaId":"ada", "state":"starting", "contextRestored":false, "models":[],
            "modes":[], "configs":[], "slashCommands":[], "capabilities":capabilities,
        }))
        .unwrap();
        let declared = State::of(&[json!({"kind":"activity", "id":"d", "ts":1, "type":"reading", "message":"Reading the lease"})]).turn;
        assert_eq!(
            activity_line(&declared, &session, &[]).as_deref(),
            Some("Getting started")
        );
        let run = |kind: &str| -> RunningSubagent {
            serde_json::from_value(json!({"runId":kind, "title":"t", "startedAt":1, "kind":kind}))
                .unwrap()
        };
        session.state = SessionState::Ready;
        let idle = Turn::default();
        assert_eq!(activity_line(&idle, &session, &[run("shell")]), None);
        assert_eq!(
            activity_line(
                &idle,
                &session,
                &[run("subagent"), run("shell"), run("subagent")]
            )
            .as_deref(),
            Some("2 helpers working")
        );
    }

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
