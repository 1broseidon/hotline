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
use crate::contract::{LoopSource, LoopState, OpenLoop, ThreadId};
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

/// A readable key for a loop the summariser gave none: the summary's first
/// five ASCII words, unless they make no key or one an open loop already
/// has, when it is the start of the loop's own id. Such a loop is matched
/// by its whole summary, never by this key, so two loops whose summaries
/// share their opening words, or have no ASCII words at all, stay two.
fn derived_key(summary: &str, id: &str, open: &[OpenLoop]) -> String {
    let words: Vec<&str> = summary
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .collect();
    let slug = normalize_key(&words.join("-"));
    let taken = open
        .iter()
        .any(|item| item.state.is_open() && item.key == slug);
    if slug.is_empty() || taken {
        id[..8.min(id.len())].to_string()
    } else {
        slug
    }
}

/// The loop key an outcome's `id` is mirrored under. The `id` names an
/// outcome within one thread, and loops are the teammate's across all of
/// them, so one opened in a work thread or a job carries that thread's tag:
/// the same `id` in two work threads is two loops. One in the main
/// conversation keeps its plain key, which is how the chapter summariser
/// names it too.
fn outcome_key(persona_id: &str, thread: &ThreadId, key: &str) -> String {
    let normalized = normalize_key(key);
    let base = if normalized.is_empty() {
        key.trim().to_lowercase()
    } else {
        normalized
    };
    if base.is_empty() || *thread == ThreadId::dm(persona_id) {
        return base;
    }
    let tag: String = normalize_key(&thread.key)
        .chars()
        .filter(|ch| *ch != '-')
        .take(8)
        .collect();
    format!("{tag}-{base}")
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
    ///
    /// The note is written up to [`super::chapters::ANSWER_MS`] after the
    /// close, while the next chapter may already be working, and an idle
    /// close dates `ended_at` to when the talk stopped, perhaps weeks ago. So
    /// a loop changed after `ended_at` is newer than anything the note can
    /// say and is left alone, and what the note does write is stamped `now`,
    /// so a loop it opens is not born stale.
    pub(super) fn reconcile_chapter_loops(
        &self,
        persona_id: &str,
        chapter_id: &str,
        deltas: &[LoopDelta],
        resolved: &[String],
        ended_at: i64,
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
                if !item.state.is_open() || item.updated_at > ended_at {
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
            let given_key = delta
                .key
                .as_deref()
                .map(normalize_key)
                .filter(|key| !key.is_empty());
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
                    && (given_key.as_ref() == Some(&item.key)
                        || item.summary.eq_ignore_ascii_case(&summary))
            }) {
                if found.updated_at > ended_at {
                    continue;
                }
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
                let id = new_id();
                let key = given_key.unwrap_or_else(|| derived_key(&summary, &id, &existing));
                let item = OpenLoop {
                    id,
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

    /// Mirrors an `outcome` declaration into the teammate's open loops stream,
    /// under its `key` scoped by the `thread` its line is on ([`outcome_key`]).
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
        thread: &ThreadId,
        key: &str,
        message: &str,
        kind: &str,
        ended: bool,
        review_by: Option<i64>,
        job_id: Option<String>,
        now: i64,
    ) {
        let loop_key = outcome_key(persona_id, thread, key);
        if loop_key.is_empty() {
            return;
        }
        let summary = clean_summary(message);
        let _guard = lock(&self.loops_lock);
        let matched = kept::of(&self.log, persona_id)
            .into_iter()
            .find(|item| item.state.is_open() && item.key == loop_key);

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

    /// Writes each teammate's loop stream again with each loop's latest line
    /// alone, as the desk opens: every change appends the whole loop again,
    /// and the stream is read whole each turn and each sweep. A line this
    /// build cannot read is kept byte for byte ([`crate::log::Log::tidy`]),
    /// as the inbox's tidy keeps one. Under the loops lock, so no change to
    /// a loop lands between the read and the rewrite.
    pub(super) fn loops_tidy(&self) {
        let _guard = lock(&self.loops_lock);
        for persona in room::roster(&self.log) {
            let stream = StreamId::Loops(persona.id.clone());
            if let Err(error) = self.log.tidy(&stream, |_| true) {
                eprintln!("{}'s open loops could not be tidied: {error}", persona.id);
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
mod tests;
