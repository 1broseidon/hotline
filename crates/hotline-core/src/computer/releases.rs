//! Which Hotline Computer release a new computer is created on.
//!
//! The desk used to pin one tag, and a computer release reached people only
//! when a desktop release bumped it. The agent now reads the guide the
//! running container serves, so the only thing that has to match is the
//! small HTTP contract in docs/computer.md, and computer releases can flow
//! on their own schedule. The pin is still here, as a floor: the release
//! this desk was tested with, the one used offline, and the line under which
//! nothing is offered. Above it, the newest published release on the same
//! major is what a fresh computer gets and what an older one is offered.
//!
//! The lookup is one unauthenticated call to the releases endpoint, once at
//! desk start and every [`CHECK_EVERY_MS`] after, and once more when a
//! computer is about to be created with nothing known yet. A pinned image —
//! a teammate's or the room's — never asks: a pin is exactly what it says.

use serde_json::Value;
use std::time::Duration;

/// Where published releases are listed. GitHub's releases API answers
/// unauthenticated for a public repository.
pub const RELEASES_URL: &str = "https://api.github.com/repos/1broseidon/hotline-computer/releases";

/// How often the newest release is looked up.
pub const CHECK_EVERY_MS: i64 = 6 * 60 * 60_000;

/// How long a failed lookup waits before trying again: an offline desk at
/// start should not have to wait the whole interval once it is back.
pub const RETRY_AFTER_MS: i64 = 15 * 60_000;

/// A lookup answers well inside this, and a computer waiting to be created
/// is not held for longer.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Every published release at or above `floor` on the same major, newest
/// first, from the JSON the releases endpoint answers: an array of releases
/// with a `tag_name`, skipping drafts and pre-releases and any tag that is
/// not a version. The floor itself is the last entry when it was published,
/// so the list is exactly what a room may be set to.
pub fn releases_in(body: &str, floor: &str) -> Vec<String> {
    let Ok(releases) = serde_json::from_str::<Vec<Value>>(body) else {
        return Vec::new();
    };
    let Some(floor) = version_of(floor) else {
        return Vec::new();
    };
    let mut versions: Vec<(u64, u64, u64)> = releases
        .iter()
        .filter(|release| {
            release.get("draft") != Some(&Value::Bool(true))
                && release.get("prerelease") != Some(&Value::Bool(true))
        })
        .filter_map(|release| release.get("tag_name").and_then(Value::as_str))
        .filter_map(version_of)
        .filter(|version| version.0 == floor.0 && *version >= floor)
        .collect();
    versions.sort_unstable_by(|left, right| right.cmp(left));
    versions.dedup();
    versions
        .into_iter()
        .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
        .collect()
}

/// The newest of [`releases_in`].
pub fn newest_in(body: &str, floor: &str) -> Option<String> {
    releases_in(body, floor).into_iter().next()
}

/// `1.2.3` or `v1.2.3` as numbers; anything else is not a release.
fn version_of(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Asks `url` for the releases at or above `floor`, newest first; an empty
/// answer is a sentence, since nothing at the floor means the list is not
/// the one expected.
pub async fn lookup(url: &str, floor: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(LOOKUP_TIMEOUT)
        .user_agent(format!("hotline-desk/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())?;
    let body = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("The releases list could not be fetched: {error}"))?
        .error_for_status()
        .map_err(|error| format!("The releases list could not be fetched: {error}"))?
        .text()
        .await
        .map_err(|error| error.to_string())?;
    let releases = releases_in(&body, floor);
    if releases.is_empty() {
        return Err(format!(
            "The releases list names no release at or above {floor} on its major."
        ));
    }
    Ok(releases)
}

/// What the desk knows about releases, and when it last asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Known {
    /// The newest release seen, once a lookup has answered.
    pub newest: Option<String>,
    /// Every release seen at or above the floor, newest first.
    pub releases: Vec<String>,
    /// When a lookup last answered, well or badly.
    pub checked_ms: Option<i64>,
    /// What the last lookup refused with, until one answers well.
    pub error: Option<String>,
    /// When the next lookup is due.
    pub due_ms: i64,
}

impl Known {
    /// Whether it is time to ask again.
    pub fn due(&self, now_ms: i64) -> bool {
        now_ms >= self.due_ms
    }

    /// Records a lookup's answer and when to ask next. A failure keeps
    /// what was known and says why it did not change.
    pub fn record(&mut self, answer: Result<Vec<String>, String>, now_ms: i64) {
        self.checked_ms = Some(now_ms);
        match answer {
            Ok(releases) => {
                self.newest = releases.first().cloned();
                self.releases = releases;
                self.error = None;
                self.due_ms = now_ms + CHECK_EVERY_MS;
            }
            Err(why) => {
                self.error = Some(why);
                self.due_ms = now_ms + RETRY_AFTER_MS;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_release_on_the_floors_major_wins_and_prereleases_do_not() {
        let body = r#"[
            {"tag_name":"v1.0.0"},
            {"tag_name":"v0.5.3","prerelease":true},
            {"tag_name":"v0.5.2","draft":true},
            {"tag_name":"v0.5.1"},
            {"tag_name":"v0.5.0"},
            {"tag_name":"v0.4.9"},
            {"tag_name":"nightly"}
        ]"#;
        assert_eq!(newest_in(body, "0.5.0").as_deref(), Some("0.5.1"));
        assert_eq!(newest_in(body, "0.5.1").as_deref(), Some("0.5.1"));
        assert_eq!(
            releases_in(body, "0.5.0"),
            ["0.5.1", "0.5.0"],
            "the list is newest first, down to the floor, published releases only"
        );
        assert_eq!(releases_in(body, "0.5.1"), ["0.5.1"]);
        assert!(releases_in(body, "0.6.0").is_empty());
        assert_eq!(
            newest_in(body, "0.6.0"),
            None,
            "nothing at or above the floor is nothing, never a lower release"
        );
        assert_eq!(newest_in("not json", "0.5.0"), None);
        assert_eq!(newest_in("[]", "0.5.0"), None);
    }

    #[test]
    fn a_failed_lookup_retries_soon_and_a_good_one_waits_the_interval() {
        let mut known = Known::default();
        assert!(known.due(0));
        known.record(Err("offline".into()), 1_000);
        assert_eq!(known.newest, None);
        assert_eq!(known.checked_ms, Some(1_000));
        assert_eq!(known.error.as_deref(), Some("offline"));
        assert!(!known.due(1_000 + RETRY_AFTER_MS - 1));
        assert!(known.due(1_000 + RETRY_AFTER_MS));
        known.record(Ok(vec!["0.5.3".into(), "0.5.0".into()]), 2_000);
        assert_eq!(known.newest.as_deref(), Some("0.5.3"));
        assert_eq!(known.releases, ["0.5.3", "0.5.0"]);
        assert_eq!(known.error, None);
        assert!(!known.due(2_000 + CHECK_EVERY_MS - 1));
        assert!(known.due(2_000 + CHECK_EVERY_MS));
        // A later failure keeps what was known and says why.
        known.record(Err("offline".into()), 3_000);
        assert_eq!(known.newest.as_deref(), Some("0.5.3"));
        assert_eq!(known.releases, ["0.5.3", "0.5.0"]);
        assert_eq!(known.checked_ms, Some(3_000));
        assert_eq!(known.error.as_deref(), Some("offline"));
    }
}

/// A releases endpoint a test can point the desk at: answers the body it is
/// given and counts how often it was asked.
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) async fn serve(body: &'static str) -> (String, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = asked.clone();
        let app = axum::Router::new().route(
            "/releases",
            axum::routing::get(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    body
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://127.0.0.1:{port}/releases"), asked)
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[tokio::test]
    async fn a_lookup_reads_the_endpoint_and_an_absent_one_is_a_sentence() {
        let (url, asked) = fake::serve(r#"[{"tag_name":"v0.5.3"},{"tag_name":"v0.5.0"}]"#).await;
        assert_eq!(lookup(&url, "0.5.0").await.unwrap(), ["0.5.3", "0.5.0"]);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        let refused = lookup("http://127.0.0.1:1/releases", "0.5.0")
            .await
            .unwrap_err();
        assert!(refused.contains("could not be fetched"), "{refused}");
    }
}
