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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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
        &ThreadId::dm("ada"),
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

fn keyless(summary: &str) -> LoopDelta {
    LoopDelta {
        key: None,
        summary: summary.into(),
        waiting: false,
        due: None,
    }
}

/// A loop the summariser gave no key is matched by its whole summary, so two
/// loops whose summaries have no ASCII words, or share their first five, stay
/// two loops, and each is given a key of its own.
#[tokio::test]
async fn keyless_loops_never_collapse_into_one_record() {
    let (_dir, log, room) = room_with_ada();
    let t0 = 1_700_000_000_000;
    room.reconcile_chapter_loops(
        "ada",
        "ch-1",
        &[
            keyless("確定申告の書類を送る"),
            keyless("Проверить резервную копию"),
            keyless("Check the Larder backup cron on staging"),
            keyless("Check the Larder backup cron on production"),
        ],
        &[],
        t0,
        t0,
    );
    let open = kept::open_of(&log, "ada");
    assert_eq!(open.len(), 4, "{open:#?}");
    let mut keys: Vec<&str> = open.iter().map(|item| item.key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 4, "each loop has a key of its own: {keys:?}");
    assert!(keys.iter().all(|key| !key.is_empty() && *key != "loop"));

    // The same keyless summary again updates its loop in place.
    room.reconcile_chapter_loops(
        "ada",
        "ch-2",
        &[LoopDelta {
            waiting: true,
            ..keyless("確定申告の書類を送る")
        }],
        &[],
        t0 + DAY_MS,
        t0 + DAY_MS,
    );
    let open = kept::open_of(&log, "ada");
    assert_eq!(open.len(), 4);
    assert_eq!(
        open.iter()
            .filter(|item| item.state == LoopState::Waiting)
            .count(),
        1
    );
}

/// A chapter's note can land after the next chapter has already moved a loop
/// on: what changed after the chapter ended is newer than the note and is
/// left as it is.
#[tokio::test]
async fn a_late_note_never_reverts_what_changed_after_its_chapter_ended() {
    let (_dir, log, room) = room_with_ada();
    let ended_at = 1_700_000_000_000;
    let home = ThreadId::dm("ada");
    room.sync_open_outcome(
        "ada",
        &home,
        "pr",
        "Waiting on review",
        "open",
        false,
        None,
        None,
        ended_at - 1_000,
    );
    room.sync_open_outcome(
        "ada",
        &home,
        "pr",
        "Review in, merging",
        "open",
        false,
        None,
        None,
        ended_at + 30_000,
    );
    room.reconcile_chapter_loops(
        "ada",
        "ch-1",
        &[LoopDelta {
            key: Some("pr".into()),
            summary: "Waiting on review".into(),
            waiting: true,
            due: None,
        }],
        &["pr".into()],
        ended_at,
        ended_at + 60_000,
    );
    let item = kept::of(&log, "ada").remove(0);
    assert_eq!(item.state, LoopState::Active, "{item:#?}");
    assert_eq!(item.summary, "Review in, merging");
    assert_eq!(item.updated_at, ended_at + 30_000);
}

/// An idle close dates its chapter to when the talk stopped, which can be
/// weeks before the note is written: a loop it opens is new now, not born
/// stale.
#[tokio::test]
async fn a_loop_from_a_long_idle_chapter_is_stamped_when_it_is_written() {
    let (_dir, log, room) = room_with_ada();
    let now = 1_700_000_000_000;
    let ended_at = now - 20 * DAY_MS;
    room.reconcile_chapter_loops(
        "ada",
        "ch-1",
        &[LoopDelta {
            key: Some("dns".into()),
            summary: "Move the DNS record".into(),
            waiting: false,
            due: None,
        }],
        &[],
        ended_at,
        now,
    );
    let item = kept::of(&log, "ada").remove(0);
    assert_eq!(item.created_at, now);
    assert_eq!(item.updated_at, now);
    room.sweep_loops(now);
    assert_eq!(kept::of(&log, "ada")[0].state, LoopState::Active);
}

/// An outcome's `id` names it within one thread, so the same `id` in two
/// work threads is two loops, and closing one leaves the other open. One
/// opened in the main conversation keeps its plain key.
#[tokio::test]
async fn the_same_outcome_id_in_two_threads_is_two_loops() {
    let (_dir, log, room) = room_with_ada();
    let t0 = 1_700_000_000_000;
    let first = ThreadId::side("7f3c2a90-0000-4000-8000-000000000001");
    let second = ThreadId::side("b81d4e22-0000-4000-8000-000000000002");
    for (thread, message) in [(&first, "PR for the parser"), (&second, "PR for the docs")] {
        room.sync_open_outcome("ada", thread, "pr", message, "open", false, None, None, t0);
    }
    room.sync_open_outcome(
        "ada",
        &ThreadId::dm("ada"),
        "pr",
        "PR for the site",
        "open",
        false,
        None,
        None,
        t0,
    );
    assert_eq!(kept::open_of(&log, "ada").len(), 3);
    assert!(
        kept::open_of(&log, "ada")
            .iter()
            .any(|item| item.key == "pr" && item.summary == "PR for the site")
    );

    room.sync_open_outcome(
        "ada",
        &first,
        "pr",
        "Parser PR merged",
        "made",
        false,
        None,
        None,
        t0 + 1_000,
    );
    let open = kept::open_of(&log, "ada");
    assert_eq!(open.len(), 2, "{open:#?}");
    assert!(open.iter().any(|item| item.summary == "PR for the docs"));
    assert!(open.iter().any(|item| item.summary == "PR for the site"));
}

/// The startup tidy writes the stream again with each loop's latest line,
/// and keeps a line this build cannot read as it was.
#[tokio::test]
async fn the_startup_tidy_keeps_each_loops_latest_line() {
    let (dir, log, room) = room_with_ada();
    let t0 = 1_700_000_000_000;
    let home = ThreadId::dm("ada");
    for step in 0..5 {
        room.sync_open_outcome(
            "ada",
            &home,
            "watch",
            &format!("Watching, pass {step}"),
            "open",
            false,
            None,
            None,
            t0 + step,
        );
    }
    room.sync_open_outcome(
        "ada", &home, "other", "Another", "open", false, None, None, t0,
    );
    let newer = serde_json::json!({"kind": "open_loop", "id": "from-newer", "state": "frozen"});
    log.append(&StreamId::Loops("ada".into()), &newer).unwrap();
    let before = kept::of(&log, "ada");
    let file = dir.path().join("loops").join("ada.jsonl");
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 7);

    room.loops_tidy();

    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(text.contains("from-newer") && text.contains("frozen"));
    assert_eq!(kept::of(&log, "ada"), before);
}
