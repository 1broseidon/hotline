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
