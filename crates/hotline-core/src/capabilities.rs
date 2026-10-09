//! What the owner can pick for each job a provider does for the room, read
//! from the same connections the resolvers read. Nothing here is a new key or
//! a new connection: a provider is offered only if it is already connected.

use crate::contract::{
    BudgetKind, CapabilityJob, CapabilityModel, CapabilityOptions, CapabilityPick,
    CapabilityProvider, CapabilitySpending, SpendingBudget, SpendingKind, SpendingLine,
};
use crate::imagegen::{self, ImageSettings};
use crate::log::Log;
use crate::spending::{SpendingSettings, SpendingSummary};
use crate::vault::Vault;
use crate::voice::dispatcher::{ProviderDispatcher, is_chat};
use crate::voice::metering::Budget;
use crate::voice::settings::{Choice, VoiceSettings};
use crate::voice::speech;
use serde_json::{Map, Value};
use std::sync::Arc;

/// The options for every job, with the choice made and what automatic would
/// pick now. `images_spent` is the image tally, and `voice` the budget that
/// keeps the tally of voice and the call assistant.
pub async fn options(
    vault: &Arc<Vault>,
    log: &Log,
    images_spent: Result<SpendingSummary, String>,
    voice: &Budget,
) -> CapabilityOptions {
    let settings = crate::room::settings(log);
    let voice_budget = voice;
    let voice = VoiceSettings::from_room(&settings);
    let images: ImageSettings =
        serde_json::from_value(settings["images"].clone()).unwrap_or_default();
    let image_options = imagegen::options(vault);
    let speech = speech::options(vault).await;

    CapabilityOptions {
        images: CapabilityJob {
            selected: images.provider.as_ref().map(|provider_id| {
                pick(
                    &image_options,
                    &Choice {
                        provider_id: provider_id.clone(),
                        model_id: images.model.clone(),
                        voice: None,
                        effort: None,
                    },
                )
            }),
            automatic: imagegen::describe(vault, &ImageSettings::default())
                .ok()
                .map(|id| {
                    pick(
                        &image_options,
                        &Choice {
                            provider_id: id.provider_id,
                            model_id: Some(id.model_id),
                            voice: None,
                            effort: None,
                        },
                    )
                }),
            unavailable: image_options
                .is_empty()
                .then(|| crate::imagegen::NOTHING_DRAWS.to_string()),
            options: image_options,
        },
        stt: CapabilityJob {
            selected: voice.stt.as_ref().map(|choice| pick(&speech.stt, choice)),
            automatic: speech.automatic_stt,
            unavailable: speech.stt.is_empty().then(|| {
                "Download a speech model for the desk, or connect OpenAI, Google, OpenRouter, Groq or Mistral to hear you.".to_string()
            }),
            options: speech.stt,
        },
        tts: CapabilityJob {
            selected: voice.tts.as_ref().map(|choice| pick(&speech.tts, choice)),
            automatic: speech.automatic_tts,
            unavailable: speech
                .tts
                .is_empty()
                .then(|| "Connect OpenAI, Google, OpenRouter or Groq to speak.".to_string()),
            options: speech.tts,
        },
        dispatcher: dispatcher(vault, &settings, &voice),
        spending: spending(&settings, images_spent, voice_budget),
    }
}

/// The chat models of the connected providers, as the model picker offers
/// them, minus the ones that only do speech, images or embeddings.
fn dispatcher(
    vault: &Arc<Vault>,
    settings: &Map<String, Value>,
    voice: &VoiceSettings,
) -> CapabilityJob {
    let choices = crate::models::choices(
        &vault.provider_auth(),
        &crate::models::enabled_models(settings),
        &vault.account_models(),
        &vault.model_metadata(),
    );
    let mut options: Vec<CapabilityProvider> = Vec::new();
    for choice in choices {
        let Some((provider_id, model)) = choice.id.split_once('/') else {
            continue;
        };
        if !is_chat(model) {
            continue;
        }
        let efforts = crate::models::efforts(&choice.id);
        let entry = CapabilityModel {
            id: model.to_string(),
            label: Some(choice.name.clone()).filter(|name| name != model),
            voices: None,
            efforts: (!efforts.is_empty()).then_some(efforts),
        };
        match options
            .iter_mut()
            .find(|provider| provider.provider_id == provider_id)
        {
            Some(provider) => provider.models.push(entry),
            None => options.push(CapabilityProvider {
                provider_id: provider_id.to_string(),
                provider_name: choice.group.clone().unwrap_or_else(|| provider_id.into()),
                models: vec![entry],
            }),
        }
    }
    let automatic = ProviderDispatcher::resolve_with(
        vault.clone(),
        settings,
        &VoiceSettings {
            dispatcher: None,
            ..voice.clone()
        },
    );
    let (automatic, unavailable) = match automatic {
        Ok(dispatcher) => {
            let id = dispatcher.id();
            let pick = pick(
                &options,
                &Choice {
                    provider_id: id.provider_id,
                    model_id: Some(id.model_id),
                    voice: None,
                    effort: None,
                },
            );
            (Some(pick), None)
        }
        Err(unavailable) => (None, Some(unavailable)),
    };
    CapabilityJob {
        selected: voice
            .dispatcher
            .as_ref()
            .map(|choice| pick(&options, choice)),
        automatic,
        unavailable: unavailable.filter(|_| options.is_empty()),
        options,
    }
}

/// The three budgets, each with its limits, what it has spent and on what.
/// Chat and voice come from the voice tally (and the room's limits as voice
/// reads them, legacy voice caps included), images from the image tally.
fn spending(
    settings: &Map<String, Value>,
    images_spent: Result<SpendingSummary, String>,
    voice: &Budget,
) -> CapabilitySpending {
    let limits: SpendingSettings =
        serde_json::from_value(settings["spending"].clone()).unwrap_or_default();
    let mut unavailable = None;
    let images = images_spent.unwrap_or_else(|error| {
        unavailable = Some(error);
        SpendingSummary {
            day_usd: 0.0,
            month_usd: 0.0,
        }
    });
    let spent = voice.spent().unwrap_or_else(|error| {
        unavailable.get_or_insert_with(|| error.to_string());
        Default::default()
    });
    let line = |kind, day_usd, month_usd| SpendingLine {
        kind,
        day_usd,
        month_usd,
    };
    let budget =
        |kind, limits: crate::contract::BudgetLimits, lines: Vec<SpendingLine>| SpendingBudget {
            kind,
            day_usd: limits.day_usd,
            month_usd: limits.month_usd,
            spent_day_usd: lines.iter().map(|line| line.day_usd).sum(),
            spent_month_usd: lines.iter().map(|line| line.month_usd).sum(),
            lines,
        };
    let (day, month) = (spent.day, spent.month);
    let mut images_budget = budget(BudgetKind::Images, limits.images, Vec::new());
    images_budget.spent_day_usd = images.day_usd;
    images_budget.spent_month_usd = images.month_usd;
    CapabilitySpending {
        budgets: vec![
            budget(
                BudgetKind::Chat,
                voice.limits(BudgetKind::Chat),
                vec![
                    line(SpendingKind::Teammates, day.teammates, month.teammates),
                    line(
                        SpendingKind::CallAssistant,
                        day.dispatcher,
                        month.dispatcher,
                    ),
                ],
            ),
            budget(
                BudgetKind::Voice,
                voice.limits(BudgetKind::Voice),
                vec![
                    line(SpendingKind::Transcription, day.stt, month.stt),
                    line(SpendingKind::Speech, day.tts, month.tts),
                ],
            ),
            images_budget,
        ],
        unavailable,
    }
}

/// A choice with its provider's name, which is its id for a provider that is
/// no longer connected.
fn pick(options: &[CapabilityProvider], choice: &Choice) -> CapabilityPick {
    CapabilityPick {
        provider_id: choice.provider_id.clone(),
        provider_name: options
            .iter()
            .find(|provider| provider.provider_id == choice.provider_id)
            .map_or_else(
                || choice.provider_id.clone(),
                |provider| provider.provider_name.clone(),
            ),
        model_id: choice.model_id.clone(),
        voice: choice.voice.clone(),
        effort: choice.effort.clone(),
    }
}
