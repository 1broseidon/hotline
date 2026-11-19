//! Living open loops: a teammate's cross-chapter working memory, kept on its
//! own append-only stream (`loops/<personaId>.jsonl`, [`StreamId::Loops`]).
//!
//! Loops enter from two places: chapter close reconciliation
//! ([`Room::reconcile_chapter_loops`]) and explicit `outcome(type="open")`
//! declarations ([`Room::sync_open_outcome`]). They close automatically when a
//! chapter resolves them, when their open outcome closes, when a bound
//! scheduled job ends ([`Room::close_job_loops`]), when the person dismisses
//! them ([`Room::dismiss_loop`]), or as they age in [`Room::sweep_loops`].

use super::{Room, lock, new_id, now_ms};
use crate::contract::{LoopSource, LoopState, OpenLoop};
use crate::log::StreamId;
use crate::room;
use crate::store::loops as kept;
use chrono::{Local, TimeZone};
use serde_json::Map;

const DAY_MS: i64 = 86_400_000;

/// Untouched open loops turn `stale` after 14 days (unless an explicit
/// `review_by` arrives sooner or later).
pub(crate) const STALE_AFTER_MS: i64 = 14 * DAY_MS;

/// Untouched loops without an explicit `review_by` archive after 30 days.
pub(crate) const ARCHIVE_STALE_AFTER_MS: i64 = 30 * DAY_MS;

/// Grace window past `review_by` before a stale loop archives automatically.
pub(crate) const ARCHIVE_DUE_GRACE_MS: i64 = 16 * DAY_MS;

/// Finished loops stay visible for a week before archiving.
pub(crate) const ARCHIVE_DONE_AFTER_MS: i64 = 7 * DAY_MS;

/// Maximum open loops rendered in the per-turn thread index.
pub(crate) const INDEX_CAP: usize = 7;

/// Recently finished loops stay in `RECENTLY DONE` for 24 hours so the next
/// chapter knows they are settled.
pub(crate) const RECENT_DONE_MS: i64 = DAY_MS;

/// Cap on one loop's summary line.
const SUMMARY_CHARS: usize = 160;

/// Cap on a loop's slug key.
const KEY_CHARS: usize = 48;

/// Maximum open loops passed to the chapter close summarizer.
const SUMMARY_INPUT_CAP: usize = 25;

/// One open-loop entry extracted by the chapter close summarizer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoopDelta {
    pub key: Option<String>,
    pub summary: String,
    pub waiting: bool,
    pub due: Option<String>,
}

/// Normalizes a slug key to lowercase ASCII alphanumerics separated by single
/// hyphens, capped to [`KEY_CHARS`].
pub(crate) fn normalize_key(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(ch.to_ascii_lowercase());
            if out.len() >= KEY_CHARS {
                break;
            }
        } else if !out.is_empty() {
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Derives a short stable slug from a summary when no explicit key was given.
fn slug_from_summary(summary: &str) -> String {
    let words: Vec<&str> = summary
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .collect();
    let slug = normalize_key(&words.join("-"));
    if slug.is_empty() {
        "loop".to_string()
    } else {
        slug
    }
}

fn clean_summary(raw: &str) -> String {
    let flat: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= SUMMARY_CHARS {
        flat
    } else {
        let mut cut: String = flat.chars().take(SUMMARY_CHARS - 1).collect();
        cut.push('…');
        cut
    }
}

fn format_date(ms: i64) -> String {
    let at = Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now);
    at.format("%-d %b %Y").to_string()
}

fn should_stale(item: &OpenLoop, now: i64) -> bool {
    match item.review_by {
        Some(due) => now >= due,
        None => now - item.updated_at >= STALE_AFTER_MS,
    }
}

fn should_archive_open(item: &OpenLoop, now: i64) -> bool {
    match item.review_by {
        Some(due) => {
            now >= due.saturating_add(ARCHIVE_DUE_GRACE_MS)
                && now - item.updated_at >= ARCHIVE_DUE_GRACE_MS
        }
        None => now - item.updated_at >= ARCHIVE_STALE_AFTER_MS,
    }
}

impl Room {
    /// Appends one loop snapshot line to `persona_id`'s loop stream and updates
    /// the search index. Caller holds `self.loops_lock`.
    fn write_loop_locked(&self, persona_id: &str, item: &OpenLoop) -> Result<(), String> {
        let event = kept::line(item);
        self.log
            .append(&StreamId::Loops(persona_id.to_string()), &event)
            .map_err(|error| format!("The open loop could not be kept: {error}"))?;
        if let Some(indexer) = lock(&self.indexer).as_mut()
            && let Err(error) = indexer.index_loop(persona_id, &event)
        {
            eprintln!(
                "an open loop for {persona_id} was not indexed for search (rebuild will pick it up): {error}"
            );
        }
        Ok(())
    }

    /// Formats the teammate's currently open loops for the chapter close
    /// summarizer so it can update active ones and resolve finished ones.
    pub(super) fn open_loops_for_summary(&self, persona_id: &str) -> Option<String> {
        let open = kept::open_of(&self.log, persona_id);
        if open.is_empty() {
            return None;
        }
        let lines: Vec<String> = open
            .into_iter()
            .take(SUMMARY_INPUT_CAP)
            .map(|item| {
                let due = item
                    .review_by
                    .map(|ms| format!(" · due {}", format_date(ms)))
                    .unwrap_or_default();
                format!(
                    "- [{}] ({}) {}{due}",
                    item.key,
                    item.state.name(),
                    item.summary
                )
            })
            .collect();
        Some(lines.join("\n"))
    }

    /// Reconciles the open loops and resolved loop keys produced when a chapter
    /// closes. Untouched existing loops are preserved as they are.
    pub(super) fn reconcile_chapter_loops(
        &self,
        persona_id: &str,
        chapter_id: &str,
        deltas: &[LoopDelta],
        resolved: &[String],
        now: i64,
    ) {
        if deltas.is_empty() && resolved.is_empty() {
            return;
        }
        let _guard = lock(&self.loops_lock);
        let mut existing = kept::of(&self.log, persona_id);

        for raw in resolved {
            let trimmed = raw.trim();
            let norm = normalize_key(trimmed);
            if trimmed.is_empty() && norm.is_empty() {
                continue;
            }
            for item in &mut existing {
                if !item.state.is_open() {
                    continue;
                }
                if item.id == trimmed || item.key == norm || item.key.eq_ignore_ascii_case(trimmed)
                {
                    item.state = LoopState::Done;
                    item.updated_at = now;
                    item.closed_at = Some(now);
                    item.last_chapter_id = Some(chapter_id.to_string());
                    if let Err(error) = self.write_loop_locked(persona_id, item) {
                        eprintln!("{persona_id}: {error}");
                    }
                }
            }
        }

        for delta in deltas {
            let summary = clean_summary(&delta.summary);
            if summary.is_empty() {
                continue;
            }
            let key = delta
                .key
                .as_deref()
                .map(normalize_key)
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| slug_from_summary(&summary));
            let target_state = if delta.waiting {
                LoopState::Waiting
            } else {
                LoopState::Active
            };
            let parsed_due = delta
                .due
                .as_deref()
                .and_then(|due| super::schedule::parse_when(due, now));

            if let Some(found) = existing.iter_mut().find(|item| {
                item.state.is_open()
                    && (item.key == key || item.summary.eq_ignore_ascii_case(&summary))
            }) {
                found.summary = summary;
                found.state = target_state;
                found.updated_at = now;
                found.closed_at = None;
                found.last_chapter_id = Some(chapter_id.to_string());
                if let Some(due) = parsed_due {
                    found.review_by = Some(due);
                }
                if let Err(error) = self.write_loop_locked(persona_id, found) {
                    eprintln!("{persona_id}: {error}");
                }
            } else {
                let item = OpenLoop {
                    id: new_id(),
                    key,
                    summary,
                    state: target_state,
                    source: LoopSource::Chapter,
                    created_at: now,
                    updated_at: now,
                    review_by: parsed_due,
                    job_id: None,
                    last_chapter_id: Some(chapter_id.to_string()),
                    closed_at: None,
                    newer: Map::new(),
                };
                if let Err(error) = self.write_loop_locked(persona_id, &item) {
                    eprintln!("{persona_id}: {error}");
                } else {
                    existing.push(item);
                }
            }
        }
    }

    /// Mirrors an `outcome` declaration into the teammate's open loops stream.
    ///
    /// - `kind == "open"` and not `ended`: opens or updates an active loop.
    /// - `ended`: archives a matching open loop.
    /// - Any other `kind`: marks a matching open loop `done` (does not create a
    ///   loop for one-shot `made`/`sent`/`changed` outcomes).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the outcome declaration's fields directly"
    )]
    pub(super) fn sync_open_outcome(
        &self,
        persona_id: &str,
        key: &str,
        message: &str,
        kind: &str,
        ended: bool,
        review_by: Option<i64>,
        job_id: Option<String>,
        now: i64,
    ) {
        let norm = normalize_key(key);
        let loop_key = if norm.is_empty() {
            key.trim().to_ascii_lowercase()
        } else {
            norm
        };
        if loop_key.is_empty() {
            return;
        }
        let summary = clean_summary(message);
        let _guard = lock(&self.loops_lock);
        let existing = kept::of(&self.log, persona_id);
        let matched = existing
            .into_iter()
            .find(|item| item.state.is_open() && (item.key == loop_key || item.key == key));

        if ended {
            if let Some(mut item) = matched {
                if !summary.is_empty() {
                    item.summary = summary;
                }
                item.state = LoopState::Archived;
                item.updated_at = now;
                item.closed_at = Some(now);
                if let Err(error) = self.write_loop_locked(persona_id, &item) {
                    eprintln!("{persona_id}: {error}");
                }
            }
            return;
        }

        if kind == "open" {
            let item = match matched {
                Some(mut item) => {
                    if !summary.is_empty() {
                        item.summary = summary;
                    }
                    item.state = LoopState::Active;
                    item.updated_at = now;
                    item.closed_at = None;
                    if let Some(due) = review_by {
                        item.review_by = Some(due);
                    }
                    if let Some(job) = job_id {
                        item.job_id = Some(job);
                    }
                    item
                }
                None => OpenLoop {
                    id: new_id(),
                    key: loop_key,
                    summary,
                    state: LoopState::Active,
                    source: LoopSource::Outcome,
                    created_at: now,
                    updated_at: now,
                    review_by,
                    job_id,
                    last_chapter_id: None,
                    closed_at: None,
                    newer: Map::new(),
                },
            };
            if let Err(error) = self.write_loop_locked(persona_id, &item) {
                eprintln!("{persona_id}: {error}");
            }
            return;
        }

        if let Some(mut item) = matched {
            if !summary.is_empty() {
                item.summary = summary;
            }
            item.state = LoopState::Done;
            item.updated_at = now;
            item.closed_at = Some(now);
            if let Err(error) = self.write_loop_locked(persona_id, &item) {
                eprintln!("{persona_id}: {error}");
            }
        }
    }

    /// Marks any open loop bound to `job_id` as `done` when that scheduled job
    /// finishes or is cancelled.
    pub(super) fn close_job_loops(&self, persona_id: &str, job_id: &str, now: i64) {
        let _guard = lock(&self.loops_lock);
        for mut item in kept::open_of(&self.log, persona_id) {
            if item.job_id.as_deref() == Some(job_id) {
                item.state = LoopState::Done;
                item.updated_at = now;
                item.closed_at = Some(now);
                if let Err(error) = self.write_loop_locked(persona_id, &item) {
                    eprintln!("{persona_id}: {error}");
                }
            }
        }
    }

    /// Archives one open loop at the person's request (`loops.dismiss`).
    pub fn dismiss_loop(&self, persona_id: &str, id: &str) -> Result<OpenLoop, String> {
        if self.persona(persona_id).is_err() {
            return Err(format!("There is no teammate {persona_id}."));
        }
        let _guard = lock(&self.loops_lock);
        let mut item = kept::of(&self.log, persona_id)
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| "There is no such open loop.".to_string())?;
        if item.state == LoopState::Archived {
            return Ok(item);
        }
        let now = now_ms();
        item.state = LoopState::Archived;
        item.updated_at = now;
        item.closed_at = Some(now);
        self.write_loop_locked(persona_id, &item)?;
        Ok(item)
    }

    /// Ages open and recently-done loops across every teammate in the roster.
    pub(super) fn sweep_loops(&self, now: i64) {
        for persona in room::roster(&self.log) {
            self.sweep_persona_loops(&persona.id, now);
        }
    }

    pub(crate) fn sweep_persona_loops(&self, persona_id: &str, now: i64) {
        let _guard = lock(&self.loops_lock);
        for mut item in kept::of(&self.log, persona_id) {
            match item.state {
                LoopState::Active | LoopState::Waiting => {
                    if should_archive_open(&item, now) {
                        item.state = LoopState::Archived;
                        item.closed_at = Some(now);
                        if let Err(error) = self.write_loop_locked(persona_id, &item) {
                            eprintln!("{persona_id}: {error}");
                        }
                    } else if should_stale(&item, now) {
                        // Do not bump `updated_at` so age remains truthful.
                        item.state = LoopState::Stale;
                        if let Err(error) = self.write_loop_locked(persona_id, &item) {
                            eprintln!("{persona_id}: {error}");
                        }
                    }
                }
                LoopState::Stale => {
                    if should_archive_open(&item, now) {
                        item.state = LoopState::Archived;
                        item.closed_at = Some(now);
                        if let Err(error) = self.write_loop_locked(persona_id, &item) {
                            eprintln!("{persona_id}: {error}");
                        }
                    }
                }
                LoopState::Done => {
                    let since = item.closed_at.unwrap_or(item.updated_at);
                    if now - since >= ARCHIVE_DONE_AFTER_MS {
                        item.state = LoopState::Archived;
                        if let Err(error) = self.write_loop_locked(persona_id, &item) {
                            eprintln!("{persona_id}: {error}");
                        }
                    }
                }
                LoopState::Archived => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Log;
    use crate::session::{ProviderAuth, ProviderKeys};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct NoKeys;
    impl ProviderKeys for NoKeys {
        fn provider_auth(&self) -> HashMap<String, ProviderAuth> {
            HashMap::new()
        }
    }

    fn room_with_ada() -> (TempDir, Log, Arc<Room>) {
        let dir = TempDir::new().unwrap();
        let log = Log::open(dir.path());
        let ada = crate::session::tests::persona("ada");
        room::append_persona(&log, &ada).unwrap();
        let room = Room::new(log.clone(), Arc::new(NoKeys));
        (dir, log, room)
    }

    #[tokio::test]
    async fn chapter_reconciliation_opens_updates_resolves_and_preserves_untouched_loops() {
        let (_dir, log, room) = room_with_ada();
        let t0 = 1_700_000_000_000;

        room.reconcile_chapter_loops(
            "ada",
            "ch-1",
            &[
                LoopDelta {
                    key: Some("CA Filing".into()),
                    summary: "File California annual report".into(),
                    waiting: true,
                    due: Some("30d".into()),
                },
                LoopDelta {
                    key: None,
                    summary: "Check Larder backup cron".into(),
                    waiting: false,
                    due: None,
                },
            ],
            &[],
            t0,
        );

        let after_ch1 = kept::of(&log, "ada");
        assert_eq!(after_ch1.len(), 2);
        let ca = after_ch1.iter().find(|l| l.key == "ca-filing").unwrap();
        assert_eq!(ca.state, LoopState::Waiting);
        assert_eq!(ca.review_by, Some(t0 + 30 * DAY_MS));
        assert_eq!(ca.last_chapter_id.as_deref(), Some("ch-1"));

        let larder = after_ch1
            .iter()
            .find(|l| l.key == "check-larder-backup-cron")
            .unwrap();
        assert_eq!(larder.state, LoopState::Active);

        // Chapter 2 updates ca-filing and resolves larder, while leaving another untouched.
        let t1 = t0 + DAY_MS;
        room.reconcile_chapter_loops(
            "ada",
            "ch-2",
            &[LoopDelta {
                key: Some("ca-filing".into()),
                summary: "California annual report drafted, awaiting signature".into(),
                waiting: true,
                due: None,
            }],
            &["check-larder-backup-cron".into()],
            t1,
        );

        let after_ch2 = kept::of(&log, "ada");
        let ca2 = after_ch2.iter().find(|l| l.key == "ca-filing").unwrap();
        assert_eq!(ca2.id, ca.id, "updated in place under same id");
        assert_eq!(ca2.review_by, Some(t0 + 30 * DAY_MS), "preserved review_by");
        assert_eq!(ca2.updated_at, t1);
        assert_eq!(ca2.last_chapter_id.as_deref(), Some("ch-2"));

        let larder2 = after_ch2
            .iter()
            .find(|l| l.key == "check-larder-backup-cron")
            .unwrap();
        assert_eq!(larder2.state, LoopState::Done);
        assert_eq!(larder2.closed_at, Some(t1));

        // Chapter 3 talks about something unrelated: ca-filing must remain open.
        let t2 = t1 + DAY_MS;
        room.reconcile_chapter_loops(
            "ada",
            "ch-3",
            &[LoopDelta {
                key: Some("dns".into()),
                summary: "Move DNS record".into(),
                waiting: false,
                due: None,
            }],
            &[],
            t2,
        );
        let open = kept::open_of(&log, "ada");
        assert_eq!(open.len(), 2);
        assert!(open.iter().any(|l| l.key == "ca-filing"));
        assert!(open.iter().any(|l| l.key == "dns"));
    }

    #[tokio::test]
    async fn outcome_sync_and_job_auto_close_work_end_to_end() {
        let (_dir, log, room) = room_with_ada();
        let t0 = 1_700_000_000_000;

        // One-shot `made` creates no open loop.
        room.sync_open_outcome(
            "ada",
            "report",
            "Wrote report",
            "made",
            false,
            None,
            None,
            t0,
        );
        assert_eq!(kept::of(&log, "ada"), Vec::new());

        // `open` with job_id creates an Outcome loop.
        room.sync_open_outcome(
            "ada",
            "hn-watch",
            "Watching HN for Hotline",
            "open",
            false,
            None,
            Some("job-42".into()),
            t0,
        );
        let items = kept::open_of(&log, "ada");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, LoopSource::Outcome);
        assert_eq!(items[0].job_id.as_deref(), Some("job-42"));

        // Ending job-42 automatically marks the loop done.
        room.close_job_loops("ada", "job-42", t0 + 1_000);
        let all = kept::of(&log, "ada");
        assert_eq!(all[0].state, LoopState::Done);
        assert_eq!(all[0].closed_at, Some(t0 + 1_000));

        // Ending an open outcome archives it.
        room.sync_open_outcome(
            "ada",
            "wait-vendor",
            "Waiting for vendor",
            "open",
            false,
            None,
            None,
            t0 + 2_000,
        );
        room.sync_open_outcome(
            "ada",
            "wait-vendor",
            "Vendor cancelled",
            "open",
            true,
            None,
            None,
            t0 + 3_000,
        );
        let vendor = kept::of(&log, "ada")
            .into_iter()
            .find(|l| l.key == "wait-vendor")
            .unwrap();
        assert_eq!(vendor.state, LoopState::Archived);
    }

    #[tokio::test]
    async fn aging_respects_default_windows_and_explicit_review_by_and_keeps_updated_at_truthful() {
        let (_dir, log, room) = room_with_ada();
        let t0 = 1_700_000_000_000;

        // Loop A: no review_by -> stale at 14d, archived at 30d.
        room.sync_open_outcome(
            "ada",
            "plain",
            "Plain open loop",
            "open",
            false,
            None,
            None,
            t0,
        );
        // Loop B: review_by 45d out -> NOT stale at 14d or 30d; stale at 45d; archived at 45+16=61d.
        room.sync_open_outcome(
            "ada",
            "future-due",
            "Tax deadline",
            "open",
            false,
            Some(t0 + 45 * DAY_MS),
            None,
            t0,
        );
        // Loop C: done at t0 -> archived at 7d.
        room.sync_open_outcome(
            "ada",
            "finished",
            "Quick fix",
            "open",
            false,
            None,
            None,
            t0,
        );
        room.sync_open_outcome(
            "ada",
            "finished",
            "Quick fix done",
            "made",
            false,
            None,
            None,
            t0,
        );

        let find = |key: &str| {
            kept::of(&log, "ada")
                .into_iter()
                .find(|l| l.key == key)
                .unwrap()
        };

        // At 7 days: finished -> archived; plain & future-due still active.
        room.sweep_loops(t0 + 7 * DAY_MS);
        assert_eq!(find("finished").state, LoopState::Archived);
        assert_eq!(find("plain").state, LoopState::Active);
        assert_eq!(find("future-due").state, LoopState::Active);

        // At 14 days: plain -> stale with original updated_at; future-due still active!
        room.sweep_loops(t0 + 14 * DAY_MS);
        let plain_14 = find("plain");
        assert_eq!(plain_14.state, LoopState::Stale);
        assert_eq!(
            plain_14.updated_at, t0,
            "aging to stale must not bump updated_at"
        );
        assert_eq!(find("future-due").state, LoopState::Active);

        // At 30 days: plain -> archived; future-due STILL active!
        room.sweep_loops(t0 + 30 * DAY_MS);
        assert_eq!(find("plain").state, LoopState::Archived);
        assert_eq!(find("future-due").state, LoopState::Active);

        // At 45 days: future-due -> stale.
        room.sweep_loops(t0 + 45 * DAY_MS);
        assert_eq!(find("future-due").state, LoopState::Stale);

        // At 61 days (45 + 16): future-due -> archived.
        room.sweep_loops(t0 + 61 * DAY_MS);
        assert_eq!(find("future-due").state, LoopState::Archived);
    }

    #[tokio::test]
    async fn dismiss_archives_loop_idempotently() {
        let (_dir, log, room) = room_with_ada();
        let t0 = 1_700_000_000_000;
        room.sync_open_outcome(
            "ada",
            "cleanup",
            "Clean old images",
            "open",
            false,
            None,
            None,
            t0,
        );
        let item = kept::open_of(&log, "ada").remove(0);

        let dismissed = room.dismiss_loop("ada", &item.id).unwrap();
        assert_eq!(dismissed.state, LoopState::Archived);
        assert!(dismissed.closed_at.is_some());
        assert_eq!(kept::open_of(&log, "ada"), Vec::new());

        // Idempotent second dismiss.
        let again = room.dismiss_loop("ada", &item.id).unwrap();
        assert_eq!(again.state, LoopState::Archived);
    }
}
