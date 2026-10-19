//! The wire: one WebSocket per client, carrying commands and subscriptions.
//!
//! Frames are JSON text, and there are three a client may send:
//!
//! - `{"id": n, "cmd": "<name>", "params": {…}}`, answered exactly once with
//!   `{"id": n, "ok": true, "result": …}` or `{"id": n, "ok": false,
//!   "error": "<sentence>"}`, and a refusal a client acts on by its reason
//!   also carries a `code`.
//! - `{"id": n, "sub": <target>}`, answered `{"id": n, "ok": true}` and then
//!   `{"sub": n, "snapshot": [...]}` once, `{"sub": n, "event": {…}}` per
//!   event as it lands, `{"sub": n, "removed": "<id>"}` when a view's row
//!   goes away, and, on a tape, `{"sub": n, "ephemeral": {…}}` for streaming
//!   deltas that are never written down.
//! - `{"id": m, "unsub": n}`, which ends subscription `n`.
//!
//! The ordering rule is the whole reason a subscription is not simply "load,
//! then listen": the broadcast is subscribed to BEFORE the fold is loaded for
//! the snapshot, so no event can land in the gap between them. An event that
//! lands in both is harmless, because every client folds a stream by event id
//! and the second copy of a line supersedes the first with itself.
//!
//! One writer task per socket, fed by an mpsc: answers, snapshots and events
//! are whole frames in the order they were queued, and never interleave.
//! Commands are answered on the read loop, in the order they arrived, because
//! the append a command makes IS its answer and a client that creates a
//! teammate and then subscribes must see it.

use serde_json::Value;

pub(crate) mod commands;
mod door;
mod files;
mod phone;
mod relay_door;
mod room_handle;
mod roster;
mod schedules;
mod seat;
mod socket;
mod subscribe;
mod threads;

pub use door::Door;
pub use room_handle::RoomHandle;
pub(crate) use roster::{activity_on, waiting_on};
use socket::Outbox;
#[cfg(test)]
pub(crate) use socket::WRITE_STALL;
pub(crate) use socket::{federation, seated_phone_v2};

#[cfg(test)]
mod tests;

/// Equal without leaking, through timing, how much of a wrong token matched.
pub(crate) fn same_secret(presented: &str, expected: &str) -> bool {
    let presented = presented.as_bytes();
    let expected = expected.as_bytes();
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// One frame onto the socket's queue. False means the socket is gone.
fn send(sender: &Outbox, frame: Value) -> bool {
    sender.send(frame.to_string()).is_ok()
}
