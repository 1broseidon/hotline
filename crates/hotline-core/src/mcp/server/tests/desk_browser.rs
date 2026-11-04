//! The `browser` tool through the teammate's own tools, on a fake of the
//! app's web views: who is offered it, what a public page answers, and the
//! guard card a local address raises and each answer to it.

use super::*;
use crate::browser::{Act, Acted, DeskBrowser, Page, Reading, Step};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

#[derive(Default)]
struct Fake {
    opened: StdMutex<Vec<String>>,
    acts: StdMutex<Vec<Act>>,
}

#[async_trait::async_trait]
impl DeskBrowser for Fake {
    async fn open(&self, _persona_id: &str, url: &url::Url) -> Result<Page, String> {
        self.opened.lock().unwrap().push(url.to_string());
        Ok(Page {
            url: url.to_string(),
            title: "Example Domain".into(),
            loaded: true,
        })
    }
    async fn step(&self, _persona_id: &str, _step: Step) -> Result<Page, String> {
        Ok(Page {
            url: "https://example.com/".into(),
            title: "Example Domain".into(),
            loaded: true,
        })
    }
    async fn read(&self, _persona_id: &str, reading: Reading) -> Result<String, String> {
        Ok(match reading {
            Reading::Text => "Page: Example Domain\n\n# Example Domain".into(),
            Reading::Links => "- More information -> https://iana.org/".into(),
        })
    }
    async fn act(&self, _persona_id: &str, act: &Act) -> Result<Acted, String> {
        self.acts.lock().unwrap().push(act.clone());
        Ok(Acted {
            page: Page {
                url: "https://example.com/next".into(),
                title: "Next".into(),
                loaded: true,
            },
            note: Some("Filled.".into()),
        })
    }
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        let mut png = Vec::new();
        image::RgbImage::new(4, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        Ok(png)
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        self.opened.lock().unwrap().last().cloned()
    }
}

fn with_browser(name: &str, never: &[u16]) -> (Arc<Room>, Arc<Fake>) {
    let room = room_with_two(name);
    let fake = Arc::new(Fake::default());
    room.install_browser(fake.clone(), never);
    (room, fake)
}

fn offered(tools: &TeammateTools) -> bool {
    tools.offered().iter().any(|tool| tool.name == BROWSER)
}

fn tape(room: &Room, persona_id: &str) -> Vec<Value> {
    room.log().load(&StreamId::Tape(persona_id.to_string()))
}

/// The guard card on a teammate's tape, once it is up.
async fn guard_card(room: &Room, persona_id: &str) -> Value {
    for _ in 0..200 {
        if let Some(card) = tape(room, persona_id).into_iter().rev().find(|event| {
            event["kind"] == "permission"
                && event["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("browser:"))
        }) {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no guard card came up for {persona_id}");
}

fn navigate(link: &str) -> Value {
    json!({ "action": "navigate", "url": link })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_browser_is_offered_with_a_desk_browser_and_no_computer() {
    let room = room_with_two("browser-offered");
    assert!(
        !offered(&tools(&room)),
        "a desk with no window has no browser to offer"
    );
    room.install_browser(Arc::new(Fake::default()), &[]);
    assert!(offered(&tools(&room)));
    assert!(tools(&room).tool_names().contains(&BROWSER));
    let mut ada = ada();
    ada.computer = Some(serde_json::from_value(json!({ "enabled": true })).unwrap());
    write_persona(room.log(), ada);
    assert!(
        !offered(&tools(&room)),
        "a teammate with a computer uses its computer's browser"
    );
    let refused = tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap_err();
    assert!(refused.contains("no desk browser"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_page_opens_and_reads_without_a_card() {
    let (room, fake) = with_browser("browser-public", &[]);
    let opened = tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    assert!(opened.contains("Page: Example Domain"), "{opened}");
    assert!(opened.contains("URL: https://example.com/"), "{opened}");
    let text = tools(&room)
        .call(BROWSER, &json!({ "action": "text" }))
        .await
        .unwrap();
    assert!(text.contains("# Example Domain"), "{text}");
    assert_eq!(*fake.opened.lock().unwrap(), ["https://example.com/"]);
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "permission"),
        "a public page asks the person nothing"
    );
    let refused = tools(&room)
        .call(BROWSER, &navigate("file:///etc/passwd"))
        .await
        .unwrap_err();
    assert!(refused.contains("http or https"), "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_page_waits_for_allow_and_then_opens_for_that_teammate_only() {
    let (room, fake) = with_browser("browser-allow", &[]);
    let asking = {
        let tools = tools(&room);
        tokio::spawn(async move {
            tools
                .call(BROWSER, &navigate("http://localhost:2322/"))
                .await
        })
    };
    let card = guard_card(&room, "ada").await;
    let title = card["title"].as_str().unwrap();
    assert!(
        title.starts_with("Ada wants to open localhost:2322 in its browser."),
        "{title}"
    );
    let names: Vec<&str> = card["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| option["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Allow", "Always allow localhost:2322", "Deny"]);
    assert!(
        fake.opened.lock().unwrap().is_empty(),
        "nothing loads before the answer"
    );
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow")
        .await
        .unwrap();
    let opened = asking.await.unwrap().unwrap();
    assert!(opened.contains("URL: http://localhost:2322/"), "{opened}");

    // Allowed for this run: the next visit asks nothing.
    tools(&room)
        .call(BROWSER, &navigate("http://localhost:2322/settings"))
        .await
        .unwrap();
    let cards = room
        .log()
        .load(&StreamId::Tape("ada".into()))
        .iter()
        .filter(|event| event["kind"] == "permission")
        .filter(|event| event["decision"].is_null())
        .count();
    assert_eq!(cards, 0, "no card is left open");

    // Ada's answer is not Bob's, and Bob's denial is a refusal.
    let bob = TeammateTools::new(&room, "bob");
    let asking =
        tokio::spawn(async move { bob.call(BROWSER, &navigate("http://localhost:2322/")).await });
    let card = guard_card(&room, "bob").await;
    room.answer_permission("bob", card["requestId"].as_str().unwrap(), "deny")
        .await
        .unwrap();
    let refused = asking.await.unwrap().unwrap_err();
    assert!(refused.contains("denied"), "{refused}");
    let decided = room
        .log()
        .load(&StreamId::Tape("bob".into()))
        .into_iter()
        .rev()
        .find(|event| event["kind"] == "permission")
        .unwrap();
    assert_eq!(decided["decidedOptionName"], "Deny");
}

#[tokio::test(flavor = "multi_thread")]
async fn always_allow_is_kept_in_the_room() {
    let (room, _) = with_browser("browser-always", &[]);
    let asking = {
        let tools = tools(&room);
        tokio::spawn(async move {
            tools
                .call(BROWSER, &navigate("http://127.0.0.1:5173/"))
                .await
        })
    };
    let card = guard_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "allow_always")
        .await
        .unwrap();
    asking.await.unwrap().unwrap();
    let kept = room.log().load(&StreamId::Room).into_iter().any(|event| {
        event["kind"] == "browser_local_allowed"
            && event["persona"] == "ada"
            && event["origin"] == "127.0.0.1:5173"
    });
    assert!(kept, "Always allow is written to the room");
    let gate = crate::browser::Gate::default();
    gate.load(room.log(), &[]);
    assert!(
        gate.granted("ada", "127.0.0.1:5173"),
        "and read back on the next run"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_desks_own_ports_never_open_and_never_ask() {
    let (room, fake) = with_browser("browser-never", &[4000]);
    for link in ["http://localhost:4000/", "http://127.0.0.1:11434/api/tags"] {
        let refused = tools(&room)
            .call(BROWSER, &navigate(link))
            .await
            .unwrap_err();
        assert!(refused.contains("Hotline's own ports"), "{link}: {refused}");
    }
    assert!(fake.opened.lock().unwrap().is_empty());
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "permission"),
        "a port that never opens raises no card to wear the person down"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_run_and_a_voice_front_have_no_browser() {
    let (room, _) = with_browser("browser-run", &[]);
    let mut run = tools(&room);
    run.run = true;
    assert!(!offered(&run));
    assert!(
        run.call(BROWSER, &json!({ "action": "text" }))
            .await
            .is_err()
    );
    let mut front = tools(&room);
    front.front = true;
    assert!(!offered(&front));
}

/// The screenshot a teammate looks at is its own; `send_file` with source
/// `browser` is how the person sees the page.
#[tokio::test(flavor = "multi_thread")]
async fn send_file_posts_the_browser_page_to_the_person() {
    let (room, _) = with_browser("browser-send", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    let sent = tools(&room)
        .call(
            SEND_FILE,
            &json!({ "source": "browser", "caption": "The page." }),
        )
        .await
        .unwrap();
    assert!(sent.contains("browser"), "{sent}");
    let posted = tape(&room, "ada")
        .into_iter()
        .any(|event| event["kind"] == "agent" && event["text"] == "The page.");
    assert!(posted, "the page is a message in the chat");

    let mut ada = ada();
    ada.computer = Some(serde_json::from_value(json!({ "enabled": true })).unwrap());
    write_persona(room.log(), ada);
    let refused = tools(&room)
        .call(SEND_FILE, &json!({ "source": "browser" }))
        .await
        .unwrap_err();
    assert!(refused.contains("no desk browser"), "{refused}");
}

/// Each act reaches the browser in the shape it was asked for, and one
/// that is malformed never touches the page.
#[tokio::test(flavor = "multi_thread")]
async fn acts_reach_the_browser_checked() {
    let (room, fake) = with_browser("browser-acts", &[]);
    let call = |arguments: Value| {
        let tools = tools(&room);
        async move { tools.call(BROWSER, &arguments).await }
    };
    let filled = call(json!({ "action": "fill", "ref": "e3", "text": "QA" }))
        .await
        .unwrap();
    assert!(filled.starts_with("Filled.\nPage: Next"), "{filled}");
    assert!(filled.contains("Read the page again"), "{filled}");
    call(json!({ "action": "click_ref", "ref": "e4" }))
        .await
        .unwrap();
    call(json!({ "action": "check", "ref": "e5", "uncheck": true }))
        .await
        .unwrap();
    call(json!({ "action": "select", "ref": "e6", "value": "Blue" }))
        .await
        .unwrap();
    call(json!({ "action": "press", "key": "Enter" }))
        .await
        .unwrap();
    call(json!({ "action": "wait_for", "text": "Saved", "timeout_ms": 90000 }))
        .await
        .unwrap();
    assert_eq!(
        *fake.acts.lock().unwrap(),
        [
            Act::Fill {
                target: "e3".into(),
                text: "QA".into()
            },
            Act::Click {
                target: "e4".into()
            },
            Act::Check {
                target: "e5".into(),
                on: false
            },
            Act::Select {
                target: "e6".into(),
                values: vec!["Blue".into()]
            },
            Act::Press {
                key: "Enter".into()
            },
            Act::WaitFor {
                text: Some("Saved".into()),
                target: None,
                timeout_ms: 15_000
            },
        ]
    );
    for (arguments, said) in [
        (json!({ "action": "click_ref" }), "needs the `ref`"),
        (json!({ "action": "fill", "ref": "e3" }), "needs `text`"),
        (
            json!({ "action": "press", "key": "Hyper" }),
            "press needs a `key`",
        ),
        (
            json!({ "action": "select", "ref": "e6" }),
            "needs a `value`",
        ),
        (json!({ "action": "wait_for" }), "needs `text`"),
    ] {
        let refused = call(arguments.clone()).await.unwrap_err();
        assert!(refused.contains(said), "{arguments}: {refused}");
    }
    assert_eq!(
        fake.acts.lock().unwrap().len(),
        6,
        "a refused act touches nothing"
    );
}
