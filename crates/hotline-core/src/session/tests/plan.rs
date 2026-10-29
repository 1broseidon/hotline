use super::*;
use crate::contract::{PlanStatus, PlanUsage, PlanWindow};
use std::sync::atomic::Ordering;

/// Three teammates: two on the one Claude Code login, one on Codex.
fn logins_room(log: Log, agents: Arc<Fake>) -> Arc<Room> {
    Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        agents,
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    )
}

fn enrol_logins(log: &Log) {
    for (id, backend) in [
        ("ada", "claude-acp"),
        ("bea", "claude-acp"),
        ("cy", "codex-acp"),
    ] {
        let mut teammate = persona(id);
        teammate.backend_id = backend.to_string();
        enrol(log, &teammate);
    }
}

fn reading(at: i64) -> PlanUsage {
    PlanUsage {
        windows: vec![
            PlanWindow {
                kind: "five_hour".into(),
                used_percent: Some(38.0),
                resets_at: Some(at + 3_600_000),
                status: PlanStatus::Allowed,
            },
            PlanWindow {
                kind: "seven_day".into(),
                used_percent: Some(61.0),
                resets_at: Some(at + 86_400_000),
                status: PlanStatus::Allowed,
            },
        ],
        at,
    }
}

/// A plan is the login's: a reading taken through one teammate shows on
/// every teammate on that login and on no other, is not taken again while
/// it is fresh, and is there for a teammate that starts after the desk
/// restarts.
#[tokio::test]
async fn a_reading_through_one_teammate_shows_on_its_login_and_survives_a_reopen() {
    let log = scratch("plan-logins");
    let root = log.root().to_path_buf();
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    for id in ["ada", "bea", "cy"] {
        room.start(id).await.unwrap();
    }
    // Each start found no reading and asked for one, and got none.
    until("each start reads the plan", || {
        fake.driver.plan_reads.load(Ordering::SeqCst) >= 3
    })
    .await;
    assert_eq!(room.info("bea").plan_usage, None);

    let now = now_ms();
    *lock(&fake.driver.plan) = Some(reading(now));
    room.refresh_plan("ada").unwrap();
    until("bea shows the reading taken through ada", || {
        room.info("bea").plan_usage.is_some()
    })
    .await;
    assert_eq!(room.info("ada").plan_usage, Some(reading(now)));
    assert_eq!(room.info("bea").plan_usage, Some(reading(now)));
    assert_eq!(room.info("cy").plan_usage, None);

    // Fresh, so the picker opening again asks nothing.
    let reads = fake.driver.plan_reads.load(Ordering::SeqCst);
    room.refresh_plan("bea").unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fake.driver.plan_reads.load(Ordering::SeqCst), reads);

    // A teammate that is not running reads nothing and is not refused.
    room.stop("cy").unwrap();
    room.refresh_plan("cy").unwrap();

    drop(room);
    let fake = Fake::new(Scripted::new(saying("b", "ok")));
    let reopened = logins_room(Log::open(&root), fake.clone());
    let info = reopened.start("bea").await.unwrap();
    assert_eq!(info.plan_usage, Some(reading(now)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    // A reading a few seconds old is not read again on a start.
    assert_eq!(fake.driver.plan_reads.load(Ordering::SeqCst), 0);
}

/// What an agent reports after a reply is merged into the login's reading,
/// so a later per-reply window joins the windows `/usage` gave, and an older
/// report changes nothing it has.
#[tokio::test]
async fn a_report_after_a_reply_joins_the_logins_reading() {
    let log = scratch("plan-reported");
    enrol_logins(&log);
    let (driver, changes) = Scripted::new(saying("a", "ok")).with_info_changes();
    let fake = Fake::new(driver);
    let room = logins_room(log, fake.clone());
    room.start("ada").await.unwrap();
    room.start("bea").await.unwrap();
    let now = now_ms();
    *lock(&fake.driver.plan) = Some(reading(now - 10_000));
    room.refresh_plan("ada").unwrap();
    until("the reading lands", || {
        room.info("bea").plan_usage.is_some()
    })
    .await;

    let opus = PlanWindow {
        kind: "seven_day_opus".into(),
        used_percent: Some(85.0),
        resets_at: None,
        status: PlanStatus::Warning,
    };
    let reported = DriverInfo {
        plan_usage: Some(PlanUsage {
            windows: vec![opus.clone()],
            at: now,
        }),
        ..fake.driver.reported()
    };
    changes.send_replace(reported);
    until("the reported window joins", || {
        room.info("bea")
            .plan_usage
            .is_some_and(|plan| plan.windows.len() == 3)
    })
    .await;
    let plan = room.info("ada").plan_usage.unwrap();
    assert_eq!(plan.at, now);
    assert_eq!(plan.windows[2], opus);
    assert_eq!(plan.windows[0].used_percent, Some(38.0));
}
