use super::container::{
    PullTally, create_args, host_time_zone, memory_of, mount_args, parse_apple, parse_inspect,
    pids_of, token_from_environment,
};
use super::fixtures::*;
use super::*;
use crate::contract::SkillPolicy;
use crate::contract::{ComputerMount, FolderGrant, McpPolicy, PersonaComputer, PolicyMode};
use std::fs;
use std::path::Path;

/// Docker's lines to a pipe, in the order it prints them: a layer is
/// counted when named and again when it lands, an old one at once, and a
/// line about nothing in particular changes nothing.
#[test]
fn docker_lines_are_counted_by_layer() {
    let mut tally = PullTally::new(Runtime::Docker);
    assert!(!tally.take("0.9.1: Pulling from 1broseidon/hotline-computer"));
    assert!(tally.take("a1b2c3d4e5f6: Pulling fs layer"));
    assert!(tally.take("f6e5d4c3b2a1: Pulling fs layer"));
    assert!(!tally.take("a1b2c3d4e5f6: Waiting"));
    assert!(!tally.take("a1b2c3d4e5f6: Downloading"));
    assert!(!tally.take("a1b2c3d4e5f6: Verifying Checksum"));
    assert!(!tally.take("a1b2c3d4e5f6: Download complete"));
    assert_eq!((tally.done(), tally.total()), (0, 2));
    assert!(tally.take("a1b2c3d4e5f6: Pull complete"));
    assert!(!tally.take("a1b2c3d4e5f6: Pull complete"));
    assert!(tally.take("00ff00ff00ff: Already exists"));
    assert_eq!((tally.done(), tally.total()), (2, 3));
    assert!(!tally.take("Digest: sha256:abc"));
    assert!(!tally.take("Status: Downloaded newer image for x"));
}

/// Podman names a blob when it starts copying and again with `done`
/// when it finishes; a blob it has is `skipped`.
#[test]
fn podman_lines_are_counted_by_blob() {
    let mut tally = PullTally::new(Runtime::Podman);
    assert!(!tally.take("Trying to pull ghcr.io/1broseidon/hotline-computer:0.9.1..."));
    assert!(!tally.take("Getting image source signatures"));
    assert!(tally.take("Copying blob sha256:aaaa"));
    assert!(tally.take("Copying blob sha256:bbbb"));
    assert_eq!((tally.done(), tally.total()), (0, 2));
    assert!(tally.take("Copying blob sha256:aaaa done  |"));
    assert!(tally.take("Copying blob sha256:cccc skipped: already exists"));
    assert_eq!((tally.done(), tally.total()), (2, 3));
    assert!(!tally.take("Copying config sha256:dddd done"));
    assert!(!tally.take("Writing manifest to image destination"));
}

/// Apple's runtime prints nothing the desk reads, so its pull is only
/// ever started and over.
#[test]
fn apple_container_lines_are_not_counted() {
    let mut tally = PullTally::new(Runtime::AppleContainer);
    assert!(!tally.take("aaaa: Pulling fs layer"));
    assert_eq!((tally.done(), tally.total()), (0, 0));
}

fn persona(id: &str, cwd: &str) -> Persona {
    Persona {
        node: None,
        id: id.to_string(),
        name: "Ada".to_string(),
        goal: String::new(),
        avatar: None,
        team: None,
        backend_id: "hotline".to_string(),
        cwd: cwd.to_string(),
        reach: None,
        model_id: None,
        mode_id: None,
        effort_id: None,
        config_choices: std::collections::BTreeMap::new(),
        effort_by_model: std::collections::BTreeMap::new(),
        harness_override: None,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::All,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: false,
        allowed_senders: Vec::new(),
        web_search_policy: None,
        folders: None,
        computer: Some(PersonaComputer {
            enabled: true,
            image: Some("hotline-computer:test".into()),
            memory: None,
            cpus: None,
            pids: None,
            mounts: None,
            secrets: None,
        }),
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: 1,
        updated_at: 1,
    }
}

async fn health_on() -> u16 {
    let app = axum::Router::new().route("/health", axum::routing::get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    port
}

fn log_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn create_line(log: &str) -> &str {
    log.lines()
        .find(|line| line.starts_with("create "))
        .unwrap_or_else(|| panic!("no create in {log}"))
}

fn in_order(line: &str, needles: &[&str]) {
    let mut from = 0;
    for needle in needles {
        let at = line[from..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} missing after {from} in {line}"));
        from += at + needle.len();
    }
}

#[tokio::test]
async fn ensure_running_on_an_absent_container_creates_with_the_hardened_args() {
    let root = scratch("create");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    let state = root.join("state");
    fs::write(&state, "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", cwd.to_str().unwrap());
    let ready = computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("ensure");
    assert_eq!(ready.url, format!("http://127.0.0.1:{port}/mcp"));
    assert_ne!(ready.token, "");

    let recorded = log_text(&log);
    let create = create_line(&recorded);
    in_order(
        create,
        &[
            "--cap-drop=ALL",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "1024",
            "--memory",
            "4g",
            "--shm-size",
            "1g",
            "-v",
            "hotline-home-ada:/home/agent",
            "-v",
            "hotline-nix-glibc:/nix",
            "-v",
            "hotline-src-ada:/home/agent/src",
            "-p",
            "127.0.0.1:0:8787",
            "-e",
            "HOTLINE_COMPUTER_TOKEN=",
            "-e",
            "TZ=",
            "-v",
            &format!(
                "{}:{WORKSPACE_MOUNT}",
                dunce::canonicalize(&cwd).unwrap().display()
            ),
            "hotline-computer:test",
        ],
    );
    assert!(create.contains("--name hotline-computer-ada"), "{create}");
    assert!(
        recorded.contains("start hotline-computer-ada"),
        "{recorded}"
    );
}

#[test]
fn the_computers_clock_is_set_to_the_hosts_zone() {
    let zone = host_time_zone();
    assert!(
        !zone.is_empty() && !zone.contains(char::is_whitespace),
        "{zone}"
    );
}

#[test]
fn limits_are_the_teammates_else_the_defaults() {
    let mut ada = persona("ada", "/tmp");
    assert_eq!(memory_of(&ada).unwrap(), "4g");
    assert_eq!(pids_of(&ada), "1024");
    let computer = ada.computer.as_mut().unwrap();
    computer.memory = Some(" 8G ".into());
    computer.pids = Some(0);
    assert_eq!(memory_of(&ada).unwrap(), "8G");
    assert_eq!(pids_of(&ada), "-1");
    ada.computer.as_mut().unwrap().pids = Some(4096);
    assert_eq!(pids_of(&ada), "4096");
    for bad in ["8 gigs", "g", "2gb", "-1"] {
        ada.computer.as_mut().unwrap().memory = Some(bad.into());
        assert!(memory_of(&ada).is_err(), "{bad}");
    }
}

// Apple accepts lower-case m as mebibytes, including 4608m (4.5 GiB).
// Source review, not a physical-Mac test: apple/container at
// 4a7d8615241b8ddecfd3bf225cd7c44f4b2ccf7c,
// Sources/ContainerPersistence/Measurement+Parse.swift (binaryUnits,
// parseUnit), and Tests/ContainerAPIClientTests/Measurement+ParseTests.swift.
#[test]
fn create_args_preserve_fractional_cpus_and_mebibyte_memory_for_each_runtime() {
    for runtime in [Runtime::Docker, Runtime::Podman, Runtime::AppleContainer] {
        let mut ada = persona("ada", "/tmp");
        ada.computer.as_mut().unwrap().memory = Some("4608m".into());
        for cpus in [None, Some(0.25), Some(2.5), Some(4.0)] {
            ada.computer.as_mut().unwrap().cpus = cpus;
            let args = create_args(runtime, "test", "image", "token", "/tmp", &ada).unwrap();
            let flag = |name: &str| {
                args.windows(2)
                    .find(|pair| pair[0] == name)
                    .map(|pair| pair[1].as_str())
            };
            assert_eq!(flag("--memory"), Some("4608m"));
            let expected = cpus.map(|cpus| {
                if runtime == Runtime::AppleContainer {
                    cpus.ceil().max(1.0)
                } else {
                    cpus
                }
                .to_string()
            });
            assert_eq!(flag("--cpus"), expected.as_deref());
        }
    }
}

#[test]
fn create_args_reject_nonfinite_and_nonpositive_cpu_limits() {
    for runtime in [Runtime::Docker, Runtime::Podman, Runtime::AppleContainer] {
        for cpus in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut ada = persona("ada", "/tmp");
            ada.computer.as_mut().unwrap().cpus = Some(cpus);
            let error = create_args(runtime, "test", "image", "token", "/tmp", &ada).unwrap_err();
            assert!(error.contains("CPU limit"), "{error}");
        }
    }
}

#[test]
fn mounts_are_existing_folders_outside_hotlines_own_paths() {
    let root = scratch("mounts");
    let checkout = root.join("checkout");
    fs::create_dir_all(&checkout).unwrap();
    let mut ada = persona("ada", "/tmp");
    let host = checkout.to_str().unwrap().to_string();
    let mount = |path: &str, readonly: bool| ComputerMount {
        host: host.clone(),
        path: path.into(),
        readonly,
    };
    ada.computer.as_mut().unwrap().mounts = Some(vec![
        mount("/home/agent/hotline-next", true),
        mount("/mnt/data/", false),
    ]);
    let canonical = dunce::canonicalize(&checkout)
        .unwrap()
        .display()
        .to_string();
    assert_eq!(
        mount_args(&ada).unwrap(),
        vec![
            format!("{canonical}:/home/agent/hotline-next:ro"),
            format!("{canonical}:/mnt/data"),
        ]
    );
    for taken in [
        "/nix",
        "/nix/store",
        "/home/agent",
        "/home/agent/workspace/x",
        "/",
    ] {
        ada.computer.as_mut().unwrap().mounts = Some(vec![mount(taken, true)]);
        assert!(mount_args(&ada).is_err(), "{taken}");
    }
    ada.computer.as_mut().unwrap().mounts = Some(vec![mount("relative", true)]);
    assert!(mount_args(&ada).is_err());
    ada.computer.as_mut().unwrap().mounts = Some(vec![ComputerMount {
        host: root.join("missing").to_str().unwrap().into(),
        path: "/mnt/missing".into(),
        readonly: true,
    }]);
    assert!(mount_args(&ada).is_err());
}

#[test]
fn granted_folders_are_mounted_read_only_unless_writable_and_cannot_be_covered() {
    let root = scratch("folders");
    let read_only = root.join("read").join("app");
    let writable = root.join("write").join("app");
    fs::create_dir_all(&read_only).unwrap();
    fs::create_dir_all(&writable).unwrap();
    let mut ada = persona("ada", "/tmp");
    ada.folders = Some(vec![
        FolderGrant {
            path: read_only.to_string_lossy().into_owned(),
            writable: false,
        },
        FolderGrant {
            path: writable.to_string_lossy().into_owned(),
            writable: true,
        },
        FolderGrant {
            path: root.join("gone").to_string_lossy().into_owned(),
            writable: true,
        },
    ]);
    let args = create_args(Runtime::Docker, "test", "image", "token", "/tmp", &ada).unwrap();
    let read_mount = format!("{}:/home/agent/folders/app:ro", read_only.display());
    let write_mount = format!("{}:/home/agent/folders/app-2", writable.display());
    assert!(args.contains(&read_mount), "{args:?}");
    assert!(args.contains(&write_mount), "{args:?}");
    assert!(
        args.contains(&format!("HOTLINE_FOLDERS={read_mount}\n{write_mount}")),
        "{args:?}"
    );
    assert!(!args.iter().any(|arg| arg.contains("gone")), "{args:?}");

    ada.computer.as_mut().unwrap().mounts = Some(vec![ComputerMount {
        host: read_only.to_string_lossy().into_owned(),
        path: "/home/agent/folders/app".into(),
        readonly: true,
    }]);
    assert!(mount_args(&ada).is_err());
}

/// A container sees the folders it was made with, so one made with
/// other folders is made again at the next start and one made with the
/// same is kept.
#[tokio::test]
async fn a_computer_made_with_other_folders_is_made_again() {
    let root = scratch("folders-remade");
    let cwd = root.join("work");
    let folder = root.join("granted");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&folder).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    fs::write(root.join("state"), "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let mut ada = persona("ada", cwd.to_str().unwrap());
    ada.folders = Some(vec![FolderGrant {
        path: folder.to_string_lossy().into_owned(),
        writable: false,
    }]);
    let cwd = cwd.to_str().unwrap();
    computers
        .ensure_running(&ada, cwd, None, None, |_| {})
        .await
        .expect("first");
    computers.stop("ada", None).await.expect("stop");
    fs::write(&log, "").unwrap();
    computers
        .ensure_running(&ada, cwd, None, None, |_| {})
        .await
        .expect("same folders");
    let kept = log_text(&log);
    assert!(
        !kept.lines().any(|line| line.starts_with("create ")),
        "{kept}"
    );

    ada.folders.as_mut().unwrap()[0].writable = true;
    fs::write(&log, "").unwrap();
    computers
        .ensure_running(&ada, cwd, None, None, |_| {})
        .await
        .expect("changed folders");
    let remade = log_text(&log);
    assert!(
        remade.lines().any(|line| line.starts_with("rm -f ")),
        "{remade}"
    );
    let create = remade
        .lines()
        .find(|line| line.starts_with("create "))
        .unwrap_or_else(|| panic!("{remade}"));
    assert!(
        create.contains(&format!(
            "{}:/home/agent/folders/granted ",
            folder.display()
        )),
        "{create}"
    );

    ada.folders = None;
    fs::write(&log, "").unwrap();
    computers
        .ensure_running(&ada, cwd, None, None, |_| {})
        .await
        .expect("no folders");
    let emptied = log_text(&log);
    let create = emptied
        .lines()
        .find(|line| line.starts_with("create "))
        .unwrap_or_else(|| panic!("a removed folder stayed mounted: {emptied}"));
    assert!(!create.contains("/home/agent/folders"), "{create}");
}

/// Stops ada's computer and starts it again, and says what the runtime
/// was asked to do for the start.
async fn restart(computers: &Computer, ada: &Persona, root: &Path) -> String {
    computers.stop("ada", None).await.expect("stop");
    fs::write(root.join("argv.log"), "").unwrap();
    computers
        .ensure_running(ada, &ada.cwd, None, None, |_| {})
        .await
        .expect("start");
    log_text(&root.join("argv.log"))
}

/// A runtime takes memory and CPUs when a container is made, so one made
/// with other limits is made again at the next start, and one made before
/// limits were recorded is made again the next time it starts from stopped.
#[tokio::test]
async fn a_computer_made_with_other_limits_is_made_again() {
    let root = scratch("limits-remade");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    fs::write(root.join("state"), "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let mut ada = persona("ada", cwd.to_str().unwrap());
    computers
        .ensure_running(&ada, &ada.cwd, None, None, |_| {})
        .await
        .expect("first");
    let kept = restart(&computers, &ada, &root).await;
    assert!(!kept.contains("create "), "{kept}");

    ada.computer.as_mut().unwrap().memory = Some("2g".into());
    let remade = restart(&computers, &ada, &root).await;
    assert!(remade.contains("rm -f "), "{remade}");
    assert!(create_line(&remade).contains("--memory 2g"), "{remade}");

    ada.computer.as_mut().unwrap().cpus = Some(2.0);
    let remade = restart(&computers, &ada, &root).await;
    assert!(create_line(&remade).contains("--cpus 2"), "{remade}");

    fs::remove_file(root.join("state.limits")).unwrap();
    let unrecorded = restart(&computers, &ada, &root).await;
    assert!(unrecorded.contains("create "), "{unrecorded}");
}

#[tokio::test]
async fn ensure_running_on_a_stopped_container_starts() {
    let root = scratch("start");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    let state = root.join("state");
    fs::write(&state, "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", cwd.to_str().unwrap());
    computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("first");
    computers.stop("ada", None).await.expect("stop");
    fs::write(&log, "").unwrap();
    computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("second");
    let recorded = log_text(&log);
    assert!(
        recorded.lines().any(|line| line.starts_with("start ")),
        "{recorded}"
    );
    assert!(
        !recorded.lines().any(|line| line.starts_with("create ")),
        "a known stopped container must not be recreated: {recorded}"
    );
}

#[test]
fn apple_inspection_recovers_only_the_computer_environment_token() {
    let seen = parse_apple(&serde_json::json!({
        "status":{"state":"running"},
        "configuration":{
            "initProcess":{"environment":["PATH=/usr/bin", "HOTLINE_COMPUTER_TOKEN=fixture-apple-token"]},
            "publishedPorts":[{"containerPort":8787,"hostPort":12345,"proto":"tcp"}]
        }
    })).unwrap();
    assert!(
        status_of(seen, None)
            .viewer
            .is_some_and(|viewer| viewer == "http://127.0.0.1:12345/#fixture-apple-token")
    );
    assert!(
        token_from_environment(Some(&serde_json::json!(["HOTLINE_COMPUTER_TOKEN="]))).is_none()
    );
}

#[test]
fn a_container_made_before_release_labels_recovers_its_image_tag_on_each_runtime() {
    // Only the fields inspection reads, preserving each runtime's shape.
    // Docker and Podman's top-level Image is deliberately an image ID.
    let fixtures = [
        (
            Runtime::AppleContainer,
            serde_json::json!({
                "status": {"state": "running"},
                "configuration": {
                    "labels": {},
                    "image": {"reference": "hotline-computer:0.11.0-rc.7"}
                }
            }),
        ),
        (
            Runtime::Docker,
            serde_json::json!({
                "State": {"Running": true},
                "Image": "sha256:container-image-id",
                "Config": {"Labels": null, "Image": "hotline-computer:0.11.0-rc.7"}
            }),
        ),
        (
            Runtime::Podman,
            serde_json::json!({
                "State": {"Running": true},
                "Image": "container-image-id",
                "ImageName": "localhost/hotline-computer:0.11.0-rc.7",
                "Config": {"Labels": {}}
            }),
        ),
    ];
    for (runtime, mut fixture) in fixtures {
        let inspect = |fixture: &Value| {
            parse_inspect(runtime, &serde_json::json!([fixture]).to_string()).unwrap()
        };
        let seen = inspect(&fixture);
        assert_eq!(seen.release.as_deref(), Some("0.11.0-rc.7"), "{runtime:?}");
        assert_eq!(
            status_of(seen, None).release.as_deref(),
            Some("0.11.0-rc.7")
        );

        let labels = if runtime == Runtime::AppleContainer {
            &mut fixture["configuration"]["labels"]
        } else {
            &mut fixture["Config"]["Labels"]
        };
        *labels = serde_json::json!({"hotline.release": "0.11.0-rc.6"});
        let mut seen = inspect(&fixture);
        assert_eq!(
            seen.release.as_deref(),
            Some("0.11.0-rc.6"),
            "label wins: {runtime:?}"
        );
        seen.running = false;
        assert_eq!(
            status_of(seen, None).release.as_deref(),
            Some("0.11.0-rc.6")
        );
    }
}

#[test]
fn every_runtime_records_the_release_when_creating_a_computer() {
    let ada = persona("ada", "/tmp");
    for runtime in [Runtime::AppleContainer, Runtime::Docker, Runtime::Podman] {
        let args = create_args(
            runtime,
            "test",
            "registry:5000/computer:0.11.0-rc.7",
            "token",
            "/tmp",
            &ada,
        )
        .unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--label", "hotline.release=0.11.0-rc.7"]),
            "{runtime:?}: {args:?}"
        );
    }
}

#[tokio::test]
async fn an_existing_desktop_remains_viewable_and_is_not_recreated_after_restart() {
    let root = scratch("recover-token");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    fs::write(root.join("state"), "running").unwrap();
    fs::write(root.join("state.token"), "present").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let status = computers.status("ada", None, None).await.unwrap();
    assert!(
        status
            .viewer
            .is_some_and(|url| url.ends_with("/#fixture-persisted-token"))
    );
    let ready = computers
        .ensure_running(
            &persona("ada", cwd.to_str().unwrap()),
            cwd.to_str().unwrap(),
            None,
            None,
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(ready.token, "fixture-persisted-token");
    let recorded = log_text(&root.join("argv.log"));
    assert!(
        !recorded.lines().any(|line| line.starts_with("rm ")
            || line.starts_with("create ")
            || line.starts_with("start ")),
        "a running desktop was mutated: {recorded}"
    );
}

#[tokio::test]
async fn an_unknown_token_existing_container_is_removed_and_recreated() {
    let root = scratch("unknown");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    let state = root.join("state");
    fs::write(&state, "running").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", cwd.to_str().unwrap());
    computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("ensure");
    let recorded = log_text(&log);
    assert!(
        recorded.lines().any(|line| line.starts_with("rm ")),
        "{recorded}"
    );
    assert!(
        recorded.lines().any(|line| line.starts_with("create ")),
        "{recorded}"
    );
}

#[tokio::test]
async fn stop_and_remove_send_the_right_subcommands() {
    let root = scratch("stop-rm");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    let state = root.join("state");
    fs::write(&state, "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", cwd.to_str().unwrap());
    computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("ensure");
    fs::write(&log, "").unwrap();
    computers.stop("ada", None).await.expect("stop");
    computers.remove("ada", None).await.expect("remove");
    let recorded = log_text(&log);
    assert!(
        recorded
            .lines()
            .any(|line| line == "stop hotline-computer-ada"),
        "{recorded}"
    );
    assert!(
        recorded
            .lines()
            .any(|line| line == "rm -f hotline-computer-ada"),
        "{recorded}"
    );
}

#[tokio::test]
async fn sweep_stops_then_removes_by_idle_age() {
    let root = scratch("sweep");
    let cwd = root.join("work");
    fs::create_dir_all(&cwd).unwrap();
    let port = health_on().await;
    fake_runtime(&root, port);
    let log = root.join("argv.log");
    let state = root.join("state");
    fs::write(&state, "absent").unwrap();
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", cwd.to_str().unwrap());
    computers
        .ensure_running(&ada, cwd.to_str().unwrap(), None, None, |_| {})
        .await
        .expect("ensure");
    fs::write(&log, "").unwrap();
    let now = now_ms();
    computers
        .sweep(now + COMPUTER_IDLE_STOP_MS + 1, &HashSet::new())
        .await;
    let after_idle = log_text(&log);
    assert!(
        after_idle
            .lines()
            .any(|line| line == "stop hotline-computer-ada"),
        "{after_idle}"
    );
    fs::write(&log, "").unwrap();
    computers
        .sweep(now + COMPUTER_HIBERNATE_MS + 1, &HashSet::new())
        .await;
    let after_hibernate = log_text(&log);
    assert!(
        after_hibernate
            .lines()
            .any(|line| line == "rm -f hotline-computer-ada"),
        "{after_hibernate}"
    );
}

#[test]
fn a_computer_tool_is_named_computer_underscore_underscore() {
    assert!(is_computer_tool("computer__capture", "see the screen"));
    assert!(is_computer_tool("other", "computer__capture"));
    assert!(!is_computer_tool("echo__shout", "echo__shout"));
    assert!(!is_computer_tool("capture", "capture"));
}

#[test]
fn the_image_is_the_teammates_else_the_rooms_else_the_pin() {
    let mut ada = persona("ada", "/tmp");
    ada.computer = Some(PersonaComputer {
        enabled: true,
        image: None,
        memory: None,
        cpus: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    assert_eq!(image_of(&ada, None, None), default_image());
    assert_eq!(image_of(&ada, None, Some("0.5.3")), image_at("0.5.3"));
    assert_eq!(
        image_of(&ada, Some("room/image:1"), Some("0.5.3")),
        "room/image:1"
    );
    ada.computer = Some(PersonaComputer {
        enabled: true,
        image: Some("  own/image:2  ".into()),
        memory: None,
        cpus: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    assert_eq!(image_of(&ada, Some("room/image:1"), None), "own/image:2");
}

#[tokio::test]
async fn no_runtime_is_the_install_sentence() {
    let root = scratch("none");
    let computers = Computer::with_path(root.as_os_str());
    let ada = persona("ada", root.to_str().unwrap());
    let error = computers
        .ensure_running(&ada, root.to_str().unwrap(), None, None, |_| {})
        .await
        .expect_err("no runtime");
    assert_eq!(
        error,
        "No container runtime was found; install Docker or Podman."
    );
}
