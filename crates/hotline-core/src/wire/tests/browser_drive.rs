//! The person takes a teammate's desk browser over from a seat away from the
//! desk, drives it and hands it back, through the real seat gate and command
//! handler, on a fake of the app's web views: who may, what waits while they
//! do, and how fresh the picture they drive by is.
use super::*;
use crate::browser::{Act, Acted, DeskBrowser, Page, Reading, Step};
use crate::contract::BrowserInput;
use crate::desk::Desk;
use crate::mcp::server::TeammateTools;
use crate::session::tests::{enrol, persona};
use std::sync::Mutex as StdMutex;

/// The app's web views, as far as the person's driving reaches them. Each
/// input, load and step changes the page, so its next picture differs.
#[derive(Default)]
struct Browser {
    page: StdMutex<Option<String>>,
    /// What the person sent, in order.
    inputs: StdMutex<Vec<BrowserInput>>,
    steps: StdMutex<Vec<Step>>,
    /// Each take-over and hand-back the core told the pane of.
    taken: StdMutex<Vec<bool>>,
    /// The teammate's acts that reached the page.
    acts: StdMutex<Vec<Act>>,
    /// How many times the page was pictured.
    pictures: StdMutex<u32>,
    /// Bumped by whatever changes the page.
    changes: StdMutex<u32>,
}

impl Browser {
    fn changed(&self) {
        *self.changes.lock().unwrap() += 1;
    }

    fn here(&self) -> Page {
        Page {
            url: self.page.lock().unwrap().clone().unwrap_or_default(),
            title: "Example Domain".into(),
            loaded: true,
        }
    }
}

#[async_trait::async_trait]
impl DeskBrowser for Browser {
    async fn open(&self, _persona_id: &str, url: &url::Url) -> Result<Page, String> {
        *self.page.lock().unwrap() = Some(url.to_string());
        self.changed();
        Ok(self.here())
    }
    async fn step(&self, _persona_id: &str, step: Step) -> Result<Page, String> {
        self.steps.lock().unwrap().push(step);
        self.changed();
        Ok(self.here())
    }
    async fn read(&self, _persona_id: &str, _reading: Reading) -> Result<String, String> {
        Ok("Example Domain".into())
    }
    async fn act(&self, _persona_id: &str, act: &Act) -> Result<Acted, String> {
        self.acts.lock().unwrap().push(act.clone());
        self.changed();
        Ok(Acted {
            page: self.here(),
            note: None,
        })
    }
    /// A picture 8 wide whose height says how often the page changed.
    async fn screenshot(&self, _persona_id: &str) -> Result<Vec<u8>, String> {
        *self.pictures.lock().unwrap() += 1;
        let height = 4 + *self.changes.lock().unwrap();
        let mut png = Vec::new();
        image::RgbImage::new(8, height)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        Ok(png)
    }
    async fn page_size(&self, _persona_id: &str) -> Option<(u32, u32)> {
        Some((1280, 800))
    }
    async fn person_input(&self, _persona_id: &str, input: &BrowserInput) -> Result<(), String> {
        if matches!(input, BrowserInput::Text { text } if text == "nowhere") {
            return Err("Nothing on the page has the focus: tap a field first.".into());
        }
        self.inputs.lock().unwrap().push(input.clone());
        self.changed();
        Ok(())
    }
    fn taken(&self, _persona_id: &str, taken: bool) {
        self.taken.lock().unwrap().push(taken);
    }
    fn current(&self, _persona_id: &str) -> Option<String> {
        self.page.lock().unwrap().clone()
    }
    async fn close(&self, _persona_id: &str, _wipe: bool) -> Result<(), String> {
        *self.page.lock().unwrap() = None;
        Ok(())
    }
}

/// A desk with Ada, who has no computer, and the fake browser installed
/// when `browser` is given: a desk with no window when it is not.
fn browser_desk(
    browser: Option<Arc<Browser>>,
) -> (tempfile::TempDir, Arc<Room>, Arc<dyn RoomHandle>) {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    enrol(&log, &persona("ada"));
    let room = Room::new(log.clone(), Arc::new(NoKeys));
    if let Some(browser) = browser {
        room.install_browser(browser, &[4545]);
    }
    let desk: Arc<dyn RoomHandle> = Arc::new(Desk::with_room(log, room.clone()));
    (root, room, desk)
}

async fn command(
    desk: &Arc<dyn RoomHandle>,
    room: &Room,
    seat: Seat,
    cmd: &str,
    params: Value,
) -> Value {
    remote_control_answer(
        seat,
        desk,
        room.log(),
        &Parts::default(),
        json!({"id": 1, "cmd": cmd, "params": params}),
    )
    .await
}

/// The teammate's own `browser` call.
async fn teammate(room: &Arc<Room>, arguments: Value) -> Result<String, String> {
    TeammateTools::new(room, "ada")
        .call("browser", &arguments)
        .await
}

/// What a refused command said.
fn refusal(answer: &Value) -> String {
    assert_eq!(answer["ok"], false, "{answer}");
    answer["error"]["message"]
        .as_str()
        .or_else(|| answer["error"].as_str())
        .unwrap_or_else(|| panic!("no refusal in {answer}"))
        .to_string()
}

/// Every command a seat drives the browser with, on Ada.
fn driving() -> Vec<(&'static str, Value)> {
    vec![
        ("browser.take_over", json!({"personaId": "ada"})),
        (
            "browser.input",
            json!({"personaId": "ada", "input": {"kind": "tap", "x": 10, "y": 20}}),
        ),
        ("browser.step", json!({"personaId": "ada", "step": "back"})),
        (
            "browser.go",
            json!({"personaId": "ada", "url": "example.org"}),
        ),
        ("browser.hand_back", json!({"personaId": "ada"})),
    ]
}

#[test]
fn only_the_owner_and_the_desk_take_a_browser_over_and_drive_it() {
    let persona_id = || "ada".to_string();
    let commands = [
        Command::BrowserTakeOver {
            persona_id: persona_id(),
        },
        Command::BrowserHandBack {
            persona_id: persona_id(),
        },
        Command::BrowserInput {
            persona_id: persona_id(),
            input: BrowserInput::Key {
                key: "Enter".into(),
            },
        },
        Command::BrowserStep {
            persona_id: persona_id(),
            step: crate::contract::BrowserStep::Reload,
        },
        Command::BrowserGo {
            persona_id: persona_id(),
            url: "example.com".into(),
        },
    ];
    for command in &commands {
        assert!(Seat::Desk.permits(command), "{command:?}");
        assert!(Seat::Owner.permits(command), "{command:?}");
        assert!(!Seat::Phone.permits(command), "{command:?}");
        assert!(!Seat::Link.permits(command), "{command:?}");
    }
    // Every seat that sees the PIP polls its picture, fresh or not.
    assert!(Seat::Phone.permits(&Command::BrowserSnapshot {
        persona_id: persona_id(),
        fresh: Some(true),
    }));
    assert!(Seat::Owner.capabilities().contains(&"browserDrive"));
    assert!(!Seat::Phone.capabilities().contains(&"browserDrive"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_companion_phone_is_refused_every_driving_command() {
    let browser = Arc::new(Browser::default());
    let (_root, room, desk) = browser_desk(Some(browser.clone()));
    teammate(
        &room,
        json!({"action": "navigate", "url": "https://example.com/"}),
    )
    .await
    .unwrap();
    for (cmd, params) in driving() {
        let answer = command(&desk, &room, Seat::Phone, cmd, params).await;
        assert_eq!(answer["ok"], false, "{cmd}: {answer}");
    }
    assert_eq!(
        room.browser_view("ada").unwrap().phase,
        crate::contract::BrowserPhase::Teammate
    );
    assert!(browser.inputs.lock().unwrap().is_empty());
    assert!(browser.taken.lock().unwrap().is_empty());
}

/// The owner's phone, end to end: refused until it takes over, then a tap,
/// typing, a key, a scroll, a step and an address reach the page, each
/// answered with the page pictured after it, while the teammate's moves
/// wait as they do for the desk's take-over; handed back, the teammate
/// drives again and the phone's input is refused again.
#[tokio::test(flavor = "multi_thread")]
async fn the_owners_phone_takes_over_taps_types_and_hands_back() {
    let browser = Arc::new(Browser::default());
    let (_root, room, desk) = browser_desk(Some(browser.clone()));
    teammate(
        &room,
        json!({"action": "navigate", "url": "https://example.com/"}),
    )
    .await
    .unwrap();
    let owner = |cmd: &'static str, params: Value| {
        let (desk, room) = (desk.clone(), room.clone());
        async move { command(&desk, &room, Seat::Owner, cmd, params).await }
    };
    let tap = json!({"personaId": "ada", "input": {"kind": "tap", "x": 640.5, "y": 120}});

    let early = refusal(&owner("browser.input", tap.clone()).await);
    assert_eq!(early, "Take over first: the teammate is driving.");
    for (cmd, params) in [
        (
            "browser.step",
            json!({"personaId": "ada", "step": "reload"}),
        ),
        (
            "browser.go",
            json!({"personaId": "ada", "url": "example.org"}),
        ),
    ] {
        assert_eq!(
            refusal(&owner(cmd, params).await),
            "Take over first: the teammate is driving.",
            "{cmd}"
        );
    }
    assert!(browser.inputs.lock().unwrap().is_empty());

    let taken = owner("browser.take_over", json!({"personaId": "ada"})).await;
    assert_eq!(taken["ok"], true, "{taken}");
    assert_eq!(taken["result"]["phase"], "person");
    assert_eq!(
        *browser.taken.lock().unwrap(),
        [true],
        "the pane's shield goes"
    );
    assert_eq!(
        room.info("ada").browser.unwrap().phase,
        crate::contract::BrowserPhase::Person,
        "the desk's PIP and pane read the same phase"
    );

    // The teammate reads but does not move the page while the phone has it.
    let waiting = teammate(&room, json!({"action": "click_ref", "ref": "e1"}))
        .await
        .unwrap_err();
    assert!(waiting.contains("taken over your browser"), "{waiting}");
    let navigating = teammate(
        &room,
        json!({"action": "navigate", "url": "https://example.net/"}),
    )
    .await
    .unwrap_err();
    assert!(navigating.contains("taken over"), "{navigating}");
    teammate(&room, json!({"action": "text"}))
        .await
        .expect("reading goes on while the person has it");
    assert!(browser.acts.lock().unwrap().is_empty());

    let tapped = owner("browser.input", tap).await;
    assert_eq!(tapped["ok"], true, "{tapped}");
    assert!(tapped["result"]["jpeg"].as_str().unwrap().len() > 100);
    assert_eq!(
        tapped["result"]["size"],
        json!({"width": 1280, "height": 800})
    );
    let typed = owner(
        "browser.input",
        json!({"personaId": "ada", "input": {"kind": "text", "text": "hello there"}}),
    )
    .await;
    assert_eq!(typed["ok"], true, "{typed}");
    assert!(typed["result"]["at"].as_i64().unwrap() >= tapped["result"]["at"].as_i64().unwrap());
    for input in [
        json!({"kind": "key", "key": "Enter"}),
        json!({"kind": "scroll", "dx": 0, "dy": 600}),
        json!({"kind": "scroll", "dx": -40, "dy": 0, "x": 100, "y": 100}),
    ] {
        let answer = owner("browser.input", json!({"personaId": "ada", "input": input})).await;
        assert_eq!(answer["ok"], true, "{input}: {answer}");
    }
    assert_eq!(
        *browser.inputs.lock().unwrap(),
        [
            BrowserInput::Tap { x: 640.5, y: 120.0 },
            BrowserInput::Text {
                text: "hello there".into()
            },
            BrowserInput::Key {
                key: "Enter".into()
            },
            BrowserInput::Scroll {
                dx: 0.0,
                dy: 600.0,
                x: None,
                y: None
            },
            BrowserInput::Scroll {
                dx: -40.0,
                dy: 0.0,
                x: Some(100.0),
                y: Some(100.0)
            },
        ]
    );
    // What the page said goes back to the phone.
    let unfocused = owner(
        "browser.input",
        json!({"personaId": "ada", "input": {"kind": "text", "text": "nowhere"}}),
    )
    .await;
    assert!(refusal(&unfocused).contains("tap a field first"));

    let back = owner("browser.step", json!({"personaId": "ada", "step": "back"})).await;
    assert_eq!(back["ok"], true, "{back}");
    assert_eq!(*browser.steps.lock().unwrap(), [Step::Back]);
    let went = owner(
        "browser.go",
        json!({"personaId": "ada", "url": " example.org/docs "}),
    )
    .await;
    assert_eq!(went["ok"], true, "{went}");
    assert_eq!(went["result"]["url"], "https://example.org/docs");
    assert_eq!(
        browser.current("ada").as_deref(),
        Some("https://example.org/docs")
    );

    let handed = owner("browser.hand_back", json!({"personaId": "ada"})).await;
    assert_eq!(handed["ok"], true, "{handed}");
    assert_eq!(handed["result"]["phase"], "teammate");
    assert_eq!(*browser.taken.lock().unwrap(), [true, false]);
    teammate(&room, json!({"action": "click_ref", "ref": "e1"}))
        .await
        .expect("handed back, the teammate drives again");
    assert_eq!(browser.acts.lock().unwrap().len(), 1);
    let late = owner(
        "browser.input",
        json!({"personaId": "ada", "input": {"kind": "key", "key": "Tab"}}),
    )
    .await;
    assert_eq!(refusal(&late), "Take over first: the teammate is driving.");
}

/// An address from away goes as one typed in the pane's bar does: the
/// desk's own ports and what is not the web never open, whoever asks.
/// Input of the wrong shape is refused before it reaches the page.
#[tokio::test(flavor = "multi_thread")]
async fn an_address_from_away_meets_the_gate_and_bad_input_reaches_nothing() {
    let browser = Arc::new(Browser::default());
    let (_root, room, desk) = browser_desk(Some(browser.clone()));
    teammate(
        &room,
        json!({"action": "navigate", "url": "https://example.com/"}),
    )
    .await
    .unwrap();
    let owner = |cmd: &'static str, params: Value| {
        let (desk, room) = (desk.clone(), room.clone());
        async move { command(&desk, &room, Seat::Owner, cmd, params).await }
    };
    assert_eq!(
        owner("browser.take_over", json!({"personaId": "ada"})).await["ok"],
        true
    );
    for url in [
        "http://127.0.0.1:4545/",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        let refused = owner("browser.go", json!({"personaId": "ada", "url": url})).await;
        assert_eq!(refused["ok"], false, "{url}: {refused}");
    }
    assert!(
        refusal(
            &owner(
                "browser.go",
                json!({"personaId": "ada", "url": "http://localhost:4545"})
            )
            .await
        )
        .contains("Hotline's own ports")
    );
    // A local address the person types is theirs to open, with no card.
    let local = owner(
        "browser.go",
        json!({"personaId": "ada", "url": "http://localhost:3000/"}),
    )
    .await;
    assert_eq!(local["ok"], true, "{local}");
    assert_eq!(
        browser.current("ada").as_deref(),
        Some("http://localhost:3000/")
    );

    for input in [
        json!({"kind": "tap", "x": -1, "y": 10}),
        json!({"kind": "scroll", "dx": 0, "dy": 1e9}),
        json!({"kind": "scroll", "dx": 0, "dy": 10, "x": 5}),
        json!({"kind": "text", "text": ""}),
        json!({"kind": "text", "text": "x".repeat(10_001)}),
        json!({"kind": "key", "key": "F13"}),
    ] {
        let refused = owner("browser.input", json!({"personaId": "ada", "input": input})).await;
        assert_eq!(refused["ok"], false, "{input}: {refused}");
    }
    assert!(browser.inputs.lock().unwrap().is_empty());
}

/// A desk with no browser (`hotline serve`, a Mac before 14) and a
/// teammate whose browser has no page answer plainly, and change nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_with_no_browser_or_no_page_says_so() {
    let (_root, room, desk) = browser_desk(None);
    for (cmd, params) in driving() {
        let answer = command(&desk, &room, Seat::Owner, cmd, params).await;
        assert_eq!(refusal(&answer), "This desk keeps no browser.", "{cmd}");
    }
    let browser = Arc::new(Browser::default());
    let (_root, room, desk) = browser_desk(Some(browser.clone()));
    for (cmd, params) in driving() {
        let answer = command(&desk, &room, Seat::Owner, cmd, params).await;
        assert_eq!(
            refusal(&answer),
            "That teammate's browser has no page open.",
            "{cmd}"
        );
    }
    let picture = command(
        &desk,
        &room,
        Seat::Owner,
        "browser.snapshot",
        json!({"personaId": "ada", "fresh": true}),
    )
    .await;
    assert_eq!(picture["ok"], false, "{picture}");
    assert!(browser.taken.lock().unwrap().is_empty());
}

/// A fresh picture is taken only while the person has the browser, at
/// most a few times a second however many seats poll, and a still page
/// keeps its picture's time and wakes no watcher.
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_picture_is_taken_only_while_the_person_drives() {
    let browser = Arc::new(Browser::default());
    let (_root, room, desk) = browser_desk(Some(browser.clone()));
    teammate(
        &room,
        json!({"action": "navigate", "url": "https://example.com/"}),
    )
    .await
    .unwrap();
    let fresh = || {
        let (desk, room) = (desk.clone(), room.clone());
        async move {
            let answer = command(
                &desk,
                &room,
                Seat::Owner,
                "browser.snapshot",
                json!({"personaId": "ada", "fresh": true}),
            )
            .await;
            assert_eq!(answer["ok"], true, "{answer}");
            answer["result"].clone()
        }
    };
    let pictures = || *browser.pictures.lock().unwrap();

    // While the teammate drives, a poll answers the picture of its last move.
    let before = pictures();
    let first = fresh().await;
    assert_eq!(pictures(), before, "nobody drives, nothing is pictured");

    room.browser_take("ada", true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut changes = room.subscribe_info();
    let before = pictures();
    // Two seats polling at once: one picture between them.
    let (still, again) = tokio::join!(fresh(), fresh());
    assert_eq!(
        pictures(),
        before + 1,
        "pictured again, once, for the driver"
    );
    // The page did not change: the same picture, as old as it was.
    assert_eq!(still["at"], room.browser_snapshot("ada").unwrap().at);
    assert_eq!(again["at"], still["at"]);
    assert!(
        changes.try_recv().is_err(),
        "a still page wakes nobody watching the session"
    );

    // The page changed on its own: the next poll past the interval sees it,
    // and the session's watchers (the desk's PIP) hear of it.
    browser.changed();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let moved = fresh().await;
    assert_eq!(pictures(), before + 2);
    assert!(
        moved["at"].as_i64() > still["at"].as_i64(),
        "{moved} {still}"
    );
    assert_ne!(moved["jpeg"], still["jpeg"]);
    assert!(
        changes.try_recv().is_ok(),
        "the PIP hears of a changed page"
    );
    assert_ne!(first["jpeg"], moved["jpeg"]);

    // Handed back, polls answer the last move's picture again.
    room.browser_take("ada", false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = pictures();
    fresh().await;
    assert_eq!(pictures(), before);
}
