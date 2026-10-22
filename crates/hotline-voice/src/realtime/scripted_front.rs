//! A fixed spoken deck exercises the production prompt against every live model.
//! Audio is heard before its transcript can be checked: this is a regression
//! alarm, not a runtime filter or a guarantee about future model replies.

use super::*;
use crate::front_prompt::{Behind, Start, instructions};
use crate::speech::Endpoint;
use std::collections::HashSet;
use std::fmt::Write;
use std::time::Duration;

const REQUESTS: [(&str, &[u8]); 6] = [
    (
        "Can you hear me?",
        include_bytes!("scripted_front/question.wav"),
    ),
    (
        "I am just thinking out loud. Maybe the parser should be simpler.",
        include_bytes!("scripted_front/thinking.wav"),
    ),
    (
        "Where are we on the parser tests?",
        include_bytes!("scripted_front/status.wav"),
    ),
    (
        "Fix the failing parser test.",
        include_bytes!("scripted_front/work.wav"),
    ),
    (
        "Actually, fix the beacon test first.",
        include_bytes!("scripted_front/steer.wav"),
    ),
    (
        "I have got time. Give me the overview.",
        include_bytes!("scripted_front/rundown.wav"),
    ),
];

fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
struct Check {
    acknowledgements: HashSet<String>,
}
impl Check {
    fn said(&mut self, text: &str, work: bool) -> Result<(), String> {
        let plain = normalized(text);
        for relay in [
            "handing that off",
            "handing it off",
            "hand this off",
            "handing off",
            "passing that on",
            "passing it on",
            "pass that on",
            "let me check that for you",
            "working session",
            "behind the scenes",
            "relay this",
            "relaying",
            "send this to the teammate",
        ] {
            if plain.contains(relay) {
                return Err(format!("relay phrasing: {text}"));
            }
        }
        if work {
            let first = text.split(['.', '!', '?']).next().unwrap_or_default();
            let ack = normalized(first);
            if !ack.is_empty() && !self.acknowledgements.insert(ack) {
                return Err(format!("acknowledgement repeated within the call: {first}"));
            }
        }
        Ok(())
    }
}

fn pcm(wav: &[u8]) -> &[u8] {
    let mut at = 12;
    while at + 8 <= wav.len() {
        let n = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        if &wav[at..at + 4] == b"data" {
            return &wav[at + 8..at + 8 + n];
        }
        at += 8 + n + n % 2;
    }
    panic!("fixture has no PCM data")
}

async fn deck(model: &dyn Realtime, voice: &str) {
    let mut tools = crate::front::tools();
    tools.push(crate::front::Tool {
        name: "read_thread".into(),
        description: "Look up the current call work, test results and recent conversation.".into(),
        parameters: serde_json::json!({"type":"object", "properties":{}}),
    });
    let base = instructions(
        &Behind::Teammate {
            public_name: "Ada",
            voice: "Warm, brisk and precise.",
        },
        &Start {
            tools: &tools,
            ..Start::default()
        },
    );
    let mut session = model
        .open(RealtimeConfig {
            instructions: base.clone(),
            voice: voice.into(),
            tools: tools
                .into_iter()
                .map(|tool| RealtimeTool {
                    name: tool.name,
                    description: tool.description,
                    parameters: tool.parameters,
                })
                .collect(),
            input_rate: 24_000,
            output_rate: 24_000,
        })
        .await
        .unwrap();
    let mut check = Check::default();
    let mut history = String::new();
    let mut running = false;
    for (request, wav) in REQUESTS {
        let input = session.input.clone();
        let microphone = tokio::spawn(async move {
            let mut clock = tokio::time::interval(Duration::from_millis(40));
            for chunk in pcm(wav).chunks(1_920) {
                clock.tick().await;
                input.push_audio(chunk.to_vec()).await.unwrap();
            }
            loop {
                clock.tick().await;
                if input.push_audio(vec![0; 1_920]).await.is_err() {
                    break;
                }
            }
        });
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            let mut heard = false;
            let mut said = String::new();
            let mut work = false;
            loop {
                match session.events.recv().await.expect("live call ended").expect("live provider failed") {
                    RealtimeEvent::Heard(_) => heard = true,
                    RealtimeEvent::Saying(piece) if heard => said.push_str(&piece),
                    RealtimeEvent::ToolCall { call_id, name, .. } => {
                        work |= name == WORK_ON_IT;
                        let (output, reply) = if name == "read_thread" {
                            ("Current lookup: one parser test fails; beacon tests pass. No changes or posts yet.", true)
                        } else {
                            ("Work accepted on this call. No result yet. Do not acknowledge again.", false)
                        };
                        session.input.tool_result(&call_id, output, reply).await.unwrap();
                    }
                    RealtimeEvent::Refused { code } => panic!("live model refused the protocol: {code:?}"),
                    RealtimeEvent::ResponseDone if heard => break (said, work),
                    _ => {}
                }
            }
        }).await;
        microphone.abort();
        let (said, work) = result.expect("live model did not finish the scripted request");
        eprintln!("{}: {request}\n{said}", model.id().model_id);
        check.said(&said, work).unwrap();
        running |= work;
        let _ = write!(history, "\nCaller: {request}\nAlready said: {said}");
        session.input.instructions(&format!("{base}\nCall so far; do not repeat acknowledgements:{history}\nWork running: {running}. Nothing posted to chat.")).await.unwrap();
    }
    session.close().await;
}

#[test]
fn the_scripted_front_check_rejects_relay_words_and_repeated_acknowledgements() {
    for line in [
        "I'm handing that off.",
        "Passing that on.",
        "Let me check that for you.",
    ] {
        assert!(Check::default().said(line, true).is_err());
    }
    let mut check = Check::default();
    check.said("Inspecting the parser.", true).unwrap();
    assert!(check.said("Inspecting the parser!", true).is_err());
    check.said("Fixing the beacon.", true).unwrap();
    check.said("Yes.", false).unwrap();
    check.said("Yes.", false).unwrap();
    for (_, wav) in REQUESTS {
        assert_ne!(pcm(wav), [] as [u8; 0]);
    }
}

#[tokio::test]
#[ignore = "runs the spoken deck on Grok Voice; needs HOTLINE_LIVE_XAI_KEY"]
async fn scripted_front_grok() {
    deck(
        &RealtimeSocket::grok(Endpoint {
            provider_id: "xai".into(),
            base_url: "https://api.x.ai/v1".into(),
            key: Some(std::env::var("HOTLINE_LIVE_XAI_KEY").expect("HOTLINE_LIVE_XAI_KEY")),
        }),
        "eve",
    )
    .await;
}

#[tokio::test]
#[ignore = "runs the spoken deck on GPT Realtime; needs HOTLINE_LIVE_OPENAI_KEY"]
async fn scripted_front_gpt_realtime() {
    deck(
        &RealtimeSocket::openai(Endpoint {
            provider_id: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            key: Some(std::env::var("HOTLINE_LIVE_OPENAI_KEY").expect("HOTLINE_LIVE_OPENAI_KEY")),
        }),
        "marin",
    )
    .await;
}

#[tokio::test]
#[ignore = "runs the spoken deck on GPT-Live; needs HOTLINE_TEST_CHATGPT_DIR with a fresh auth.json"]
async fn scripted_front_gpt_live() {
    fn login(
        path: &std::path::Path,
    ) -> futures_util::future::BoxFuture<'_, Result<live::Login, String>> {
        Box::pin(async move {
            let auth: Value = serde_json::from_slice(
                &std::fs::read(path.join("auth.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            Ok(live::Login {
                access_token: auth["access_token"].as_str().map(String::from),
                account_id: auth["account_id"].as_str().map(String::from),
            })
        })
    }
    deck(
        &GptLive::new(
            std::env::var("HOTLINE_TEST_CHATGPT_DIR")
                .expect("HOTLINE_TEST_CHATGPT_DIR")
                .into(),
            login,
        ),
        "cove",
    )
    .await;
}
