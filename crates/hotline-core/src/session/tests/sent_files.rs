//! A teammate handing the person a file (BRO-98), through its own tool.

use super::*;
use base64::{Engine, prelude::BASE64_STANDARD};

/// A room where Ada works in a folder of her own, in the workspace
/// reach, with a picture, a PDF and a note in it.
fn workspace(name: &str) -> (Arc<Room>, std::path::PathBuf) {
    let log = scratch(name);
    let cwd = std::env::temp_dir().join(format!(
        "hotline-core-sent-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&cwd).unwrap();
    let mut picture = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        30,
        20,
        image::Rgba([10, 120, 200, 128]),
    ))
    .write_to(&mut picture, image::ImageFormat::Png)
    .unwrap();
    std::fs::write(cwd.join("chart.png"), picture.into_inner()).unwrap();
    std::fs::write(cwd.join("report.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
    std::fs::write(cwd.join("notes.txt"), b"crane oiled\n").unwrap();
    let mut ada = persona("ada");
    ada.cwd = cwd.to_string_lossy().into_owned();
    ada.reach = Some(Reach::Workspace);
    enrol(&log, &ada);
    let room = Room::with_agents_and_computers(
        log,
        Arc::new(DeskKeys),
        Fake::new(Scripted::new(spoken_turn())),
        crate::computer::Computer::with_path(std::env::temp_dir().join("no-runtime")),
    );
    (room, cwd)
}

/// The messages on the tape that carry a file.
fn files_on(room: &Room) -> Vec<Value> {
    tape(room, "ada")
        .into_iter()
        .filter(|event| event.get("attachments").is_some())
        .collect()
}

fn kept(room: &Room) -> usize {
    std::fs::read_dir(room.log.root().join("files").join("ada")).map_or(0, Iterator::count)
}

#[tokio::test]
async fn a_picture_reaches_the_person_as_a_jpeg_with_its_caption() {
    let (room, _) = workspace("send-picture");
    let tools = TeammateTools::new(&room, "ada");
    let sent = tools
        .call(
            "send_file",
            &json!({ "source": "workspace", "path": "chart.png", "caption": " The chart " }),
        )
        .await
        .unwrap();
    assert!(sent.starts_with("Sent chart.jpg (30 × 20, "), "{sent}");

    let files = files_on(&room);
    assert_eq!(files.len(), 1);
    let message = &files[0];
    assert_eq!(message["kind"], "agent");
    assert_eq!(message["text"], "The chart");
    let attachment = &message["attachments"][0];
    assert_eq!(attachment["kind"], "image");
    assert_eq!(attachment["name"], "chart.jpg");
    assert_eq!(attachment["mimeType"], "image/jpeg");
    assert_eq!(attachment["width"], 30);
    assert_eq!(attachment["height"], 20);
    assert_eq!(attachment["origin"], "chart.png in the workspace");

    // The desk keeps its own copy, and reads it back by the message.
    let id = message["id"].as_str().unwrap();
    let path = std::path::PathBuf::from(attachment["path"].as_str().unwrap());
    assert!(path.starts_with(room.log.root().join("files").join("ada").join(id)));
    let kept = std::fs::read(&path).unwrap();
    assert!(kept.starts_with(&[0xFF, 0xD8]), "a JPEG");
    assert_eq!(attachment["size"], kept.len());
    let chunk = crate::sent::read(room.log.root(), "ada", id, 0).unwrap();
    assert_eq!(chunk.name, "chart.jpg");
    assert_eq!(chunk.mime_type, "image/jpeg");
    assert_eq!(BASE64_STANDARD.decode(chunk.data).unwrap(), kept);
    assert_eq!(chunk.next, None);
}

#[tokio::test]
async fn a_pdf_and_a_note_are_sent_as_they_are_and_remembered_by_name() {
    let (room, _) = workspace("send-files");
    let tools = TeammateTools::new(&room, "ada");
    assert_eq!(
        tools
            .call(
                "send_file",
                &json!({ "source": "workspace", "path": "./report.pdf", "caption": "" }),
            )
            .await
            .unwrap(),
        "Sent report.pdf (15 bytes)."
    );
    assert_eq!(
        tools
            .call(
                "send_file",
                &json!({ "source": "workspace", "path": "notes.txt", "caption": "Done." }),
            )
            .await
            .unwrap(),
        "Sent notes.txt (12 bytes)."
    );

    let files = files_on(&room);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["text"], "");
    assert_eq!(files[0]["attachments"][0]["kind"], "file");
    assert_eq!(files[0]["attachments"][0]["mimeType"], "application/pdf");
    assert_eq!(files[0]["attachments"][0].get("width"), None);
    assert_eq!(files[1]["attachments"][0]["mimeType"], "text/plain");
    assert_eq!(
        std::fs::read(files[1]["attachments"][0]["path"].as_str().unwrap()).unwrap(),
        b"crane oiled\n"
    );

    // The model's history, the search index and the chapter notes know
    // each file by its name, caption or not.
    assert_eq!(
        said(&tape(&room, "ada")),
        vec![Said::Agent(
            "[file: report.pdf]\n\nDone.\n[file: notes.txt]".to_string()
        )]
    );
    let hits = crate::store::search::search(room.log.root(), "ada", "report.pdf", None).unwrap();
    assert!(hits.to_string().contains("report.pdf"), "{hits}");
}

#[tokio::test]
async fn what_cannot_be_sent_is_refused_and_leaves_nothing_behind() {
    let (room, cwd) = workspace("send-refused");
    let big = std::fs::File::create(cwd.join("big.bin")).unwrap();
    big.set_len(crate::sent::MAX_BYTES + 1).unwrap();
    let tools = TeammateTools::new(&room, "ada");
    let refused = |arguments: Value| {
        let tools = tools.clone();
        async move { tools.call("send_file", &arguments).await.unwrap_err() }
    };

    assert_eq!(
        refused(json!({ "source": "workspace", "path": "big.bin" })).await,
        "big.bin is larger than the 25 MB a file can be."
    );
    assert_ne!(
        refused(json!({ "source": "workspace", "path": "/etc/hostname" })).await,
        ""
    );
    assert_ne!(
        refused(json!({ "source": "workspace", "path": "missing.txt" })).await,
        ""
    );
    assert_ne!(
        refused(json!({ "source": "workspace", "path": "." })).await,
        ""
    );
    assert_eq!(
        refused(json!({ "source": "workspace" })).await,
        "send_file needs the `path` of the file on the workspace."
    );
    assert_eq!(
        refused(json!({ "source": "mailbox", "path": "notes.txt" })).await,
        "send_file needs a `source`: `workspace`, `computer` or `screen`."
    );
    assert_eq!(
        refused(json!({
            "source": "workspace",
            "path": "notes.txt",
            "caption": "a".repeat(2001),
        }))
        .await,
        "A caption is at most 2000 characters; say the rest in your reply."
    );
    assert_eq!(
        refused(json!({ "source": "screen", "region": [0, 0, -5, 10] })).await,
        "A `region` is four whole numbers, [x, y, width, height], with a width and a height above zero."
    );
    assert_eq!(
        refused(json!({ "source": "computer", "path": "report.txt" })).await,
        "Your computer is not running, so there is nothing on it to send. `computer_status` says where it is."
    );
    assert_eq!(files_on(&room), [] as [Value; 0]);
    assert_eq!(kept(&room), 0);
}

/// A job's file is posted, stamped with the job and from its thread, as the
/// turn's one post, and nothing else the turn sends after it reaches the
/// chat. The thread keeps a line naming it.
#[tokio::test]
async fn a_jobs_file_is_its_one_post() {
    let (room, _) = workspace("send-job");
    let job = crate::contract::ScheduledRun {
        job_id: "j1".to_string(),
        kind: crate::contract::ScheduleKind::Schedule,
        name: "Crane check".to_string(),
        operator_created: false,
        quiet: Some(true),
    };
    let _turn = room.open_posting(&ThreadId::job("j1"), Some(job));
    let tools = TeammateTools::new(&room, "ada").for_job("j1");
    assert_eq!(
        tools
            .call(
                "send_file",
                &json!({ "source": "workspace", "path": "notes.txt", "caption": "The log" }),
            )
            .await
            .unwrap(),
        "Sent notes.txt (12 bytes)."
    );
    let again = tools
        .call("post_to_chat", &json!({ "text": "And a note." }))
        .await
        .unwrap_err();
    assert!(again.contains("one per turn"), "{again}");
    let again = tools
        .call(
            "send_file",
            &json!({ "source": "workspace", "path": "report.pdf" }),
        )
        .await
        .unwrap_err();
    assert!(again.contains("one per turn"), "{again}");

    let files = files_on(&room);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0]["kind"], "agent");
    assert_eq!(files[0]["text"], "The log");
    assert_eq!(files[0]["scheduled"]["jobId"], "j1");
    assert_eq!(files[0]["from"], json!({"thread": "j1", "kind": "job"}));
    assert_eq!(kept(&room), 1, "nothing refused was kept");
    let thread = room.log.load(&StreamId::Job("j1".to_string()));
    assert!(
        thread
            .iter()
            .any(|event| event["text"] == "Sent notes.txt to the chat."),
        "{thread:?}"
    );
}

/// A file sent from the DM's turn on a call is the call's, as its replies
/// are: in the chat stamped from the call, with a line on the call naming
/// it.
#[tokio::test]
async fn a_file_sent_on_a_call_is_posted_from_the_call() {
    let (room, _) = workspace("send-on-call");
    room.start("ada").await.unwrap();
    let session = lock(&room.sessions).get("ada").cloned().unwrap();
    *lock(&session.voice_origin) = Some(crate::voice::Origin {
        call_id: "c1".to_string(),
        seq: 2,
        direct: true,
        front: false,
    });
    let sent = TeammateTools::new(&room, "ada")
        .call(
            "send_file",
            &json!({ "source": "workspace", "path": "report.pdf" }),
        )
        .await
        .unwrap();
    assert_eq!(sent, "Sent report.pdf (15 bytes).");

    let files = files_on(&room);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0]["from"], json!({"thread": "c1", "kind": "call"}));
    let call = room
        .log
        .load(&StreamId::of_thread(&ThreadId::call("c1")).unwrap());
    assert!(
        call.iter()
            .any(|event| event["kind"] == "notice"
                && event["text"] == "Sent report.pdf to the chat."),
        "{call:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_file_and_the_screen_come_off_the_computer() {
    let ComputerRoom { room, .. } =
        computer_room("send-computer", Some("0.10.3"), TWO_RELEASES, true).await;
    room.start("ada").await.unwrap();
    let tools = TeammateTools::new(&room, "ada");
    assert_eq!(
        tools
            .call(
                "send_file",
                &json!({ "source": "computer", "path": "report.txt", "caption": "The numbers" }),
            )
            .await
            .unwrap(),
        "Sent report.txt (18 bytes)."
    );
    let shot = tools
        .call(
            "send_file",
            &json!({ "source": "screen", "window": "", "region": [0, 0, 0, 0] }),
        )
        .await
        .unwrap();
    assert!(shot.starts_with("Sent screenshot.jpg (8 × 6, "), "{shot}");
    let refused = tools
        .call(
            "send_file",
            &json!({ "source": "screen", "window": "Nowhere" }),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("no window is"), "{refused}");

    let files = files_on(&room);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["text"], "The numbers");
    assert_eq!(
        files[0]["attachments"][0]["origin"],
        "/home/agent/report.txt on the computer"
    );
    assert_eq!(
        std::fs::read(files[0]["attachments"][0]["path"].as_str().unwrap()).unwrap(),
        crate::computer::guide::fake::FILE_BODY
    );
    assert_eq!(files[1]["attachments"][0]["kind"], "image");
    assert_eq!(
        files[1]["attachments"][0]["origin"],
        "the computer's screen"
    );
}

/// A file sent from a work thread is in the chat all the same, stamped with
/// the thread, and the thread keeps a line naming it.
#[tokio::test]
async fn a_file_from_a_work_thread_is_posted_to_the_chat() {
    let (room, _) = workspace("send-from-thread");
    let sent = room
        .send_file(
            "ada",
            crate::session::files::Source::Workspace("report.pdf".into()),
            "The report",
            None,
            &ThreadId::side("side-1"),
        )
        .await
        .unwrap();
    assert_eq!(sent, "Sent report.pdf (15 bytes).");

    let files = files_on(&room);
    assert_eq!(files.len(), 1, "the file is a message of the chat");
    assert_eq!(files[0]["text"], "The report");
    assert_eq!(
        files[0]["from"],
        json!({"thread": "side-1", "kind": "side"})
    );
    let id = files[0]["id"].as_str().unwrap();
    assert_eq!(
        crate::sent::read(room.log.root(), "ada", id, 0)
            .unwrap()
            .name,
        "report.pdf"
    );
    let thread = room.log.load(&StreamId::Side("side-1".into()));
    assert!(
        thread
            .iter()
            .any(|event| event["kind"] == "notice"
                && event["text"] == "Sent report.pdf to the chat."),
        "{thread:?}"
    );
    // The model, reading the chat back, knows where it was sent from.
    assert_eq!(
        said(&tape(&room, "ada")),
        vec![Said::Agent(
            "(From your work thread (thread side:side-1)) The report\n[file: report.pdf]"
                .to_string()
        )]
    );
}

#[tokio::test]
async fn automatic_call_files_keep_the_call_lease_reach_and_work_boundary() {
    let (room, cwd) = workspace("automatic-call-files");
    room.start("ada").await.unwrap();
    let lease = room.capability_lease("ada").scoped();
    let first = ThreadId::call("first");
    let changed = crate::contract::ChangedFile {
        path: "report.pdf".into(),
        created: false,
        added: None,
        removed: None,
    };
    room.post_call_files("ada", &first, std::slice::from_ref(&changed), &lease)
        .await;
    assert_eq!(files_on(&room).len(), 1);
    std::fs::write(cwd.join("report.pdf"), b"%PDF-1.4\nupdated").unwrap();
    room.post_call_files("ada", &first, std::slice::from_ref(&changed), &lease)
        .await;
    assert_eq!(
        files_on(&room).len(),
        2,
        "a later work result can deliver a correction"
    );
    let delivered = files_on(&room);
    assert_eq!(
        std::fs::read(delivered[1]["attachments"][0]["path"].as_str().unwrap()).unwrap(),
        b"%PDF-1.4\nupdated"
    );
    let second = ThreadId::call("second");
    room.post_call_files("ada", &second, &[], &lease).await;
    assert_eq!(
        files_on(&room).len(),
        2,
        "another call does not replay old records"
    );
    room.post_call_files("ada", &second, std::slice::from_ref(&changed), &lease)
        .await;
    assert_eq!(files_on(&room).len(), 3, "another call owns its own result");
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("secret.txt");
    std::fs::write(&path, "private").unwrap();
    room.post_call_files(
        "ada",
        &second,
        &[crate::contract::ChangedFile {
            path: path.display().to_string(),
            created: true,
            added: None,
            removed: None,
        }],
        &lease,
    )
    .await;
    assert_eq!(
        files_on(&room).len(),
        3,
        "a reported path cannot expand reach"
    );
    lease.revoke();
    room.post_call_files("ada", &ThreadId::call("revoked"), &[changed], &lease)
        .await;
    assert_eq!(
        files_on(&room).len(),
        3,
        "a revoked call cannot read or post"
    );
}
