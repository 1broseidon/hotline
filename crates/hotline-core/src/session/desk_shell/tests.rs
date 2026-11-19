//! What a teammate's desk shell is handed: nothing without `on: desk`, a
//! readable variable for every kind of teammate, and a protected key only
//! as a stand-in in a confined shell, proved in a real sandbox against a
//! fake service.

use super::*;
use crate::contract::{UsedWith, VariableUse};
use crate::keyproxy::tests::{API, REAL};
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
#[cfg(unix)]
use hotline_tools::{OutputTail, RunCommand, RunCommandArgs, Workspace};
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

const PLAIN: &str = "plain-readable-value-0001";

fn room_of(name: &str, teammates: &[Persona]) -> Arc<Room> {
    let log = scratch(name);
    for teammate in teammates {
        enrol(&log, teammate);
    }
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    let vault_for_room = vault.clone();
    vault.set_shared_secret("PLAIN", PLAIN).unwrap();
    vault.set_shared_secret("API_KEY", REAL).unwrap();
    vault
        .set_secret_used_with("API_KEY", Some(used_with_fake_service()))
        .unwrap();
    Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(Vec::new())),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
        Some(vault_for_room),
    )
}

/// "Used with" Other, on the fake service's host.
fn used_with_fake_service() -> VariableUse {
    crate::vault::resolve_used_with(&UsedWith {
        service: "other".into(),
        host: Some(API.into()),
    })
    .unwrap()
}

fn teammate(id: &str, backend: &str, reach: Reach) -> Persona {
    let mut teammate = persona(id);
    teammate.backend_id = backend.into();
    teammate.reach = Some(reach);
    teammate
}

fn grant(name: &str, readable: bool, on: Option<Vec<GrantPlace>>) -> SecretGrant {
    SecretGrant {
        name: name.into(),
        readable,
        on,
    }
}

/// Gives `id` exactly `grants` on the record, the way the pane would, and
/// hands its desk shell what that is.
fn give(room: &Room, id: &str, grants: Vec<SecretGrant>) {
    let mut teammate = room.persona(id).unwrap();
    teammate.secrets = Some(grants.iter().map(|grant| grant.name.clone()).collect());
    teammate.secret_grants = Some(grants);
    enrol(&room.log, &teammate);
    room.refresh_desk_shell(&room.persona(id).unwrap());
}

fn names(env: &RunEnv) -> Vec<&str> {
    env.vars.iter().map(|(name, _)| name.as_str()).collect()
}

fn value<'a>(env: &'a RunEnv, name: &str) -> Option<&'a str> {
    env.vars
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn nothing_reaches_the_desk_shell_without_on_desk() {
    let room = room_of(
        "desk-shell-none",
        &[teammate("ada", "hotline", Reach::Workspace)],
    );
    // Every grant from before the desk shell took secrets: no `on`.
    give(
        &room,
        "ada",
        vec![grant("API_KEY", false, None), grant("PLAIN", true, None)],
    );
    let env = room.desk_shell.run_env("ada", Reach::Workspace);
    assert!(env.vars.is_empty() && env.ca_pem.is_none() && env.held.is_none());
    // Its computer alone, said out loud: the same.
    let computer = Some(vec![GrantPlace::Computer]);
    give(
        &room,
        "ada",
        vec![
            grant("API_KEY", false, computer.clone()),
            grant("PLAIN", true, computer),
        ],
    );
    assert_eq!(
        room.desk_shell.run_env("ada", Reach::Workspace).vars,
        [] as [(String, String); 0]
    );
    assert!(
        lock(&room.desk_shell.proxy).is_none(),
        "no proxy was started"
    );
}

#[tokio::test]
async fn a_readable_desk_grant_reaches_every_kind_of_teammate() {
    let room = room_of(
        "desk-shell-readable",
        &[
            teammate("ada", "hotline", Reach::Workspace),
            teammate("max", "hotline", Reach::Machine),
            teammate("cleo", "claude-code", Reach::Workspace),
        ],
    );
    for id in ["ada", "max", "cleo"] {
        give(
            &room,
            id,
            vec![grant("PLAIN", true, Some(vec![GrantPlace::Desk]))],
        );
    }
    let confined = room.desk_shell.run_env("ada", Reach::Workspace);
    assert_eq!(value(&confined, "PLAIN"), Some(PLAIN));
    let machine = room.desk_shell.run_env("max", Reach::Machine);
    assert_eq!(value(&machine, "PLAIN"), Some(PLAIN));
    // What an ACP harness is started with.
    assert_eq!(
        room.desk_shell_readable("cleo"),
        vec![("PLAIN".to_owned(), PLAIN.to_owned())]
    );
    // A whole-machine command really has it, as the shell tool runs one.
    #[cfg(unix)]
    assert_eq!(
        run(&room, "max", Reach::Machine, "printf %s \"$PLAIN\"").await,
        PLAIN
    );
    // Taken back, it is gone from the next command.
    give(&room, "max", Vec::new());
    assert_eq!(
        room.desk_shell.run_env("max", Reach::Machine).vars,
        [] as [(String, String); 0]
    );
}

#[tokio::test]
async fn a_protected_desk_key_is_a_stand_in_for_a_confined_teammate_only() {
    let room = room_of(
        "desk-shell-protected",
        &[
            teammate("ada", "hotline", Reach::Workspace),
            teammate("max", "hotline", Reach::Machine),
            teammate("cleo", "claude-code", Reach::Workspace),
        ],
    );
    // As a record could say it; the pane never lets one who runs as the
    // person be given a key protected.
    for id in ["ada", "max", "cleo"] {
        give(
            &room,
            id,
            vec![grant("API_KEY", false, Some(vec![GrantPlace::Desk]))],
        );
    }
    let env = room.desk_shell.run_env("ada", Reach::Workspace);
    let stand_in = value(&env, "API_KEY").unwrap().to_owned();
    assert_ne!(stand_in, REAL);
    assert!(stand_in.starts_with("ghp_"), "{stand_in}");
    let proxy = value(&env, "HTTPS_PROXY").unwrap();
    assert!(
        proxy.starts_with("http://hl-") && proxy.contains("@127.0.0.1:"),
        "{proxy}"
    );
    assert_eq!(value(&env, "NODE_USE_ENV_PROXY"), Some("1"));
    assert!(env.ca_pem.is_some() && env.held.is_some());
    assert!(!env.vars.iter().any(|(_, value)| value.contains(REAL)));
    // The same teammate with the whole machine, and the two that run as the
    // person, are handed nothing for it.
    for (id, reach) in [("ada", Reach::Machine), ("max", Reach::Machine)] {
        let env = room.desk_shell.run_env(id, reach);
        assert!(
            !names(&env).contains(&"API_KEY") && env.ca_pem.is_none(),
            "{id}"
        );
    }
    assert_eq!(
        room.desk_shell_readable("cleo"),
        [] as [(String, String); 0]
    );
}

/// Runs `command` as `id`'s shell tool would, in a workspace of its own.
#[cfg(unix)]
async fn run(room: &Arc<Room>, id: &str, reach: Reach, command: &str) -> String {
    let workspace = tempfile::tempdir().unwrap();
    run_in(room, id, reach, command, workspace.path()).await
}

#[cfg(unix)]
async fn run_in(
    room: &Arc<Room>,
    id: &str,
    reach: Reach,
    command: &str,
    workspace: &std::path::Path,
) -> String {
    let shell = RunCommand::new(
        Workspace::open(workspace.to_path_buf(), reach, workspace.join(".out")).unwrap(),
    )
    .with_secrets(for_teammate(Arc::downgrade(room), id.to_owned()));
    let args: RunCommandArgs = serde_json::from_value(json!({ "command": command })).unwrap();
    shell
        .run(args, CancellationToken::new(), &OutputTail::default())
        .await
        .unwrap()
        .output
}

/// A room whose desk shell's proxy sends every host to a fake service.
#[cfg(target_os = "macos")]
async fn proxied_room(name: &str) -> (Arc<Room>, crate::keyproxy::tests::Upstream) {
    let up = crate::keyproxy::tests::upstream().await;
    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    roots.add(up.ca_der.clone()).unwrap();
    let keys = KeyProxy::for_test(roots, up.addr);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(keys.clone().serve(listener));
    let room = room_of(name, &[teammate("ada", "hotline", Reach::Workspace)]);
    room.desk_shell.install(keys, port);
    give(
        &room,
        "ada",
        vec![grant("API_KEY", false, Some(vec![GrantPlace::Desk]))],
    );
    (room, up)
}

/// The audit's shell probes, in a real confined shell: every spelling of
/// the variable is the stand-in's, while curl through the proxy reaches the
/// service with the key.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_audit_probes_see_only_the_stand_in_in_a_real_confined_shell() {
    let (room, up) = proxied_room("desk-shell-probes").await;
    let out = run(
        &room,
        "ada",
        Reach::Workspace,
        &format!(
            "echo \"$(printf %s \"$API_KEY\" | base64)\"; echo \"$(printf %s \"$API_KEY\" | rev)\"; \
             echo \"$(printf %s \"$API_KEY\" | fold -w1 | tr -d '\\n')\"; /bin/bash -c 'echo \"${{API_KEY:0:7}}\"'; \
             curl -sS https://{API}/user -H \"Authorization: Bearer $API_KEY\""
        ),
    )
    .await;
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.len() >= 5, "{out}");
    use base64::Engine;
    let stand_in = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(lines[0].trim())
            .unwrap(),
    )
    .unwrap();
    assert_ne!(stand_in, REAL);
    assert_eq!(lines[1], stand_in.chars().rev().collect::<String>());
    assert_eq!(lines[2], stand_in);
    assert_eq!(lines[3], &stand_in[..7]);
    assert!(lines[4].contains("\"ok\":true"), "{out}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(REAL);
    let reversed: String = REAL.chars().rev().collect();
    for leak in [REAL, encoded.as_str(), reversed.as_str()] {
        assert!(!out.contains(leak), "{out}");
    }
    let seen = up.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].auth, format!("Bearer {REAL}"));
}

/// Unticking the key mid-command: the next request carries the stand-in.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_stops_the_stand_in_mid_run() {
    let (room, up) = proxied_room("desk-shell-revoke").await;
    let workspace = tempfile::tempdir().unwrap();
    let stop = workspace.path().join("stop");
    // -k: once revoked, the tunnel reaches the fake service, whose CA the
    // command's bundle does not hold; what it is sent is what matters.
    let command = format!(
        "i=0; while [ ! -e stop ] && [ $i -lt 600 ]; do curl -sk -o /dev/null https://{API}/loop -H \"Authorization: Bearer $API_KEY\"; i=$((i+1)); sleep 0.05; done"
    );
    let running = {
        let room = room.clone();
        let path = workspace.path().to_path_buf();
        tokio::spawn(async move { run_in(&room, "ada", Reach::Workspace, &command, &path).await })
    };
    let up = &up;
    let reached = |count: usize| async move {
        for _ in 0..400 {
            if up.seen().len() >= count {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        false
    };
    assert!(reached(3).await, "the command never reached the service");
    give(&room, "ada", Vec::new());
    let at = up.seen().len();
    assert!(
        reached(at + 3).await,
        "the command stopped after the revoke"
    );
    std::fs::write(&stop, "").unwrap();
    running.await.unwrap();
    let seen = up.seen();
    assert!(
        seen[..3].iter().all(|s| s.auth == format!("Bearer {REAL}")),
        "{seen:?}"
    );
    // One request may have been on its way as the grant was taken back.
    for request in &seen[at + 1..] {
        assert!(!request.auth.contains(REAL), "{seen:?}");
    }
}

#[tokio::test]
async fn the_teammate_is_told_its_desk_shells_variables_by_name_only() {
    let room = room_of(
        "desk-shell-preamble",
        &[teammate("ada", "hotline", Reach::Workspace)],
    );
    let desk = Some(vec![GrantPlace::Desk]);
    give(
        &room,
        "ada",
        vec![
            grant("API_KEY", false, desk.clone()),
            grant("PLAIN", true, desk),
        ],
    );
    let ada = room.persona("ada").unwrap();
    let heard = super::super::preamble::preamble(
        &ada,
        Some(Reach::Workspace),
        None,
        &room.stored_secrets(),
        false,
    );
    assert!(
        heard.contains(&format!(
            "Your shell on this desk has these environment variables, by name: API_KEY (a key used with {API}), PLAIN."
        )),
        "{heard}"
    );
    assert!(!heard.contains(REAL) && !heard.contains(PLAIN), "{heard}");
    // A grant that names no place is its computer's, and is not said here.
    give(&room, "ada", vec![grant("PLAIN", true, None)]);
    let ada = room.persona("ada").unwrap();
    let heard = super::super::preamble::preamble(
        &ada,
        Some(Reach::Workspace),
        None,
        &room.stored_secrets(),
        false,
    );
    assert!(!heard.contains("Your shell on this desk"), "{heard}");
}
