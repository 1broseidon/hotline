use super::*;

/// A file a teammate sent is part of the conversation the phone reads, so
/// the phone reads the file too — by its message, never by a path.
#[test]
fn the_phone_seat_reads_a_sent_file_by_its_message() {
    assert!(Seat::Phone.permits(&Command::FileRead {
        persona_id: "ada".to_string(),
        event_id: "e1".to_string(),
        index: None,
        offset: 0,
        size: None,
    }));
}

#[tokio::test]
async fn a_sent_file_is_read_by_its_message_a_part_at_a_time() {
    let (root, _log, port) = door("file-read");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let kept = paths::sent_file_dir(&root, id, "e1").unwrap();
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("report.txt"), b"quarterly numbers\n").unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "file.read", "params": { "personaId": id, "eventId": "e1" } }),
    )
    .await;
    let read = answered(&mut socket, 2).await;
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["result"]["name"], "report.txt");
    assert_eq!(read["result"]["mimeType"], "text/plain");
    assert_eq!(read["result"]["size"], 18);
    assert_eq!(read["result"]["offset"], 0);
    assert_eq!(read["result"]["data"], "cXVhcnRlcmx5IG51bWJlcnMK");
    assert_eq!(read["result"].get("next"), None);

    ask(
        &mut socket,
        json!({ "id": 3, "cmd": "file.read", "params": { "personaId": id, "eventId": "e1", "offset": 10 } }),
    )
    .await;
    let rest = answered(&mut socket, 3).await;
    assert_eq!(rest["result"]["offset"], 10);
    assert_eq!(rest["result"]["data"], "bnVtYmVycwo=");

    for (n, params, error) in [
        (
            4,
            json!({ "personaId": id, "eventId": "e2" }),
            "That message has no file.",
        ),
        (
            5,
            json!({ "personaId": id, "eventId": "../e1" }),
            "That message has no file.",
        ),
        (
            6,
            json!({ "personaId": id, "eventId": "e1", "offset": 19 }),
            "The file is 18 bytes; 19 is not a place in it.",
        ),
        (
            7,
            json!({ "personaId": "nobody", "eventId": "e1" }),
            "There is no teammate nobody.",
        ),
        (
            8,
            json!({ "personaId": id, "eventId": "e1", "index": 1 }),
            "A teammate's file has only attachment index zero.",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "file.read", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"], error);
    }
}

/// A picture is no more private than the name beside it, so every seat reads
/// one, and only by the hash the roster names.
#[tokio::test]
async fn a_teammates_picture_is_read_by_its_hash_and_by_every_seat() {
    let command = Command::AvatarRead {
        persona_id: "ada".to_string(),
        hash: "0".repeat(64),
        offset: 0,
        size: None,
    };
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        assert!(seat.permits(&command));
    }

    let (root, _log, port) = door("avatar-read");
    let mut socket = desk(port).await;
    let ada = create(&mut socket, 1, "Ada").await;
    let id = ada["id"].as_str().unwrap();
    let hash = "ab".repeat(32);
    let path = paths::avatar_path(&root, id, &hash).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(b"pixels");
    std::fs::write(&path, &png).unwrap();

    ask(
        &mut socket,
        json!({ "id": 2, "cmd": "avatar.read", "params": { "personaId": id, "hash": hash } }),
    )
    .await;
    let read = answered(&mut socket, 2).await;
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["result"]["mimeType"], "image/png");
    assert_eq!(read["result"]["size"], png.len());
    assert_eq!(read["result"].get("next"), None);

    for (n, params, error) in [
        (
            3,
            json!({ "personaId": id, "hash": "../../room.jsonl" }),
            "A picture is named by 64 lowercase hex digits.",
        ),
        (
            4,
            json!({ "personaId": id, "hash": "AB".repeat(32) }),
            "A picture is named by 64 lowercase hex digits.",
        ),
        (
            5,
            json!({ "personaId": id, "hash": "cd".repeat(32) }),
            "That teammate has no such picture.",
        ),
        (
            6,
            json!({ "personaId": "nobody", "hash": hash }),
            "There is no teammate nobody.",
        ),
    ] {
        ask(
            &mut socket,
            json!({ "id": n, "cmd": "avatar.read", "params": params }),
        )
        .await;
        let refused = answered(&mut socket, n).await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"], error);
    }
}
