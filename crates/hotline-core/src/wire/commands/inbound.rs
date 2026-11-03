//! The person's inbound sources (`inbound.*`): the standing grants a place
//! outside Hotline needs to leave a teammate asides, added, changed and
//! removed only from the owner's seat, and muted from a phone too
//! (`wire/seat.rs`). The record and its checks are `hotline_room::inbound`.

use super::now;
use crate::contract::{AsideLevel, InboundDraft};
use crate::log::Log;
use crate::wire::RoomHandle;
use hotline_room::inbound;
use serde_json::{Value, json};
use std::sync::Arc;

/// What `inbound.update` may change. Absent is unchanged.
pub(super) struct Patch {
    pub muted: Option<bool>,
    pub teammates: Option<Vec<String>>,
    pub max_level: Option<AsideLevel>,
    pub files_allowed: Option<bool>,
    pub rate: Option<u32>,
}

pub(super) fn list(log: &Log) -> Value {
    json!(inbound::sources(log))
}

/// `inbound.add`: the draft checked, with a secret named only if the vault
/// stores one by that name, so the record holds a reference and never a value.
pub(super) async fn add(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    draft: InboundDraft,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let source = inbound::drafted(log, uuid::Uuid::new_v4().to_string(), &draft, now())?;
    if let Some(secret) = &source.secret
        && !room
            .secrets_list()?
            .iter()
            .any(|stored| stored.name == *secret)
    {
        return Err(format!(
            "No secret called {secret} is stored. Store it under Settings → Secrets, then name it here."
        ));
    }
    inbound::write(log, &source)?;
    Ok(json!(source))
}

/// `inbound.update`: the patch folded over the record and the whole record
/// written again. Unmuting releases the brake.
pub(super) async fn update(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: Patch,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let mut source = inbound::source(log, id).ok_or_else(|| inbound::missing(id))?;
    if let Some(muted) = patch.muted {
        if !muted {
            source.wakes = 0;
        }
        source.muted = muted;
    }
    if let Some(teammates) = patch.teammates {
        source.teammates = inbound::checked_teammates(log, &teammates)?;
    }
    if let Some(level) = patch.max_level {
        source.max_level = level;
    }
    if let Some(files) = patch.files_allowed {
        source.files_allowed = files;
    }
    if let Some(rate) = patch.rate {
        source.rate = inbound::checked_rate(rate)?;
    }
    inbound::write(log, &source)?;
    Ok(json!(source))
}

pub(super) async fn remove(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    inbound::remove(log, id)?;
    Ok(Value::Null)
}
