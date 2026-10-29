//! Each harness's whole model list with every model's own effort levels, as
//! its own tools read it out of band, with no session and no teammate.
//!
//! An ACP session reports effort levels for the model it is on and only once
//! it runs, so a teammate could not choose a model's effort before its first
//! turn, nor the effort of any model but the one in use. The plan readers
//! already start each harness's CLI without a turn (`plan/`); the same
//! process answers this too: Claude Code's `initialize` lists its models
//! with their levels, and `codex app-server` answers `model/list`. The Grok
//! CLI keeps its list on disk.
//!
//! A catalogue names each model by the id its ACP session takes, so a model
//! chosen from it is one `session.set_model` can set. What a session itself
//! reports stays first: the models it lists, and the levels of the model it
//! is on. The catalogue fills the rest: the levels of every other model, the
//! models the session did not list (a Grok session that opens while its
//! sign-in renews lists only the CLI's two fallback models), and the whole
//! list when no session of the agent has opened on this desk.

use crate::contract::{BackendOffer, ConfigChoice, SessionConfig, SessionConfigCategory};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    /// Each model by its ACP id, with `efforts` always set: an empty list is
    /// a model known to have no levels.
    pub models: Vec<ConfigChoice>,
    /// Another id the harness knows a model by, to its ACP id: Claude Code's
    /// resolved model, Codex's model slug.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
    /// The model a session opens on before anyone chooses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// The id and name the harness's ACP session gives its effort picker,
    /// so a teammate at rest is set with the id its next session takes.
    pub effort_id: String,
    pub effort_name: String,
}

impl Catalog {
    fn model(&self, id: &str) -> Option<&ConfigChoice> {
        let id = self.aliases.get(id).map_or(id, String::as_str);
        self.models.iter().find(|model| model.id == id)
    }

    /// The catalogue's id for a model a session names, when it lists it.
    fn id_of(&self, id: &str) -> Option<&str> {
        self.model(id).map(|model| model.id.as_str())
    }
}

/// The effort picker among a session's settings.
pub(crate) fn effort_config(configs: &[SessionConfig]) -> Option<&SessionConfig> {
    configs
        .iter()
        .find(|config| config.category == Some(SessionConfigCategory::Effort))
}

/// An agent's offer with each model's levels: the model it opened on has
/// the levels its session reported, every other one the catalogue's. The
/// list is the catalogue's, in its order, with any model only the session
/// named after it; an offer with no models takes the catalogue's whole list,
/// and one with no effort picker is given the harness's, so a teammate at
/// rest on a model with levels can be set to one.
pub(crate) fn fill(offer: &mut BackendOffer, catalog: Option<&Catalog>) {
    let reported = effort_config(&offer.configs).cloned();
    if let Some(catalog) = catalog {
        if offer.models.is_empty() && offer.current_model_id.is_none() {
            offer.current_model_id.clone_from(&catalog.default_model);
        }
        offer.models = merged(std::mem::take(&mut offer.models), catalog);
    }
    for model in &mut offer.models {
        if let Some(config) = &reported
            && offer.current_model_id.as_ref() == Some(&model.id)
        {
            model.efforts = Some(config.options.clone());
            model.default_effort.clone_from(&config.current_id);
        } else if model.efforts.is_none()
            && let Some(listed) = catalog.and_then(|catalog| catalog.model(&model.id))
        {
            model.efforts.clone_from(&listed.efforts);
            model.default_effort.clone_from(&listed.default_effort);
        }
    }
    if reported.is_none()
        && let Some(catalog) = catalog
    {
        let current = offer
            .current_model_id
            .as_ref()
            .and_then(|id| offer.models.iter().find(|model| model.id == *id));
        offer.configs.push(SessionConfig {
            id: catalog.effort_id.clone(),
            name: catalog.effort_name.clone(),
            category: Some(SessionConfigCategory::Effort),
            current_id: current.and_then(|model| model.default_effort.clone()),
            options: current
                .and_then(|model| model.efforts.clone())
                .unwrap_or_default(),
        });
    }
}

/// A session's list with the catalogue's: the catalogue's models in its
/// order, each one the session also lists in the session's own words, then
/// any only the session lists.
pub(crate) fn merged(session: Vec<ConfigChoice>, catalog: &Catalog) -> Vec<ConfigChoice> {
    let mut models = catalog.models.clone();
    for model in session {
        match models
            .iter_mut()
            .find(|listed| Some(listed.id.as_str()) == catalog.id_of(&model.id))
        {
            Some(listed) => *listed = model,
            None => models.push(model),
        }
    }
    models
}

/// A live session's models with the levels of each it does not report: its
/// own model's are its effort picker's, the others the agent's offer's.
pub(crate) fn with_levels(
    models: &mut [ConfigChoice],
    current_model_id: &str,
    configs: &[SessionConfig],
    offered: &[ConfigChoice],
) {
    let reported = effort_config(configs);
    for model in models {
        if model.id == current_model_id {
            model.efforts = Some(
                reported
                    .map(|config| config.options.clone())
                    .unwrap_or_default(),
            );
            model.default_effort = reported.and_then(|config| config.current_id.clone());
        } else if model.efforts.is_none()
            && let Some(listed) = offered.iter().find(|one| one.id == model.id)
        {
            model.efforts.clone_from(&listed.efforts);
            model.default_effort.clone_from(&listed.default_effort);
        }
    }
}

/// A teammate at rest on `model_id`: its effort picker lists that model's
/// levels, at the teammate's own level where the model has it, else at the
/// model's default. A model known to have no levels has no picker; one
/// whose levels are not known keeps the picker the agent last offered.
pub(crate) fn fit_effort(
    configs: &mut Vec<SessionConfig>,
    models: &[ConfigChoice],
    model_id: Option<&str>,
    chosen: Option<&String>,
) {
    let Some(at) = configs
        .iter()
        .position(|config| config.category == Some(SessionConfigCategory::Effort))
    else {
        return;
    };
    let Some(model) = model_id.and_then(|id| models.iter().find(|model| model.id == id)) else {
        return;
    };
    let Some(levels) = &model.efforts else {
        return;
    };
    if levels.is_empty() {
        configs.remove(at);
        return;
    }
    let config = &mut configs[at];
    config.options.clone_from(levels);
    let offered = |value: &&String| levels.iter().any(|level| level.id == **value);
    config.current_id = chosen
        .filter(offered)
        .or(model.default_effort.as_ref().filter(offered))
        .or(levels.first().map(|level| &level.id))
        .cloned();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(id: &str) -> ConfigChoice {
        choice(id, None)
    }

    fn choice(id: &str, levels: Option<&[&str]>) -> ConfigChoice {
        ConfigChoice {
            id: id.into(),
            name: id.into(),
            description: None,
            group: None,
            account: None,
            efforts: levels.map(|levels| levels.iter().map(|id| level(id)).collect()),
            default_effort: levels.and_then(|levels| levels.first().map(|id| (*id).to_string())),
        }
    }

    fn effort(current: &str, levels: &[&str]) -> SessionConfig {
        SessionConfig {
            id: "effort".into(),
            name: "Effort".into(),
            category: Some(SessionConfigCategory::Effort),
            current_id: Some(current.into()),
            options: levels.iter().map(|id| level(id)).collect(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            models: vec![
                choice("default", Some(&["default", "low", "high", "max"])),
                choice("opus", Some(&["default", "low", "high", "max"])),
                choice("claude-haiku-4-5", Some(&[])),
            ],
            aliases: BTreeMap::from([("claude-opus-5-5".to_string(), "opus".to_string())]),
            default_model: Some("default".into()),
            effort_id: "effort".into(),
            effort_name: "Effort".into(),
        }
    }

    fn ids(levels: Option<&Vec<ConfigChoice>>) -> Vec<&str> {
        levels
            .into_iter()
            .flatten()
            .map(|level| level.id.as_str())
            .collect()
    }

    /// An agent that has never opened a session here offers the
    /// catalogue's list, on its default model, with an effort picker.
    #[test]
    fn an_agent_never_run_here_offers_the_catalogues_models_and_levels() {
        let mut offer = BackendOffer::default();
        fill(&mut offer, Some(&catalog()));
        assert_eq!(offer.models, catalog().models);
        assert_eq!(offer.current_model_id.as_deref(), Some("default"));
        let picker = effort_config(&offer.configs).expect("an effort picker");
        assert_eq!(picker.id, "effort");
        assert_eq!(picker.current_id.as_deref(), Some("default"));
        assert_eq!(
            ids(Some(&picker.options)),
            ["default", "low", "high", "max"]
        );
    }

    /// The session's own report of the model it opened on beats the
    /// catalogue's; every other model, by its id or another name the
    /// harness gives it, takes the catalogue's levels.
    #[test]
    fn the_sessions_report_wins_for_its_model_and_the_catalogue_fills_the_rest() {
        let mut offer = BackendOffer {
            models: vec![
                choice("default", None),
                choice("claude-opus-5-5", None),
                choice("claude-haiku-4-5", None),
                choice("unlisted", None),
            ],
            current_model_id: Some("default".into()),
            configs: vec![effort("high", &["default", "high"])],
            ..BackendOffer::default()
        };
        fill(&mut offer, Some(&catalog()));
        assert_eq!(ids(offer.models[0].efforts.as_ref()), ["default", "high"]);
        assert_eq!(offer.models[0].default_effort.as_deref(), Some("high"));
        assert_eq!(
            ids(offer.models[1].efforts.as_ref()),
            ["default", "low", "high", "max"]
        );
        assert_eq!(offer.models[2].efforts, Some(Vec::new()));
        assert_eq!(offer.models[3].efforts, None);
        assert_eq!(offer.configs.len(), 1, "the reported picker stays the one");
    }

    /// A session that listed fewer models than the harness has (a Grok
    /// session opened while its sign-in renewed lists only two fallback
    /// ones) does not hide the rest: the list is the catalogue's, in its
    /// order, and the session's model stays the one it opened on.
    #[test]
    fn a_short_session_list_does_not_hide_the_catalogues_models() {
        let mut offer = BackendOffer {
            models: vec![choice("opus", None)],
            current_model_id: Some("opus".into()),
            configs: vec![effort("high", &["low", "high"])],
            ..BackendOffer::default()
        };
        fill(&mut offer, Some(&catalog()));
        let listed: Vec<&str> = offer.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(listed, ["default", "opus", "claude-haiku-4-5"]);
        assert_eq!(offer.current_model_id.as_deref(), Some("opus"));
        assert_eq!(ids(offer.models[1].efforts.as_ref()), ["low", "high"]);
        assert_eq!(
            ids(offer.models[0].efforts.as_ref()),
            ["default", "low", "high", "max"]
        );
    }

    #[test]
    fn a_live_sessions_own_model_has_its_pickers_levels_and_the_others_the_offers() {
        let mut models = vec![choice("default", None), choice("opus", None)];
        let mut offered = catalog().models;
        offered[0].efforts = Some(vec![level("stale")]);
        with_levels(
            &mut models,
            "default",
            &[effort("low", &["default", "low"])],
            &offered,
        );
        assert_eq!(ids(models[0].efforts.as_ref()), ["default", "low"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("low"));
        assert_eq!(
            ids(models[1].efforts.as_ref()),
            ["default", "low", "high", "max"]
        );
    }

    /// At rest the picker is the teammate's model's: the teammate's level
    /// when the model has it, else the model's default; a model with no
    /// levels has no picker, and one whose levels are unknown keeps it.
    #[test]
    fn a_resting_teammates_picker_is_its_models() {
        let models = vec![
            choice("opus", Some(&["medium", "high", "max"])),
            choice("haiku", Some(&[])),
            choice("cursor-model", None),
        ];
        let chosen = "max".to_string();
        let mut configs = vec![effort("low", &["low"])];
        fit_effort(&mut configs, &models, Some("opus"), Some(&chosen));
        assert_eq!(configs[0].current_id.as_deref(), Some("max"));
        assert_eq!(ids(Some(&configs[0].options)), ["medium", "high", "max"]);
        let other = "ultra".to_string();
        fit_effort(&mut configs, &models, Some("opus"), Some(&other));
        assert_eq!(configs[0].current_id.as_deref(), Some("medium"));
        let mut unknown = vec![effort("low", &["low"])];
        fit_effort(&mut unknown, &models, Some("cursor-model"), None);
        assert_eq!(unknown, vec![effort("low", &["low"])]);
        fit_effort(&mut configs, &models, Some("haiku"), None);
        assert_eq!(configs, Vec::<SessionConfig>::new());
    }
}
