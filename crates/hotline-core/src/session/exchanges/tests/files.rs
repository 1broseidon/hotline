//! Files between teammates on one desk: read where the sender's read tools
//! find them, copied into the recipient's received folder on the desk,
//! `received/<recipient>/<sender>/`, outside its working directory, once the
//! item carrying them is let in, and named in the item; refused, with why,
//! outside the sender's reach or over the cap.

use super::*;
use crate::contract::Reach;
use std::path::Path;
use tempfile::TempDir;

/// Ada and Bob, each working in a directory of their own, Ada with her own
/// folders, so she may send only what is in hers, and Bob with the whole
/// machine, so her items to him wait for the person's card.
fn apart(name: &str) -> (Arc<Room>, Arc<Fake>, TempDir, TempDir) {
    let (room, agents) = setup(name);
    let dirs = [("ada", Reach::Workspace), ("bob", Reach::Machine)].map(|(id, reach)| {
        let dir = TempDir::new().unwrap();
        let mut persona = room.persona(id).unwrap();
        persona.cwd = dir.path().to_string_lossy().into_owned();
        persona.reach = Some(reach);
        crate::room::append_persona(&room.log, &persona).unwrap();
        dir
    });
    let [ada, bob] = dirs;
    (room, agents, ada, bob)
}

async fn call(
    room: &Arc<Room>,
    name: &str,
    arguments: serde_json::Value,
) -> Result<String, String> {
    TeammateTools::new(room, "ada").call(name, &arguments).await
}

fn collaboration_card(room: &Room) -> Option<serde_json::Value> {
    room.tape("ada").into_iter().find(|v| {
        v["kind"] == "permission"
            && v["requestId"]
                .as_str()
                .is_some_and(|id| id.starts_with(crate::session::peers::COLLAB_REQUEST_PREFIX))
            && v.get("decision").is_none()
    })
}

/// Where Bob's file from Ada called `name` is kept, spelled as the item
/// names it.
fn from_ada(room: &Room, name: &str) -> std::path::PathBuf {
    let folder = crate::session::files_between::received_dir(room.log.root(), "bob");
    dunce::canonicalize(&folder)
        .unwrap_or(folder)
        .join("ada")
        .join(name)
}

/// The line an item gains for Bob's files from Ada.
fn told(room: &Room, names: &[&str]) -> String {
    let paths: Vec<std::path::PathBuf> = names.iter().map(|name| from_ada(room, name)).collect();
    crate::session::files_between::files_line("Ada", &paths)
}

/// The item Bob has by its id.
fn kept(room: &Room, id: &str) -> crate::contract::InboxItem {
    crate::store::inbox::of(&room.log, "bob")
        .into_iter()
        .find(|item| item.id == id)
        .unwrap()
}

/// Ada sends Bob a file. Nothing lands while the person's first-contact
/// card waits; once they allow it, the file is in Bob's received folder
/// under `ada/`, nothing is in his working directory, the item says where,
/// and the desk holds nothing more of it.
#[tokio::test]
async fn an_item_carries_files_into_the_recipients_inbox_once_it_is_let_in() {
    let (room, _agents, ada, bob) = apart("files-message");
    std::fs::write(ada.path().join("q3.csv"), "quarter,total\nq3,12").unwrap();
    let sent = crate::session::tests::send_past_card(
        &room,
        "ada",
        json!({"to": ["bob"], "title": "Q3", "message": "The Q3 numbers.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    let id = sent["sent"][0]["id"].as_str().unwrap().to_string();
    until("the person's card", || collaboration_card(&room).is_some()).await;
    let landed = from_ada(&room, "q3.csv");
    assert!(!landed.exists(), "nothing lands before the grant");
    let held = room
        .log
        .root()
        .join("files-between")
        .join(format!("inbox-{id}"));
    assert!(held.exists());
    let card = collaboration_card(&room).unwrap();
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow")
        .await
        .unwrap();
    until("it lands", || !kept(&room, &id).held).await;
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        "quarter,total\nq3,12"
    );
    assert_eq!(
        kept(&room, &id).body,
        format!("The Q3 numbers.\n\n{}", told(&room, &["q3.csv"]))
    );
    assert!(!held.exists());
    assert!(
        !bob.path().join("inbox").exists(),
        "nothing is in his project"
    );
}

/// An item between two who may already reach each other lands its files as
/// it is sent; a second file of the same name never overwrites the first.
#[tokio::test]
async fn an_allowed_items_files_land_at_once_and_never_over_another() {
    let (room, _agents, ada, _bob) = apart("files-aside");
    room.allow_sender("bob", "ada").unwrap();
    std::fs::write(ada.path().join("q3.csv"), "first").unwrap();
    call(
        &room,
        "send",
        json!({"to": ["bob"], "title": "Numbers", "message": "The numbers are in.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    std::fs::write(ada.path().join("q3.csv"), "second").unwrap();
    let sent = call(
        &room,
        "send",
        json!({"to": ["bob"], "title": "Corrected", "message": "Corrected.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(from_ada(&room, "q3.csv")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(from_ada(&room, "q3 (2).csv")).unwrap(),
        "second"
    );
    let id = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["sent"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        kept(&room, &id).body,
        format!("Corrected.\n\n{}", told(&room, &["q3 (2).csv"]))
    );
}

/// What the sender's read tools cannot reach is refused, and so is more
/// than the cap together; each says why, and nothing is kept or copied.
#[tokio::test]
async fn files_outside_the_senders_reach_or_over_the_cap_are_refused_with_a_reason() {
    let (room, _agents, ada, _bob) = apart("files-refused");
    room.allow_sender("bob", "ada").unwrap();
    let elsewhere = TempDir::new().unwrap();
    let outside = elsewhere.path().join("secret.txt");
    std::fs::write(&outside, "not Ada's").unwrap();
    for path in [
        outside.to_string_lossy().into_owned(),
        "../secret.txt".into(),
    ] {
        let refused = call(
            &room,
            "send",
            json!({"to": ["bob"], "title": "Look", "files": [path]}),
        )
        .await
        .unwrap_err();
        assert_ne!(refused, "");
    }
    let half = crate::sent::MAX_BYTES as usize / 2 + 1;
    std::fs::write(ada.path().join("a.bin"), vec![0u8; half]).unwrap();
    std::fs::write(ada.path().join("b.bin"), vec![0u8; half]).unwrap();
    let refused = call(
        &room,
        "send",
        json!({"to": ["bob"], "title": "Both halves", "files": ["a.bin", "b.bin"]}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refused,
        "Files to a teammate come to at most 25 MB together, and these come to more."
    );
    let too_many: Vec<String> = (0..=crate::session::files_between::FILES_COUNT_MAX)
        .map(|n| format!("{n}.txt"))
        .collect();
    assert!(
        call(
            &room,
            "send",
            json!({"to": ["bob"], "title": "All of them", "files": too_many}),
        )
        .await
        .unwrap_err()
        .starts_with("At most 10 files")
    );
    assert!(
        crate::store::inbox::of(&room.log, "bob").is_empty(),
        "nothing was kept"
    );
    assert!(!from_ada(&room, "").exists());
    assert!(
        std::fs::read_dir(room.log.root().join("files-between"))
            .map_or(true, |mut held| held.next().is_none()),
        "nothing is held"
    );
}

/// A job its home agent made `without` folders reads, for what it sends a
/// colleague, only what its narrowing reaches: the working directory, never
/// a granted folder nor, for a whole-machine teammate, the rest of the
/// machine. The teammate's own conversation still sends the same file.
#[tokio::test]
async fn a_narrowed_job_sends_no_file_from_beyond_its_narrowing() {
    for reach in [
        crate::contract::Reach::Workspace,
        crate::contract::Reach::Machine,
    ] {
        let (room, _agents, ada, _bob) = apart(&format!("files-narrowed-{reach:?}"));
        room.allow_sender("bob", "ada").unwrap();
        let granted = TempDir::new().unwrap();
        let folder = granted.path().canonicalize().unwrap();
        std::fs::write(folder.join("plan.md"), "the plan").unwrap();
        std::fs::write(ada.path().join("notes.md"), "the notes").unwrap();
        let mut persona = room.persona("ada").unwrap();
        persona.reach = Some(reach);
        persona.folders = Some(vec![crate::contract::FolderGrant {
            identity: None,
            path: folder.to_string_lossy().into_owned(),
            writable: false,
        }]);
        crate::room::append_persona(&room.log, &persona).unwrap();
        let job: crate::contract::ScheduledJob = serde_json::from_value(json!({
            "id": "without-folders", "personaId": "ada", "kind": "schedule",
            "prompt": "Send Bob the plan", "without": ["folders"],
            "nextAt": now_ms() + 60_000, "createdAt": now_ms(),
        }))
        .unwrap();
        crate::room::append_schedule(&room.log, &job).unwrap();
        // What a colleague once sent Ada is kept outside her working
        // directory too, and a job without folders reaches none of it.
        let grant = crate::session::files_between::received_grant(room.log.root(), "ada").unwrap();
        let brief = Path::new(&grant.path).join("carol").join("brief.md");
        std::fs::create_dir_all(brief.parent().unwrap()).unwrap();
        std::fs::write(&brief, "carol's brief").unwrap();
        let brief = brief.to_string_lossy().into_owned();
        let from_job = TeammateTools::new(&room, "ada").for_job(&job.id);
        let plan = folder.join("plan.md").to_string_lossy().into_owned();
        for beyond in [&plan, &brief] {
            let refused = from_job
                .call(
                    "send",
                    &json!({"to": ["bob"], "title": "The plan", "files": [beyond]}),
                )
                .await
                .unwrap_err();
            assert_ne!(refused, "", "{reach:?}");
        }
        assert!(
            crate::store::inbox::of(&room.log, "bob").is_empty(),
            "{reach:?}: nothing was sent"
        );
        // Inside its narrowing it sends, and the home conversation, which
        // is not narrowed, sends the granted folder's file.
        from_job
            .call(
                "send",
                &json!({"to": ["bob"], "title": "The notes", "files": ["notes.md"]}),
            )
            .await
            .unwrap();
        call(
            &room,
            "send",
            json!({"to": ["bob"], "title": "The plan", "files": [plan, brief]}),
        )
        .await
        .unwrap();
        assert!(from_ada(&room, "notes.md").exists(), "{reach:?}");
        assert!(from_ada(&room, "plan.md").exists(), "{reach:?}");
        assert!(from_ada(&room, "brief.md").exists(), "{reach:?}");
    }
}

/// Bob's own session is handed his received folder read-only: Hotline
/// Agent's file tools, opened on the view the room starts his agent with,
/// read what Ada sent him there and change nothing in it, and reach no
/// other teammate's received folder. Whole-machine reach reads it too. His
/// preamble lists only the person's folders, and his working directory gains
/// nothing; a file an older build put there stays.
#[tokio::test]
async fn a_receiver_reads_what_it_was_sent_and_changes_nothing_there() {
    let (room, agents, ada, bob) = apart("files-read-only");
    crate::room::append_persona(&room.log, &persona("carol")).unwrap();
    room.allow_sender("bob", "ada").unwrap();
    room.allow_sender("carol", "ada").unwrap();
    std::fs::create_dir_all(bob.path().join("inbox/ada")).unwrap();
    std::fs::write(bob.path().join("inbox/ada/old.csv"), "from before").unwrap();
    std::fs::write(ada.path().join("brief.md"), "the brief").unwrap();
    std::fs::write(ada.path().join("private.md"), "for carol").unwrap();
    for (to, file) in [("bob", "brief.md"), ("carol", "private.md")] {
        call(
            &room,
            "send",
            json!({"to": [to], "title": "For you", "files": [file]}),
        )
        .await
        .unwrap();
    }
    let landed = from_ada(&room, "brief.md");
    let carols = crate::session::files_between::received_dir(room.log.root(), "carol")
        .join("ada")
        .join("private.md");
    assert_eq!(std::fs::read_to_string(&carols).unwrap(), "for carol");
    assert_eq!(
        std::fs::read_to_string(bob.path().join("inbox/ada/old.csv")).unwrap(),
        "from before"
    );
    assert!(!bob.path().join("inbox/ada/brief.md").exists());

    for reach in [Reach::Workspace, Reach::Machine] {
        let mut record = room.persona("bob").unwrap();
        record.reach = Some(reach);
        crate::room::append_persona(&room.log, &record).unwrap();
        room.start("bob").await.unwrap();
        let view = agents.views().pop().unwrap();
        room.stop("bob").unwrap();
        let grant = view.folders.clone().unwrap_or_default();
        // Which directory it is, the session reads for itself when it starts.
        let unbound: Vec<_> = grant
            .iter()
            .cloned()
            .map(|grant| crate::contract::FolderGrant {
                identity: None,
                ..grant
            })
            .collect();
        assert_eq!(
            unbound,
            [crate::contract::FolderGrant {
                identity: None,
                path: landed
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                writable: false,
            }],
            "{reach:?}"
        );
        assert!(
            !agents
                .preambles
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains("received"),
            "the preamble lists the person's folders alone"
        );
        // As Hotline Agent opens its file tools and confined shell.
        let tools = crate::tools::Workspace::open_with_capability(
            std::path::PathBuf::from(&view.cwd),
            reach,
            room.log.root().join("tool-output").join("bob"),
            &grant,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            tools.read_text_path(&landed, None, None).unwrap(),
            "the brief",
            "{reach:?}"
        );
        if reach == Reach::Machine {
            continue;
        }
        let over = tools.write_text_path(&landed, "changed").unwrap_err();
        assert!(over.to_string().contains("read but not change"), "{over}");
        let beside = landed.with_file_name("new.md");
        assert!(tools.write_text_path(&beside, "new").is_err());
        assert!(tools.read_text_path(&carols, None, None).is_err());
        assert_eq!(std::fs::read_to_string(&landed).unwrap(), "the brief");
        assert!(!beside.exists());
    }
}

/// A name or a sender's id that would climb out of the received folder
/// lands nothing anywhere: a file's name must be one plain name, and a
/// sender's id is made one, so the files stay in the recipient's own
/// folder under a folder of the sender's.
#[tokio::test]
async fn a_name_or_a_sender_that_would_leave_the_received_folder_is_refused_or_kept_inside() {
    let (room, _agents, _ada, _bob) = apart("files-traversal");
    let received = room.log.root().join("received");
    for name in ["../../escape.txt", "..", "a/b.txt", "../carol/x.txt"] {
        assert!(
            room.deliver_files("ada", "bob", &[(name.to_string(), b"x".to_vec())])
                .is_err(),
            "{name}"
        );
    }
    let landed = room
        .deliver_files("../carol", "bob", &[("x.txt".to_string(), b"x".to_vec())])
        .unwrap();
    let bobs = dunce::canonicalize(received.join("bob")).unwrap();
    assert_eq!(landed.len(), 1);
    assert!(landed[0].starts_with(&bobs), "{landed:?}");
    assert_eq!(landed[0].parent().unwrap().parent().unwrap(), bobs);
    assert!(!received.join("carol").exists());
    assert!(!room.log.root().join("escape.txt").exists());
}
