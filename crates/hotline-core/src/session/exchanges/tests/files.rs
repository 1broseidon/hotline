//! Files between teammates on one desk: read where the sender's read tools
//! find them, copied into the recipient's working directory under
//! `inbox/<sender>/` once what carries them is let in, and named in the
//! message or the aside; refused, with why, outside the sender's reach or
//! over the cap.

use super::*;
use tempfile::TempDir;

/// Ada and Bob, each working in a directory of their own with workspace
/// reach, so Ada may send only what is in hers.
fn apart(name: &str) -> (Arc<Room>, Arc<Fake>, TempDir, TempDir) {
    let (room, agents) = setup(name);
    let dirs = ["ada", "bob"].map(|id| {
        let dir = TempDir::new().unwrap();
        let mut persona = room.persona(id).unwrap();
        persona.cwd = dir.path().to_string_lossy().into_owned();
        persona.reach = Some(crate::contract::Reach::Workspace);
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

/// Ada asks Bob with a file. Nothing lands while the person's first-contact
/// card waits; once they allow it, the file is in Bob's working directory
/// under `inbox/ada/`, the question Bob is asked says where, and the desk
/// holds nothing more of it.
#[tokio::test]
async fn a_message_carries_files_into_the_recipients_inbox_once_it_is_let_in() {
    let (room, agents, ada, bob) = apart("files-message");
    std::fs::write(ada.path().join("q3.csv"), "quarter,total\nq3,12").unwrap();
    let sent = call(
        &room,
        "message_teammate",
        json!({"to": "bob", "message": "The Q3 numbers.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    let id = serde_json::from_str::<serde_json::Value>(&sent).unwrap()["requestId"]
        .as_str()
        .unwrap()
        .to_string();
    until("the person's card", || collaboration_card(&room).is_some()).await;
    let landed = bob.path().join("inbox/ada/q3.csv");
    assert!(!landed.exists(), "nothing lands before the grant");
    assert!(room.log.root().join("files-between").join(&id).exists());
    let card = collaboration_card(&room).unwrap();
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_session")
        .await
        .unwrap();
    done(&room, &id).await;
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        "quarter,total\nq3,12"
    );
    let asked = agents
        .prompts()
        .into_iter()
        .find(|prompt| prompt.contains("The Q3 numbers."))
        .expect("Bob was asked");
    assert!(
        asked.contains("Files from Ada, now in your working directory: inbox/ada/q3.csv"),
        "{asked}"
    );
    assert!(!room.log.root().join("files-between").join(&id).exists());
}

/// An aside's files land as it is left, past the same grant, and the aside
/// says where; a second file of the same name never overwrites the first.
#[tokio::test]
async fn an_aside_carries_files_into_the_recipients_inbox_and_names_where() {
    let (room, _agents, ada, bob) = apart("files-aside");
    room.allow_sender("bob", "ada").unwrap();
    std::fs::write(ada.path().join("q3.csv"), "first").unwrap();
    call(
        &room,
        "aside",
        json!({"to": "bob", "text": "The numbers are in.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    std::fs::write(ada.path().join("q3.csv"), "second").unwrap();
    call(
        &room,
        "aside",
        json!({"to": "bob", "text": "Corrected.", "files": ["q3.csv"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(bob.path().join("inbox/ada/q3.csv")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(bob.path().join("inbox/ada/q3 (2).csv")).unwrap(),
        "second"
    );
    let left = crate::store::asides::of(&room.log, "bob");
    assert_eq!(
        left[1].text,
        "Corrected.\n\nFiles from Ada, now in your working directory: inbox/ada/q3 (2).csv"
    );
}

/// What the sender's read tools cannot reach is refused, and so is more
/// than the cap together; each says why, and nothing is queued, kept or
/// copied.
#[tokio::test]
async fn files_outside_the_senders_reach_or_over_the_cap_are_refused_with_a_reason() {
    let (room, _agents, ada, bob) = apart("files-refused");
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
            "message_teammate",
            json!({"to": "bob", "message": "Look.", "files": [path]}),
        )
        .await
        .unwrap_err();
        assert_ne!(refused, "");
        let refused = call(
            &room,
            "aside",
            json!({"to": "bob", "text": "Look.", "files": [path]}),
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
        "message_teammate",
        json!({"to": "bob", "message": "Both halves.", "files": ["a.bin", "b.bin"]}),
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
            "aside",
            json!({"to": "bob", "text": "All of them.", "files": too_many}),
        )
        .await
        .unwrap_err()
        .starts_with("At most 10 files")
    );
    assert!(
        room.exchange_pair("ada~bob").is_none(),
        "nothing was queued"
    );
    assert_eq!(crate::store::asides::of(&room.log, "bob").len(), 0);
    assert!(!bob.path().join("inbox").exists());
    assert!(
        std::fs::read_dir(room.log.root().join("files-between"))
            .map_or(true, |mut held| held.next().is_none()),
        "nothing is held"
    );
}
