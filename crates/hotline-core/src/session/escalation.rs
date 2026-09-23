//! How a quiet scheduled run is heard (BRO-96).
//!
//! A quiet run's words are thinking by kind: they stay out of the
//! conversation and notify nobody, which is what lets a watcher check every
//! few minutes without paging the person each time. This is the one door out.
//! The run asks for it by name with `tell_person`, and what it hands over is
//! not posted as it stands: once the quiet turn is over, the teammate is
//! prompted with it in the open, and answers the person in its own voice,
//! as any reply — on the rail, in a toast, on the phone.

use crate::contract::ScheduledRun;
use crate::driver::Escalate;
use std::sync::Mutex;

/// One quiet run's right to hand its teammate one thing to say.
pub(super) struct Escalation {
    note: Mutex<Option<String>>,
    used: Mutex<bool>,
}

impl Escalation {
    pub(super) fn new() -> Self {
        Self {
            note: Mutex::new(None),
            used: Mutex::new(false),
        }
    }

    /// What the run asked to have said. Taken once its turn is over.
    pub(super) fn take(&self) -> Option<String> {
        super::lock(&self.note).take()
    }
}

impl Escalate for Escalation {
    fn escalate(&self, text: &str) -> Result<(), String> {
        let mut used = super::lock(&self.used);
        if *used {
            return Err(
                "Already handed over: one per run. Put everything in that one next time."
                    .to_string(),
            );
        }
        *used = true;
        *super::lock(&self.note) = Some(text.to_string());
        Ok(())
    }
}

/// What the teammate hears when its quiet run found something: which job,
/// what it found, and that the person is waiting to be told.
pub(super) fn follow_up(run: &ScheduledRun, note: &str) -> String {
    format!(
        "Your quiet scheduled job \"{}\" found something the person should know:\n\n{note}\n\nTell them now, in your own words.",
        run.name
    )
}
