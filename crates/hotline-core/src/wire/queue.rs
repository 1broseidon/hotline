//! A bounded remote queue that sheds tool chatter before losing the seat.
//!
//! An mpsc of frames cannot evict an older tool update to admit a reply. The
//! deque owns the frames; the one-slot channel only wakes the socket writer
//! and tells producers when that writer has gone away.

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::mpsc;

pub(super) const CAPACITY: usize = 256;

pub(super) enum Frame {
    Required(String),
    Droppable(String),
    Roster { sub: i64, row: Value },
}

impl Frame {
    fn len(&self) -> usize {
        match self {
            Self::Required(text) | Self::Droppable(text) => text.len(),
            Self::Roster { sub, row } => json!({ "sub": sub, "event": row }).to_string().len(),
        }
    }

    fn into_text(self) -> String {
        match self {
            Self::Required(text) | Self::Droppable(text) => text,
            Self::Roster { sub, row } => json!({ "sub": sub, "event": row }).to_string(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Sender {
    frames: Arc<Mutex<VecDeque<Frame>>>,
    wake: mpsc::Sender<()>,
}

pub(super) struct Receiver {
    frames: Arc<Mutex<VecDeque<Frame>>>,
    wake: mpsc::Receiver<()>,
}

pub(super) fn channel() -> (Sender, Receiver) {
    let frames = Arc::new(Mutex::new(VecDeque::new()));
    let (tx, rx) = mpsc::channel(1);
    (
        Sender {
            frames: frames.clone(),
            wake: tx,
        },
        Receiver { frames, wake: rx },
    )
}

impl Sender {
    pub(super) fn send(&self, mut frame: Frame, max: usize) -> Result<(), ()> {
        if self.wake.is_closed() || frame.len() > max {
            return Err(());
        }
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if let Frame::Roster { sub, row } = &mut frame
            && let Some(at) = frames
                .iter()
                .enumerate()
                .rev()
                // A snapshot may reset the catalogs. Never carry an older
                // row's omitted fields across it (or any required frame).
                .take_while(|(_, pending)| !matches!(pending, Frame::Required(_)))
                .find_map(|(at, pending)| {
                    matches!(pending, Frame::Roster { sub: pending_sub, row: pending_row }
                        if sub == pending_sub && row["persona"]["id"] == pending_row["persona"]["id"])
                        .then_some(at)
                })
        {
            let Some(Frame::Roster { row: previous, .. }) = frames.remove(at) else {
                unreachable!("the matching frame is a roster row");
            };
            // Omission means unchanged since the last queued row, which the
            // phone has not received yet. Carry its catalogs forward too.
            for key in ["models", "slashCommands"] {
                if row["session"].get(key).is_none()
                    && let Some(value) = previous["session"].get(key)
                {
                    row["session"][key] = value.clone();
                }
            }
            if frame.len() > max {
                return Err(());
            }
        }
        if frames.len() == CAPACITY {
            if let Some(at) = frames.iter().position(|f| matches!(f, Frame::Droppable(_))) {
                frames.remove(at);
            } else if matches!(frame, Frame::Droppable(_)) {
                return Ok(());
            } else {
                return Err(());
            }
        }
        // Replacements join at the end, so no newer row overtakes a snapshot
        // or a removal already queued for its subscription.
        frames.push_back(frame);
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(())) => Err(()),
        }
    }
}

impl Receiver {
    pub(super) async fn recv(&mut self) -> Option<String> {
        // A continuously ready socket must still let cancellation and other
        // tasks run, just as a Tokio channel receiver does.
        tokio::task::coop::consume_budget().await;
        loop {
            let frame = self
                .frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            if let Some(frame) = frame {
                return Some(frame.into_text());
            }
            self.wake.recv().await?;
        }
    }
}
