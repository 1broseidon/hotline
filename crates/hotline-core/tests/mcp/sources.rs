//! The pages a step opened, through the real core: an ACP child reports its
//! own steps as Claude Code and Cursor reported them on George's desk
//! (2026-11-12), and each is written on the tape with the `sources` the
//! window counts. A page its browser went to is opened, a search's results
//! are only found, and a click or a failure opened nothing. The scripted
//! desk is `post_to_chat.rs`'s; `STEP <json>` is the fixture's own step.

use super::post_to_chat::{Driver, open};
use hotline_core::log::StreamId;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn an_acp_childs_own_steps_are_written_with_the_pages_they_opened() {
    let mut desk = open().await;
    let (ada, tape) = desk.teammate("Ada", Driver::Acp, false).await;
    let cbs = "https://www.cbsnews.com/news/nvidia-ceo-jensen-huang-resigns/";
    let steps = [
        json!({"title": "mcp__Computer__browser", "kind": "other", "output": format!("navigated to {cbs}")}),
        json!({"title": "mcp__Computer__browser", "kind": "other", "output": "clicked e12"}),
        json!({"title": "mcp__hotline__browser", "kind": "other", "output": "Page: Apple I - Wikipedia\nURL: https://en.wikipedia.org/wiki/Apple_I"}),
        json!({"title": "Fetch https://lapu.ai/blog/desktop-computer-use-agents-compared", "kind": "fetch", "output": "# Desktop AI Agents: Summary by Tool"}),
        json!({"title": "Web Fetch: https://rentamac.io/mac-mini-history/", "kind": "fetch", "output": ""}),
        json!({"title": "Search \"jensen huang resigns\"", "kind": "fetch", "output": format!("Nvidia CEO resigns - CBS News ({cbs})\n")}),
        json!({"title": "mcp__Computer__browser", "kind": "other", "output": "navigated to https://example.com/", "failed": true}),
    ];
    let prompt = steps
        .iter()
        .map(|step| format!("STEP {step}"))
        .collect::<Vec<_>>()
        .join("\n");
    let sent = desk
        .client
        .call("session.prompt", json!({"personaId": ada, "text": prompt}))
        .await;
    assert_eq!(sent["ok"], true, "{sent}");
    desk.event(tape, |event| {
        event["kind"] == "agent" && event["text"] == "On it."
    })
    .await;

    let written: Vec<(Value, Value)> = desk
        .log
        .load(&StreamId::Tape(ada))
        .into_iter()
        .filter(|event| {
            event["kind"] == "tool"
                && matches!(event["status"].as_str(), Some("completed" | "failed"))
        })
        .map(|event| (event["title"].clone(), event["sources"].clone()))
        .collect();
    assert_eq!(
        written,
        [
            (json!("mcp__Computer__browser"), json!([{"url": cbs}])),
            (json!("mcp__Computer__browser"), Value::Null),
            (
                json!("mcp__hotline__browser"),
                json!([{"url": "https://en.wikipedia.org/wiki/Apple_I", "title": "Apple I - Wikipedia"}])
            ),
            (
                json!("Fetch https://lapu.ai/blog/desktop-computer-use-agents-compared"),
                json!([{"url": "https://lapu.ai/blog/desktop-computer-use-agents-compared"}])
            ),
            (
                json!("Web Fetch: https://rentamac.io/mac-mini-history/"),
                json!([{"url": "https://rentamac.io/mac-mini-history/"}])
            ),
            (
                json!("Search \"jensen huang resigns\""),
                json!([{"url": cbs, "title": "Nvidia CEO resigns - CBS News", "found": true}])
            ),
            (json!("mcp__Computer__browser"), Value::Null),
        ],
        "each step's pages as the tape keeps them"
    );
}
