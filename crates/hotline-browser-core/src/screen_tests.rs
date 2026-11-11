use super::*;

#[test]
fn a_frame_leads_with_its_geometry_in_little_endian_u16s() {
    let frame = Frame {
        rect: Rect::new(1, 2, 3, 4),
        screen: (1280, 800),
        png: vec![0x89, b'P'],
    };
    let bytes = frame.encode();
    assert_eq!(
        bytes,
        vec![1, 0, 2, 0, 3, 0, 4, 0, 0x00, 0x05, 0x20, 0x03, 0x89, b'P']
    );
    assert_eq!(
        Frame::header(&bytes),
        Some((Rect::new(1, 2, 3, 4), (1280, 800)))
    );
}

#[test]
fn a_header_that_is_short_empty_or_off_its_screen_is_no_frame() {
    assert_eq!(Frame::header(&[1, 0, 2, 0]), None);
    let off = Frame {
        rect: Rect::new(1270, 0, 20, 10),
        screen: (1280, 800),
        png: Vec::new(),
    };
    assert_eq!(Frame::header(&off.encode()), None);
    let empty = Frame {
        rect: Rect::new(0, 0, 0, 10),
        screen: (1280, 800),
        png: Vec::new(),
    };
    assert_eq!(Frame::header(&empty.encode()), None);
}

#[test]
fn two_changes_are_sent_as_their_bounding_box_clipped_to_the_screen() {
    let both = Rect::new(10, 10, 10, 10).union(Rect::new(15, 0, 30, 5));
    assert_eq!(both, Rect::new(10, 0, 35, 20));
    assert_eq!(
        Rect::default().union(both),
        both,
        "nothing yet is no change"
    );
    assert_eq!(
        Rect::new(1200, 700, 200, 200).clip(1280, 800),
        Some(Rect::new(1200, 700, 80, 100))
    );
    assert_eq!(Rect::new(1300, 0, 10, 10).clip(1280, 800), None);
    assert!(Rect::new(0, 0, 10, 10).intersects(Rect::new(9, 9, 5, 5)));
    assert!(!Rect::new(0, 0, 10, 10).intersects(Rect::new(10, 0, 5, 5)));
}

#[test]
fn a_viewer_speaks_the_computers_messages() {
    let cases = [
        (r#"{"t":"ping","id":7}"#, FromViewer::Ping { id: 7 }),
        (
            r#"{"t":"control","take":true}"#,
            FromViewer::Control { take: true },
        ),
        (
            r#"{"t":"move","x":10,"y":-2}"#,
            FromViewer::Move { x: 10, y: -2 },
        ),
        (
            r#"{"t":"button","b":1,"down":true}"#,
            FromViewer::Button { b: 1, down: true },
        ),
        (
            r#"{"t":"wheel","dy":1}"#,
            FromViewer::Wheel { dx: 0, dy: 1 },
        ),
        (
            r#"{"t":"key","key":"Enter","down":false}"#,
            FromViewer::Key {
                key: "Enter".into(),
                down: false,
            },
        ),
        (
            r#"{"t":"paste","text":"héllo"}"#,
            FromViewer::Paste {
                text: "héllo".into(),
            },
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(FromViewer::parse(text), Ok(expected), "{text}");
    }
    assert!(!FromViewer::Ping { id: 1 }.is_input());
    assert!(!FromViewer::Control { take: false }.is_input());
    assert!(FromViewer::Move { x: 0, y: 0 }.is_input());
}

#[test]
fn a_viewer_message_outside_the_protocol_is_refused() {
    for text in [
        "not json",
        r#"{"t":"files","id":1}"#,
        r#"{"t":"move","x":1}"#,
        r#"{"t":"button","b":9,"down":true}"#,
        r#"{"t":"wheel","dy":5}"#,
        r#"{"t":"key","key":"","down":true}"#,
        r#"{"t":"control","take":true,"extra":1}"#,
    ] {
        assert!(FromViewer::parse(text).is_err(), "{text}");
    }
    let long = serde_json::json!({"t": "paste", "text": "a".repeat(PASTE_LIMIT + 1)}).to_string();
    assert_eq!(
        FromViewer::parse(&long),
        Err("Paste exceeds 1 MiB".to_string())
    );
    let nul = serde_json::json!({"t": "paste", "text": "a\0b"}).to_string();
    assert!(FromViewer::parse(&nul).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn messages_cross_the_helpers_socket_whole_and_in_order() {
    let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
    let frame = Frame {
        rect: Rect::new(0, 0, 2, 2),
        screen: (2, 2),
        png: vec![7; 70_000],
    }
    .encode();
    let sent = frame.clone();
    // A socket's buffer is smaller than the frame: the reader reads as the
    // writer writes.
    tokio::spawn(async move {
        write_message(&mut a, TEXT, b"{\"t\":\"pointer\"}")
            .await
            .unwrap();
        write_message(&mut a, BINARY, &sent).await.unwrap();
    });
    assert_eq!(
        read_message(&mut b).await.unwrap(),
        Some((TEXT, b"{\"t\":\"pointer\"}".to_vec()))
    );
    assert_eq!(read_message(&mut b).await.unwrap(), Some((BINARY, frame)));
    assert_eq!(read_message(&mut b).await.unwrap(), None, "the end");
}

#[cfg(unix)]
#[tokio::test]
async fn a_message_of_no_known_kind_or_past_the_limit_ends_the_socket() {
    use tokio::io::AsyncWriteExt;
    let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
    a.write_all(&[9, 1, 0, 0, 0, 0]).await.unwrap();
    assert!(read_message(&mut b).await.is_err());
    let (mut a, mut b) = tokio::net::UnixStream::pair().unwrap();
    let mut head = vec![BINARY];
    head.extend(((MESSAGE_LIMIT + 1) as u32).to_le_bytes());
    a.write_all(&head).await.unwrap();
    assert!(read_message(&mut b).await.is_err());
}

/// A viewer's link holds one shown message: while the viewer does not take
/// it, the helper's next write waits instead of piling pictures up.
#[cfg(unix)]
#[tokio::test]
async fn a_slow_viewer_holds_the_helper_back_rather_than_queueing_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let helper = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (kind, hello) = read_message(&mut socket).await.unwrap().unwrap();
        assert_eq!(kind, TEXT);
        let hello: Hello = serde_json::from_slice(&hello).unwrap();
        assert_eq!(hello.persona, "ada");
        let input = read_message(&mut socket).await.unwrap().unwrap();
        assert_eq!(input, (TEXT, RELEASE.as_bytes().to_vec()));
        // Big pictures: the socket's buffers fill and a write waits. The link
        // takes the first before its viewer takes anything, so that write
        // ends however long a slow machine takes over it; only a later write
        // that waits is the helper held back.
        let picture = vec![1u8; 4 * 1024 * 1024];
        write_message(&mut socket, BINARY, &picture).await.unwrap();
        let mut sent = 1;
        while tokio::time::timeout(
            std::time::Duration::from_millis(300),
            write_message(&mut socket, BINARY, &picture),
        )
        .await
        .is_ok()
        {
            sent += 1;
            assert!(sent < 8, "the helper was never held back");
        }
    });
    let mut link = connect(&path, "ada").await.unwrap();
    link.input.send(RELEASE.to_string()).await.unwrap();
    helper.await.unwrap();
    assert!(matches!(link.shown.recv().await, Some(Shown::Frame(_))));
}
