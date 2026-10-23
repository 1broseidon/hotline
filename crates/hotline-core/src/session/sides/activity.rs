//! Background commands belong to the thread that launched them.
use super::*;
use crate::session::jobs::Processes;

impl Sides {
    pub(in crate::session) fn processes(&self, persona: &str) -> Vec<Arc<Processes>> {
        lock(&self.inner)
            .live
            .values()
            .filter(|side| side.persona_id == persona && side.capability.is_current())
            .filter_map(|side| side.driver().and_then(|driver| driver.processes()))
            .collect()
    }
}

impl Room {
    pub(super) fn watch_side_activity(
        self: &Arc<Self>,
        side: &Arc<LiveSide>,
        agent: &mut super::super::agent::ThreadAgent,
    ) {
        if let Some(mut wakes) = agent.driver.subscribe_wakes() {
            let room = Arc::downgrade(self);
            let side = Arc::downgrade(side);
            tokio::spawn(async move {
                while let Some(text) = wakes.recv().await {
                    let (Some(room), Some(side)) = (room.upgrade(), side.upgrade()) else {
                        break;
                    };
                    if side.closed.load(Ordering::SeqCst) || !side.capability.is_current() {
                        break;
                    }
                    side.say(
                        &room,
                        &TranscriptEvent::Notice {
                            id: new_id(),
                            ts: now_ms(),
                            level: NoticeLevel::Info,
                            text: text.clone(),
                        },
                    );
                    room.queue_in_side(
                        &side,
                        Line {
                            voice: room.attached_voice(&side.thread),
                            text,
                            attachments: Vec::new(),
                            request: None,
                            from: None,
                            delivery: None,
                            wake: false,
                        },
                    );
                }
            });
        }
        if let Some(processes) = agent.driver.processes() {
            let mut changes = processes.changes();
            let room = Arc::downgrade(self);
            let side = Arc::downgrade(side);
            tokio::spawn(async move {
                while changes.changed().await.is_ok() {
                    let (Some(room), Some(side)) = (room.upgrade(), side.upgrade()) else {
                        break;
                    };
                    if side.closed.load(Ordering::SeqCst) || !side.capability.is_current() {
                        break;
                    }
                    let _ = room.info_changes.send(room.info(&side.persona_id));
                }
            });
        }
    }
}
