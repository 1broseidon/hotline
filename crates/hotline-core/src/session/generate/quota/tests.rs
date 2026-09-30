use super::*;
use serde_json::json;
use std::path::Path;

fn read(root: &Path) -> Total {
    serde_json::from_slice(&fs::read(root.join(FILE_NAME)).unwrap()).unwrap()
}

#[test]
fn twenty_slots_survive_restart_and_the_twenty_first_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    for expected in 1..=LIMIT {
        drop(quota.reserve().unwrap());
        assert_eq!(read(root.path()).count, expected);
    }
    assert_eq!(quota.reserve().unwrap_err(), LIMIT_ERROR);
    let restarted = SubscriptionQuota::new(root.path().into());
    assert_eq!(restarted.reserve().unwrap_err(), LIMIT_ERROR);
    assert_eq!(read(root.path()).count, LIMIT);
}

#[test]
fn clones_share_the_gate_and_live_slots_hold_no_lock() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    let barrier = Arc::new(std::sync::Barrier::new(40));
    let workers: Vec<_> = (0..40)
        .map(|_| {
            let quota = quota.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                quota.reserve()
            })
        })
        .collect();
    let slots: Vec<_> = workers
        .into_iter()
        .filter_map(|worker| match worker.join().unwrap() {
            Ok(slot) => Some(slot),
            Err(error) => {
                assert_eq!(error, LIMIT_ERROR);
                None
            }
        })
        .collect();
    assert_eq!(slots.len(), LIMIT as usize);
    assert_eq!(read(root.path()).count, LIMIT);
    for slot in slots {
        slot.release().unwrap();
    }
    assert_eq!(read(root.path()).count, 0);
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, 1);
}

#[test]
fn the_next_day_resets_and_a_late_release_cannot_lower_it() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    let day = today();
    let late = quota.reserve_on(day).unwrap();
    for _ in 1..LIMIT {
        quota.reserve_on(day).unwrap();
    }
    assert_eq!(quota.reserve_on(day).unwrap_err(), LIMIT_ERROR);
    quota.reserve_on(day + 1).unwrap();
    late.release().unwrap();
    let total = read(root.path());
    assert_eq!(total.day, day + 1);
    assert_eq!(total.count, 1);
}

#[test]
fn backward_clocks_keep_the_counter_and_the_slots_effective_day() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    let future = today() + 2;
    quota.reserve_on(future).unwrap();
    let slot = quota.reserve_on(future - 1).unwrap();
    assert_eq!(slot.day, future);
    assert_eq!(read(root.path()).day, future);
    assert_eq!(read(root.path()).count, 2);
    slot.release().unwrap();
    assert_eq!(read(root.path()).day, future);
    assert_eq!(read(root.path()).count, 1);
    for _ in 1..LIMIT {
        quota.reserve_on(future - 1).unwrap();
    }
    assert_eq!(quota.reserve_on(future - 1).unwrap_err(), LIMIT_ERROR);
}

#[test]
fn corrupt_or_oversized_counters_remain_closed_until_repaired() {
    for bytes in [
        b"not json with a private value".to_vec(),
        b"{}".to_vec(),
        vec![b' '; MAX_BYTES as usize + 1],
        json!({"version":2,"day":today(),"count":1})
            .to_string()
            .into_bytes(),
        json!({"version":1,"day":i64::MAX,"count":1})
            .to_string()
            .into_bytes(),
        json!({"version":1,"day":today(),"count":21})
            .to_string()
            .into_bytes(),
        json!({"version":1,"day":today(),"count":-1})
            .to_string()
            .into_bytes(),
        json!({"version":1,"day":today(),"count":1,"extra":true})
            .to_string()
            .into_bytes(),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE_NAME);
        fs::write(&path, &bytes).unwrap();
        let quota = SubscriptionQuota::new(root.path().into());
        assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
        fs::write(
            &path,
            json!({"version":1,"day":today(),"count":7}).to_string(),
        )
        .unwrap();
        quota.reserve().unwrap();
        assert_eq!(read(root.path()).count, 8);
    }
}

#[test]
fn missing_or_nonregular_existing_counters_retry_only_after_repair() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    quota.reserve().unwrap();
    let path = root.path().join(FILE_NAME);
    let saved = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
    fs::create_dir(&path).unwrap();
    assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
    assert_eq!(
        SubscriptionQuota::new(root.path().into())
            .reserve()
            .unwrap_err(),
        STORAGE_ERROR
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, saved).unwrap();
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, 2);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_not_a_counter() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.json");
    fs::write(
        &target,
        json!({"version":1,"day":today(),"count":0}).to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, root.path().join(FILE_NAME)).unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
    assert_eq!(
        serde_json::from_slice::<Total>(&fs::read(target).unwrap())
            .unwrap()
            .count,
        0
    );
}

#[test]
fn read_failures_retry_when_the_room_directory_returns() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("room");
    let moved = parent.path().join("moved");
    fs::create_dir(&root).unwrap();
    let quota = SubscriptionQuota::new(root.clone());
    quota.reserve().unwrap();
    fs::rename(&root, &moved).unwrap();
    fs::write(&root, b"unavailable").unwrap();
    assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
    fs::remove_file(&root).unwrap();
    fs::rename(&moved, &root).unwrap();
    quota.reserve().unwrap();
    assert_eq!(read(&root).count, 2);
}

#[test]
fn failed_saves_can_retry_without_spending_an_extra_slot() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    quota.reserve().unwrap();
    let path = root.path().join(FILE_NAME);
    let saved = fs::read(&path).unwrap();
    let mut total = read(root.path());
    total.count += 1;
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    {
        let mut state = quota.inner.lock().unwrap();
        assert_eq!(
            quota.inner.save(&mut state, &total).unwrap_err(),
            STORAGE_ERROR
        );
    }
    fs::remove_dir(&path).unwrap();
    fs::write(&path, saved).unwrap();
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, 2);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn failed_releases_are_retried_before_the_next_reservation() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    let slot = quota.reserve().unwrap();
    for _ in 1..LIMIT {
        quota.reserve().unwrap();
    }
    let path = root.path().join(FILE_NAME);
    let saved = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(slot.release().unwrap_err(), STORAGE_ERROR);
    assert_eq!(quota.reserve().unwrap_err(), STORAGE_ERROR);
    fs::write(&path, saved).unwrap();
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, LIMIT);
    assert!(quota.inner.lock().unwrap().pending_releases.is_empty());
    assert_eq!(quota.reserve().unwrap_err(), LIMIT_ERROR);
}

#[test]
fn a_failed_refund_save_is_retained_until_atomic_replacement_succeeds() {
    let root = tempfile::tempdir().unwrap();
    let quota = SubscriptionQuota::new(root.path().into());
    let slot = quota.reserve().unwrap();
    let path = root.path().join(FILE_NAME);
    let saved = fs::read(&path).unwrap();
    {
        let mut state = quota.inner.lock().unwrap();
        state.pending_releases.push(slot.day);
        let refunded = quota.inner.load(&mut state, today()).unwrap();
        assert_eq!(refunded.count, 0);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(
            quota.inner.save(&mut state, &refunded).unwrap_err(),
            STORAGE_ERROR
        );
        assert_eq!(state.pending_releases, vec![slot.day]);
    }
    fs::remove_dir(&path).unwrap();
    fs::write(&path, saved).unwrap();
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, 1);
    quota.reserve().unwrap();
    assert_eq!(read(root.path()).count, 2);
    assert!(quota.inner.lock().unwrap().pending_releases.is_empty());
}
