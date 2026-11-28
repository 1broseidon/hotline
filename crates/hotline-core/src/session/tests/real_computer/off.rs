//! Turning a real computer off, on Apple `container`, through the wire.
//!
//! A throwaway teammate's computer is made by the desk's own code from the
//! 0.13.0 release image, with a folder and a made-up secret granted. A
//! loop is left running in it, holding the granted file open with the
//! secret in its environment, and a file is written in its home. The
//! owner's `persona.update` turning the computer off is answered only once
//! the runtime's own `inspect` says the container is stopped (and still
//! there), within the answer's wait; turned on again, the same container
//! starts, with the file and without the loop. Then, the computer staying
//! on, the secret is ungranted (no new job has it) and the folder is taken
//! away (the container is made again without the mount); neither answer
//! finds it running. Each of these is said as an `EVIDENCE` line.
//!
//! No agent runs: the session's driver is a script, and the test calls the
//! computer's tools through the gate the session was handed.
//!
//! The desk finds the runtime as a wrapper that refuses any command naming
//! another `hotline-computer-*` container than this test's, and any
//! stop, rm, kill, delete, exec or start that does not name this test's,
//! and logs every call with its start and end time. The containers already
//! on this machine are listed before and after, and must still be running
//! from the same start time. The test's container is removed at the end by
//! its exact name, pass or fail.
//!
//! ```sh
//! HOTLINE_E2E_IMAGE=ghcr.io/1broseidon/hotline-computer:0.13.0 \
//!   cargo test -p hotline-core --lib real_computer::off -- --ignored --nocapture --test-threads=1
//! ```

use super::*;

const RELEASE_IMAGE: &str = "ghcr.io/1broseidon/hotline-computer:0.13.0";
const PREFIX: &str = "hotline-computer-hl-off-e2e-";

/// The test's own container, removed by its exact name whatever happens.
struct Mine {
    runtime: String,
    container: String,
}

impl Drop for Mine {
    fn drop(&mut self) {
        assert!(self.container.starts_with(PREFIX), "{}", self.container);
        let _ = run(&self.runtime, &["stop", &self.container]);
        let _ = run(&self.runtime, &["rm", "-f", &self.container]);
        say(format!("removed {}", self.container));
    }
}

/// `container ls --all` as rows: name, state, started.
fn listed(runtime: &str) -> Vec<(String, String, String)> {
    let out = run(runtime, &["ls", "--all"]);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // ID IMAGE OS ARCH STATE [IP] CPUS MEMORY [MB] [STARTED]
            let started = fields
                .last()
                .filter(|last| last.contains('T') && last.ends_with('Z'))
                .map_or_else(String::new, |last| (*last).to_string());
            (
                fields[0].to_string(),
                fields.get(4).copied().unwrap_or_default().to_string(),
                started,
            )
        })
        .collect()
}

fn state_of(runtime: &str, name: &str) -> Option<String> {
    listed(runtime)
        .into_iter()
        .find(|(id, _, _)| id == name)
        .map(|(_, state, _)| state)
}

/// The wrapper the desk's `container` is found as. TEST ONLY.
fn guard(dir: &Path, real: &Path, mine: &str) {
    use std::os::unix::fs::PermissionsExt;
    let script = format!(
        r#"#!/bin/sh
# TEST ONLY: hl-off-e2e's guard around Apple container.
real='{real}'
dir='{dir}'
mine='{mine}'
named=no
for a in "$@"; do
  case "$a" in
    hotline-computer-*)
      if [ "$a" != "$mine" ]; then
        printf '%s\n' "$*" >> "$dir/refused.log"
        echo "TEST GUARD: refused a command naming another container" >&2
        exit 97
      fi
      named=yes ;;
  esac
done
case "$1" in
  stop|rm|kill|delete|exec|start)
    if [ "$named" != yes ]; then
      printf '%s\n' "$*" >> "$dir/refused.log"
      echo "TEST GUARD: refused $1 without this test's container" >&2
      exit 97
    fi ;;
esac
now() {{ perl -MTime::HiRes=time -e 'printf "%.3f", time'; }}
t0=$(now)
"$real" "$@"
code=$?
printf '%s %s %s %s\n' "$t0" "$(now)" "$code" "$*" >> "$dir/calls.log"
exit $code
"#,
        real = real.display(),
        dir = dir.display(),
    );
    let path = dir.join("container");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Each logged runtime call: seconds it took, exit code, arguments.
fn calls(dir: &Path) -> Vec<(f64, i32, String)> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, ' ');
            let start: f64 = parts.next()?.parse().ok()?;
            let end: f64 = parts.next()?.parse().ok()?;
            let code: i32 = parts.next()?.parse().ok()?;
            Some((
                end - start,
                code,
                parts.next().unwrap_or_default().to_owned(),
            ))
        })
        .collect()
}

/// Each process in the computer running the marker loop, as its jobs see
/// it, with what the loop said of itself when it began: whether the secret
/// was in its environment, and what its fd 3 (the granted file it holds
/// open) pointed at. The computer keeps one job's `/proc/<pid>/environ` and
/// `fd` from another, so only the loop can say those. The scan's own
/// command does not hold the marker whole, so it never finds itself.
async fn markers(gate: &Gate, marker: &str) -> Vec<String> {
    let (head, tail) = marker.split_at(5);
    let (code, out) = gate
        .sh(&format!(
            r#"m={head}; m="${{m}}{tail}"; for p in /proc/[0-9]*; do c=$(tr '\0' ' ' < "$p/cmdline" 2>/dev/null); case "$c" in "sh -c"*"$m"*) echo "marker pid=${{p#/proc/}} says: $(tr '\n' ' ' < /tmp/hl-off-e2e-loop.txt 2>&1)";; esac; done; true"#
        ))
        .await;
    assert_eq!(code, Some(0), "{out}");
    out.lines()
        .filter(|line| line.starts_with("marker "))
        .map(str::to_owned)
        .collect()
}

/// The runtime's own `inspect` of the test's container: its state.
fn inspected(runtime: &str, container: &str) -> String {
    assert!(container.starts_with(PREFIX), "{container}");
    let out = run(runtime, &["inspect", container]);
    let value: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let object = value
        .as_array()
        .and_then(|all| all.first())
        .unwrap_or(&value);
    object["status"]["state"]
        .as_str()
        .or_else(|| object["status"].as_str())
        .unwrap_or("absent")
        .to_owned()
}

/// Whether a new job in the computer has the secret in its environment.
async fn secret_in_new_job(gate: &Gate) -> String {
    let (code, out) = gate
        .sh(r#"if [ -n "${HL_OFF_E2E_SECRET-}" ]; then echo secret-present; else echo secret-absent; fi"#)
        .await;
    assert_eq!(code, Some(0), "{out}");
    out.trim().to_owned()
}

async fn owner(room: &Arc<Room>, cmd: &str, params: Value) -> (Value, Duration) {
    let asked = Instant::now();
    let answer = tokio::time::timeout(
        Duration::from_secs(20),
        crate::wire::tests::owner_answer(room, cmd, params),
    )
    .await
    .unwrap_or_else(|_| panic!("{cmd} was not answered within a phone's wait"));
    (answer, asked.elapsed())
}

fn computer_patch(room: &Room, persona_id: &str, enabled: bool) -> Value {
    let mut computer = json!(room.persona(persona_id).unwrap().computer);
    computer["enabled"] = json!(enabled);
    json!({ "id": persona_id, "patch": { "computer": computer } })
}

/// Waits for the restart a change placed, and answers the gate it handed.
async fn back(
    room: &Arc<Room>,
    agents: &Fake,
    persona_id: &str,
    with_computer: bool,
    old: Option<&Gate>,
) -> Option<Gate> {
    eventually("the teammate is back after the change", 180, || {
        let ready = room.info(persona_id).state == SessionState::Ready
            && room.computer_in_use(persona_id) == with_computer;
        let fresh = !with_computer || Gate::of(agents).as_ref() != old;
        std::future::ready(ready && fresh)
    })
    .await;
    with_computer.then(|| Gate::of(agents).unwrap())
}

/// One line of evidence for the report, said and kept.
fn evidence(lines: &mut Vec<String>, line: String) {
    say(&line);
    lines.push(line);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "drives a real Hotline Computer on Apple container: real_computer::off"]
async fn turning_a_real_computer_off_stops_it_before_the_answer_and_on_brings_the_same_one_back() {
    let image = std::env::var("HOTLINE_E2E_IMAGE").unwrap_or_else(|_| RELEASE_IMAGE.to_owned());
    let real = PathBuf::from(
        String::from_utf8(run("/usr/bin/which", &["container"]).stdout)
            .unwrap()
            .trim(),
    );
    assert!(real.is_file(), "Apple container is on PATH");
    let runtime = real.to_string_lossy().into_owned();
    assert!(
        run(&runtime, &["image", "inspect", &image])
            .status
            .success(),
        "{image} is here; nothing is pulled"
    );
    let before: Vec<_> = listed(&runtime)
        .into_iter()
        .filter(|(_, state, _)| state == "running")
        .collect();
    say(format!("running before, none is touched: {before:?}"));
    let mut proof = Vec::new();

    let run_id = hex(4);
    let persona_id = format!("hl-off-e2e-{run_id}");
    let container = crate::computer::container_name(&persona_id);
    assert!(container.starts_with(PREFIX) && state_of(&runtime, &container).is_none());
    let _mine = Mine {
        runtime: runtime.clone(),
        container: container.clone(),
    };

    let root = std::env::temp_dir().join(format!("hl-off-e2e-{run_id}"));
    let guard_dir = root.join("runtime");
    let work = root.join("work");
    let folder = root.join("granted");
    for dir in [&guard_dir, &work, &folder] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    std::fs::write(folder.join("granted.txt"), "a granted folder").unwrap();
    guard(&guard_dir, &real, &container);

    let log = scratch(&format!("off-e2e-{run_id}"));
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    // Made up here, nobody's.
    vault
        .set_shared_secret("HL_OFF_E2E_SECRET", &format!("hl-off-e2e-{}", hex(8)))
        .unwrap();
    let mut teammate = super::super::persona(&persona_id);
    teammate.name = "OffE2E".into();
    teammate.reach = Some(crate::contract::Reach::Workspace);
    teammate.cwd = work.to_string_lossy().into_owned();
    teammate.computer = Some(PersonaComputer {
        cpus: None,
        enabled: true,
        image: Some(image.clone()),
        memory: None,
        pids: None,
        mounts: None,
    });
    teammate.secrets = Some(vec!["HL_OFF_E2E_SECRET".into()]);
    teammate.secret_grants = Some(vec![crate::contract::SecretGrant {
        name: "HL_OFF_E2E_SECRET".into(),
        readable: true,
        on: None,
    }]);
    enrol(&log, &teammate);
    let agents = Fake::new(Scripted::new(Vec::new()));
    let room = Room::with_agents_computers_and_vault(
        log,
        Arc::new(DeskKeys),
        agents.clone(),
        crate::computer::Computer::with_path(guard_dir.as_os_str()),
        Some(vault),
    );
    // The folder through the owner's own change, which checks it.
    let path = folder.to_string_lossy().into_owned();
    let (granted, _) = owner(
        &room,
        "persona.update",
        json!({ "id": persona_id, "patch": { "folders": [{ "path": path, "writable": false }] } }),
    )
    .await;
    assert_eq!(granted["ok"], true, "{granted}");

    // ---- Up, with a loop left running and a file in its home. -------------
    say(format!("starting {persona_id} on {image}"));
    let started = Instant::now();
    room.start(&persona_id).await.unwrap();
    say(format!("up in {:?}", started.elapsed()));
    assert!(
        room.computer_in_use(&persona_id),
        "{:?}",
        tape(&room, &persona_id)
    );
    let gate = Gate::of(&agents).expect("the session was handed the computer's gate");
    assert_eq!(secret_in_new_job(&gate).await, "secret-present");
    let marker = format!("hloffmarker{}", hex(4));
    let note = format!("hl-off-e2e-{}", hex(6));
    let (code, out) = gate
        .sh(&format!(
            "echo {note} > \"$HOME/hl-off-e2e.txt\" && (setsid nohup sh -c 'exec 3< /home/agent/folders/granted/granted.txt; {{ [ -n \"$HL_OFF_E2E_SECRET\" ] && echo secret-in-env=1 || echo secret-in-env=0; echo fd3=$(readlink /proc/self/fd/3); }} > /tmp/hl-off-e2e-loop.txt; while :; do sleep 1; done; : {marker}' >/dev/null 2>&1 &) && sleep 1 && echo \"home=$HOME\""
        ))
        .await;
    assert_eq!(code, Some(0), "{out}");
    say(out.trim());
    let running = markers(&gate, &marker).await;
    evidence(&mut proof, format!("a. before the stop: {running:?}"));
    assert_eq!(running.len(), 1, "the loop runs: {running:?}");
    assert!(
        running[0].contains("secret-in-env=1")
            && running[0].contains("fd3=/home/agent/folders/granted/granted.txt"),
        "the loop holds the granted file open with the secret in its environment: {running:?}"
    );
    let creates = || {
        calls(&guard_dir)
            .iter()
            .filter(|(_, _, args)| args.starts_with("create"))
            .count()
    };
    assert_eq!(creates(), 1);

    // ---- Off, through the owner's wire. ------------------------------------
    let (off, took) = owner(
        &room,
        "persona.update",
        computer_patch(&room, &persona_id, false),
    )
    .await;
    let state = inspected(&runtime, &container);
    let stops: Vec<_> = calls(&guard_dir)
        .into_iter()
        .filter(|(_, _, args)| args == &format!("stop {container}"))
        .collect();
    evidence(
        &mut proof,
        format!(
            "b. inspect at the answer: state={state}; ls: {:?}",
            state_of(&runtime, &container)
        ),
    );
    evidence(
        &mut proof,
        format!(
            "c. off answered in {:.3}s (ok={}); the runtime's stop took {:?}s",
            took.as_secs_f64(),
            off["ok"],
            stops
                .iter()
                .map(|(seconds, code, _)| (*seconds, *code))
                .collect::<Vec<_>>()
        ),
    );
    assert_eq!(off["ok"], true, "{off}");
    assert!(
        took < Duration::from_secs(12),
        "within the answer's wait: {took:?}"
    );
    assert_eq!(state, "stopped", "not running, still there");
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0].1, 0);
    back(&room, &agents, &persona_id, false, None).await;
    assert_eq!(inspected(&runtime, &container), "stopped");

    // ---- On again: the same container, its home, no loop. -----------------
    let (on, took) = owner(
        &room,
        "persona.update",
        computer_patch(&room, &persona_id, true),
    )
    .await;
    say(format!("on answered in {took:?}"));
    assert_eq!(on["ok"], true, "{on}");
    let gate = back(&room, &agents, &persona_id, true, Some(&gate))
        .await
        .unwrap();
    assert_eq!(inspected(&runtime, &container), "running");
    let (code, kept) = gate.sh("cat \"$HOME/hl-off-e2e.txt\"").await;
    let left = markers(&gate, &marker).await;
    evidence(
        &mut proof,
        format!(
            "a. after off and on: loop processes {left:?}; creates {}; home file {:?}",
            creates(),
            kept.trim()
        ),
    );
    assert_eq!(creates(), 1, "the same container, not a new one");
    assert_eq!(
        (code, kept.trim()),
        (Some(0), note.as_str()),
        "the home kept"
    );
    assert!(left.is_empty(), "the loop left running is gone: {left:?}");

    // ---- The secret ungranted, the computer on. ---------------------------
    assert_eq!(secret_in_new_job(&gate).await, "secret-present");
    let (taken, took) = owner(
        &room,
        "persona.update",
        json!({ "id": persona_id, "patch": { "secrets": [] } }),
    )
    .await;
    let state = inspected(&runtime, &container);
    say(format!(
        "secret taken, answered in {took:?}, inspect {state}"
    ));
    assert_eq!(taken["ok"], true, "{taken}");
    assert_ne!(state, "running", "not running at the answer");
    let gate = back(&room, &agents, &persona_id, true, Some(&gate))
        .await
        .unwrap();
    let in_job = secret_in_new_job(&gate).await;
    evidence(
        &mut proof,
        format!(
            "e. after the secret was ungranted, answered in {:.3}s with the container {state}: a new job says {in_job}",
            took.as_secs_f64()
        ),
    );
    assert_eq!(in_job, "secret-absent");

    // ---- The granted folder taken away, the computer on. ------------------
    let (code, out) = gate.sh("cat /home/agent/folders/granted/granted.txt").await;
    assert_eq!((code, out.trim()), (Some(0), "a granted folder"), "mounted");
    let (taken, took) = owner(
        &room,
        "persona.update",
        json!({ "id": persona_id, "patch": { "folders": [] } }),
    )
    .await;
    let state = inspected(&runtime, &container);
    assert_eq!(taken["ok"], true, "{taken}");
    assert_ne!(state, "running", "not running at the answer");
    let gate = back(&room, &agents, &persona_id, true, Some(&gate))
        .await
        .unwrap();
    let (_, listing) = gate
        .sh("ls -A /home/agent/folders 2>&1; cat /home/agent/folders/granted/granted.txt 2>&1; true")
        .await;
    let recorded = calls(&guard_dir)
        .into_iter()
        .filter(|(_, _, args)| args.starts_with("create"))
        .map(|(_, _, args)| args.contains("/home/agent/folders/granted"))
        .collect::<Vec<_>>();
    evidence(
        &mut proof,
        format!(
            "d. folder taken, answered in {:.3}s with the container {state}; creates mounting it: {recorded:?}; in the computer now: {:?}",
            took.as_secs_f64(),
            listing.trim()
        ),
    );
    assert_eq!(recorded, vec![true, false], "made again without the mount");
    assert!(
        !listing.contains("a granted folder"),
        "no longer mounted: {listing}"
    );

    for (seconds, code, args) in calls(&guard_dir) {
        if ["stop", "start", "create", "rm"]
            .iter()
            .any(|word| args.starts_with(word))
        {
            say(format!(
                "{seconds:>7.3}s exit {code}: {}",
                args.chars().take(70).collect::<String>()
            ));
        }
    }
    assert!(
        !guard_dir.join("refused.log").exists(),
        "the desk named no other container: {}",
        std::fs::read_to_string(guard_dir.join("refused.log")).unwrap_or_default()
    );
    let after: Vec<_> = listed(&runtime)
        .into_iter()
        .filter(|(_, state, _)| state == "running")
        .filter(|(name, _, _)| name != &container)
        .collect();
    say(format!("running after: {after:?}"));
    for line in &proof {
        say(format!("EVIDENCE {line}"));
    }
    assert_eq!(
        after, before,
        "every container that was running still runs, from the same start"
    );
}
