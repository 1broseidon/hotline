//! The ports this process listens on, each held while its listener serves:
//! the Door, the teammates' MCP server, the phone's Remote, a sign-in
//! callback, an OAuth proxy, the next launch's knock. A teammate's desk
//! browser never opens one of them, whatever asks (`hotline_core::browser`),
//! and it asks here at each load rather than being told at start, so a
//! listener opened later is refused too.
//!
//! One table for the process, not for a desk: a desktop app with two desks
//! open has two Doors, and a teammate on either must reach neither.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

static HELD: Mutex<BTreeMap<u16, usize>> = Mutex::new(BTreeMap::new());

/// A port this process listens on, until dropped. Kept beside the listener,
/// in the task that serves it, so the two end together.
#[derive(Debug)]
#[must_use = "a port is held only while this is kept"]
pub struct Held(u16);

impl Held {
    pub fn port(&self) -> u16 {
        self.0
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = held.get_mut(&self.0) {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.0);
            }
        }
    }
}

/// Holds `port` as one this process listens on.
pub fn hold(port: u16) -> Held {
    *HELD
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(port)
        .or_default() += 1;
    Held(port)
}

/// Whether this process listens on `port` now.
pub fn held(port: u16) -> bool {
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(&port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_held_while_any_holder_of_it_is_kept() {
        // A port no listener here can be given: the kernel hands out
        // ephemeral ports well above it, and nothing else in this crate's
        // tests holds it.
        let port = 1;
        assert!(!held(port));
        let first = hold(port);
        let second = hold(port);
        assert!(held(port));
        drop(first);
        assert!(held(port), "the second listener still serves");
        assert_eq!(second.port(), port);
        drop(second);
        assert!(!held(port));
    }
}
