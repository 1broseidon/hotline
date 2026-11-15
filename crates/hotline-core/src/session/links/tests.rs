//! Two desks, linked: what crosses and what does not.
//!
//! The channel between them is stood in for here by a pump that hands each
//! call to the other room's federation handler, the way `remote/link.rs`
//! does over the sealed channel. What is under test is the rooms' half: the
//! roster each publishes, and that the inbox crosses no link.

use super::*;
use crate::driver::{MessageKind, Update};
use crate::log::StreamId;
use crate::mcp::server::TeammateTools;
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use tokio::sync::mpsc;

const LAPTOP: &str = "laptop-desk";
const GRIZZLY: &str = "grizzly-desk";

fn answers() -> Scripted {
    Scripted::turns(
        (0..20)
            .map(|i| {
                vec![
                    Update::Message {
                        kind: MessageKind::Agent,
                        id: format!("lou-{i}"),
                        text: format!("runner {i} is green"),
                    },
                    Update::Turn {
                        stop_reason: "end_turn".into(),
                        usage: None,
                    },
                ]
            })
            .collect(),
    )
}

/// A desk with one teammate, linked with the other desk by record.
fn desk(name: &str, teammate: (&str, &str), other: (&str, &str), dials: bool) -> Arc<Room> {
    desk_with(name, teammate, other, dials, Fake::new(answers()))
}

/// [`desk`], its agents being `agents`.
fn desk_with(
    name: &str,
    teammate: (&str, &str),
    other: (&str, &str),
    dials: bool,
    agents: Arc<Fake>,
) -> Arc<Room> {
    let log = scratch(name);
    let mut one = persona(teammate.0);
    one.name = teammate.1.to_string();
    enrol(&log, &one);
    linked::save(
        &log,
        &Record {
            desk_id: other.0.into(),
            name: other.1.into(),
            dials,
            key: (!dials).then(|| "a-key".to_string()),
            teammates: Vec::new(),
        },
    )
    .unwrap();
    Room::with_agents(log, Arc::new(DeskKeys), agents)
}

/// The laptop with Clementine and grizzly with Lou, linked both ways.
fn pair(name: &str) -> (Arc<Room>, Arc<Room>) {
    (
        desk(
            &format!("{name}-laptop"),
            ("clementine", "Clementine"),
            (GRIZZLY, "grizzly"),
            true,
        ),
        desk(
            &format!("{name}-grizzly"),
            ("lou", "Lou"),
            (LAPTOP, "laptop"),
            false,
        ),
    )
}

/// One direction of an open channel: `from`'s roster published to `to`, as
/// the channel does before anything else, then every call `from` makes for
/// `to_id`'s desk answered by `to`'s federation handler, as from `from_id`.
fn pump(
    from: &Arc<Room>,
    to: &Arc<Room>,
    from_id: &str,
    to_id: &str,
) -> (u64, tokio::task::JoinHandle<()>) {
    pump_saying(from, to, from_id, to_id, linked::FEATURES)
}

/// [`pump`], with `from`'s roster saying it takes `features`: none for a
/// desk older than them.
fn pump_saying(
    from: &Arc<Room>,
    to: &Arc<Room>,
    from_id: &str,
    to_id: &str,
    features: &[&str],
) -> (u64, tokio::task::JoinHandle<()>) {
    let features = features.iter().map(ToString::to_string).collect();
    to.federation_roster(from_id, from.federation_roster_entries(), features)
        .unwrap();
    let (calls, mut waiting) = mpsc::channel::<linked::Call>(32);
    let generation = from.links.opened(to_id, calls);
    let to = to.clone();
    let from_id = from_id.to_string();
    let task = tokio::spawn(async move {
        while let Some(call) = waiting.recv().await {
            let answer = match call.cmd {
                "federation.deliver" => {
                    let delivery = serde_json::from_value(call.params["delivery"].clone()).unwrap();
                    to.federation_deliver(&from_id, delivery)
                }
                "federation.unlink" => to.unlinked(&from_id).await.map(|()| Value::Null),
                other => panic!("nothing sends {other} here"),
            };
            let _ = call.answer.send(answer.map_err(linked::CallError::Refused));
        }
    });
    (generation, task)
}

/// The channel between the two, open: each side's roster published to the
/// other, and calls answered both ways for as long as the test runs.
struct Channel {
    _ends: [(u64, tokio::task::JoinHandle<()>); 2],
}

impl Channel {
    fn open(laptop: &Arc<Room>, grizzly: &Arc<Room>) -> Self {
        Self {
            _ends: [
                pump(laptop, grizzly, LAPTOP, GRIZZLY),
                pump(grizzly, laptop, GRIZZLY, LAPTOP),
            ],
        }
    }
}

async fn send(room: &Arc<Room>, from: &str, to: &str) -> Result<String, String> {
    TeammateTools::new(room, from)
        .call(
            "send",
            &json!({"to": [to], "title": "CI runners", "message": "Check the CI runners."}),
        )
        .await
}

async fn list(room: &Arc<Room>, persona_id: &str) -> Vec<Value> {
    let listed = TeammateTools::new(room, persona_id)
        .call("list_teammates", &json!({}))
        .await
        .unwrap();
    serde_json::from_str::<Value>(&listed).unwrap()["teammates"]
        .as_array()
        .unwrap()
        .clone()
}

/// The inbox does not cross a link: `send` refuses a teammate on a linked
/// desk with a sentence, and a linked desk's message or aside is refused at
/// this desk's door, leaving nothing anywhere.
#[tokio::test]
async fn the_inbox_reaches_no_linked_teammate_and_takes_nothing_from_a_linked_desk() {
    let (laptop, grizzly) = pair("inbox-only");
    let _channel = Channel::open(&laptop, &grizzly);
    let refused = send(&laptop, "clementine", "Lou").await.unwrap_err();
    assert!(refused.contains("linked desk"), "{refused}");
    assert_eq!(
        crate::store::inbox::of(&grizzly.log, "lou"),
        [] as [crate::contract::InboxItem; 0]
    );

    let request = LinkedDelivery::Request {
        request_id: "r1".into(),
        from_persona_id: "clementine".into(),
        to: "lou".into(),
        message: "Check the CI runners.".into(),
        intent: "ask".into(),
        continues: None,
        about: None,
        files: Vec::new(),
    };
    assert_eq!(
        grizzly.federation_deliver(LAPTOP, request).unwrap_err(),
        INBOX_ONLY
    );
    let aside = LinkedDelivery::Aside {
        id: "a1".into(),
        from_persona_id: "clementine".into(),
        to: "lou".into(),
        from: crate::contract::ThreadId::dm("clementine"),
        text: "psst".into(),
        level: crate::contract::AsideLevel::Nudge,
        about: None,
        request_id: None,
        files: Vec::new(),
    };
    assert_eq!(
        grizzly.federation_deliver(LAPTOP, aside).unwrap_err(),
        INBOX_ONLY
    );
    assert!(
        grizzly
            .log
            .load(&StreamId::Room)
            .iter()
            .all(|event| event["kind"] != "exchange_pair"),
        "nothing was queued"
    );
    assert_eq!(
        crate::store::asides::of(&grizzly.log, "lou"),
        [] as [crate::contract::Aside; 0]
    );
    assert_eq!(grizzly.tape("lou"), [] as [Value; 0]);
}

#[tokio::test]
async fn a_name_two_teammates_share_is_refused_with_every_choice() {
    let (laptop, grizzly) = pair("ambiguous");
    let mut local_lou = persona("lou-here");
    local_lou.name = "Lou".into();
    enrol(&laptop.log, &local_lou);
    let _channel = Channel::open(&laptop, &grizzly);

    let refused = send(&laptop, "clementine", "lou").await.unwrap_err();
    assert_eq!(
        refused,
        format!(
            "2 teammates are called 'lou': Lou on this desk (personaId lou-here); Lou on grizzly \
             (personaId {GRIZZLY}/lou). Name the one you mean by its personaId, or as Name on Desk \
             for one on a linked desk."
        )
    );
    // Each is named plainly: the one here is reached, the linked one is
    // refused as linked.
    send(&laptop, "clementine", "lou-here").await.unwrap();
    let linked = send(&laptop, "clementine", "Lou on grizzly")
        .await
        .unwrap_err();
    assert!(linked.contains("linked desk"), "{linked}");
}

/// A linked desk is sent Lou's roster row and profile — his goal, agent,
/// model and capabilities, a granted server by its name — and lists them
/// as a local colleague's; never his workspace, the server's address or
/// login, his grants' ids or his allowed senders.
#[tokio::test]
async fn a_linked_desk_is_sent_roster_rows_and_nothing_else() {
    let (laptop, grizzly) = pair("roster-fields");
    grizzly
        .log
        .append(
            &StreamId::Room,
            &json!({"kind": "setting", "id": "mcpServers", "value": [
                {"id": "docs-server-id", "name": "Docs", "type": "http",
                 "url": "https://mcp.example/sk-live-URLTOKEN",
                 "auth": {"mode": "header", "name": "X-Leaky-Header"}},
            ]}),
        )
        .unwrap();
    let mut lou = grizzly.persona("lou").unwrap();
    lou.cwd = "/Users/lou/sk-live-CWDTOKEN".into();
    lou.model_id = Some("openai/gpt-5".into());
    lou.allowed_senders = vec!["someone-allowed".into()];
    lou.mcp_policy = crate::contract::McpPolicy {
        mode: crate::contract::PolicyMode::All,
        server_ids: Vec::new(),
    };
    crate::room::append_persona(&grizzly.log, &lou).unwrap();
    let published = serde_json::to_value(grizzly.federation_roster_entries()).unwrap();
    for row in published.as_array().unwrap() {
        let mut fields: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "activity",
                "agent",
                "capabilities",
                "effort",
                "goal",
                "model",
                "name",
                "personaId",
                "state",
                "workingOn"
            ]
        );
        let mut working_on: Vec<&str> = row["workingOn"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        working_on.sort_unstable();
        assert_eq!(working_on, ["lastTurnAt", "title"]);
    }
    let sent = published.to_string();
    for never in [
        "sk-live",
        "mcp.example",
        "X-Leaky-Header",
        "docs-server-id",
        "someone-allowed",
        &*std::env::temp_dir().to_string_lossy(),
    ] {
        assert!(!sent.contains(never), "{never} crossed the link: {sent}");
    }

    let _channel = Channel::open(&laptop, &grizzly);
    let listed = list(&laptop, "clementine").await;
    assert_eq!(listed[0]["desk"], "grizzly");
    assert_eq!(listed[0]["goal"], lou.goal);
    assert_eq!(listed[0]["agent"], "Hotline Agent");
    assert_eq!(listed[0]["model"], "openai/gpt-5");
    assert_eq!(listed[0]["capabilities"]["computer"], "none");
    assert_eq!(listed[0]["capabilities"]["mcpServers"], json!(["Docs"]));
    assert_eq!(listed[0]["capabilities"]["reach"], "whole machine");
}

#[test]
fn an_answer_is_cut_to_fit_one_of_the_links_frames_however_it_escapes() {
    let fits = |reply: &str| {
        let frame = json!({"id": 1, "cmd": "federation.deliver", "params": {"delivery":
            LinkedDelivery::Result { request_id: "r".into(), reply: crossing(reply), failed: false, stopped_by_person: false }}});
        frame.to_string().len() < 1024 * 1024
    };
    assert_eq!(crossing("runner 3 is green"), "runner 3 is green");
    assert_eq!(crossing(&"x".repeat(300_000)).len(), REPLY_MAX);
    for worst in ["\"", "\u{1}", "\u{1F980}"] {
        assert!(fits(&worst.repeat(REPLY_MAX)), "{worst:?}");
    }
}
