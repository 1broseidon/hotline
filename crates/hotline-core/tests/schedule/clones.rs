//! Clones of the home agent through the real core (ADR 0008 §0): a subagent
//! and a job are the teammate itself, with the home thread's permission set
//! under a lease derived from the home thread's, narrowed only where the home
//! agent narrows it, starting from their brief and reaching back into the
//! home thread with `search_thread` and `list_chapters`.
//!
//! The teammate is Ada on Hotline Agent, on the disposable model of the
//! module above, with a folder granted besides her workspace. What a clone
//! did is read off its own thread: its last message is `RESULTS`, one line
//! per tool it called with what came back. A tool a clone is not given is
//! never offered to it, so a call to one comes back refused before any
//! handler runs; one it is given is gated in the handler.

use super::*;

/// What the granted folder holds, so a clone that reached it says so.
const CANARY: &str = "granted-canary";

/// Ada's workspace, and a folder she is granted beside it with one file.
struct Granted {
    _dir: TempDir,
    workspace: PathBuf,
    folder: PathBuf,
}

impl Granted {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // As the desk spells a folder: resolved, without Windows's prefix.
        let base = dunce::canonicalize(dir.path()).unwrap();
        let workspace = base.join("workspace");
        let folder = base.join("harbour-records");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("notes.txt"), CANARY).unwrap();
        Self {
            _dir: dir,
            workspace,
            folder,
        }
    }

    /// The draft fields that grant the folder.
    fn draft(&self) -> Value {
        json!({"folders": [{"path": self.folder.to_string_lossy()}]})
    }

    fn notes(&self) -> String {
        self.folder.join("notes.txt").to_string_lossy().into_owned()
    }
}

/// A script of tool calls, one `CALL` line each, as a model is handed it.
fn script(calls: &[(&str, Value)]) -> String {
    calls
        .iter()
        .map(|(tool, arguments)| format!("CALL {tool} {arguments}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a clone does to show what it has: reach back into the home thread,
/// read the granted folder, and try the doors it may or may not have.
fn reach_back(granted: &Granted, doors: &[(&str, Value)]) -> String {
    let mut calls = vec![
        ("search_thread", json!({"query": "Bertha"})),
        ("list_chapters", json!({})),
        ("read", json!({"path": granted.notes()})),
    ];
    calls.extend(doors.iter().cloned());
    script(&calls)
}

/// What each tool came back with, by name, from a `RESULTS` message. A
/// result can run over several lines; each starts on a line that opens with
/// a tool's name.
fn results(said: &str) -> Vec<(String, String)> {
    let mut results: Vec<(String, String)> = Vec::new();
    for line in said.lines().skip(1) {
        let opens = line.split_once(": ").filter(|(tool, _)| {
            !tool.is_empty() && tool.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        });
        match (opens, results.last_mut()) {
            (Some((tool, result)), _) => results.push((tool.to_string(), result.to_string())),
            (None, Some((_, result))) => {
                result.push('\n');
                result.push_str(line);
            }
            (None, None) => {}
        }
    }
    results
}

fn result<'a>(results: &'a [(String, String)], tool: &str) -> &'a str {
    results
        .iter()
        .find(|(called, _)| called == tool)
        .map_or_else(
            || panic!("no result for {tool}: {results:#?}"),
            |(_, said)| said,
        )
}

/// The `RESULTS` a thread's agent wrote, once it has.
async fn said_on(log: &Log, stream: StreamId) -> Vec<(String, String)> {
    let said = || {
        log.load(&stream)
            .into_iter()
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| event["text"].as_str().map(str::to_string))
            .find(|text| text.starts_with("RESULTS"))
    };
    until(&format!("{stream:?} reports"), || said().is_some()).await;
    results(&said().unwrap())
}

/// A desk with Ada granted the folder, and something said in her DM for a
/// clone to find there.
async fn home(granted: &Granted) -> Desk {
    let (base, heard) = endpoint().await;
    let root = tempfile::tempdir().unwrap();
    let (mut client, log) = door(root.path()).await;
    let persona_id = teammate_with(&mut client, &base, &granted.workspace, granted.draft()).await;
    let mut desk = Desk {
        client,
        log,
        persona_id,
        heard,
        _roots: vec![root],
    };
    desk.say("SAY Bertha is the blue crane.").await;
    desk
}

impl Desk {
    /// One line to the DM, and its turn to its end.
    async fn say(&mut self, text: &str) {
        let started = self
            .client
            .call("session.start", json!({"personaId": self.persona_id}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
        let before = self
            .tape()
            .iter()
            .filter(|event| event["kind"] == "turn")
            .count();
        let said = self
            .client
            .call(
                "session.prompt",
                json!({"personaId": self.persona_id, "text": text}),
            )
            .await;
        assert_eq!(said["ok"], true, "{said}");
        until("the DM's turn ends", || {
            self.tape()
                .iter()
                .filter(|event| event["kind"] == "turn")
                .count()
                > before
        })
        .await;
    }

    /// The run a subagent with this title was started as.
    fn run(&self, title: &str) -> Option<String> {
        self.tape()
            .into_iter()
            .find(|event| event["kind"] == "link" && event["title"] == title)
            .and_then(|link| link["thread"].as_str().map(str::to_string))
    }
}

/// A subagent is a clone: it reaches back into the home thread with
/// `search_thread` and `list_chapters` and reads the folder its teammate is
/// granted, but is not offered the doors to the person, since its report is
/// its teammate's to tell. Left without folders by the home agent, it cannot
/// read the granted one, and still reaches back.
#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_is_a_clone_that_reaches_back_and_is_narrowed_when_asked() {
    let granted = Granted::new();
    let mut desk = home(&granted).await;
    let doors = [
        ("post_to_chat", json!({"text": "From a clone."})),
        ("request_human", json!({"reason": "Which crane?"})),
    ];
    let task = reach_back(&granted, &doors);
    desk.say(&script(&[(
        "subagent",
        json!({"task": task, "title": "Whole"}),
    )]))
    .await;
    desk.say(&script(&[(
        "subagent",
        json!({"task": task, "title": "Narrowed", "without": ["folders"]}),
    )]))
    .await;
    until("both runs started", || {
        desk.run("Whole").is_some() && desk.run("Narrowed").is_some()
    })
    .await;

    let whole = said_on(&desk.log, StreamId::Run(desk.run("Whole").unwrap())).await;
    assert!(
        result(&whole, "search_thread").contains("Bertha is the blue crane."),
        "{whole:#?}"
    );
    assert!(
        result(&whole, "list_chapters").contains("chapters"),
        "{whole:#?}"
    );
    assert!(result(&whole, "read").contains(CANARY), "{whole:#?}");
    for refused in ["post_to_chat", "request_human"] {
        let said = result(&whole, refused);
        assert!(
            !said.contains("Posted") && !said.starts_with("Asked"),
            "{refused}: {said}"
        );
    }

    let narrowed = said_on(&desk.log, StreamId::Run(desk.run("Narrowed").unwrap())).await;
    assert!(
        result(&narrowed, "search_thread").contains("Bertha is the blue crane."),
        "{narrowed:#?}"
    );
    assert!(!result(&narrowed, "read").contains(CANARY), "{narrowed:#?}");

    assert!(
        desk.chat()
            .iter()
            .all(|message| message["text"] != "From a clone."),
        "a run posts nothing"
    );
    assert!(
        desk.tape()
            .iter()
            .all(|event| event["kind"] != "human_action"),
        "a run raises no card"
    );
}

/// A job is a clone on a thread of its own: the home thread's tools but the
/// DM's chapter moves, the granted folder, and its own card, raised on its
/// thread and answered back into it. A job the teammate made without
/// folders cannot read the granted one. And the `schedule` tool is where
/// the home agent says so: the job it writes carries what it left out.
///
/// The jobs run on a copy of the first desk's data with no search index,
/// as after a restart: the index is rebuilt from the streams, and what an
/// earlier job said on its own thread is found with what the DM said.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_is_a_clone_on_its_own_thread_and_reaches_back_after_a_restart() {
    let granted = Granted::new();
    let mut first = home(&granted).await;
    first
        .say(&script(&[(
            "schedule",
            json!({"when": "1h", "prompt": "Check the cranes.", "without": ["folders"]}),
        )]))
        .await;
    let listed = first.client.call("schedule.list", json!({})).await;
    assert_eq!(
        listed["result"][0]["without"],
        json!(["folders"]),
        "{listed}"
    );
    let earlier = first
        .once("SAY Bertha moved to pier four.", json!({}))
        .await;
    until("the earlier job said it", || {
        first.kinds(&earlier).last().map(String::as_str) == Some("turn")
    })
    .await;

    // The restart: the first desk's data, its index left behind, and two
    // loops added to the room stream, due in a moment.
    let doors = [
        ("search_thread", json!({"query": "pier"})),
        ("request_human", json!({"reason": "Which crane is Bertha?"})),
        ("new_chapter", json!({})),
    ];
    let task = reach_back(&granted, &doors);
    let mut narrowed = looping("narrowed", &first.persona_id, 3_600_000, &task);
    narrowed["without"] = json!(["folders"]);
    let jobs = [
        looping("whole", &first.persona_id, 3_600_000, &task),
        narrowed,
    ];
    let second = tempfile::tempdir().unwrap();
    copy_without_index(first.log.root(), second.path());
    let mut room = std::fs::read_to_string(second.path().join("room.jsonl")).unwrap();
    for job in jobs {
        room.push_str(&job.to_string());
        room.push('\n');
    }
    std::fs::write(second.path().join("room.jsonl"), room).unwrap();
    let (client, log) = door(second.path()).await;
    let mut desk = Desk {
        client,
        log,
        persona_id: first.persona_id.clone(),
        heard: first.heard.clone(),
        _roots: vec![second],
    };

    let whole = said_on(&desk.log, StreamId::Job("whole".to_string())).await;
    assert!(
        result(&whole, "search_thread").contains("Bertha is the blue crane."),
        "the home thread: {whole:#?}"
    );
    let found = &whole
        .iter()
        .filter(|(tool, _)| tool == "search_thread")
        .nth(1)
        .unwrap()
        .1;
    assert!(
        found.contains("Bertha moved to pier four.") && found.contains(&format!("job:{earlier}")),
        "the earlier job's thread, rebuilt: {found}"
    );
    assert!(
        result(&whole, "list_chapters").contains("chapters"),
        "{whole:#?}"
    );
    assert!(result(&whole, "read").contains(CANARY), "{whole:#?}");
    assert!(
        result(&whole, "request_human").starts_with("Asked."),
        "{whole:#?}"
    );
    assert!(
        !result(&whole, "new_chapter").contains("\"closed\":true"),
        "{whole:#?}"
    );

    let narrowed = said_on(&desk.log, StreamId::Job("narrowed".to_string())).await;
    assert!(!result(&narrowed, "read").contains(CANARY), "{narrowed:#?}");

    // The card is on the job's thread, not the DM, and the answer comes
    // back to the job as a turn there.
    let card = desk
        .thread("whole")
        .into_iter()
        .find(|event| event["kind"] == "human_action")
        .expect("the job's card is on its thread");
    assert!(
        desk.tape()
            .iter()
            .all(|event| event["kind"] != "human_action"),
        "{:?}",
        desk.tape()
    );
    let answered = desk
        .client
        .call(
            "human.answer",
            json!({
                "personaId": desk.persona_id,
                "actionId": card["actionId"],
                "status": "done",
                "note": "SAY The blue one, at pier four.",
            }),
        )
        .await;
    assert_eq!(answered["ok"], true, "{answered}");
    until("the job heard the answer", || {
        desk.thread("whole")
            .iter()
            .any(|event| event["kind"] == "agent" && event["text"] == "The blue one, at pier four.")
    })
    .await;
    let delivery = desk
        .thread("whole")
        .into_iter()
        .find(|event| event["kind"] == "delivery")
        .unwrap();
    assert_eq!(delivery["from"]["kind"], "job");
    assert_eq!(delivery["receipt"], "read");
}

/// Copies a desk's data to `to`, all but its search index, which a desk that
/// starts without one rebuilds from the streams.
fn copy_without_index(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("index.sqlite") {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_without_index(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
