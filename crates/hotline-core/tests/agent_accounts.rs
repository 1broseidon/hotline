use crate::common::{ANSWER, Client, acp_fixture, open_desk, until};
use hotline_core::desk::Desk;
use hotline_core::log::{Log, StreamId};
use hotline_core::wire::Door;
use hotline_core::{paths, room};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::OwnedRwLockWriteGuard;
use tokio::task::JoinHandle;

const TOKEN: &str = "disposable-agent-accounts-test";
const CASE: &str = "HOTLINE_TEST_AGENT_ACCOUNTS";
const BACKEND: &str = "claude-acp";
const INHERITED: [(&str, &str); 6] = [
    ("ANTHROPIC_API_KEY", "fake-api-key"),
    ("ANTHROPIC_AUTH_TOKEN", "fake-auth-token"),
    ("CLAUDE_CODE_OAUTH_TOKEN", "fake-oauth-token"),
    ("ANTHROPIC_BASE_URL", "https://inherited.example.invalid"),
    ("ANTHROPIC_CUSTOM_HEADERS", "X-Inherited: fake"),
    ("CLAUDE_CODE_USE_BEDROCK", "1"),
];

async fn isolated(name: &str) -> Option<PathBuf> {
    if let Some(sentinel) = std::env::var_os(CASE) {
        for (variable, fake) in INHERITED {
            assert!(
                std::env::var(variable).as_deref() == Ok(fake),
                "{variable} is not the harness's fake"
            );
        }
        return Some(PathBuf::from(sentinel));
    }
    let shims = tempfile::tempdir().unwrap();
    let sentinel = tempfile::tempdir().unwrap();
    for shim in ["npx", "claude"] {
        let path = shims
            .path()
            .join(format!("{shim}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(acp_fixture::built(), path).unwrap();
    }
    let old = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(shims.path().to_path_buf()).chain(std::env::split_paths(&old)),
    )
    .unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &format!("agent_accounts::{name}"), "--nocapture"])
        .env(CASE, sentinel.path())
        .env("CLAUDE_CONFIG_DIR", sentinel.path())
        .envs(INHERITED)
        .env("PATH", path)
        .env_remove("FIXTURE_LOGIN_MODE")
        .env_remove("FIXTURE_AUTH")
        .env_remove("FIXTURE_STATUS_MODE")
        .kill_on_drop(true);
    let output = tokio::time::timeout(ANSWER * 5, command.output())
        .await
        .unwrap_or_else(|_| panic!("{name} did not finish within {:?}", ANSWER * 5))
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "{name}:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    None
}

#[derive(Debug)]
struct Report {
    env: BTreeMap<String, Option<String>>,
    child: BTreeMap<String, Option<String>>,
}

impl Report {
    fn parsed(text: &str) -> Report {
        let mut report = Report {
            env: BTreeMap::new(),
            child: BTreeMap::new(),
        };
        for line in text.lines() {
            let (into, said) = if let Some(said) = line.strip_prefix("ENV ") {
                (&mut report.env, said)
            } else if let Some(said) = line.strip_prefix("CHILD_ENV ") {
                (&mut report.child, said)
            } else {
                continue;
            };
            match said.split_once('=') {
                Some((name, value)) => into.insert(name.to_string(), Some(value.to_string())),
                None => into.insert(said.trim_end_matches(" unset").to_string(), None),
            };
        }
        report
    }
}

fn folder(root: &Path, id: &str) -> PathBuf {
    dunce::canonicalize(paths::agent_account_dir(root, id).unwrap()).unwrap()
}

fn same(reported: Option<&str>, dir: &Path) -> bool {
    let wanted = dunce::canonicalize(dir).unwrap();
    reported.is_some_and(|path| dunce::canonicalize(path).is_ok_and(|path| path == wanted))
}

struct TestDesk {
    desk: Arc<Desk>,
    client: Client,
    log: Log,
    root: PathBuf,
    door: JoinHandle<io::Result<()>>,
    started: Vec<String>,
}

impl TestDesk {
    async fn open(root: &Path) -> TestDesk {
        acp_fixture::install(root);
        let desk = Arc::new(open_desk(root).unwrap());
        let log = desk.log.clone();
        let parts = desk.parts();
        let door = Door::bind(log.clone(), TOKEN.to_string(), desk.clone(), parts).unwrap();
        let port = door.port();
        let door = tokio::spawn(door.run());
        TestDesk {
            client: Client::connect(port, TOKEN).await,
            desk,
            log,
            root: root.to_path_buf(),
            door,
            started: Vec::new(),
        }
    }

    async fn ok(&mut self, cmd: &str, params: Value) -> Value {
        let answered = self.client.call(cmd, params).await;
        assert_eq!(answered["ok"], true, "{cmd}: {answered}");
        answered["result"].clone()
    }

    async fn account(&mut self, name: &str) -> String {
        let created = self
            .ok(
                "accounts.create",
                json!({"backendId": BACKEND, "name": name, "startWith": "empty"}),
            )
            .await;
        created["account"]["account"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn teammate(&mut self, name: &str, account: Option<&str>) -> String {
        let cwd = self.root.join("work").join(name);
        std::fs::create_dir_all(&cwd).unwrap();
        let mut draft = json!({
            "name": name, "goal": "Say what it sees.", "cwd": cwd.to_string_lossy(), "backendId": BACKEND,
        });
        if let Some(account) = account {
            draft["accountId"] = json!(account);
        }
        let created = self.ok("persona.create", json!({"draft": draft})).await;
        created["id"].as_str().unwrap().to_owned()
    }

    async fn start(&mut self, teammates: &[&str]) {
        let mut sent = Vec::new();
        for persona_id in teammates {
            let id = self
                .client
                .send("session.start", json!({"personaId": persona_id}))
                .await;
            sent.push(id);
        }
        for id in sent {
            let started = self.client.reply(id, ANSWER).await;
            assert!(
                started["ok"] == true && started["result"]["state"] != "error",
                "{started}"
            );
        }
        self.started
            .extend(teammates.iter().map(ToString::to_string));
    }

    async fn report<const N: usize>(&mut self, teammates: [&str; N]) -> [Report; N] {
        let names: Vec<&str> = std::iter::once("CLAUDE_CONFIG_DIR")
            .chain(INHERITED.iter().map(|(name, _)| *name))
            .collect();
        let text = format!("REPORT_ENV {}", names.join(" "));
        let mut sent = Vec::new();
        for persona_id in teammates {
            let id = self
                .client
                .send(
                    "session.prompt",
                    json!({"personaId": persona_id, "text": text}),
                )
                .await;
            sent.push(id);
        }
        for id in sent {
            let prompted = self.client.reply(id, ANSWER).await;
            assert_eq!(prompted["ok"], true, "{prompted}");
        }
        let mut reports = Vec::new();
        for persona_id in teammates {
            let name = self.name_of(persona_id);
            until(&format!("{name} reports its environment"), || {
                self.reported(persona_id).is_some()
            })
            .await;
            reports.push(Report::parsed(&self.reported(persona_id).unwrap()));
        }
        reports.try_into().unwrap()
    }

    async fn listed(&mut self, account_id: &str) -> Option<Value> {
        let listed = self.ok("accounts.list", json!({})).await;
        listed["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|view| view["account"]["id"] == account_id)
            .cloned()
    }

    fn name_of(&self, persona_id: &str) -> String {
        room::roster(&self.log)
            .into_iter()
            .find(|persona| persona.id == persona_id)
            .unwrap()
            .name
    }

    fn reported(&self, persona_id: &str) -> Option<String> {
        self.log
            .load(&StreamId::Tape(persona_id.to_string()))
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .find(|text| text.starts_with("ENV "))
    }

    async fn close(self) -> OwnedRwLockWriteGuard<()> {
        let TestDesk {
            desk,
            mut client,
            door,
            started,
            ..
        } = self;
        for persona_id in started {
            let stopped = client
                .call("session.stop", json!({"personaId": persona_id}))
                .await;
            assert_eq!(stopped["ok"], true, "{stopped}");
        }
        let mut held = None;
        until("the desk is idle with every stream synced", || {
            held = desk.prepare_restart().ok();
            held.is_some()
        })
        .await;
        door.abort();
        drop(client);
        drop(desk);
        held.unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_and_the_system_default_start_at_once_each_in_its_own_folder() {
    let Some(sentinel) =
        isolated("two_accounts_and_the_system_default_start_at_once_each_in_its_own_folder").await
    else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let b = desk.account("Account 2").await;
    let ada = desk.teammate("Ada", Some(&a)).await;
    let bob = desk.teammate("Bob", Some(&b)).await;
    let cy = desk.teammate("Cy", None).await;
    desk.start(&[&ada, &bob, &cy]).await;
    let [on_a, on_b, on_default] = desk.report([&ada, &bob, &cy]).await;
    assert!(
        same(on_a.env["CLAUDE_CONFIG_DIR"].as_deref(), &folder(root, &a))
            && same(on_b.env["CLAUDE_CONFIG_DIR"].as_deref(), &folder(root, &b)),
        "{on_a:?}\n{on_b:?}"
    );
    assert_ne!(folder(root, &a), folder(root, &b));
    assert!(
        same(on_default.env["CLAUDE_CONFIG_DIR"].as_deref(), &sentinel),
        "{on_default:?}"
    );
    for (key, value) in INHERITED {
        assert_eq!(on_a.env[key], None, "{key}");
        assert_eq!(on_b.env[key], None, "{key}");
        assert_eq!(on_default.env[key].as_deref(), Some(value), "{key}");
    }
    assert_eq!(
        std::env::var_os("CLAUDE_CONFIG_DIR"),
        Some(sentinel.into_os_string())
    );
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_teammates_subprocesses_see_the_same_environment_as_the_agent() {
    if isolated("an_account_teammates_subprocesses_see_the_same_environment_as_the_agent")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let ada = desk.teammate("Ada", Some(&a)).await;
    let cy = desk.teammate("Cy", None).await;
    desk.start(&[&ada, &cy]).await;
    let [on_a, on_default] = desk.report([&ada, &cy]).await;
    assert_eq!(on_a.child, on_a.env);
    assert_eq!(on_default.child, on_default.env);
    assert!(
        same(
            on_a.child["CLAUDE_CONFIG_DIR"].as_deref(),
            &folder(root, &a)
        ),
        "{on_a:?}"
    );
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_choice_survives_reopening_the_desk() {
    if isolated("an_account_choice_survives_reopening_the_desk")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let ada = desk.teammate("Ada", None).await;
    desk.start(&[&ada]).await;
    desk.ok("persona.account", json!({"personaId": ada, "accountId": a}))
        .await;
    let _closed = desk.close().await;

    let mut desk = TestDesk::open(root).await;
    let listed = desk.listed(&a).await.unwrap();
    assert_eq!(listed["usedBy"], json!([ada]), "{listed}");
    desk.start(&[&ada]).await;
    let [on_a] = desk.report([&ada]).await;
    assert!(
        same(on_a.env["CLAUDE_CONFIG_DIR"].as_deref(), &folder(root, &a)),
        "{on_a:?}"
    );
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_unused_account_keeps_its_folder() {
    if isolated("removing_an_unused_account_keeps_its_folder")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let credentials = folder(root, &a).join(".credentials.json");
    std::fs::write(&credentials, "left by a login").unwrap();
    desk.ok("accounts.remove", json!({"accountId": a})).await;
    assert_eq!(desk.listed(&a).await, None);
    assert!(paths::agent_account_dir(root, &a).unwrap().is_dir());
    assert_eq!(std::fs::read(&credentials).unwrap(), b"left by a login");
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_account_in_use_is_refused_naming_its_teammates() {
    if isolated("removing_an_account_in_use_is_refused_naming_its_teammates")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let ada = desk.teammate("Ada", Some(&a)).await;
    let bob = desk.teammate("Bob", Some(&a)).await;
    let refused = desk
        .client
        .call("accounts.remove", json!({"accountId": a}))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    let error = refused["error"].as_str().unwrap();
    assert!(
        error.starts_with("Account 1 can't be removed while ")
            && error.contains("Ada")
            && error.contains("Bob")
            && error.ends_with(" use it."),
        "{error}"
    );
    assert!(paths::agent_account_dir(root, &a).unwrap().is_dir());
    for persona_id in [&ada, &bob] {
        desk.ok(
            "persona.account",
            json!({"personaId": persona_id, "accountId": null}),
        )
        .await;
    }
    desk.ok("accounts.remove", json!({"accountId": a})).await;
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

fn leaks_nothing(error: &str, root: &Path) -> bool {
    let scratch = root.file_name().unwrap().to_string_lossy();
    !error.contains(&*root.to_string_lossy())
        && !error.contains(&*scratch)
        && !error.contains("agent-accounts")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_teammate_set_to_an_account_not_on_this_desk_is_refused_and_never_falls_back() {
    if isolated("a_teammate_set_to_an_account_not_on_this_desk_is_refused_and_never_falls_back")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let ada = desk.teammate("Ada", Some(&a)).await;
    let _closed = desk.close().await;
    room::write_agent_accounts(&Log::open(root), &[]).unwrap();
    std::fs::remove_dir_all(folder(root, &a)).unwrap();

    let mut desk = TestDesk::open(root).await;
    let started = desk
        .client
        .call("session.start", json!({"personaId": ada}))
        .await;
    assert_eq!(started["ok"], false, "{started}");
    let error = started["error"].as_str().unwrap();
    assert!(
        error.contains(
            "Pick a Claude Code account for Ada: the one it was set to is not on this desk."
        ) && error.contains(r#""kind":"account_unavailable""#)
            && error.contains(r#""code":"unknown_account""#)
            && leaks_nothing(error, root),
        "{error}"
    );
    let prompted = desk
        .client
        .call(
            "session.prompt",
            json!({"personaId": ada, "text": "REPORT_ENV CLAUDE_CONFIG_DIR"}),
        )
        .await;
    assert_eq!(prompted["ok"], false, "{prompted}");
    assert_eq!(prompted["error"], "That teammate is not running.");
    assert!(leaks_nothing(prompted["error"].as_str().unwrap(), root));
    assert_eq!(desk.reported(&ada), None);
    assert!(!paths::agent_account_dir(root, &a).unwrap().exists());
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_under_a_data_directory_with_spaces_and_accents_runs() {
    if isolated("an_account_under_a_data_directory_with_spaces_and_accents_runs")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("Conta Teste ção");
    std::fs::create_dir(&root).unwrap();
    let mut desk = TestDesk::open(&root).await;
    let a = desk.account("Account 1").await;
    let ada = desk.teammate("Ada", Some(&a)).await;
    desk.start(&[&ada]).await;
    let [on_a] = desk.report([&ada]).await;
    let reported = on_a.env["CLAUDE_CONFIG_DIR"].as_deref();
    assert!(
        same(reported, &folder(&root, &a))
            && reported.is_some_and(|dir| dir.contains("Conta Teste ção")),
        "{on_a:?}"
    );
    std::fs::write(
        folder(&root, &a).join(".credentials.json"),
        r#"{"email":"ada@example.invalid"}"#,
    )
    .unwrap();
    let status = desk.ok("accounts.status", json!({"accountId": a})).await;
    assert_eq!(status["state"], "signed_in", "{status}");
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}

#[tokio::test(flavor = "multi_thread")]
async fn status_says_signed_in_only_for_the_account_the_agent_confirms() {
    if isolated("status_says_signed_in_only_for_the_account_the_agent_confirms")
        .await
        .is_none()
    {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let mut desk = TestDesk::open(root).await;
    let a = desk.account("Account 1").await;
    let b = desk.account("Account 2").await;
    std::fs::write(
        folder(root, &a).join(".credentials.json"),
        r#"{"email":"ada@example.invalid"}"#,
    )
    .unwrap();
    let on_a = desk.ok("accounts.status", json!({"accountId": a})).await;
    let on_b = desk.ok("accounts.status", json!({"accountId": b})).await;
    assert_eq!(on_a["state"], "signed_in", "{on_a}");
    assert_eq!(on_b["state"], "signed_out", "{on_b}");
    for status in [&on_a, &on_b] {
        assert!(
            status
                .as_object()
                .unwrap()
                .keys()
                .all(|key| ["state", "checkedAt", "email", "plan"].contains(&key.as_str())),
            "{status}"
        );
    }
    let listed = desk.listed(&a).await.unwrap();
    assert_eq!(listed["status"]["state"], "signed_in", "{listed}");
    let seen = std::fs::read_to_string(folder(root, &a).join("fixture-status-env.json")).unwrap();
    assert!(seen.contains(r#""CLAUDE_CONFIG_DIR""#), "{seen}");
    for (key, _) in INHERITED {
        assert!(!seen.contains(&format!("\"{key}\"")), "{key}: {seen}");
    }
    drop(desk.close().await);
    hotline_core::driver::acp::end_every_agent();
}
