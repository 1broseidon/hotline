//! The `browser` tool through the teammate's own tools, on a fake of the
//! app's web views: who is offered it, what a public page answers, and the
//! guard card a local address raises and each answer to it.

use super::*;
use crate::browser::{DeskBrowser, Page, Reading, Step};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

#[derive(Default)]
struct Fake {
    opened: StdMutex<Vec<String>>,
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
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        Ok(vec![0x89, b'P', b'N', b'G'])
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
