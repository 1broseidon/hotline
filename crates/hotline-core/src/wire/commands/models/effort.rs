//! Which effort a model switch lands on, and where each effort chosen is
//! remembered: on the teammate for that model, and on the desk for that
//! agent's model across teammates.
//!
//! A switch lands, in this order, on the teammate's own last effort for the
//! new model; else the desk's last for it; else the current effort, when
//! the new model offers it; else the highest level the new model offers
//! that is not above the current one, never a step up; else the new
//! model's default. Auto (an agent's "Default", the level that leaves the
//! choice to the agent) is a level of its own: it carries over only to a
//! model that offers it, and is never ranked against the others, so a model
//! without it lands on its default.

use std::collections::BTreeMap;

/// The room setting holding the desk's last effort for each model, by
/// [`key`].
pub(super) const DESK_SETTING: &str = "effortByModel";

/// Effort levels by how much they think, for levels two models' own lists
/// cannot place against each other. A level not here is placed only by a
/// list that holds it.
const LADDER: [&str; 8] = crate::models::EFFORT_LADDER;

/// A model as both memories name it: the agent and the model, since two
/// agents may share a model id that means different things to each.
pub(super) fn key(backend_id: &str, model_id: &str) -> String {
    format!("{backend_id}/{model_id}")
}

/// The level that leaves the effort to the agent: Claude Code's "Default".
pub(super) fn is_auto(level: &str) -> bool {
    level.eq_ignore_ascii_case("default") || level.eq_ignore_ascii_case("auto")
}

/// What a switch knows when it picks the new model's effort.
#[derive(Debug, Default)]
pub(super) struct Switch<'a> {
    /// The new model's levels in its own order; empty when it has none.
    pub levels: &'a [String],
    /// The level a session on the new model starts at, when it says.
    pub default: Option<&'a str>,
    /// This teammate's last effort for the new model.
    pub teammate: Option<&'a str>,
    /// The desk's last effort for the new model, from any teammate.
    pub desk: Option<&'a str>,
    /// The effort the teammate runs at now.
    pub current: Option<&'a str>,
    /// The levels of the model it runs on now, in that model's order: what
    /// places the current effort against the new model's levels.
    pub current_levels: &'a [String],
}

/// The effort the switch lands on, or none when the new model has no levels.
pub(super) fn land(switch: &Switch<'_>) -> Option<String> {
    let offered = |level: &&str| switch.levels.iter().any(|one| one == level);
    if switch.levels.is_empty() {
        return None;
    }
    if let Some(level) = switch
        .teammate
        .filter(offered)
        .or(switch.desk.filter(offered))
        .or(switch.current.filter(offered))
    {
        return Some(level.to_string());
    }
    let stepped = switch
        .current
        .filter(|current| !is_auto(current))
        .and_then(|current| {
            switch
                .levels
                .iter()
                .rfind(|level| !is_auto(level) && not_above(level, current, switch.current_levels))
        });
    stepped
        .map(String::as_str)
        .or(switch.default.filter(offered))
        .or_else(|| switch.levels.first().map(String::as_str))
        .map(str::to_string)
}

/// Whether `level` thinks no more than `current`: by the current model's
/// own order when it lists both, else by [`LADDER`]. A level neither can
/// place is never taken, so a switch cannot step up by not knowing.
fn not_above(level: &str, current: &str, current_levels: &[String]) -> bool {
    let at = |list: &[&str], one: &str| list.iter().position(|each| *each == one);
    let own: Vec<&str> = current_levels
        .iter()
        .map(String::as_str)
        .filter(|one| !is_auto(one))
        .collect();
    if let (Some(level), Some(current)) = (at(&own, level), at(&own, current)) {
        return level <= current;
    }
    match (at(&LADDER, level), at(&LADDER, current)) {
        (Some(level), Some(current)) => level <= current,
        _ => false,
    }
}

/// Both memories with `level` set for `key`: the teammate's map and the
/// desk's setting, each changed only when it said something else.
pub(super) fn remembered(
    mut map: BTreeMap<String, String>,
    key: &str,
    level: &str,
) -> Option<BTreeMap<String, String>> {
    if map.get(key).map(String::as_str) == Some(level) {
        return None;
    }
    map.insert(key.to_string(), level.to_string());
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(levels: &[&str]) -> Vec<String> {
        levels.iter().map(|one| (*one).to_string()).collect()
    }

    const CLAUDE: [&str; 6] = ["default", "low", "medium", "high", "xhigh", "max"];
    const SONNET: [&str; 5] = ["default", "low", "medium", "high", "max"];
    const CODEX: [&str; 5] = ["low", "medium", "high", "xhigh", "ultra"];

    /// Each rule in turn, as a table: what the switch knows, where it lands.
    #[test]
    fn a_switch_lands_by_the_rules_in_order() {
        struct Case {
            why: &'static str,
            levels: &'static [&'static str],
            default: Option<&'static str>,
            teammate: Option<&'static str>,
            desk: Option<&'static str>,
            current: Option<&'static str>,
            current_levels: &'static [&'static str],
            lands: Option<&'static str>,
        }
        let base = Case {
            why: "",
            levels: &CLAUDE,
            default: Some("default"),
            teammate: None,
            desk: None,
            current: None,
            current_levels: &CLAUDE,
            lands: None,
        };
        let cases = [
            Case {
                why: "a: the teammate's own for the model wins over all",
                teammate: Some("low"),
                desk: Some("max"),
                current: Some("high"),
                lands: Some("low"),
                ..base
            },
            Case {
                why: "a: one the model no longer offers is passed over",
                teammate: Some("ultra"),
                desk: Some("max"),
                current: Some("high"),
                lands: Some("max"),
                ..base
            },
            Case {
                why: "b: the desk's last for the model, from any teammate",
                desk: Some("xhigh"),
                current: Some("low"),
                lands: Some("xhigh"),
                ..base
            },
            Case {
                why: "c: the current effort carries over where offered",
                current: Some("medium"),
                lands: Some("medium"),
                ..base
            },
            Case {
                why: "d: xhigh onto Sonnet clamps down to high, never up to max",
                levels: &SONNET,
                current: Some("xhigh"),
                lands: Some("high"),
                ..base
            },
            Case {
                why: "d: Codex's ultra onto Opus clamps by the ladder to max",
                current: Some("ultra"),
                current_levels: &CODEX,
                lands: Some("max"),
                ..base
            },
            Case {
                why: "d: below everything the model offers, its default",
                levels: &["medium", "high"],
                default: Some("high"),
                current: Some("low"),
                current_levels: &CODEX,
                lands: Some("high"),
                ..base
            },
            Case {
                why: "c: Auto carries over to a model with Auto",
                levels: &SONNET,
                current: Some("default"),
                lands: Some("default"),
                ..base
            },
            Case {
                why: "d: Auto onto a model without it is the model's default, not a rank",
                levels: &CODEX,
                default: Some("medium"),
                current: Some("default"),
                lands: Some("medium"),
                ..base
            },
            Case {
                why: "d: a ranked level never clamps onto Auto",
                levels: &["default", "high", "max"],
                default: Some("max"),
                current: Some("low"),
                current_levels: &CODEX,
                lands: Some("max"),
                ..base
            },
            Case {
                why: "d: a level no list or ladder places is not a step down",
                levels: &["deep", "deeper"],
                default: Some("deep"),
                current: Some("turbo"),
                current_levels: &["turbo"],
                lands: Some("deep"),
                ..base
            },
            Case {
                why: "d: the current model's own order places what the ladder cannot",
                levels: &["deep", "deeper"],
                default: Some("deep"),
                current: Some("deepest"),
                current_levels: &["deep", "deeper", "deepest"],
                lands: Some("deeper"),
                ..base
            },
            Case {
                why: "e: nothing to go on is the model's default",
                levels: &CODEX,
                default: Some("medium"),
                lands: Some("medium"),
                ..base
            },
            Case {
                why: "e: a model that names no default starts at its first level",
                levels: &CODEX,
                default: None,
                lands: Some("low"),
                ..base
            },
            Case {
                why: "a model without levels has no effort to land on",
                levels: &[],
                default: None,
                teammate: Some("high"),
                current: Some("high"),
                lands: None,
                ..base
            },
        ];
        for case in cases {
            let levels = list(case.levels);
            let current_levels = list(case.current_levels);
            let landed = land(&Switch {
                levels: &levels,
                default: case.default,
                teammate: case.teammate,
                desk: case.desk,
                current: case.current,
                current_levels: &current_levels,
            });
            assert_eq!(landed.as_deref(), case.lands, "{}", case.why);
        }
    }

    #[test]
    fn a_memory_is_written_only_when_it_changes() {
        let once = remembered(BTreeMap::new(), "claude-acp/opus", "high").unwrap();
        assert_eq!(
            once.get("claude-acp/opus").map(String::as_str),
            Some("high")
        );
        assert!(remembered(once.clone(), "claude-acp/opus", "high").is_none());
        let changed = remembered(once, "claude-acp/opus", "max").unwrap();
        assert_eq!(
            changed.get("claude-acp/opus").map(String::as_str),
            Some("max")
        );
    }
}
