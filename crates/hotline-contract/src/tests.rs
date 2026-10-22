//! The wire shapes that grow by adding: a reader of the old shape still
//! reads the new one, and the new one still reads the old.

use serde_json::json;
use ts_rs::TS;

use super::*;

#[test]
fn voice_input_mode_is_additive_and_legacy_calls_keep_audio() {
    for value in [
        json!({"cmd":"voice.status","params":{}}),
        json!({"cmd":"voice.call_start","params":{"callId":"fixture"}}),
    ] {
        let command = serde_json::from_value::<Command>(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), value);
    }
    let old_reply: VoiceCall = serde_json::from_value(json!({
        "callId":"fixture", "input":["audio/wav"], "output":"audio/wav"
    }))
    .unwrap();
    assert_eq!(old_reply.input_mode, VoiceInputMode::Audio);
    let text_start = serde_json::from_value::<Command>(json!({
        "cmd":"voice.call_start", "params":{"callId":"fixture","inputMode":"text"}
    }))
    .unwrap();
    assert!(matches!(
        text_start,
        Command::VoiceCallStart {
            input_mode: Some(VoiceInputMode::Text),
            ..
        }
    ));
    assert!(
        serde_json::from_value::<Command>(json!({
            "cmd":"voice.call_start", "params":{"callId":"fixture","inputMode":"partial"}
        }))
        .is_err()
    );
}

#[test]
fn direct_voice_readiness_is_an_additive_optional_field() {
    let old: VoiceStatus = serde_json::from_value(json!({
        "capabilities":["voiceDirectCalls"], "available":true,
        "budget":{"dayUsd":2,"monthUsd":20,"spentDayUsd":0,"spentMonthUsd":0}
    }))
    .unwrap();
    assert!(!old.direct_available);
    assert!(VoiceStatus::decl(&ts_rs::Config::default()).contains("directAvailable?: boolean"));
    // The counts of how replies were said are additive too.
    assert_eq!(old.replies, None);
    assert!(VoiceStatus::decl(&ts_rs::Config::default()).contains("replies?: Array<VoiceReplies>"));
    assert!(serde_json::to_value(&old).unwrap().get("replies").is_none());
}

#[test]
fn images_settings_and_status_have_additive_wire_shapes() {
    let config = ts_rs::Config::default();
    assert_eq!(
        serde_json::to_value(ImageSettings::default()).unwrap(),
        json!({})
    );
    assert!(ImageSettings::decl(&config).contains("provider?: string"));
    assert!(ImageSettings::decl(&config).contains("model?: string"));
    assert!(SpendingSettings::decl(&config).contains("voice: BudgetLimits"));
    assert!(BudgetLimits::decl(&config).contains("dayUsd?: number"));
    assert!(BudgetLimits::decl(&config).contains("monthUsd?: number"));
    assert!(SpendingBudget::decl(&config).contains("dayUsd: number | null"));
    assert!(VoiceBudget::decl(&config).contains("dayUsd?: number"));
    assert!(ImagesStatus::decl(&config).contains("unavailable?: string"));
    assert!(ImagesStatus::decl(&config).contains("provider?: string"));
    assert!(ImagesStatus::decl(&config).contains("model?: string"));
    assert!(ImagesStatus::decl(&config).contains("spending?: SpendingSummary"));
    assert!(ImagesStatus::decl(&config).contains("spendingUnavailable?: string"));
    let command = Command::ImagesStatus {};
    let value = json!({"cmd": "images.status", "params": {}});
    assert_eq!(serde_json::to_value(&command).unwrap(), value);
    assert_eq!(serde_json::from_value::<Command>(value).unwrap(), command);
}

#[test]
fn an_agent_event_is_stamped_with_where_it_came_from_additively() {
    // A tape written before posts were stamped reads, and writes back, as it
    // was: the importer copies it unchanged and this is how it is read.
    let old = json!({"kind":"agent","id":"a1","ts":1,"text":"hello"});
    let event: TranscriptEvent = serde_json::from_value(old.clone()).unwrap();
    assert!(matches!(
        &event,
        TranscriptEvent::Agent {
            from: None,
            scheduled: None,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(&event).unwrap(), old);
    let posted = json!({
        "kind":"agent","id":"a2","ts":2,"text":"the fix is in",
        "from":{"thread":"c1","kind":"call"},
        "scheduled":{"jobId":"j1","kind":"loop","name":"Crane check","operatorCreated":true,"quiet":true},
    });
    let event: TranscriptEvent = serde_json::from_value(posted.clone()).unwrap();
    assert_eq!(serde_json::to_value(&event).unwrap(), posted);
}

const A: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";

#[test]
fn agent_account_fields_are_additive_and_absent_is_system_default() {
    let config = ts_rs::Config::default();
    let old = json!({"backendId":"claude-acp","sessionId":"s1"});
    let c: SessionCheckpoint = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(c.account_id, None);
    assert_eq!(serde_json::to_value(&c).unwrap(), old);
    let on = json!({"backendId":"claude-acp","sessionId":"s1","accountId":A});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SessionCheckpoint>(on.clone()).unwrap())
            .unwrap(),
        on
    );
    for line in [
        json!({"kind":"chapter","id":"c1","ts":1,"backendId":"claude-acp","sessionId":"s1"}),
        json!({"kind":"chapter","id":"c1","ts":1,"backendId":"claude-acp","accountId":A}),
    ] {
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<TranscriptEvent>(line.clone()).unwrap())
                .unwrap(),
            line
        );
    }
    let p: Persona = serde_json::from_value(json!({
        "id":"ada","name":"Ada","goal":"","backendId":"claude-acp","cwd":"/w",
        "mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],
        "createdAt":1,"updatedAt":1
    }))
    .unwrap();
    assert!(
        serde_json::to_value(&p)
            .unwrap()
            .get("agentAccountId")
            .is_none()
    );
    assert!(SessionCheckpoint::decl(&config).contains("accountId?: string"));
    assert!(Persona::decl(&config).contains("agentAccountId?: string"));
    assert!(PersonaDraft::decl(&config).contains("agentAccountId?: string"));
    assert!(SessionInfo::decl(&config).contains("account?: SessionAccount"));
    let events = TranscriptEvent::decl(&config);
    assert!(events.contains("backendId: string, accountId?: string"));
    assert_eq!(
        events
            .matches("backendId?: string, accountId?: string")
            .count(),
        2
    );
}

#[test]
fn account_commands_round_trip_under_their_wire_names() {
    let config = ts_rs::Config::default();
    for value in [
        json!({"cmd":"accounts.list","params":{}}),
        json!({"cmd":"accounts.create","params":{
            "backendId":"claude-acp","name":"Work","startWith":"system_default"}}),
        json!({"cmd":"accounts.create","params":{
            "backendId":"claude-acp","name":"Work","startWith":"empty",
            "categories":["instructions","skills"]}}),
        json!({"cmd":"accounts.rename","params":{"accountId":A,"name":"Home"}}),
        json!({"cmd":"accounts.remove","params":{"accountId":A}}),
        json!({"cmd":"accounts.status","params":{"accountId":A}}),
        json!({"cmd":"accounts.login.start","params":{"accountId":A}}),
        json!({"cmd":"accounts.login.start","params":{"accountId":A,"email":"a@b.co"}}),
        json!({"cmd":"accounts.login.poll","params":{"accountId":A,"id":"l1"}}),
        json!({"cmd":"accounts.login.input","params":{"accountId":A,"id":"l1","input":"y\n"}}),
        json!({"cmd":"accounts.login.cancel","params":{"accountId":A,"id":"l1"}}),
        json!({"cmd":"accounts.setup.list","params":{"accountId":A}}),
        json!({"cmd":"accounts.setup.copy","params":{"accountId":A,"categories":["skills"]}}),
        json!({"cmd":"persona.account","params":{"personaId":"ada","accountId":A}}),
        json!({"cmd":"persona.account","params":{"personaId":"ada","accountId":null}}),
    ] {
        let command = serde_json::from_value::<Command>(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&command).unwrap(), value);
    }
    assert!(matches!(
        serde_json::from_value::<Command>(
            json!({"cmd":"persona.account","params":{"personaId":"ada"}})
        )
        .unwrap(),
        Command::PersonaAccount {
            account_id: None,
            ..
        }
    ));
    let commands = Command::decl(&config);
    assert!(commands.contains(
        r#""cmd": "persona.account", "params": { personaId: string, accountId?: string | null, }"#
    ));
    assert!(commands.contains(
        r#""cmd": "accounts.login.start", "params": { accountId: string, email?: string, }"#
    ));
}

#[test]
fn an_agent_account_reads_back_as_written_and_its_states_are_snake_case() {
    let config = ts_rs::Config::default();
    let bare = json!({"id":A,"backendId":"claude-acp","name":"Account 1","createdAt":1});
    let a: AgentAccount = serde_json::from_value(bare.clone()).unwrap();
    assert_eq!((a.login_at, a.setup.is_none()), (None, true));
    assert_eq!(serde_json::to_value(&a).unwrap(), bare);
    let mut full = bare;
    full["loginAt"] = json!(2);
    full["setup"] = json!({"from":"system_default","at":1,"categories":["instructions"]});
    let a: AgentAccount = serde_json::from_value(full.clone()).unwrap();
    assert_eq!(serde_json::to_value(&a).unwrap(), full);
    assert_eq!(
        serde_json::to_value(SetupFrom::SystemDefault).unwrap(),
        "system_default"
    );
    assert_eq!(serde_json::to_value(SetupFrom::Empty).unwrap(), "empty");
    assert_eq!(
        serde_json::to_value(AccountStatusState::NotChecked).unwrap(),
        "not_checked"
    );
    assert_eq!(
        serde_json::to_value(AccountStatusState::SignedIn).unwrap(),
        "signed_in"
    );
    assert_eq!(
        serde_json::to_value(LoginAttemptState::Succeeded).unwrap(),
        "succeeded"
    );
    assert_eq!(
        serde_json::to_value(AccountHold::SigningIn).unwrap(),
        "signing_in"
    );
    assert_eq!(
        serde_json::to_value(AccountHold::NotSignedIn).unwrap(),
        "not_signed_in"
    );
    assert_eq!(
        serde_json::to_value(SessionAccount {
            running: None,
            wanted: None,
            held: None
        })
        .unwrap(),
        json!({})
    );
    assert!(AccountView::decl(&config).contains("folder?: string"));
}
