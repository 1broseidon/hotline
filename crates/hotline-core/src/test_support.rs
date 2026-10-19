//! What the unit tests of more than one module need.

use std::time::Duration;

/// How long a test waits for something before calling it lost. Generous,
/// because a loaded runner is slow and a passing test never waits it out; the
/// harnesses' `common::ANSWER` is the same minute.
pub(crate) const PATIENCE: Duration = Duration::from_secs(60);

/// Waits until `condition` holds, and fails the test with `what` if it does
/// not within `PATIENCE`. A test waits on the signal itself this way, never
/// on a sleep it hopes is long enough.
pub(crate) async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
