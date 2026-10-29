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
        ("dot", "hotline"),
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
                window_minutes: Some(300),
                label: None,
                status: PlanStatus::Allowed,
            },
            PlanWindow {
                kind: "seven_day".into(),
                used_percent: Some(61.0),
                resets_at: Some(at + 86_400_000),
                window_minutes: Some(10_080),
                label: None,
                status: PlanStatus::Allowed,
            },
        ],
        at,
        plan: Some("Max".into()),
        banked: None,
        banked_expires_at: None,
    }
}

fn reads_of(fake: &Fake, backend_id: &str) -> usize {
    lock(&fake.driver.plan_logins)
        .iter()
        .filter(|(login, _)| login == backend_id)
        .count()
}

/// A plan is the login's: a reading of one login shows on every teammate
/// on it and on no other, is not taken again while it is fresh, is taken
/// for a teammate that is not running, and is there for a teammate that
/// starts after the desk restarts.
#[tokio::test]
async fn a_logins_reading_shows_on_its_teammates_and_survives_a_reopen() {
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
    until("bea shows the reading of ada's login", || {
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

    // A teammate that is not running is read for all the same: the plan is
    // its login's, read with no session.
    room.stop("cy").unwrap();
    room.refresh_plan("cy").unwrap();
    until("cy's login is read", || room.plan_of("codex-acp").is_some()).await;
    assert!(
        lock(&fake.driver.plan_logins)
            .iter()
            .all(|(login, how)| login != "hotline" && *how == PlanRead::Now)
    );
    assert!(room.refresh_plan("nobody").is_err());

    drop(room);
    let fake = Fake::new(Scripted::new(saying("b", "ok")));
    let reopened = logins_room(Log::open(&root), fake.clone());
    let info = reopened.start("bea").await.unwrap();
    assert_eq!(info.plan_usage, Some(reading(now)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    // A reading a few seconds old is not read again on a start.
    assert_eq!(fake.driver.plan_reads.load(Ordering::SeqCst), 0);
}

/// When the desk starts it reads each login some teammate runs on, once,
/// and not one whose reading is fresh; Hotline Agent's is never read.
#[tokio::test]
async fn the_desk_reads_each_login_its_teammates_run_on() {
    let log = scratch("plan-watch");
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    let now = now_ms();
    *lock(&fake.driver.plan) = Some(reading(now));
    assert!(
        crate::session::plan::read_logins(
            &Arc::downgrade(&room),
            crate::session::plan::REFRESH_AFTER_MS
        )
        .await
    );
    assert_eq!(reads_of(&fake, "claude-acp"), 1);
    assert_eq!(reads_of(&fake, "codex-acp"), 1);
    assert_eq!(reads_of(&fake, "hotline"), 0);
    assert_eq!(room.plan_of("claude-acp"), Some(reading(now)));
    // A round soon after finds both fresh.
    assert!(
        crate::session::plan::read_logins(
            &Arc::downgrade(&room),
            crate::session::plan::REFRESH_AFTER_MS
        )
        .await
    );
    assert_eq!(fake.driver.plan_reads.load(Ordering::SeqCst), 2);
    // A gone room ends the rounds.
    let gone = Arc::downgrade(&room);
    drop(room);
    assert!(!crate::session::plan::read_logins(&gone, 0).await);
}

/// What an agent reports after a reply is merged into the login's reading,
/// so a later per-reply window joins the windows read out of band, and the
/// plan's name stays.
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
        window_minutes: Some(10_080),
        label: None,
        status: PlanStatus::Warning,
    };
    let reported = DriverInfo {
        plan_usage: Some(PlanUsage {
            windows: vec![opus.clone()],
            at: now,
            plan: None,
            banked: None,
            banked_expires_at: None,
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
    assert_eq!(plan.plan.as_deref(), Some("Max"));
}

/// After a turn only what costs nothing is read: the room asks for the
/// after-turn reading, never a read out of band.
#[tokio::test]
async fn a_turn_ending_reads_only_what_the_harness_wrote() {
    let log = scratch("plan-after-turn");
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    room.start("cy").await.unwrap();
    until("the start's read", || reads_of(&fake, "codex-acp") >= 1).await;
    let session = room.session("cy").unwrap();
    room.read_plan_after_turn(&session);
    until("the after-turn read", || {
        lock(&fake.driver.plan_logins)
            .iter()
            .any(|(_, how)| *how == PlanRead::AfterTurn)
    })
    .await;
}

/// A reading of a login reaches the teammates on it that are not running
/// too: each is told to be drawn again, and the roster takes the reading
/// from the login when it draws them.
#[tokio::test]
async fn a_teammate_not_running_is_told_of_its_logins_reading() {
    let log = scratch("plan-resting");
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    let mut infos = room.subscribe_info();
    let now = now_ms();
    *lock(&fake.driver.plan) = Some(reading(now));
    room.refresh_plan("bea").unwrap();
    let mut told = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !(told.contains("ada") && told.contains("bea")) {
        let info = tokio::time::timeout_at(deadline, infos.recv())
            .await
            .expect("both teammates on the login are told")
            .unwrap();
        told.insert(info.persona_id);
    }
    assert!(!told.contains("cy"), "another login's teammate is not told");
    assert_eq!(room.plan_of("claude-acp"), Some(reading(now)));
}

/// Claude Code's models as its `initialize` lists them: Opus with levels.
fn claude_catalog() -> crate::driver::acp::catalog::Catalog {
    let level = |id: &str| ConfigChoice {
        id: id.into(),
        name: id.into(),
        description: None,
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    };
    crate::driver::acp::catalog::Catalog {
        models: vec![ConfigChoice {
            efforts: Some(vec![level("default"), level("max")]),
            default_effort: Some("default".into()),
            fast: None,
            ..level("opus")
        }],
        default_model: Some("opus".into()),
        effort_id: "effort".into(),
        effort_name: "Effort".into(),
        ..Default::default()
    }
}

/// The read of a login that lists its harness's models keeps them per
/// agent, where the roster finds them for a teammate not running, tells
/// that teammate to be drawn again, and keeps them for the desk's next
/// start; a picker opening while the list and the plan are fresh reads
/// nothing, and one opening on a fresh plan with an old list reads again.
#[tokio::test]
async fn a_logins_read_keeps_its_models_for_teammates_at_rest_and_after_a_reopen() {
    let log = scratch("plan-catalog");
    let root = log.root().to_path_buf();
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    let mut infos = room.subscribe_info();
    *lock(&fake.driver.plan) = Some(reading(now_ms()));
    *lock(&fake.driver.catalog) = Some(claude_catalog());
    room.refresh_plan("ada").unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut told = std::collections::HashSet::new();
    while !told.contains("bea") {
        let info = tokio::time::timeout_at(deadline, infos.recv())
            .await
            .expect("the teammates at rest are told")
            .unwrap();
        told.insert(info.persona_id);
    }
    let offer = crate::driver::acp::models::offer(&root, "claude-acp").expect("the models");
    assert_eq!(offer.models, claude_catalog().models);
    assert_eq!(offer.configs[0].current_id.as_deref(), Some("default"));
    assert_eq!(crate::driver::acp::models::offer(&root, "codex-acp"), None);

    let reads = fake.driver.plan_reads.load(Ordering::SeqCst);
    room.refresh_plan("bea").unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fake.driver.plan_reads.load(Ordering::SeqCst), reads);
    // Ten minutes on, the plan still fresh: the list is read again.
    let file = crate::paths::acp_models_path(&root);
    let mut kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    kept["claude-acp"]["catalogAt"] = json!(now_ms() - 10 * 60_000);
    std::fs::write(&file, serde_json::to_vec(&kept).unwrap()).unwrap();
    room.refresh_plan("bea").unwrap();
    until("the old list is read again", || {
        fake.driver.plan_reads.load(Ordering::SeqCst) > reads
    })
    .await;

    drop(room);
    let reopened = logins_room(
        Log::open(&root),
        Fake::new(Scripted::new(saying("b", "ok"))),
    );
    assert_eq!(
        crate::driver::acp::models::offer(&root, "claude-acp").map(|offer| offer.models),
        Some(claude_catalog().models)
    );
    drop(reopened);
}

/// A teammate's menu or pane opening looks at its agent, running or not,
/// for a harness only; a look that finds a new offer has the teammates at
/// rest on that agent drawn again, so the modes it found are offered.
#[tokio::test]
async fn opening_a_teammate_looks_at_its_agent_and_a_new_offer_redraws_its_resting_teammates() {
    let log = scratch("plan-look");
    enrol_logins(&log);
    let fake = Fake::new(Scripted::new(saying("a", "ok")));
    let room = logins_room(log, fake.clone());
    let mut infos = room.subscribe_info();
    fake.driver.look_changes.store(true, Ordering::SeqCst);
    room.refresh_plan("ada").unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut told = std::collections::HashSet::new();
    while !(told.contains("ada") && told.contains("bea")) {
        let info = tokio::time::timeout_at(deadline, infos.recv())
            .await
            .expect("the teammates at rest are told")
            .unwrap();
        told.insert(info.persona_id);
    }
    assert!(!told.contains("cy"), "another agent's teammate is not");
    room.refresh_plan("dot").unwrap();
    room.refresh_plan("cy").unwrap();
    until("codex is looked at", || lock(&fake.driver.looks).len() >= 2).await;
    assert_eq!(*lock(&fake.driver.looks), ["claude-acp", "codex-acp"]);
}
