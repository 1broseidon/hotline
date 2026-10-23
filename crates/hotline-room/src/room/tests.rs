use super::*;
use hotline_contract::AgentAccount;
use hotline_contract::SkillPolicy;
use hotline_contract::{CookieImport, CookieSite};
use hotline_contract::{McpPolicy, PolicyMode};
use serde_json::json;

fn scratch(name: &str) -> Log {
    let root =
        std::env::temp_dir().join(format!("hotline-core-room-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Log::open(root)
}

fn persona(id: &str, name: &str) -> Persona {
    Persona {
        node: None,
        id: id.to_string(),
        name: name.to_string(),
        goal: "Keep the harbour running.".to_string(),
        avatar: None,
        team: None,
        backend_id: "hotline".to_string(),
        cwd: "/tmp/harbour".to_string(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        harness_override: None,
        account_id: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: None,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    }
}

/// A persona event is the teammate's record with the kind beside it.
fn persona_event(persona: &Persona) -> Value {
    let mut event = serde_json::to_value(persona).unwrap();
    let fields = event.as_object_mut().unwrap();
    fields.insert("kind".into(), Value::from("persona"));
    event
}

/// A teammate whose policy names a provider this build does not have
/// still reads: the provider is dropped from its policy, not the teammate from the room.
#[test]
fn a_teammate_whose_policy_names_a_provider_that_is_gone_still_loads() {
    let log = scratch("firecrawl-policy");
    let mut event = persona_event(&persona("ada", "Ada"));
    event["webSearchPolicy"] = json!({
        "mode": "some",
        "providers": ["youcom", "exa", "brave", "firecrawl"],
    });
    append(&log, &event);
    let ada = roster(&log)
        .into_iter()
        .next()
        .expect("ada is still on the roster");
    let policy = ada.web_search_policy.expect("the policy survived");
    assert_eq!(policy.mode, PolicyMode::Some);
    assert_eq!(
        policy.providers,
        vec![
            hotline_contract::WebSearchProvider::Exa,
            hotline_contract::WebSearchProvider::Firecrawl
        ]
    );
    // Written back, it is in the new vocabulary only.
    assert_eq!(
        serde_json::to_value(&policy).unwrap(),
        json!({"mode": "some", "providers": ["exa", "firecrawl"]})
    );
}

fn setting(key: &str, value: Value) -> Value {
    json!({"kind": "setting", "id": key, "value": value})
}

fn tombstone(kind: &str, id: &str) -> Value {
    json!({"kind": kind, "id": id, "deleted": true})
}

fn append(log: &Log, event: &Value) {
    log.append(&StreamId::Room, event).unwrap();
}

#[test]
fn a_persona_event_is_the_contracts_persona_beside_its_kind() {
    let log = scratch("persona-round-trip");
    let ada = persona("ada", "Ada");
    let written = persona_event(&ada);
    append(&log, &written);

    // Out of the stream and back into the struct the wire is defined by,
    // and out again as the same bytes: a field spelled differently here
    // than the contract spells it fails on one side or the other.
    assert_eq!(roster(&log), [ada]);
    assert_eq!(log.load(&StreamId::Room), [written]);
}

#[test]
fn pins_are_validated_as_a_setting_and_settled_against_the_team() {
    assert_eq!(
        normalize_setting("pinnedTeammates", &json!(["a", "b", "a"])).unwrap(),
        json!(["a", "b"])
    );
    for bad in [
        json!("a"),
        json!([1]),
        json!([""]),
        json!(["a", "b", "c", "d"]),
    ] {
        assert!(normalize_setting("pinnedTeammates", &bad).is_err(), "{bad}");
    }
    // A hand-written list that names a stranger or repeats one still reads.
    let team = [persona("a", "A"), persona("b", "B")];
    assert_eq!(
        settled_pins(Some(&json!(["x", "b", "b", "a"])), &team),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(settled_pins(None, &team), [] as [String; 0]);
}

#[test]
fn the_roster_is_the_living_teammates_in_the_order_they_arrived() {
    let log = scratch("roster");
    let ada = persona("ada", "Ada");
    let bob = persona("bob", "Bob");
    append(&log, &persona_event(&ada));
    append(&log, &persona_event(&bob));

    // A rename is the same id again, and keeps the place it arrived in.
    let mut renamed = ada;
    renamed.name = "Ada Lovelace".to_string();
    renamed.updated_at = 1_700_000_001_000;
    append(&log, &persona_event(&renamed));

    assert_eq!(roster(&log), [renamed, bob]);
}

#[test]
fn a_tombstone_is_the_last_word_on_a_teammate() {
    let log = scratch("tombstone");
    let ada = persona("ada", "Ada");
    let bob = persona("bob", "Bob");
    append(&log, &persona_event(&ada));
    append(&log, &persona_event(&bob));
    append(&log, &tombstone("persona", "ada"));

    assert_eq!(roster(&log), [bob]);
    // The stream still carries the delete, which is what a mirror ships.
    let ids: Vec<String> = log
        .load(&StreamId::Room)
        .iter()
        .map(|event| event["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["ada", "bob"]);
}

/// Set up is read off the stream, so a delete cannot undo it and a
/// compaction, which writes the fold back, keeps the tombstone that says so.
#[test]
fn a_room_that_ever_had_a_teammate_stays_set_up_after_the_last_is_deleted() {
    let log = scratch("set-up");
    append(&log, &setting("theme", Value::from("dark")));
    append(&log, &persona_event(&persona("", "Nobody")));
    assert!(!has_had_a_teammate(&log.load(&StreamId::Room)));

    append(&log, &persona_event(&persona("ada", "Ada")));
    assert!(has_had_a_teammate(&log.load(&StreamId::Room)));

    append(&log, &tombstone("persona", "ada"));
    log.compact(&StreamId::Room).unwrap();
    let events = log.load(&StreamId::Room);
    assert_eq!(personas(&events), [] as [Persona; 0]);
    assert!(has_had_a_teammate(&events));
}

#[test]
fn a_setting_nobody_set_is_the_default_and_a_set_one_stands_over_it() {
    let log = scratch("settings");
    assert_eq!(settings(&log)["defaultBackendId"], "hotline");
    assert_eq!(settings(&log)["chapterIdleHours"], 8);
    assert_eq!(settings(&log)["enabledModels"], json!({}));

    append(&log, &setting("chapterIdleHours", Value::from(2)));
    append(&log, &setting("theme", Value::from("dark")));
    assert_eq!(settings(&log)["chapterIdleHours"], 2);
    assert_eq!(settings(&log)["theme"], "dark");
    assert_eq!(settings(&log)["defaultBackendId"], "hotline");

    // Clearing a setting is a tombstone, and puts the default back. A
    // setting that never had one is simply gone.
    append(&log, &tombstone("setting", "chapterIdleHours"));
    append(&log, &tombstone("setting", "theme"));
    assert_eq!(settings(&log)["chapterIdleHours"], 8);
    assert!(!settings(&log).contains_key("theme"));
}

#[test]
fn a_malformed_mcp_server_entry_is_skipped_on_read() {
    let log = scratch("mcp-bad-entry");
    append(
        &log,
        &setting(
            "mcpServers",
            json!([
                { "id": "no-name", "type": "stdio", "command": "echo" },
                { "id": "good", "type": "stdio", "name": "Good", "command": "run" },
            ]),
        ),
    );
    let folded = settings(&log);
    let servers = folded["mcpServers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["id"], "good");
}

fn account(n: u8, name: &str) -> AgentAccount {
    AgentAccount {
        id: format!("00000000-0000-4000-a000-{n:012}"),
        backend_id: "claude-acp".to_string(),
        name: name.to_string(),
        created_at: 1,
        login_at: None,
        setup: None,
    }
}

#[test]
fn agent_accounts_default_to_empty_and_read_back_what_was_written() {
    let log = scratch("agent-accounts");
    assert!(agent_accounts(&log).is_empty());
    assert_eq!(settings(&log)[AGENT_ACCOUNTS], json!([]));

    let (one, two) = (account(1, "Work"), account(2, "Personal"));
    let mut written = log.subscribe(&StreamId::Room);
    write_agent_accounts(&log, &[one.clone(), two.clone()]).unwrap();
    assert_eq!(written.try_recv().unwrap()["kind"], "setting");
    assert!(
        written.try_recv().is_err(),
        "one setting event, not one per account"
    );
    assert_eq!(agent_accounts(&log), [one.clone(), two]);

    append(&log, &setting(AGENT_ACCOUNTS, json!([{"id": "bad"}, one])));
    assert_eq!(agent_accounts(&log), std::slice::from_ref(&one));
    assert_eq!(settings(&log)[AGENT_ACCOUNTS], json!([one]));
}

fn checkpoints_of(log: &Log, id: &str) -> Vec<(String, Option<String>, String)> {
    roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
        .expect("the teammate is on the roster")
        .session_checkpoints
        .into_iter()
        .map(|checkpoint| {
            (
                checkpoint.backend_id,
                checkpoint.account_id,
                checkpoint.session_id,
            )
        })
        .collect()
}

fn kept(backend: &str, account: Option<&str>, session: &str) -> (String, Option<String>, String) {
    (
        backend.to_string(),
        account.map(str::to_string),
        session.to_string(),
    )
}

#[test]
fn a_checkpoint_is_kept_per_backend_and_account() {
    let log = scratch("checkpoint-per-account");
    append(&log, &persona_event(&persona("ada", "Ada")));

    checkpoint_session(&log, "ada", "claude-acp", None, "s0").unwrap();
    checkpoint_session(&log, "ada", "claude-acp", Some("acc-1"), "s1").unwrap();
    checkpoint_session(&log, "ada", "claude-acp", Some("acc-1"), "s2").unwrap();
    assert_eq!(
        checkpoints_of(&log, "ada"),
        [
            kept("claude-acp", None, "s0"),
            kept("claude-acp", Some("acc-1"), "s2")
        ],
        "an account replaces its own checkpoint and leaves the others"
    );

    clear_checkpoint(&log, "ada", "claude-acp", Some("acc-1")).unwrap();
    assert_eq!(
        checkpoints_of(&log, "ada"),
        [kept("claude-acp", None, "s0")]
    );
}

#[test]
fn a_legacy_checkpoint_is_the_system_default_one() {
    let log = scratch("checkpoint-legacy");
    let mut event = persona_event(&persona("ada", "Ada"));
    event["sessionCheckpoints"] = json!([{ "backendId": "claude-acp", "sessionId": "old" }]);
    append(&log, &event);

    checkpoint_session(&log, "ada", "claude-acp", Some("acc-1"), "new").unwrap();
    assert_eq!(
        checkpoints_of(&log, "ada"),
        [
            kept("claude-acp", None, "old"),
            kept("claude-acp", Some("acc-1"), "new")
        ],
        "a checkpoint from before accounts is This computer's"
    );

    clear_checkpoint(&log, "ada", "claude-acp", None).unwrap();
    assert_eq!(
        checkpoints_of(&log, "ada"),
        [kept("claude-acp", Some("acc-1"), "new")]
    );
    for event in log.load(&StreamId::Room) {
        assert!(
            !event.to_string().contains("\"accountId\":null"),
            "an absent account is absent, not null: {event}"
        );
    }
}

#[test]
fn clearing_an_account_reaches_every_teammate_and_clearing_a_backend_every_account() {
    let log = scratch("checkpoint-clears");
    append(&log, &persona_event(&persona("ada", "Ada")));
    append(&log, &persona_event(&persona("bob", "Bob")));
    checkpoint_session(&log, "ada", "claude-acp", None, "a0").unwrap();
    checkpoint_session(&log, "ada", "claude-acp", Some("acc-1"), "a1").unwrap();
    checkpoint_session(&log, "ada", "claude-acp", Some("acc-2"), "a2").unwrap();
    checkpoint_session(&log, "ada", "cursor", None, "c0").unwrap();
    checkpoint_session(&log, "bob", "claude-acp", Some("acc-1"), "b1").unwrap();

    let mut written = log.subscribe(&StreamId::Room);
    clear_account_checkpoints(&log, "acc-1").unwrap();
    assert_eq!(
        checkpoints_of(&log, "ada"),
        [
            kept("claude-acp", None, "a0"),
            kept("claude-acp", Some("acc-2"), "a2"),
            kept("cursor", None, "c0")
        ]
    );
    assert!(checkpoints_of(&log, "bob").is_empty());
    let mut rewritten = Vec::new();
    while let Ok(event) = written.try_recv() {
        rewritten.push(event["id"].as_str().unwrap().to_string());
    }
    assert_eq!(
        rewritten,
        ["ada", "bob"],
        "one record for each teammate it changed"
    );

    clear_account_checkpoints(&log, "acc-9").unwrap();
    assert!(written.try_recv().is_err());

    clear_backend_checkpoints(&log, "ada", "claude-acp").unwrap();
    assert_eq!(checkpoints_of(&log, "ada"), [kept("cursor", None, "c0")]);
}

#[test]
fn checkpoint_writes_on_one_teammate_never_lose_each_other() {
    let log = scratch("checkpoint-concurrent");
    append(&log, &persona_event(&persona("ada", "Ada")));
    let ready = std::sync::Arc::new(std::sync::Barrier::new(16));
    let writers: Vec<_> = (0..16)
        .map(|i| {
            let (log, ready) = (log.clone(), ready.clone());
            std::thread::spawn(move || {
                ready.wait();
                checkpoint_session(&log, "ada", &format!("b{i}"), None, "s").unwrap();
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(checkpoints_of(&log, "ada").len(), 16);
}

#[test]
fn voices_own_caps_count_only_until_the_owner_sets_spending() {
    use crate::voice::settings::VoiceSettings;
    use hotline_contract::BudgetLimits;
    let log = scratch("voice-shared-caps");
    append(
        &log,
        &setting("voice", json!({"dayUsd": 5, "monthUsd": 50})),
    );
    assert!(!is_set(&log, "spending"));
    let voice = VoiceSettings::from_log(&log);
    assert_eq!(voice.voice, BudgetLimits::new(Some(5.0), Some(50.0)));
    assert_eq!(voice.chat, BudgetLimits::default());
    append(
        &log,
        &setting("spending", json!({"dayUsd": 0.5, "monthUsd": 4})),
    );
    assert!(is_set(&log, "spending"));
    let voice = VoiceSettings::from_log(&log);
    assert_eq!(voice.voice, BudgetLimits::new(Some(0.5), Some(4.0)));
    assert_eq!(voice.chat, BudgetLimits::default());
    append(&log, &setting("spending", json!({"chat": {"dayUsd": 1}})));
    let voice = VoiceSettings::from_log(&log);
    assert_eq!(
        voice.voice,
        BudgetLimits::default(),
        "voice's own caps are gone"
    );
    assert_eq!(voice.chat, BudgetLimits::new(Some(1.0), None));
    append(&log, &tombstone("setting", "spending"));
    assert!(!is_set(&log, "spending"));
    let voice = VoiceSettings::from_log(&log);
    assert_eq!(voice.voice, BudgetLimits::new(Some(5.0), Some(50.0)));
}

#[test]
fn image_and_spending_settings_have_typed_defaults_and_tombstones_restore_them() {
    let log = scratch("image-spending-settings");
    assert_eq!(settings(&log)["images"], json!({}));
    assert_eq!(
        settings(&log)["spending"],
        json!({"chat": {}, "voice": {}, "images": {}})
    );
    append(
        &log,
        &setting(
            "images",
            json!({"provider": "openai", "model": "gpt-image-1-mini"}),
        ),
    );
    append(&log, &setting("spending", json!({"dayUsd": 0.0})));
    let selected = settings(&log);
    let images: ImageSettings = serde_json::from_value(selected["images"].clone()).unwrap();
    assert_eq!(images.provider.as_deref(), Some("openai"));
    assert_eq!(images.model.as_deref(), Some("gpt-image-1-mini"));
    // The shared limit of earlier versions reads as voice's and images'.
    assert_eq!(
        selected["spending"],
        json!({"chat": {}, "voice": {"dayUsd": 0.0}, "images": {"dayUsd": 0.0}})
    );
    append(&log, &tombstone("setting", "images"));
    append(&log, &tombstone("setting", "spending"));
    assert_eq!(settings(&log)["images"], json!(ImageSettings::default()));
    assert_eq!(
        settings(&log)["spending"],
        json!(SpendingSettings::default())
    );
}

#[test]
fn malformed_image_and_spending_settings_remain_visible_to_fail_closed() {
    let log = scratch("invalid-image-spending-settings");
    append(&log, &setting("images", json!({"provider": 17})));
    append(
        &log,
        &setting("spending", json!({"dayUsd": -1.0, "monthUsd": 0.0})),
    );
    assert_eq!(settings(&log)["images"], json!({"provider": 17}));
    assert_eq!(
        settings(&log)["spending"],
        json!({"dayUsd": -1.0, "monthUsd": 0.0})
    );
    assert!(serde_json::from_value::<ImageSettings>(settings(&log)["images"].clone()).is_err());
    assert!(normalize_setting("spending", &settings(&log)["spending"]).is_err());
    assert_eq!(try_settings(&log).unwrap(), settings(&log));
}

#[test]
fn strict_settings_preserve_zero_limits_and_refuse_corrupt_saved_caps() {
    let log = scratch("strict-spending-settings");
    assert_eq!(try_settings(&log).unwrap(), settings(&log));
    append(&log, &setting("spending", json!({"dayUsd": 0.0})));
    assert_eq!(
        try_settings(&log).unwrap()["spending"],
        json!({"chat": {}, "voice": {"dayUsd": 0.0}, "images": {"dayUsd": 0.0}})
    );
    std::fs::write(
        hotline_support::paths::room_path(log.root()),
        "{\"kind\":\"setting\",\"id\":\"spending\",\"value\":{\"dayUsd\":0,\"monthUsd\":0},\"private\":\"corrupt-room-canary\"\n",
    ).unwrap();
    // Corruption is a saved-file fixture, outside the live log's one
    // writer. Reopen it as startup does rather than retain its old fold.
    let log = Log::open(log.root());
    assert_eq!(
        settings(&log)["spending"],
        json!(SpendingSettings::default())
    );
    let error = try_settings(&log).unwrap_err();
    assert!(error.contains("could not be safely read"));
    assert!(!error.contains("corrupt-room-canary"));
}

#[test]
fn strict_settings_refuse_reserved_ids_with_other_event_kinds() {
    let log = scratch("strict-setting-event-kinds");
    for key in ["images", "spending"] {
        append(&log, &setting(key, json!({})));
        assert!(try_settings(&log).is_ok());
        for deleted in [false, true] {
            append(
                &log,
                &json!({"kind": "credential", "id": key, "deleted": deleted}),
            );
            assert!(
                try_settings(&log)
                    .unwrap_err()
                    .contains("invalid event kind")
            );
        }
        append(&log, &tombstone("setting", key));
        assert_eq!(try_settings(&log).unwrap(), settings(&log));
    }
}

fn job(id: &str, persona_id: &str, kind: ScheduleKind, prompt: &str, quiet: bool) -> ScheduledJob {
    ScheduledJob {
        id: id.to_string(),
        persona_id: persona_id.to_string(),
        kind,
        when: (kind == ScheduleKind::Schedule).then_some(1_700_000_100_000),
        every: (kind == ScheduleKind::Loop).then_some(15_000),
        prompt: prompt.to_string(),
        quiet: quiet.then_some(true),
        reminder: None,
        goal: None,
        until: None,
        times: None,
        without: None,
        operator_created: false,
        next_at: 1_700_000_100_000,
        created_at: 1_700_000_000_000,
        last_post: None,
        ended_at: None,
    }
}

#[test]
fn a_schedule_event_folds_as_the_job_it_was() {
    let log = scratch("schedule-create");
    let once = job(
        "job-1",
        "ada",
        ScheduleKind::Schedule,
        "check the crane",
        false,
    );
    append_schedule(&log, &once).unwrap();

    assert_eq!(&schedules(&log)[0], &once);
    // The stream's kind is always `schedule`, even though this job's kind
    // is too — a loop has to share the slot, so the job's own kind is not
    // what `kind` says on disk.
    assert_eq!(log.load(&StreamId::Room)[0]["kind"], "schedule");
    assert_eq!(log.load(&StreamId::Room)[0].get("every"), None);
    assert_eq!(log.load(&StreamId::Room)[0]["when"], once.when.unwrap());
}

#[test]
fn operator_schedule_provenance_survives_the_room_fold() {
    let log = scratch("schedule-operator");
    let mut operator = job(
        "job-operator",
        "ada",
        ScheduleKind::Schedule,
        "operator check",
        false,
    );
    operator.operator_created = true;
    append_schedule(&log, &operator).unwrap();

    assert_eq!(schedules(&log), vec![operator]);
    assert_eq!(log.load(&StreamId::Room)[0]["operatorCreated"], true);
}

#[test]
fn a_loop_is_the_schedule_event_that_carries_every() {
    let log = scratch("schedule-loop");
    let loop_job = job("job-2", "ada", ScheduleKind::Loop, "sweep the inbox", true);
    append_schedule(&log, &loop_job).unwrap();

    assert_eq!(schedules(&log)[0], loop_job);
    assert_eq!(log.load(&StreamId::Room)[0]["kind"], "schedule");
    assert_eq!(log.load(&StreamId::Room)[0]["every"], 15_000);
    assert_eq!(log.load(&StreamId::Room)[0]["quiet"], true);
}

/// An ended job no longer fires, and is still listed: its thread stays
/// readable. Its end and its last post survive the fold.
#[test]
fn an_ended_job_leaves_the_clock_and_stays_on_the_list() {
    let log = scratch("schedule-ended");
    let mut ended = job("job-3", "ada", ScheduleKind::Loop, "watch CI", false);
    ended.goal = Some("CI is green".to_string());
    ended.times = Some(2);
    ended.last_post = Some(hotline_contract::JobPost {
        at: 1_700_000_050_000,
        text: "CI is green.".to_string(),
    });
    ended.ended_at = Some(1_700_000_060_000);
    append_schedule(&log, &ended).unwrap();

    assert_eq!(schedules(&log), []);
    assert_eq!(all_jobs(&log), vec![ended]);
}

/// A teammate whose id names no file could never be opened, and the
/// startup fold opens every teammate's tape before it serves one.
#[test]
fn a_record_with_an_empty_id_is_not_a_teammate() {
    let log = scratch("empty-id");
    append(&log, &persona_event(&persona("", "Nobody")));
    append(&log, &persona_event(&persona("ada", "Ada")));

    let ids: Vec<String> = roster(&log).into_iter().map(|persona| persona.id).collect();
    assert_eq!(ids, ["ada"]);
}

#[test]
fn kind_is_the_first_key_on_every_room_event_this_writes() {
    let log = scratch("kind-leads");
    append_persona(&log, &persona("ada", "Ada")).unwrap();
    append_schedule(
        &log,
        &job(
            "job-1",
            "ada",
            ScheduleKind::Schedule,
            "check the crane",
            false,
        ),
    )
    .unwrap();
    tombstone_schedule(&log, "job-1").unwrap();

    let raw = std::fs::read_to_string(log.root().join("room.jsonl")).unwrap();
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 3, "{raw}");
    for line in &lines {
        assert!(
            line.starts_with("{\"kind\":"),
            "kind was not the leading key: {line}"
        );
    }
    assert!(
        lines[0].starts_with("{\"kind\":\"persona\""),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("{\"kind\":\"schedule\""),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("{\"kind\":\"schedule\""),
        "{}",
        lines[2]
    );
}

#[test]
fn one_teammates_schedules_are_its_own_and_none_is_an_answer() {
    let log = scratch("teammate-schedules");
    append_persona(&log, &persona("ada", "Ada")).unwrap();
    append_persona(&log, &persona("bob", "Bob")).unwrap();

    // A teammate with nothing waiting is a list with nothing in it; a
    // teammate the room does not hold is no list at all.
    assert_eq!(teammate_schedules(&log, "ada").unwrap(), Some(Vec::new()));
    assert_eq!(teammate_schedules(&log, "nobody").unwrap(), None);

    let mut sweep = job("job-1", "ada", ScheduleKind::Loop, "sweep the inbox", true);
    let once = job(
        "job-2",
        "ada",
        ScheduleKind::Schedule,
        "check the crane",
        false,
    );
    let theirs = job(
        "job-3",
        "bob",
        ScheduleKind::Schedule,
        "count the boats",
        false,
    );
    for job in [&sweep, &once, &theirs] {
        append_schedule(&log, job).unwrap();
    }
    assert_eq!(
        teammate_schedules(&log, "ada").unwrap(),
        Some(vec![sweep.clone(), once])
    );
    assert_eq!(teammate_schedules(&log, "bob").unwrap(), Some(vec![theirs]));

    // A loop that ran is the same job with a later next run, once; a
    // one-shot that fired is gone.
    sweep.next_at += 15_000;
    append_schedule(&log, &sweep).unwrap();
    tombstone_schedule(&log, "job-2").unwrap();
    assert_eq!(teammate_schedules(&log, "ada").unwrap(), Some(vec![sweep]));

    // A teammate deleted is no longer one whose jobs can be read, though
    // the clock has not yet cleared them away.
    append(&log, &tombstone("persona", "ada"));
    assert_eq!(teammate_schedules(&log, "ada").unwrap(), None);
}

#[test]
fn a_room_that_cannot_be_read_is_an_error_not_an_empty_list() {
    let log = scratch("teammate-schedules-unreadable");
    // Nobody has written to the room yet: that is a room with no
    // teammates, which is an answer.
    assert_eq!(teammate_schedules(&log, "ada").unwrap(), None);
    // A room stream that is there but cannot be read is not.
    std::fs::create_dir_all(log.root().join("room.jsonl")).unwrap();
    let error = teammate_schedules(&log, "ada").unwrap_err();
    assert!(error.contains("could not be read"), "{error}");
    assert!(log.try_load(&StreamId::Room).is_err());
    assert_eq!(log.load(&StreamId::Room), [] as [Value; 0]);
}

#[test]
fn a_tombstone_cancels_a_job_and_a_rewrite_flips_its_silence() {
    let log = scratch("schedule-cancel-quiet");
    let mut loud = job("job-1", "ada", ScheduleKind::Loop, "check the order", false);
    let quiet = job("job-2", "bob", ScheduleKind::Schedule, "say the word", true);
    append_schedule(&log, &loud).unwrap();
    append_schedule(&log, &quiet).unwrap();
    assert_eq!(schedules(&log), [loud.clone(), quiet.clone()]);

    loud.quiet = Some(true);
    append_schedule(&log, &loud).unwrap();
    assert_eq!(schedules(&log)[0].quiet, Some(true));

    loud.quiet = None;
    append_schedule(&log, &loud).unwrap();
    assert_eq!(schedules(&log)[0].quiet, None);

    tombstone_schedule(&log, "job-1").unwrap();
    assert_eq!(schedules(&log), [quiet]);
    // The stream still carries the delete, which is what a mirror ships.
    let ids: Vec<String> = log
        .load(&StreamId::Room)
        .iter()
        .map(|event| event["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["job-1", "job-2"]);
}

fn import(browser: &str, profile: &str, sites: &[(&str, u32)], at: i64) -> CookieImport {
    CookieImport {
        browser_id: browser.to_owned(),
        browser_name: browser.to_uppercase(),
        profile_id: profile.to_owned(),
        profile_name: profile.to_owned(),
        imported_at: at,
        sites: sites
            .iter()
            .map(|(domain, cookies)| CookieSite {
                domain: (*domain).to_owned(),
                cookies: *cookies,
            })
            .collect(),
    }
}

#[test]
fn an_import_from_the_same_browser_and_profile_merges_and_another_is_listed_beside_it() {
    let mut imports = Vec::new();
    merge_cookie_import(
        &mut imports,
        import("chrome", "Default", &[("github.com", 3)], 1),
    );
    merge_cookie_import(
        &mut imports,
        import(
            "chrome",
            "Default",
            &[("gitlab.com", 2), ("github.com", 5)],
            2,
        ),
    );
    merge_cookie_import(
        &mut imports,
        import("firefox", "default", &[("github.com", 1)], 3),
    );
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].imported_at, 2);
    assert_eq!(
        imports[0]
            .sites
            .iter()
            .map(|site| (site.domain.as_str(), site.cookies))
            .collect::<Vec<_>>(),
        vec![("github.com", 5), ("gitlab.com", 2)],
        "the union, sorted, with the latest count"
    );
    assert_eq!(imports[1].browser_id, "firefox");
}

#[test]
fn the_record_is_the_latest_whole_list_per_teammate() {
    let log = scratch("cookie-imports");
    assert_eq!(cookie_imports(&log, "ada"), [] as [CookieImport; 0]);
    let first = vec![import("chrome", "Default", &[("github.com", 3)], 1)];
    record_cookie_imports(&log, "ada", &first).unwrap();
    record_cookie_imports(
        &log,
        "bob",
        &[import("firefox", "default", &[("x.test", 1)], 2)],
    )
    .unwrap();
    assert_eq!(cookie_imports(&log, "ada"), first);
    record_cookie_imports(&log, "ada", &[]).unwrap();
    assert!(
        cookie_imports(&log, "ada").is_empty(),
        "the latest record wins"
    );
    assert_eq!(cookie_imports(&log, "bob").len(), 1);
}
