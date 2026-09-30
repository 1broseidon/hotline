//! What the owner has said about voice, read from the room's `voice` setting:
//!
//! ```json
//! { "dayUsd": 2, "monthUsd": 20,
//!   "stt": { "provider": "groq", "model": "whisper-large-v3-turbo" },
//!   "tts": { "provider": "openai", "model": "gpt-4o-mini-tts", "voice": "marin" },
//!   "fallbackTts": { "provider": "google" } }
//! ```
//!
//! Every key is optional. A value that cannot be read costs its own
//! preference and nothing else, like the other room settings.

use serde_json::{Map, Value};

pub const DEFAULT_DAY_USD: f64 = 2.0;
pub const DEFAULT_MONTH_USD: f64 = 20.0;

/// The owner's pick for one job: a connected provider, and optionally the
/// model and voice to use there instead of that provider's defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub provider_id: String,
    pub model_id: Option<String>,
    pub voice: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceSettings {
    pub day_usd: f64,
    pub month_usd: f64,
    pub stt: Option<Choice>,
    pub tts: Option<Choice>,
    pub fallback_tts: Option<Choice>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            day_usd: DEFAULT_DAY_USD,
            month_usd: DEFAULT_MONTH_USD,
            stt: None,
            tts: None,
            fallback_tts: None,
        }
    }
}

impl VoiceSettings {
    /// The voice preferences in the room's settings, as `room::settings`
    /// returns them.
    pub fn from_room(settings: &Map<String, Value>) -> VoiceSettings {
        let Some(voice) = settings.get("voice").and_then(Value::as_object) else {
            return VoiceSettings::default();
        };
        VoiceSettings {
            day_usd: cap(voice, "dayUsd", DEFAULT_DAY_USD),
            month_usd: cap(voice, "monthUsd", DEFAULT_MONTH_USD),
            stt: choice(voice, "stt"),
            tts: choice(voice, "tts"),
            fallback_tts: choice(voice, "fallbackTts"),
        }
    }
}

/// Zero is a cap (voice is off); a negative or non-numeric one is not.
fn cap(voice: &Map<String, Value>, key: &str, default: f64) -> f64 {
    voice
        .get(key)
        .and_then(Value::as_f64)
        .filter(|usd| usd.is_finite() && *usd >= 0.0)
        .unwrap_or(default)
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
    fn a_room_that_never_set_voice_gets_the_default_caps_and_no_picks() {
        assert_eq!(
            VoiceSettings::from_room(&Map::new()),
            VoiceSettings::default()
        );
        assert_eq!(VoiceSettings::default().day_usd, 2.0);
        assert_eq!(VoiceSettings::default().month_usd, 20.0);
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
        assert_eq!(settings.day_usd, 0.5);
        assert_eq!(settings.month_usd, 7.0);
        assert_eq!(
            settings.stt,
            Some(Choice {
                provider_id: "groq".into(),
                model_id: None,
                voice: None
            })
        );
        assert_eq!(
            settings.tts,
            Some(Choice {
                provider_id: "openai".into(),
                model_id: Some("gpt-4o-mini-tts".into()),
                voice: Some("cedar".into())
            })
        );
        assert_eq!(
            settings.fallback_tts.unwrap().voice,
            None,
            "an empty voice is not a voice"
        );
    }

    #[test]
    fn a_zero_cap_is_kept_and_a_bad_one_costs_only_itself() {
        let settings = VoiceSettings::from_room(&room(json!({
            "dayUsd": 0,
            "monthUsd": -5,
            "stt": "groq",
            "tts": {"model": "gpt-4o-mini-tts"},
        })));
        assert_eq!(settings.day_usd, 0.0);
        assert_eq!(settings.month_usd, DEFAULT_MONTH_USD);
        assert_eq!(settings.stt, None);
        assert_eq!(settings.tts, None);
        assert_eq!(
            VoiceSettings::from_room(&room(json!("nonsense"))),
            VoiceSettings::default()
        );
        assert_eq!(
            VoiceSettings::from_room(&room(json!({"dayUsd": "2"}))).day_usd,
            DEFAULT_DAY_USD
        );
    }
}
