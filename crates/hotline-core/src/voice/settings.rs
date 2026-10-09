//! What the owner has said about voice, read from the room's `voice` setting:
//!
//! ```json
//! { "stt": { "provider": "groq", "model": "whisper-large-v3-turbo" },
//!   "tts": { "provider": "openai", "model": "gpt-4o-mini-tts", "voice": "marin" },
//!   "fallbackTts": { "provider": "google" },
//!   "dispatcher": { "provider": "openai", "model": "gpt-5-mini", "effort": "low" } }
//! ```
//!
//! Every key is optional. A value that cannot be read costs its own
//! preference and nothing else, like the other room settings.
//!
//! What voice may spend comes from the room's `spending` setting: the Voice
//! budget covers transcription and speech, and the Chat budget the call
//! assistant. Neither has a limit unless the owner set one. A room that never
//! set `spending` but kept the voice-only `dayUsd` and `monthUsd` of early
//! versions here has those as its Voice limits.

use crate::contract::{BudgetLimits, SpendingSettings};
use crate::log::Log;
use serde_json::{Map, Value};

/// The owner's pick for one job: a connected provider, and optionally the
/// model and voice to use there instead of that provider's defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub provider_id: String,
    pub model_id: Option<String>,
    pub voice: Option<String>,
    /// A thinking level for a chat model, as its Effort picker names it.
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceSettings {
    /// The Chat budget's limits, which the call assistant spends against.
    pub chat: BudgetLimits,
    /// The Voice budget's limits, for transcription and speech.
    pub voice: BudgetLimits,
    pub stt: Option<Choice>,
    pub tts: Option<Choice>,
    pub fallback_tts: Option<Choice>,
    /// The chat model that routes what was said. `provider` alone lets the
    /// desk pick that provider's quickest model; without this the desk uses
    /// the room's default provider.
    pub dispatcher: Option<Choice>,
}

impl VoiceSettings {
    /// The voice preferences of the room behind `log`. `room::settings` always
    /// carries a default `spending`, which says nothing the owner chose, so it
    /// counts only once the owner has set it.
    pub fn from_log(log: &Log) -> VoiceSettings {
        let mut settings = crate::room::settings(log);
        if !crate::room::is_set(log, "spending") {
            settings.remove("spending");
        }
        VoiceSettings::from_room(&settings)
    }

    /// The voice preferences in the room's settings, as `room::settings`
    /// returns them.
    pub fn from_room(settings: &Map<String, Value>) -> VoiceSettings {
        let empty = Map::new();
        let voice = settings
            .get("voice")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let (chat, voice_limits) = match settings.get("spending") {
            Some(spending) => match serde_json::from_value::<SpendingSettings>(spending.clone())
                .ok()
                .filter(|spending| spending.validate().is_ok())
            {
                Some(spending) => (spending.chat, spending.voice),
                // A spending setting that cannot be read turns paid chat and
                // voice off rather than leaving them without limits.
                None => {
                    let off = BudgetLimits::new(Some(0.0), Some(0.0));
                    (off, off)
                }
            },
            None => (
                BudgetLimits::default(),
                BudgetLimits::new(cap(voice, "dayUsd"), cap(voice, "monthUsd")),
            ),
        };
        VoiceSettings {
            chat,
            voice: voice_limits,
            stt: choice(voice, "stt"),
            tts: choice(voice, "tts"),
            fallback_tts: choice(voice, "fallbackTts"),
            dispatcher: choice(voice, "dispatcher"),
        }
    }
}

/// An early version's voice-only cap. Zero is a cap (paid voice is off); a
/// negative or non-numeric one is none.
fn cap(source: &Map<String, Value>, key: &str) -> Option<f64> {
    source
        .get(key)
        .and_then(Value::as_f64)
        .filter(|usd| usd.is_finite() && *usd >= 0.0)
}

fn choice(voice: &Map<String, Value>, key: &str) -> Option<Choice> {
    let pick = voice.get(key)?.as_object()?;
    let text = |field: &str| {
        pick.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    Some(Choice {
        provider_id: text("provider")?,
        model_id: text("model"),
        voice: text("voice"),
        effort: text("effort"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn room(voice: Value) -> Map<String, Value> {
        let mut settings = Map::new();
        settings.insert("voice".into(), voice);
        settings
    }

    #[test]
    fn a_room_that_never_set_voice_or_spending_has_no_limits_and_no_picks() {
        let settings = VoiceSettings::from_room(&Map::new());
        assert_eq!(settings, VoiceSettings::default());
        assert_eq!(settings.chat, BudgetLimits::default());
        assert_eq!(settings.voice, BudgetLimits::default());
    }

    #[test]
    fn the_owners_caps_and_picks_are_read() {
        let settings = VoiceSettings::from_room(&room(json!({
            "dayUsd": 0.5,
            "monthUsd": 7,
            "stt": {"provider": "groq"},
            "tts": {"provider": "openai", "model": " gpt-4o-mini-tts ", "voice": "cedar"},
            "fallbackTts": {"provider": "google", "voice": ""},
        })));
        assert_eq!(settings.voice, BudgetLimits::new(Some(0.5), Some(7.0)));
        assert_eq!(settings.chat, BudgetLimits::default());
        assert_eq!(
            settings.stt,
            Some(Choice {
                provider_id: "groq".into(),
                model_id: None,
                voice: None,
                effort: None
            })
        );
        assert_eq!(
            settings.tts,
            Some(Choice {
                provider_id: "openai".into(),
                model_id: Some("gpt-4o-mini-tts".into()),
                voice: Some("cedar".into()),
                effort: None
            })
        );
        assert_eq!(
            settings.fallback_tts.unwrap().voice,
            None,
            "an empty voice is not a voice"
        );
    }

    #[test]
    fn the_dispatcher_can_be_named_by_provider_and_model() {
        let settings = VoiceSettings::from_room(&room(json!({
            "dispatcher": {"provider": "openai", "model": " gpt-5-mini "},
        })));
        assert_eq!(
            settings.dispatcher,
            Some(Choice {
                provider_id: "openai".into(),
                model_id: Some("gpt-5-mini".into()),
                voice: None,
                effort: None
            })
        );
        let by_provider =
            VoiceSettings::from_room(&room(json!({"dispatcher": {"provider": "groq"}})));
        assert_eq!(by_provider.dispatcher.unwrap().model_id, None);
        let thinking = VoiceSettings::from_room(&room(json!({
            "dispatcher": {"provider": "anthropic", "model": "claude-sonnet-4-6", "effort": "low"},
        })));
        assert_eq!(thinking.dispatcher.unwrap().effort.as_deref(), Some("low"));
        // A model with no provider is not a choice.
        assert_eq!(
            VoiceSettings::from_room(&room(json!({"dispatcher": {"model": "gpt-5-mini"}})))
                .dispatcher,
            None
        );
    }

    #[test]
    fn the_spending_setting_replaces_voices_own_caps() {
        let mut settings = room(json!({"dayUsd": 5, "monthUsd": 50}));
        settings.insert(
            "spending".into(),
            json!({"chat": {"dayUsd": 1}, "voice": {"monthUsd": 3}, "images": {}}),
        );
        let voice = VoiceSettings::from_room(&settings);
        assert_eq!(voice.chat, BudgetLimits::new(Some(1.0), None));
        assert_eq!(voice.voice, BudgetLimits::new(None, Some(3.0)));
        // The shared limits of earlier versions are voice's, and chat's none.
        settings.insert("spending".into(), json!({"dayUsd": 0.25, "monthUsd": 3}));
        let voice = VoiceSettings::from_room(&settings);
        assert_eq!(voice.voice, BudgetLimits::new(Some(0.25), Some(3.0)));
        assert_eq!(voice.chat, BudgetLimits::default());
        // One that cannot be read turns paid use off.
        settings.insert("spending".into(), json!({"dayUsd": -1, "monthUsd": 3}));
        let voice = VoiceSettings::from_room(&settings);
        assert!(voice.voice.off() && voice.chat.off());
        // With no spending setting at all, voice's own caps stand.
        settings.remove("spending");
        let voice = VoiceSettings::from_room(&settings);
        assert_eq!(voice.voice, BudgetLimits::new(Some(5.0), Some(50.0)));
        assert_eq!(voice.chat, BudgetLimits::default());
    }

    #[test]
    fn a_zero_cap_is_kept_and_a_bad_one_costs_only_itself() {
        let settings = VoiceSettings::from_room(&room(json!({
            "dayUsd": 0,
            "monthUsd": -5,
            "stt": "groq",
            "tts": {"model": "gpt-4o-mini-tts"},
        })));
        assert_eq!(settings.voice, BudgetLimits::new(Some(0.0), None));
        assert_eq!(settings.stt, None);
        assert_eq!(settings.tts, None);
        assert_eq!(
            VoiceSettings::from_room(&room(json!("nonsense"))),
            VoiceSettings::default()
        );
        assert_eq!(
            VoiceSettings::from_room(&room(json!({"dayUsd": "2"}))).voice,
            BudgetLimits::default()
        );
    }
}
