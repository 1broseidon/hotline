//! The `browser` tool through the teammate's own tools, on a fake of the
//! app's web views: who is offered it, what a public page answers, and the
//! guard card a local address raises and each answer to it.

use super::*;
use crate::browser::{Act, Acted, DeskBrowser, Field, FullPage, Page, Reading, Showing, Step};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

#[derive(Default)]
struct Fake {
    opened: StdMutex<Vec<String>>,
    acts: StdMutex<Vec<Act>>,
    /// Set by a load of `expired.example`, which fails as a refused
    /// certificate does.
    needs: StdMutex<Option<String>>,
    /// The names the core tagged the teammate's pointer with.
    named: StdMutex<Vec<String>>,
    /// How many viewports tall the page is, for a full-page look.
    viewports: StdMutex<u32>,
    /// The fields of a sign-in page by ref; a ref not here is a password
    /// field.
    fields: StdMutex<std::collections::HashMap<String, Field>>,
    /// What a picture of the page would show, as words.
    showing: StdMutex<(String, Vec<String>)>,
    /// What the next act answers instead of the next page: a page, a note,
    /// or an error.
    acted: StdMutex<Option<Result<Acted, String>>>,
    /// Each close the core asked for: whose, and whether it wiped.
    closed: StdMutex<Vec<(String, bool)>>,
    /// Each size `viewport` asked for, `None` for the desk's own.
    sized: StdMutex<Vec<Option<(u32, u32)>>>,
}

/// A field on a sign-in page.
fn field(kind: &str, autocomplete: &str, words: &str) -> Field {
    Field {
        tag: "input".into(),
        kind: kind.into(),
        autocomplete: autocomplete.into(),
        words: words.into(),
        with_password: true,
        ..Field::default()
    }
}

/// A blank PNG 4 wide and `height` tall, so a picture's height says which
/// one it is.
fn png(height: u32) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbImage::new(4, height)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[async_trait::async_trait]
impl DeskBrowser for Fake {
    async fn open(&self, _persona_id: &str, url: &url::Url) -> Result<Page, String> {
        self.opened.lock().unwrap().push(url.to_string());
        if url.host_str() == Some("expired.example") {
            *self.needs.lock().unwrap() = Some("Certificate expired".into());
            return Err("expired.example:443 did not open: its security certificate has expired. Your browser shows a warning in its place.".into());
        }
        *self.needs.lock().unwrap() = None;
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
            Reading::Text => {
                "Page: Example Domain\n\n# Example Domain\nWelcome back, your token is hunter22"
                    .into()
            }
            Reading::Links => "- More information -> https://iana.org/".into(),
            Reading::Title => "Example Domain".into(),
        })
    }
    async fn act(&self, _persona_id: &str, act: &Act) -> Result<Acted, String> {
        self.acts.lock().unwrap().push(act.clone());
        if let Some(acted) = self.acted.lock().unwrap().take() {
            return acted;
        }
        Ok(Acted {
            page: Page {
                url: "https://example.com/next".into(),
                title: "Next".into(),
                loaded: true,
            },
            note: Some("Filled.".into()),
        })
    }
    async fn viewport(&self, _persona_id: &str, size: Option<(u32, u32)>) -> Result<Page, String> {
        self.sized.lock().unwrap().push(size);
        Ok(Page {
            url: "https://example.com/".into(),
            title: "Example".into(),
            loaded: true,
        })
    }

    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        Ok(png(3))
    }
    /// The page's viewports from the top, the first 1 tall, the second 2.
    async fn full_page(&self, _persona_id: &str, most: usize) -> Result<FullPage, String> {
        let viewports = *self.viewports.lock().unwrap();
        let shown = viewports.min(u32::try_from(most).unwrap());
        Ok(FullPage {
            pictures: (1..=shown).map(png).collect(),
            more: viewports > shown,
        })
    }
    async fn field(&self, _persona_id: &str, target: &str) -> Result<Field, String> {
        Ok(self
            .fields
            .lock()
            .unwrap()
            .get(target)
            .cloned()
            .unwrap_or_else(|| field("password", "current-password", "password")))
    }
    async fn showing(&self, _persona_id: &str) -> Result<Showing, String> {
        let (text, fields) = self.showing.lock().unwrap().clone();
        Ok(Showing { text, fields })
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        self.opened.lock().unwrap().last().cloned()
    }
    fn needs(&self, _persona_id: &str) -> Option<String> {
        self.needs.lock().unwrap().clone()
    }
    fn name_teammate(&self, _persona_id: &str, name: &str) {
        self.named.lock().unwrap().push(name.to_string());
    }
    async fn close(&self, persona_id: &str, wipe: bool) -> Result<(), String> {
        self.opened.lock().unwrap().clear();
        self.closed
            .lock()
            .unwrap()
            .push((persona_id.to_string(), wipe));
        Ok(())
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
async fn the_browser_tags_the_teammates_pointer_with_its_name() {
    let (room, fake) = with_browser("browser-named", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    let name = ada().name;
    assert_eq!(*fake.named.lock().unwrap(), [name]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_load_that_fails_for_the_person_still_moves_the_pip_and_marks_it() {
    let (room, _fake) = with_browser("browser-needs", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    let before = room.browser_view("ada").unwrap();
    assert_eq!(before.needs, None);
    tokio::time::sleep(Duration::from_millis(5)).await;
    let failed = tools(&room)
        .call(BROWSER, &navigate("https://expired.example/"))
        .await
        .unwrap_err();
    assert!(failed.contains("certificate has expired"), "{failed}");
    let after = room.browser_view("ada").unwrap();
    assert_eq!(after.needs.as_deref(), Some("Certificate expired"));
    assert_eq!(after.url, "https://expired.example/");
    assert!(
        after.active_at > before.active_at,
        "the PIP is told it moved"
    );
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    assert_eq!(
        room.browser_view("ada").unwrap().needs,
        None,
        "the mark clears"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_address_the_person_types_opens_without_a_card_but_not_the_desks_own() {
    let (room, fake) = with_browser("browser-person-local", &[4000]);
    let local = url::Url::parse("http://localhost:2322/").unwrap();
    room.browser_person_opens("ada", &local).await.unwrap();
    let opened = tools(&room)
        .call(BROWSER, &navigate("http://localhost:2322/"))
        .await
        .unwrap();
    assert!(opened.contains("Page: Example Domain"), "{opened}");
    assert_eq!(*fake.opened.lock().unwrap(), ["http://localhost:2322/"]);
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "permission"),
        "what the person typed asks nobody"
    );
    let own = url::Url::parse("http://localhost:4000/").unwrap();
    let refused = room.browser_person_opens("ada", &own).await.unwrap_err();
    assert!(refused.contains("Hotline's own ports"), "{refused}");
    let public = url::Url::parse("https://example.com/").unwrap();
    room.browser_person_opens("ada", &public).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_desks_own_ports_never_open_and_never_ask() {
    let (room, fake) = with_browser("browser-never", &[4000]);
    // A listener of this process's opened after install, as the MCP server
    // or a sign-in callback is, is the desk's too.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let _held = crate::listening::hold(port);
    let listening = format!("http://127.0.0.1:{port}/mcp");
    for link in [
        "http://localhost:4000/",
        "http://127.0.0.1:11434/api/tags",
        // A name that resolves to this machine is this machine.
        "http://127.0.0.1.nip.io:4000/",
        "http://localhost.:4000/",
        &listening,
    ] {
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

/// A deleted teammate leaves nothing of its browser behind: the app closes
/// it and wipes its web storage, lent cookies and all, and the room lets go
/// of its picture and the person's answers about its local addresses, those
/// kept in the room included.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_teammate_closes_and_wipes_its_browser_and_forgets_its_answers() {
    let (room, fake) = with_browser("browser-delete", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    assert!(room.browser_view("ada").is_some());
    let gate = room.browser_gate();
    gate.allow_always(room.log(), "ada", "localhost:2322")
        .unwrap();
    gate.allow("ada", "localhost:5173");
    gate.allow("bob", "localhost:5173");

    room.forget("ada");
    for _ in 0..200 {
        if !fake.closed.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(*fake.closed.lock().unwrap(), [("ada".to_string(), true)]);
    assert!(
        room.browser_view("ada").is_none(),
        "the PIP has nothing left"
    );
    assert!(room.browser_snapshot("ada").is_none());
    assert!(!gate.granted("ada", "localhost:2322"));
    assert!(!gate.granted("ada", "localhost:5173"));
    assert!(gate.granted("bob", "localhost:5173"), "Bob's answer is his");
    let next = crate::browser::Gate::default();
    next.load(room.log(), &[]);
    assert!(
        !next.granted("ada", "localhost:2322"),
        "an Always allow does not outlive the teammate"
    );
}

/// A teammate whose computer is turned on has its computer's browser: the
/// desk browser closes, and its storage and the person's answers stay for
/// when the computer is turned off again, so its sign-ins move back.
#[tokio::test(flavor = "multi_thread")]
async fn a_computer_turned_on_closes_the_desk_browser_and_keeps_its_store() {
    let (room, fake) = with_browser("browser-computer-on", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    room.browser_gate().allow("ada", "localhost:2322");
    let mut ada = ada();
    ada.computer = Some(serde_json::from_value(json!({ "enabled": true })).unwrap());
    write_persona(room.log(), ada);
    room.reattach("ada").await.unwrap();
    assert_eq!(*fake.closed.lock().unwrap(), [("ada".to_string(), false)]);
    assert!(room.browser_view("ada").is_none(), "its PIP goes");
    assert!(
        room.browser_gate().granted("ada", "localhost:2322"),
        "the person's answers stay with the teammate"
    );
    // A reattach for anything else, with no computer, closes nothing.
    room.reattach("bob").await.unwrap();
    assert_eq!(fake.closed.lock().unwrap().len(), 1);
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

/// What a screenshot answers an MCP client: each picture's height, in
/// order, and the note after them.
async fn looked(room: &Arc<Room>, arguments: Value) -> (Vec<u32>, String) {
    let served = serve(tools(room)).await.unwrap();
    let connected = client()
        .serve(transport(served.url(), served.token()))
        .await
        .unwrap();
    let called = connected
        .peer()
        .call_tool(
            CallToolRequestParams::new(BROWSER)
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    connected.cancel().await.ok();
    assert_ne!(called.is_error, Some(true), "{:?}", called.content);
    let last = called.content.last().and_then(|block| block.as_text());
    let note = last.map(|text| text.text.clone()).unwrap_or_default();
    let heights = called
        .content
        .iter()
        .filter_map(|block| block.as_image())
        .map(|image| {
            assert_eq!(image.mime_type, "image/png");
            image::load_from_memory(&STANDARD.decode(&image.data).unwrap())
                .unwrap()
                .height()
        })
        .collect();
    assert_eq!(
        called.content.len(),
        called
            .content
            .iter()
            .filter(|block| block.as_image().is_some())
            .count()
            + 1,
        "pictures, then one note"
    );
    (heights, note)
}

/// A full-page look answers the page top to bottom, at most four
/// pictures, and says when the page goes on; a plain screenshot is one.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_page_look_answers_the_page_top_to_bottom() {
    let (room, fake) = with_browser("browser-full-page", &[]);
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    let full = json!({ "action": "screenshot", "full_page": true });

    *fake.viewports.lock().unwrap() = 3;
    let (heights, note) = looked(&room, full.clone()).await;
    assert_eq!(heights, [1, 2, 3]);
    assert!(note.starts_with("These pictures are yours"), "{note}");
    assert!(!note.contains("goes on"), "{note}");

    *fake.viewports.lock().unwrap() = 6;
    let (heights, note) = looked(&room, full.clone()).await;
    assert_eq!(heights, [1, 2, 3, 4]);
    assert!(
        note.ends_with("The page goes on below; scroll and look again."),
        "{note}"
    );

    *fake.viewports.lock().unwrap() = 1;
    let (heights, short) = looked(&room, full).await;
    assert_eq!(heights, [1]);
    let (heights, plain) = looked(&room, json!({ "action": "screenshot" })).await;
    assert_eq!(heights, [3]);
    assert_eq!(
        short, plain,
        "a short page answers as a plain screenshot does"
    );

    let refused = tools(&room)
        .call(
            BROWSER,
            &json!({ "action": "screenshot", "full_page": true }),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("cannot be handed a picture"), "{refused}");
}

/// A page can be seen at a phone's width: `viewport` sizes it, says the
/// size it got, goes back to the desk's own without a width, and refuses a
/// size out of range rather than clamping it.
#[tokio::test(flavor = "multi_thread")]
async fn the_page_is_sized_to_a_phone_and_back() {
    let (room, fake) = with_browser("browser-viewport", &[]);
    let phone = tools(&room)
        .call(
            BROWSER,
            &json!({ "action": "viewport", "width": 390, "height": 844 }),
        )
        .await
        .unwrap();
    assert!(phone.contains("390 by 844"), "{phone}");
    let back = tools(&room)
        .call(BROWSER, &json!({ "action": "viewport" }))
        .await
        .unwrap();
    assert!(back.contains("1280 by 800"), "{back}");
    let refused = tools(&room)
        .call(BROWSER, &json!({ "action": "viewport", "width": 40 }))
        .await
        .unwrap_err();
    assert!(refused.contains("from 240 to 3840"), "{refused}");
    assert_eq!(*fake.sized.lock().unwrap(), [Some((390, 844)), None]);
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

/// The PIP's state: a move leaves the page and a snapshot on the teammate's
/// session info, and while the person has the browser the teammate reads
/// but does not move it.
#[tokio::test(flavor = "multi_thread")]
async fn the_person_takes_control_and_the_teammate_waits() {
    let (room, fake) = with_browser("browser-take", &[]);
    assert!(room.info("ada").browser.is_none(), "no page, no PIP");
    tools(&room)
        .call(BROWSER, &navigate("https://example.com/"))
        .await
        .unwrap();
    let view = room.info("ada").browser.expect("a move is on the info");
    assert_eq!(view.phase, crate::contract::BrowserPhase::Teammate);
    assert_eq!(view.title, "Example Domain");
    assert!(view.snapshot_at.is_some());
    let snapshot = room.browser_snapshot("ada").expect("a picture for the PIP");
    assert!(snapshot.jpeg.len() > 100, "a JPEG, not nothing");

    room.browser_take("ada", true).await.unwrap();
    assert_eq!(
        room.info("ada").browser.unwrap().phase,
        crate::contract::BrowserPhase::Person
    );
    let waiting = tools(&room)
        .call(BROWSER, &json!({ "action": "click_ref", "ref": "e1" }))
        .await
        .unwrap_err();
    assert!(waiting.contains("taken over your browser"), "{waiting}");
    let refused = tools(&room)
        .call(BROWSER, &navigate("https://example.org/"))
        .await
        .unwrap_err();
    assert!(refused.contains("taken over"), "{refused}");
    let scrolls = tools(&room)
        .call(
            BROWSER,
            &json!({ "action": "screenshot", "full_page": true }),
        )
        .await
        .unwrap_err();
    assert!(scrolls.contains("taken over"), "{scrolls}");
    tools(&room)
        .call(BROWSER, &json!({ "action": "text" }))
        .await
        .expect("reading goes on while the person has it");
    assert!(fake.acts.lock().unwrap().is_empty());

    room.browser_take("ada", false).await.unwrap();
    tools(&room)
        .call(BROWSER, &json!({ "action": "click_ref", "ref": "e1" }))
        .await
        .expect("handed back, the teammate drives again");
    assert!(
        room.browser_take("bob", true).await.is_err(),
        "a browser with no page cannot be taken"
    );
}

/// A room whose vault holds GITHUB, a login for github.com with a code, and
/// Ada granted it; `ask` stores it as an ask-me key.
fn with_login(name: &str, ask: bool) -> (Arc<Room>, Arc<Fake>) {
    use crate::vault::{StoredSecret, Vault};
    let log = Log::open(scratch(name));
    let mut ada = ada();
    ada.secrets = Some(vec!["GITHUB".into()]);
    write_persona(&log, ada);
    write_persona(&log, bob());
    let vault = Arc::new(
        Vault::open_with_store(
            log.root(),
            log.clone(),
            Arc::new(crate::credentials::MemoryStore::default()),
        )
        .unwrap(),
    );
    vault
        .set_shared(
            "GITHUB",
            StoredSecret::Login {
                r#use: crate::contract::SecretUse::Auto,
                sites: vec!["https://github.com".into()],
                username: "ada".into(),
                password: "hunter22".into(),
                totp: Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()),
            },
        )
        .unwrap();
    if ask {
        vault
            .set_secret_use("GITHUB", crate::contract::SecretUse::Ask)
            .unwrap();
    }
    let room = Room::new_with_vault(log, Arc::new(NoKeys), vault);
    let fake = Arc::new(Fake::default());
    fake.fields.lock().unwrap().extend([
        ("e1".to_string(), field("text", "username", "login_field")),
        (
            "e3".to_string(),
            Field {
                inputmode: "numeric".into(),
                with_password: false,
                ..field("text", "one-time-code", "app_totp")
            },
        ),
    ]);
    room.install_browser(fake.clone(), &[]);
    (room, fake)
}

fn typed(fake: &Fake) -> Vec<String> {
    fake.acts
        .lock()
        .unwrap()
        .iter()
        .filter_map(|act| match act {
            Act::Fill { text, .. } | Act::FillSecret { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_fills_on_its_own_site_and_never_comes_back() {
    let (room, fake) = with_login("browser-login", false);
    let fill = |secret: &str| {
        let target = match secret.rsplit_once('.').map(|(_, part)| part) {
            Some("username") => "e1",
            Some("code") => "e3",
            _ => "e2",
        };
        json!({ "action": "fill", "ref": target, "secret": secret })
    };
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    let said = tools(&room)
        .call(BROWSER, &fill("GITHUB.password"))
        .await
        .unwrap();
    assert!(said.contains("Typed GITHUB.password"), "{said}");
    assert!(
        !said.contains("hunter22"),
        "the value never comes back: {said}"
    );
    tools(&room)
        .call(BROWSER, &fill("GITHUB.username"))
        .await
        .unwrap();
    tools(&room)
        .call(BROWSER, &fill("GITHUB.code"))
        .await
        .unwrap();
    let Some(Act::FillSecret { field, .. }) = fake.acts.lock().unwrap().last().cloned() else {
        panic!("a secret's fill is its own act");
    };
    assert_eq!(
        field.autocomplete, "one-time-code",
        "the page is told the field the core checked, to check again as it types"
    );
    let values = typed(&fake);
    assert_eq!(&values[..2], ["hunter22", "ada"]);
    assert!(values[2].len() == 6 && values[2].chars().all(|c| c.is_ascii_digit()));

    let read = tools(&room)
        .call(BROWSER, &json!({ "action": "text" }))
        .await
        .unwrap();
    assert!(!read.contains("hunter22"), "{read}");
    assert!(read.contains("[redacted GITHUB]"), "{read}");

    for (secret, said) in [
        ("NPM.password", "not one of your secrets"),
        ("GITHUB.pin", "NAME.username"),
    ] {
        let refused = tools(&room).call(BROWSER, &fill(secret)).await.unwrap_err();
        assert!(refused.contains(said), "{secret}: {refused}");
    }
    tools(&room)
        .call(BROWSER, &navigate("https://github.com.evil.dev/login"))
        .await
        .unwrap();
    let refused = tools(&room)
        .call(BROWSER, &fill("GITHUB.password"))
        .await
        .unwrap_err();
    assert!(refused.contains("Nothing was typed"), "{refused}");
    assert_eq!(typed(&fake).len(), 3, "a refused fill types nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_me_login_waits_for_the_persons_approval() {
    let (room, fake) = with_login("browser-ask", true);
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    let asking = {
        let tools = tools(&room);
        tokio::spawn(async move {
            tools
                .call(
                    BROWSER,
                    &json!({ "action": "fill", "ref": "e2", "secret": "GITHUB.password" }),
                )
                .await
        })
    };
    let card = guard_card(&room, "ada").await;
    let title = card["title"].as_str().unwrap();
    assert!(
        title.starts_with("Ada wants to use GITHUB on https://github.com."),
        "{title}"
    );
    assert!(!title.contains("hunter22"));
    assert!(
        typed(&fake).is_empty(),
        "nothing is typed before the answer"
    );
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "approve")
        .await
        .unwrap();
    asking.await.unwrap().unwrap();
    assert_eq!(typed(&fake), ["hunter22"]);
    // The rest of the sign-in fills without asking again.
    tools(&room)
        .call(
            BROWSER,
            &json!({ "action": "fill", "ref": "e1", "secret": "GITHUB.username" }),
        )
        .await
        .unwrap();
    let cards = tape(&room, "ada")
        .iter()
        .filter(|event| event["kind"] == "permission" && event["decision"].is_null())
        .count();
    assert_eq!(cards, 0);
}

/// A login's part is typed only into a field made for it: on the login's
/// own site, a search box, a text area or a plain text field takes none,
/// and a refused one types nothing and asks the person nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_secret_goes_only_into_a_field_made_for_its_part() {
    let (room, fake) = with_login("browser-fields", true);
    fake.fields.lock().unwrap().extend([
        (
            "e7".to_string(),
            Field {
                with_password: false,
                ..field("text", "", "q search or jump to…")
            },
        ),
        (
            "e8".to_string(),
            Field {
                tag: "textarea".into(),
                kind: String::new(),
                ..field("", "", "comment")
            },
        ),
        (
            "e9".to_string(),
            Field {
                tag: "editable".into(),
                kind: String::new(),
                ..field("", "", "")
            },
        ),
        (
            "e10".to_string(),
            Field {
                with_password: false,
                ..field("text", "", "name")
            },
        ),
    ]);
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    let fill = |target: &str, secret: &str| {
        let tools = tools(&room);
        let arguments = json!({ "action": "fill", "ref": target, "secret": secret });
        async move { tools.call(BROWSER, &arguments).await }
    };
    for (target, secret, said) in [
        ("e7", "GITHUB.password", "e7 is a search field"),
        ("e7", "GITHUB.username", "e7 is a search field"),
        ("e7", "GITHUB.code", "e7 is a search field"),
        ("e8", "GITHUB.password", "e8 is a text area"),
        (
            "e9",
            "GITHUB.password",
            "e9 is an editable part of the page",
        ),
        ("e10", "GITHUB.username", "e10 is a text field"),
        ("e1", "GITHUB.password", "goes only into a password field"),
        ("e3", "GITHUB.password", "goes only into a password field"),
        (
            "e2",
            "GITHUB.username",
            "goes only into a username or email field",
        ),
        (
            "e1",
            "GITHUB.code",
            "a one-time-code field or a password field",
        ),
    ] {
        let refused = fill(target, secret).await.unwrap_err();
        assert!(refused.contains(said), "{target} {secret}: {refused}");
        assert!(refused.ends_with("Nothing was typed."), "{refused}");
    }
    assert!(typed(&fake).is_empty(), "a refused field is typed nothing");
    assert!(
        !tape(&room, "ada")
            .iter()
            .any(|event| event["kind"] == "permission"),
        "a refused field asks the person nothing"
    );

    // A password into the password field is the sign-in it is for.
    let asking = {
        let fill = fill("e2", "GITHUB.password");
        tokio::spawn(fill)
    };
    let card = guard_card(&room, "ada").await;
    room.answer_permission("ada", card["requestId"].as_str().unwrap(), "approve")
        .await
        .unwrap();
    let said = asking.await.unwrap().unwrap();
    assert!(said.contains("Typed GITHUB.password"), "{said}");
    assert_eq!(typed(&fake), ["hunter22"]);
}

/// Whatever the page does with a value, the agent is never answered it: not
/// in the page line's address or title after the act, not in the act's
/// note, not in an error, and not encoded in a link.
#[tokio::test(flavor = "multi_thread")]
async fn a_value_never_comes_back_in_a_page_line_a_note_or_an_error() {
    let (room, fake) = with_login("browser-leaks", false);
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    let act = |arguments: Value| {
        let tools = tools(&room);
        async move { tools.call(BROWSER, &arguments).await }
    };
    *fake.acted.lock().unwrap() = Some(Ok(Acted {
        page: Page {
            url: "https://github.com/search?q=hunter22&type=code".into(),
            title: "hunter22 · Search".into(),
            loaded: true,
        },
        note: Some("Pressed in search field \"hunter22\".".into()),
    }));
    let pressed = act(json!({ "action": "press", "key": "Enter" }))
        .await
        .unwrap();
    assert!(!pressed.contains("hunter22"), "{pressed}");
    assert!(
        pressed.contains("URL: https://github.com/search?q=[redacted GITHUB]&type=code"),
        "{pressed}"
    );
    assert!(
        pressed.contains("Page: [redacted GITHUB] · Search"),
        "{pressed}"
    );
    assert!(
        pressed.contains("Pressed in search field \"[redacted GITHUB]\"."),
        "{pressed}"
    );

    // The value as a link carries it, percent-encoded, and the code the
    // login's seed makes now.
    let now = u64::try_from(crate::session::now_ms() / 1000).unwrap();
    let code = crate::browser::totp_code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", now).unwrap();
    *fake.acted.lock().unwrap() = Some(Ok(Acted {
        page: Page {
            url: format!("https://github.com/sessions/two-factor?otp={code}&p=%68unter22"),
            title: "Two-factor".into(),
            loaded: true,
        },
        note: None,
    }));
    let clicked = act(json!({ "action": "click_ref", "ref": "e4" }))
        .await
        .unwrap();
    assert!(!clicked.contains(&code), "{clicked}");
    assert!(!clicked.contains("unter22"), "{clicked}");

    *fake.acted.lock().unwrap() = Some(Err(
        "The page went on to http://localhost:4000/?p=hunter22, which your browser never opens."
            .into(),
    ));
    let refused = act(json!({ "action": "click_ref", "ref": "e4" }))
        .await
        .unwrap_err();
    assert!(!refused.contains("hunter22"), "{refused}");
    assert!(refused.contains("?p=[redacted GITHUB]"), "{refused}");

    // A secret's own fill answers its page line hidden too.
    *fake.acted.lock().unwrap() = Some(Ok(Acted {
        page: Page {
            url: "https://github.com/login?echo=hunter22".into(),
            title: "Sign in".into(),
            loaded: true,
        },
        note: None,
    }));
    let typed_in = act(json!({ "action": "fill", "ref": "e2", "secret": "GITHUB.password" }))
        .await
        .unwrap();
    assert!(!typed_in.contains("hunter22"), "{typed_in}");
}

/// No picture of the page is taken while a value shows on it outside a
/// masked password field: in a field, in the page's text, or as the code
/// being typed; a value in a password field the page keeps masked is no
/// bar.
#[tokio::test(flavor = "multi_thread")]
async fn no_picture_is_taken_while_a_secret_shows_outside_a_password_field() {
    let (room, fake) = with_login("browser-pictures", false);
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    let (heights, _) = looked(&room, json!({ "action": "screenshot" })).await;
    assert_eq!(heights, [3], "a page with no secret on it is looked at");

    let now = u64::try_from(crate::session::now_ms() / 1000).unwrap();
    let code = crate::browser::totp_code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", now).unwrap();
    for showing in [
        (String::new(), vec!["hunter22".to_string()]),
        ("Results for hunter22".to_string(), Vec::new()),
        (String::new(), vec![code]),
    ] {
        *fake.showing.lock().unwrap() = showing.clone();
        for arguments in [
            json!({ "action": "screenshot" }),
            json!({ "action": "screenshot", "full_page": true }),
        ] {
            let refused = tools(&room).call(BROWSER, &arguments).await.unwrap_err();
            assert!(
                refused.contains("no picture of it is taken"),
                "{showing:?} {arguments}: {refused}"
            );
        }
        let refused = tools(&room)
            .call(SEND_FILE, &json!({ "source": "browser" }))
            .await
            .unwrap_err();
        assert!(refused.contains("no picture of it is taken"), "{refused}");
    }
    *fake.showing.lock().unwrap() = ("Signed in as ada".into(), vec!["ada".into()]);
    let (heights, _) = looked(&room, json!({ "action": "screenshot" })).await;
    assert_eq!(heights, [3], "a username is not a secret a picture hides");
}

/// A `wait_for` the page found only inside a value answers as one that ran
/// out, so it cannot spell the value out a guess at a time.
#[tokio::test(flavor = "multi_thread")]
async fn wait_for_never_finds_text_inside_a_value() {
    let (room, fake) = with_login("browser-wait", false);
    tools(&room)
        .call(BROWSER, &navigate("https://github.com/login"))
        .await
        .unwrap();
    *fake.showing.lock().unwrap() = ("Results for hunter22 in code".into(), Vec::new());
    let wait = |text: &str| {
        let tools = tools(&room);
        let arguments = json!({ "action": "wait_for", "text": text, "timeout_ms": 50 });
        async move { tools.call(BROWSER, &arguments).await }
    };
    let missed = wait("Results for hunt").await.unwrap_err();
    assert!(missed.contains("Still not on the page"), "{missed}");
    wait("Results for").await.unwrap();
    wait("in code").await.unwrap();
}

mod passkeys;
