//! Modes a harness has that its ACP session does not list. Cursor runs
//! every command without asking ("Run Everything") when its CLI is started
//! with `--force`, which is a choice of launch, not one `session/set_mode`
//! can make: its session lists Agent, Plan and Ask. Hotline offers it beside
//! them, starts the agent with the flag for a teammate in it, and starts the
//! session again when a live teammate moves into it or out of it. Cursor
//! still keeps to the deny list and to a team that forbids it.

use super::registry::Launch;
use crate::contract::ConfigChoice;

/// The mode id Hotline gives Cursor's Run Everything.
pub const RUN_EVERYTHING: &str = "run-everything";

const CURSOR: &str = "cursor";

/// The session mode Run Everything is: Cursor's Agent, started with `--force`.
pub(crate) const CURSOR_AGENT: &str = "agent";

/// Adds the modes the agent has outside its session to the ones it listed,
/// once each. An agent that lists no modes is left with none.
pub(crate) fn add(backend_id: &str, modes: &mut Vec<ConfigChoice>) {
    if backend_id != CURSOR
        || modes.is_empty()
        || modes.iter().any(|mode| mode.id == RUN_EVERYTHING)
    {
        return;
    }
    modes.push(ConfigChoice {
        id: RUN_EVERYTHING.to_string(),
        name: "Run Everything".to_string(),
        description: Some(
            "Runs every command without asking, unless Cursor's deny list or team says no."
                .to_string(),
        ),
        group: None,
        account: None,
        efforts: None,
        default_effort: None,
        fast: None,
    });
}

/// Whether moving into or out of this mode starts the agent again.
pub fn relaunches(backend_id: &str, mode_id: &str) -> bool {
    backend_id == CURSOR && mode_id == RUN_EVERYTHING
}

/// The launch for a teammate in `mode_id`: Cursor in Run Everything has
/// `--force` before its subcommand, where its CLI reads it.
pub(crate) fn launch_for(backend_id: &str, mode_id: Option<&str>, mut launch: Launch) -> Launch {
    if mode_id.is_some_and(|mode| relaunches(backend_id, mode))
        && !launch.args.iter().any(|arg| arg == "--force")
    {
        launch.args.insert(0, "--force".to_string());
    }
    launch
}

/// The mode a session started with `launch` is in, when the launch says it
/// rather than the session.
pub(crate) fn launched_in(backend_id: &str, launch: Option<&Launch>) -> Option<&'static str> {
    (backend_id == CURSOR
        && launch.is_some_and(|launch| launch.args.iter().any(|arg| arg == "--force")))
    .then_some(RUN_EVERYTHING)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str) -> ConfigChoice {
        ConfigChoice {
            id: id.into(),
            name: id.into(),
            description: None,
            group: None,
            account: None,
            efforts: None,
            default_effort: None,
            fast: None,
        }
    }

    fn cursor() -> Launch {
        Launch {
            command: "/usr/local/bin/cursor-agent".into(),
            args: vec!["acp".into()],
            env: Vec::new(),
        }
    }

    #[test]
    fn cursor_offers_run_everything_beside_its_own_modes_once() {
        let mut modes = vec![mode("agent"), mode("plan"), mode("ask")];
        add("cursor", &mut modes);
        add("cursor", &mut modes);
        let ids: Vec<&str> = modes.iter().map(|one| one.id.as_str()).collect();
        assert_eq!(ids, ["agent", "plan", "ask", RUN_EVERYTHING]);
        assert_eq!(modes[3].name, "Run Everything");
        // Another agent, or one that lists no modes, gets nothing.
        let mut claude = vec![mode("default")];
        add("claude-acp", &mut claude);
        assert_eq!(claude.len(), 1);
        let mut none = Vec::new();
        add("cursor", &mut none);
        assert_eq!(none, []);
    }

    #[test]
    fn run_everything_is_cursor_started_with_force_on_its_agent_mode() {
        let forced = launch_for("cursor", Some(RUN_EVERYTHING), cursor());
        assert_eq!(forced.args, ["--force", "acp"]);
        assert_eq!(
            launch_for("cursor", Some(RUN_EVERYTHING), forced.clone()).args,
            ["--force", "acp"]
        );
        assert_eq!(launch_for("cursor", Some("plan"), cursor()).args, ["acp"]);
        assert_eq!(
            launch_for("claude-acp", Some(RUN_EVERYTHING), cursor()).args,
            ["acp"]
        );
        assert_eq!(launched_in("cursor", Some(&forced)), Some(RUN_EVERYTHING));
        assert_eq!(launched_in("cursor", Some(&cursor())), None);
        assert!(relaunches("cursor", RUN_EVERYTHING));
        assert!(!relaunches("cursor", "agent"));
    }
}
