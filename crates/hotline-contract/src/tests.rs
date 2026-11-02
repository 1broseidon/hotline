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

#[test]
fn a_question_delivery_round_trips_with_and_without_its_work() {
    let plain = json!({
        "kind":"delivery","id":"q","ts":1,
        "cause":{"kind":"question","requestId":"r","personaId":"ada","name":"Ada","threadKey":"ada~bob","about":"x"},
        "text":"t",
    });
    let event: TranscriptEvent = serde_json::from_value(plain.clone()).unwrap();
    let written = serde_json::to_value(&event).unwrap();
    assert_eq!(written, plain);
    assert!(written["cause"].get("work").is_none());
    let mut about_work = plain;
    about_work["cause"]["work"] = json!("w");
    let event: TranscriptEvent = serde_json::from_value(about_work.clone()).unwrap();
    assert_eq!(serde_json::to_value(&event).unwrap(), about_work);
}

/// The phone's and an older window's cancel, which says nothing more, is the
/// Stop it always was; the window's Interrupt says so, on either command.
#[test]
fn a_cancel_is_a_stop_unless_it_says_it_interrupts() {
    let dm = json!({"kind":"dm","key":"ada"});
    for (stop, interrupt) in [
        (
            json!({"cmd":"session.cancel","params":{"personaId":"ada"}}),
            json!({"cmd":"session.cancel","params":{"personaId":"ada","interrupt":true}}),
        ),
        (
            json!({"cmd":"thread.cancel","params":{"thread":dm}}),
            json!({"cmd":"thread.cancel","params":{"thread":dm,"interrupt":true}}),
        ),
    ] {
        for (value, interrupts) in [(&stop, false), (&interrupt, true)] {
            let command = serde_json::from_value::<Command>(value.clone()).unwrap();
            assert!(
                matches!(
                    command,
                    Command::SessionCancel { interrupt, .. }
                        | Command::ThreadCancel { interrupt, .. } if interrupt == interrupts
                ),
                "{value}"
            );
            assert_eq!(&serde_json::to_value(&command).unwrap(), value);
        }
    }
    let decl = Command::decl(&ts_rs::Config::default());
    assert!(
        decl.contains(
            r#""cmd": "session.cancel", "params": { personaId: string, interrupt?: boolean, }"#
        ),
        "{decl}"
    );
    assert!(
        decl.contains(
            r#""cmd": "thread.cancel", "params": { thread: ThreadId, interrupt?: boolean, }"#
        ),
        "{decl}"
    );
}

#[test]
fn a_thread_names_its_desk_only_when_it_is_on_a_linked_one() {
    let here = ThreadId::side("s1");
    assert_eq!(
        serde_json::to_value(&here).unwrap(),
        json!({"kind":"side","key":"s1"})
    );
    assert_eq!(here.to_string(), "side:s1");
    let there = ThreadId::side("s1").on_desk("grizzly-desk");
    let written = json!({"desk":"grizzly-desk","kind":"side","key":"s1"});
    assert_eq!(serde_json::to_value(&there).unwrap(), written);
    assert_eq!(serde_json::from_value::<ThreadId>(written).unwrap(), there);
    assert_eq!(there.to_string(), "grizzly-desk/side:s1");
    assert_ne!(here, there);
    for thread in [&here, &there, &ThreadId::pair("laptop-desk/clementine~lou")] {
        assert_eq!(ThreadId::parse(&thread.to_string()).as_ref(), Some(thread));
    }
    for bad in ["side", "side:", "/side:s1", "nowhere:s1", "desk/side"] {
        assert_eq!(ThreadId::parse(bad), None, "{bad}");
    }
    assert!(ThreadId::decl(&ts_rs::Config::default()).contains("desk?: string"));
}

#[test]
fn what_crosses_a_link_grows_only_by_fields_an_older_desk_leaves_out() {
    let plain = json!({
        "kind":"request","requestId":"r","fromPersonaId":"ada","to":"bob",
        "message":"m","intent":"ask",
    });
    let request: LinkedDelivery = serde_json::from_value(plain.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), plain);
    let mut grown = plain;
    grown["about"] = json!("h");
    grown["files"] = json!([{"name":"q3.csv","data":"YSxi"}]);
    let request: LinkedDelivery = serde_json::from_value(grown.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), grown);

    let result = json!({"kind":"result","requestId":"r","reply":"done","failed":false});
    let read: LinkedDelivery = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap(), result);
    let mut stopped = json!({"kind":"result","requestId":"r","reply":"stopped","failed":true});
    stopped["stoppedByPerson"] = json!(true);
    let read: LinkedDelivery = serde_json::from_value(stopped.clone()).unwrap();
    assert!(matches!(
        read,
        LinkedDelivery::Result {
            stopped_by_person: true,
            ..
        }
    ));
    assert_eq!(serde_json::to_value(&read).unwrap(), stopped);

    let aside = json!({"kind":"aside","id":"a","fromPersonaId":"lou","to":"clementine",
        "from":{"kind":"side","key":"s1"},"text":"runner 3 is green","level":"note",
        "requestId":"h"});
    let read: LinkedDelivery = serde_json::from_value(aside.clone()).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap(), aside);
    let decl = LinkedDelivery::decl(&ts_rs::Config::default());
    for optional in [
        "about?: string",
        "stoppedByPerson?: boolean",
        "requestId?: string",
    ] {
        assert!(decl.contains(optional), "{optional}: {decl}");
    }

    let roster: Command =
        serde_json::from_value(json!({"cmd":"federation.roster","params":{"teammates":[]}}))
            .unwrap();
    assert_eq!(
        roster,
        Command::FederationRoster {
            teammates: Vec::new(),
            features: Vec::new()
        }
    );
    assert_eq!(
        serde_json::to_value(&roster).unwrap(),
        json!({"cmd":"federation.roster","params":{"teammates":[]}})
    );
    assert!(
        Command::decl(&ts_rs::Config::default())
            .contains("teammates: Array<LinkedTeammate>, features?: Array<string>")
    );
}
