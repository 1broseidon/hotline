//! The roster view joins the cached room fold, each tape's tail and live
//! session state. Tape pumps remember dirty teammate IDs rather than queueing
//! one disk read for every event. A single wake covers the pending set.

use super::{RoomHandle, roster_entry, send};
use crate::contract::{Persona, SessionInfo};
use crate::log::{Log, StreamId};
use crate::room;
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

pub(super) async fn view(
    id: i64,
    log: Log,
    handle: Arc<dyn RoomHandle>,
    mut room_events: broadcast::Receiver<Value>,
    infos: broadcast::Receiver<SessionInfo>,
    sender: super::Outbox,
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
    if !send_snapshot(id, &log, &handle, &sender, &personas, &pins) {
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
                    if event.get("kind").and_then(Value::as_str) != Some("persona") {
                        continue;
                    }
                    let Some(persona_id) = event.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
                        continue;
                    };
                    if event.get("deleted").and_then(Value::as_bool) == Some(true) {
                        personas.retain(|persona| persona.id != persona_id);
                        if let Some(pump) = watched.remove(persona_id) {
                            pump.abort();
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
                    dirty.take();
                    if !send_snapshot(id, &log, &handle, &sender, &personas, &pins) {
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
                        if !send_snapshot(id, &log, &handle, &sender, &personas, &pins) {
                            return;
                        }
                    }
                }
                // Session notifications are hints, not the state. Every row
                // must be refreshed when one may have lost its final hint.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    dirty.take();
                    if !send_snapshot(id, &log, &handle, &sender, &personas, &pins) {
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
                    let row = roster_entry(&log, &handle, persona.clone(), &pins);
                    if !send(&sender, json!({ "sub": id, "event": row })) {
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
    log: &Log,
    handle: &Arc<dyn RoomHandle>,
    sender: &super::Outbox,
    personas: &[Persona],
    pins: &[String],
) -> bool {
    let rows: Vec<_> = personas
        .iter()
        .map(|persona| roster_entry(log, handle, persona.clone(), pins))
        .collect();
    send(sender, json!({ "sub": id, "snapshot": rows }))
}

/// The room is read once on opening and again only after room-stream lag.
fn refresh(
    log: &Log,
    dirty: &Arc<Dirty>,
    watched: &mut HashMap<String, JoinHandle<()>>,
) -> Vec<Persona> {
    let personas = room::roster(log);
    watched.retain(|id, pump| {
        if personas.iter().any(|persona| persona.id == *id) {
            true
        } else {
            pump.abort();
            false
        }
    });
    for persona in &personas {
        watch(log, &persona.id, dirty, watched);
    }
    personas
}

fn watch(
    log: &Log,
    persona_id: &str,
    dirty: &Arc<Dirty>,
    watched: &mut HashMap<String, JoinHandle<()>>,
) {
    if watched.contains_key(persona_id) {
        return;
    }
    let mut events = log.subscribe(&StreamId::Tape(persona_id.to_string()));
    let dirty = dirty.clone();
    let persona_id = persona_id.to_string();
    let watched_id = persona_id.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                // Dropping the view closes its receiver, including for tapes
                // that remain silent forever. No detached pump outlives it.
                () = dirty.wake.closed() => return,
                event = events.recv() => {
                    let mut changed = match event {
                        Ok(event) => changes_row(&event),
                        Err(broadcast::error::RecvError::Lagged(_)) => true,
                        Err(broadcast::error::RecvError::Closed) => return,
                    };
                    // Drain the burst already queued before waking the view.
                    // Take a fixed budget so a busy producer cannot keep this
                    // pump running forever without yielding to other tasks.
                    for _ in 0..events.len() {
                        match events.try_recv() {
                            Ok(event) => changed |= changes_row(&event),
                            Err(broadcast::error::TryRecvError::Lagged(_)) => changed = true,
                            Err(_) => break,
                        }
                    }
                    if changed && !dirty.mark(&persona_id) {
                        return;
                    }
                },
            }
        }
    });
    watched.insert(watched_id, pump);
}

fn changes_row(event: &Value) -> bool {
    matches!(
        event.get("kind").and_then(Value::as_str),
        Some(
            "user"
                | "agent"
                | "tool"
                | "turn"
                | "permission"
                | "human_action"
                | "passkey_ask"
                | "exchange_paused"
                | "delivery"
        )
    )
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
