use super::*;
use crate::driver::acp::registry::{Launch, mark_started};
use std::collections::{HashMap, HashSet};
use tokio::sync::Semaphore;

/// A machine whose commands answer from a script: what `which` finds, where
/// links lead, which files exist, and what each command line says.
#[derive(Default)]
pub(crate) struct Scripted {
    paths: HashMap<String, PathBuf>,
    links: HashMap<PathBuf, PathBuf>,
    files: Mutex<HashSet<PathBuf>>,
    answers: Mutex<HashMap<String, Result<String, String>>>,
    /// Answers that take over once an update has run, by command line.
    after_update: Mutex<HashMap<String, Result<String, String>>>,
    ran: Mutex<Vec<String>>,
    /// Held before any update step runs, when set.
    gate: Option<Arc<Semaphore>>,
}

impl Scripted {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn found(mut self, command: &str, at: &str) -> Self {
        self.paths.insert(command.into(), at.into());
        self
    }
    pub(crate) fn link(mut self, from: &str, to: &str) -> Self {
        self.links.insert(from.into(), to.into());
        self
    }
    pub(crate) fn file(self, path: &str) -> Self {
        lock(&self.files).insert(path.into());
        self
    }
    pub(crate) fn says(self, line: &str, said: Result<&str, &str>) -> Self {
        lock(&self.answers).insert(
            line.into(),
            said.map(str::to_string).map_err(str::to_string),
        );
        self
    }
    pub(crate) fn then_says(self, line: &str, said: &str) -> Self {
        lock(&self.after_update).insert(line.into(), Ok(said.into()));
        self
    }
    fn gated(mut self, gate: Arc<Semaphore>) -> Self {
        self.gate = Some(gate);
        self
    }
    pub(crate) fn ran(&self) -> Vec<String> {
        lock(&self.ran).clone()
    }
}

/// The command as the script names it: `/` between the parts of its path on
/// every platform, as the scripts write them, though Windows joins with `\`.
fn line(program: &Path, args: &[String]) -> String {
    std::iter::once(program.display().to_string().replace('\\', "/"))
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn updating(args: &[String]) -> bool {
    args.first()
        .is_some_and(|word| matches!(word.as_str(), "install" | "upgrade" | "update" | "cache"))
}

#[async_trait]
impl World for Scripted {
    fn which(&self, command: &str) -> Option<PathBuf> {
        self.paths.get(command).cloned()
    }
    fn real_path(&self, path: &Path) -> PathBuf {
        self.links
            .get(path)
            .cloned()
            .unwrap_or_else(|| path.to_path_buf())
    }
    fn exists(&self, path: &Path) -> bool {
        lock(&self.files).contains(path)
    }
    async fn run(&self, program: &Path, args: &[String], _: Duration) -> Result<String, String> {
        let line = line(program, args);
        if updating(args)
            && let Some(gate) = &self.gate
        {
            gate.acquire().await.expect("the gate closed").forget();
        }
        lock(&self.ran).push(line.clone());
        if updating(args) {
            let after = std::mem::take(&mut *lock(&self.after_update));
            lock(&self.answers).extend(after);
        }
        lock(&self.answers)
            .get(&line)
            .cloned()
            .unwrap_or_else(|| Err(format!("{line}: not scripted")))
    }
    async fn refresh_catalogue(&self, _: &Path) {}
    async fn fetch_release(&self, _: &Path, _: &str) -> Result<(), String> {
        lock(&self.ran).push("fetch release".into());
        Ok(())
    }
}

fn part(kind: Kind, installed: Option<&str>, latest: Option<&str>, step: bool) -> Part {
    let behind = matches!((latest, installed), (Some(l), Some(i)) if newer(l, i));
    Part {
        kind,
        installed: installed.map(str::to_string),
        latest: latest.map(str::to_string),
        behind,
        step: step.then(|| Step::Run {
            program: "/bin/x".into(),
            args: vec![],
        }),
    }
}

// -- versions -----------------------------------------------------------------

#[test]
fn a_version_is_read_from_what_each_cli_says() {
    assert_eq!(version_in("2.1.3 (Claude Code)").as_deref(), Some("2.1.3"));
    assert_eq!(version_in("codex-cli 0.5.0\n").as_deref(), Some("0.5.0"));
    assert_eq!(version_in("v1.4.0").as_deref(), Some("1.4.0"));
    assert_eq!(
        version_in("2026.10.31-a1b2c3").as_deref(),
        Some("2026.10.31-a1b2c3")
    );
    assert_eq!(version_in("cursor-agent ready"), None);
    assert!(newer("2.1.10", "2.1.9"));
    assert!(!newer("2.1.9", "2.1.9"));
    assert!(!newer("1.0.0", "1.0.1"));
    assert!(!newer("unknown", "1.0.0"));
}

// -- folding ------------------------------------------------------------------

#[test]
fn a_harness_whose_cli_is_behind_shows_the_clis_versions() {
    let folded = fold(&[
        part(Kind::Cli, Some("2.1.3"), Some("2.1.5"), true),
        part(Kind::Adapter, Some("0.78.0"), Some("0.78.0"), false),
    ]);
    assert_eq!(folded.state, HarnessUpdateState::Available);
    assert_eq!(folded.version.as_deref(), Some("2.1.3"));
    assert_eq!(folded.latest.as_deref(), Some("2.1.5"));
    assert!(folded.can_update);
}

#[test]
fn a_harness_whose_adapter_alone_is_behind_is_behind_without_naming_it() {
    let folded = fold(&[
        part(Kind::Cli, Some("2.1.5"), Some("2.1.5"), true),
        part(Kind::Adapter, Some("0.78.0"), Some("0.80.1"), true),
    ]);
    assert_eq!(folded.state, HarnessUpdateState::Available);
    assert_eq!(folded.version.as_deref(), Some("2.1.5"), "known by its CLI");
    assert_eq!(
        folded.latest, None,
        "the adapter's version is not the person's"
    );
    assert!(folded.can_update);
}

#[test]
fn a_harness_with_both_parts_behind_is_one_update() {
    let folded = fold(&[
        part(Kind::Cli, Some("0.4.0"), Some("0.5.0"), true),
        part(Kind::Adapter, Some("1.11.0"), Some("1.12.0"), true),
    ]);
    assert_eq!(folded.state, HarnessUpdateState::Available);
    assert_eq!(folded.latest.as_deref(), Some("0.5.0"));
}

#[test]
fn a_harness_with_nothing_behind_is_up_to_date() {
    let folded = fold(&[
        part(Kind::Cli, Some("2.1.5"), Some("2.1.5"), true),
        part(Kind::Adapter, Some("0.80.1"), Some("0.80.1"), false),
    ]);
    assert_eq!(folded.state, HarnessUpdateState::UpToDate);
    assert!(!folded.can_update);
}

#[test]
fn an_unreadable_latest_is_unknown_and_updatable_only_by_the_clis_own_updater() {
    let own = fold(&[part(Kind::Cli, Some("2026.10.1"), None, true)]);
    assert_eq!(own.state, HarnessUpdateState::Unknown);
    assert!(own.can_update, "cursor-agent update knows what is newest");
    let none = fold(&[part(Kind::Cli, Some("1.0.0"), None, false)]);
    assert_eq!(none.state, HarnessUpdateState::Unknown);
    assert!(!none.can_update);
}

// -- how a CLI was installed --------------------------------------------------

#[test]
fn a_cli_under_an_npm_prefix_is_that_prefixs_package() {
    let world = Scripted::new().file("/opt/node/bin/npm");
    let method = method_for(
        &world,
        "claude",
        Path::new("/opt/node/bin/claude"),
        Path::new("/opt/node/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
    );
    assert_eq!(
        method,
        Method::Npm {
            npm: "/opt/node/bin/npm".into(),
            package: "@anthropic-ai/claude-code".into(),
        }
    );
    let world = Scripted::new().file("/opt/node/bin/npm");
    let plain = method_for(
        &world,
        "opencode",
        Path::new("/opt/node/bin/opencode"),
        Path::new("/opt/node/lib/node_modules/opencode-ai/bin/opencode"),
    );
    assert!(matches!(plain, Method::Npm { package, .. } if package == "opencode-ai"));
}

#[test]
fn a_cli_in_a_homebrew_cellar_or_caskroom_is_brews() {
    let world = Scripted::new().file("/opt/homebrew/bin/brew");
    assert_eq!(
        method_for(
            &world,
            "gemini",
            Path::new("/opt/homebrew/bin/gemini"),
            Path::new("/opt/homebrew/Cellar/gemini-cli/0.9.0/bin/gemini"),
        ),
        Method::Brew {
            brew: "/opt/homebrew/bin/brew".into(),
            name: "gemini-cli".into(),
            cask: false,
        }
    );
    assert!(matches!(
        method_for(
            &world,
            "codex",
            Path::new("/opt/homebrew/bin/codex"),
            Path::new("/opt/homebrew/Caskroom/codex/0.5.0/codex"),
        ),
        Method::Brew { cask: true, .. }
    ));
}

#[test]
fn a_cli_installed_its_own_way_is_updated_by_itself_or_not_at_all() {
    let world = Scripted::new();
    assert_eq!(
        method_for(
            &world,
            "claude",
            Path::new("/home/me/.local/bin/claude"),
            Path::new("/home/me/.local/share/claude/versions/2.1.3"),
        ),
        Method::Own {
            command: "/home/me/.local/bin/claude".into(),
            args: vec!["update".into()],
        }
    );
    assert_eq!(
        method_for(
            &world,
            "grok",
            Path::new("/usr/local/bin/grok"),
            Path::new("/usr/local/bin/grok")
        ),
        Method::Unknown,
        "no package manager is guessed"
    );
}

#[test]
fn an_npm_package_whose_npm_is_not_beside_it_is_not_given_another() {
    let world = Scripted::new().file("/usr/bin/npm");
    assert_eq!(
        method_for(
            &world,
            "codex",
            Path::new("/opt/node/bin/codex"),
            Path::new("/opt/node/lib/node_modules/@openai/codex/bin/codex.js"),
        ),
        Method::Unknown
    );
}

#[tokio::test]
async fn a_mise_shim_is_followed_to_the_cli_it_stands_for() {
    let world = Scripted::new()
        .found("codex", "/home/me/.local/share/mise/shims/codex")
        .found("mise", "/usr/local/bin/mise")
        .link("/home/me/.local/share/mise/shims/codex", "/usr/local/bin/mise")
        .link(
            "/home/me/.local/share/mise/installs/node/26/bin/codex",
            "/home/me/.local/share/mise/installs/node/26/lib/node_modules/@openai/codex/bin/codex.js",
        )
        .file("/home/me/.local/share/mise/installs/node/26/bin/npm")
        .says(
            "/usr/local/bin/mise which codex",
            Ok("/home/me/.local/share/mise/installs/node/26/bin/codex\n"),
        )
        .says("/home/me/.local/share/mise/shims/codex --version", Ok("codex-cli 0.4.0"))
        .says(
            "/home/me/.local/share/mise/installs/node/26/bin/npm view @openai/codex version",
            Ok("0.5.0\n"),
        );
    let part = cli_part(&world, "codex").await;
    assert_eq!(part.installed.as_deref(), Some("0.4.0"));
    assert_eq!(part.latest.as_deref(), Some("0.5.0"));
    assert!(part.behind);
    assert_eq!(
        part.step,
        Some(Step::Run {
            program: "/home/me/.local/share/mise/installs/node/26/bin/npm".into(),
            args: vec!["install".into(), "-g".into(), "@openai/codex@latest".into()],
        })
    );
}

// -- adapters and releases ----------------------------------------------------

#[test]
fn an_adapter_is_behind_when_todays_pin_is_newer_than_the_last_start() {
    let root = tempfile::tempdir().unwrap();
    mark_started(
        root.path(),
        &Launch {
            command: "/opt/node/bin/npx".into(),
            args: vec![
                "-y".into(),
                "@agentclientprotocol/claude-agent-acp@0.78.0".into(),
            ],
            env: vec![],
        },
        true,
    );
    let world = Scripted::new()
        .found("/opt/node/bin/npx", "/opt/node/bin/npx")
        .file("/opt/node/bin/npm");
    let package = Package {
        spec: "@agentclientprotocol/claude-agent-acp@0.80.1".into(),
        runner: "/opt/node/bin/npx".into(),
    };
    let part = package_part(&world, root.path(), &package);
    assert_eq!(part.installed.as_deref(), Some("0.78.0"));
    assert_eq!(part.latest.as_deref(), Some("0.80.1"));
    assert!(part.behind);
    assert_eq!(
        part.step,
        Some(Step::Run {
            program: "/opt/node/bin/npm".into(),
            args: vec!["cache".into(), "add".into(), package.spec.clone()],
        }),
        "fetched now, so the next start does not wait"
    );

    let never = tempfile::tempdir().unwrap();
    let fresh = package_part(&world, never.path(), &package);
    assert!(
        !fresh.behind,
        "an adapter no start has run is fetched at its first"
    );
}

#[test]
fn a_release_is_behind_only_when_an_older_one_is_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let releases = root.path().join("agents").join("kimi");
    let today = releases.join("new");
    let release = Release {
        command: today.join("kimi"),
        dir: today.clone(),
    };
    let world = Scripted::new();
    assert!(
        !release_part(&world, &release).behind,
        "nothing fetched yet"
    );
    std::fs::create_dir_all(releases.join("old")).unwrap();
    let behind = release_part(&world, &release);
    assert!(behind.behind);
    assert_eq!(behind.step, Some(Step::FetchRelease));
}

// -- the state of each harness --------------------------------------------------

fn claude_world() -> Scripted {
    Scripted::new()
        .found("claude", "/opt/node/bin/claude")
        .link(
            "/opt/node/bin/claude",
            "/opt/node/lib/node_modules/@anthropic-ai/claude-code/cli.js",
        )
        .file("/opt/node/bin/npm")
        .says("/opt/node/bin/claude --version", Ok("2.1.3 (Claude Code)"))
        .says(
            "/opt/node/bin/npm view @anthropic-ai/claude-code version",
            Ok("2.1.5"),
        )
        .says(
            "/opt/node/bin/npm install -g @anthropic-ai/claude-code@latest",
            Ok("changed 1 package"),
        )
        .then_says("/opt/node/bin/claude --version", "2.1.5 (Claude Code)")
}

#[tokio::test]
async fn an_update_runs_the_install_checks_again_and_says_when_it_takes_effect() {
    let root = tempfile::tempdir().unwrap();
    let world = Arc::new(claude_world());
    let updates = Updates::new(world.clone());
    updates.seed(vec![harness_for_tests("claude-acp", Some("claude"), None)]);
    let checked = updates.check_seeded(root.path(), 1).await;
    assert_eq!(checked.harnesses[0].state, HarnessUpdateState::Available);
    assert_eq!(checked.harnesses[0].version.as_deref(), Some("2.1.3"));
    assert_eq!(checked.harnesses[0].latest.as_deref(), Some("2.1.5"));

    let done = updates.update(root.path(), "claude-acp").await.unwrap();
    assert!(
        world
            .ran()
            .contains(&"/opt/node/bin/npm install -g @anthropic-ai/claude-code@latest".to_string())
    );
    assert_eq!(done.state, HarnessUpdateState::Updated);
    assert_eq!(done.version.as_deref(), Some("2.1.5"));
    assert!(!done.can_update);
    assert_eq!(
        done.detail.as_deref(),
        Some("claude-acp is 2.1.5. A teammate on it gets it at its next start.")
    );
}

#[tokio::test]
async fn an_update_that_fails_says_why_and_can_be_tried_again() {
    let root = tempfile::tempdir().unwrap();
    let world = Arc::new(claude_world().says(
        "/opt/node/bin/npm install -g @anthropic-ai/claude-code@latest",
        Err("EACCES: permission denied, rename '/opt/node/lib/node_modules/@anthropic-ai'"),
    ));
    lock(&world.after_update).clear();
    let updates = Updates::new(world.clone());
    updates.seed(vec![harness_for_tests("claude-acp", Some("claude"), None)]);
    updates.check_seeded(root.path(), 1).await;
    let failed = updates.update(root.path(), "claude-acp").await.unwrap();
    assert_eq!(failed.state, HarnessUpdateState::Failed);
    assert_eq!(
        failed.detail.as_deref(),
        Some(
            "Updating claude-acp failed: EACCES: permission denied, rename '/opt/node/lib/node_modules/@anthropic-ai'."
        )
    );
    assert!(failed.can_update, "still behind, so the button stays");
}

#[tokio::test]
async fn an_update_is_refused_while_one_runs_and_for_a_harness_with_nothing_to_do() {
    let root = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let world = Arc::new(claude_world().gated(gate.clone()));
    let updates = Arc::new(Updates::new(world.clone()));
    updates.seed(vec![
        harness_for_tests("claude-acp", Some("claude"), None),
        harness_for_tests("missing", Some("nothing-here"), None),
    ]);
    updates.check_seeded(root.path(), 1).await;

    let running = tokio::spawn({
        let updates = updates.clone();
        let root = root.path().to_path_buf();
        async move { updates.update(&root, "claude-acp").await }
    });
    crate::test_support::until("the update is under way", || {
        updates.view().harnesses[0].state == HarnessUpdateState::Updating
    })
    .await;
    assert!(!updates.view().harnesses[0].can_update);
    assert_eq!(
        updates.update(root.path(), "claude-acp").await.unwrap_err(),
        "claude-acp is already being updated."
    );
    gate.add_permits(1);
    assert_eq!(
        running.await.unwrap().unwrap().state,
        HarnessUpdateState::Updated
    );

    assert_eq!(
        updates.update(root.path(), "claude-acp").await.unwrap_err(),
        "claude-acp has nothing to update."
    );
    assert_eq!(
        updates.update(root.path(), "missing").await.unwrap_err(),
        "missing has nothing to update."
    );
    assert_eq!(
        updates.update(root.path(), "nobody").await.unwrap_err(),
        "That harness is not one this desk runs. Check again."
    );
}

#[tokio::test]
async fn a_check_is_due_every_six_hours_and_not_while_one_runs() {
    let updates = Updates::new(Arc::new(Scripted::new()));
    assert!(updates.due(0), "never checked");
    let root = tempfile::tempdir().unwrap();
    updates.seed(vec![]);
    updates.check_seeded(root.path(), 1_000).await;
    assert!(!updates.due(1_000 + CHECK_EVERY_MS - 1));
    assert!(updates.due(1_000 + CHECK_EVERY_MS));
}
