//! The pipe between `hotline serve` and its browser helper (`hotline-browser`,
//! `crates/hotline-browser`): [`DeskBrowser`] carried over a socket, one JSON
//! object per line, so the server's binary never links GTK or WebKit and a
//! crashed browser takes nothing of the desk with it.
//!
//! The desk sends [`Down`] lines: a call for one teammate's browser, with an
//! id it waits on a reply to, or without one when nothing is answered (who
//! took the browser over, the teammate's name); and the gate's answer to an
//! ask. The helper sends [`Up`] lines: that it is ready, each call's reply,
//! the address of every load it asks the desk's [`Gate`](super::Gate) about,
//! and where a teammate's browser is ([`State`]) whenever that may have
//! moved, so the trait's quick reads (`current`, `needs`, `size`) are
//! answered on the desk's side from what was last said, without a wait.
//!
//! The gate stays the desk's alone: the helper asks it about every load, as
//! the app's web views ask it in-process, because what it refuses (the desk's
//! own ports, those every listener and running computer of this process
//! holds) is known only here.
//!
//! This file is both ends' shapes and the helper's end ([`answer`]); the
//! desk's end, which starts and supervises the helper, is `helper.rs`.

use super::{Act, DeskBrowser, Reading, Step};
use crate::contract::BrowserInput;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};

/// The descriptor the helper is handed its end of the pipe on.
pub const PIPE_FD: i32 = 3;

/// One call to a teammate's browser, as [`DeskBrowser`] names it. `Taken`
/// and `Name` are told, never answered.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Open { url: String },
    Step { step: Step },
    Read { reading: Reading },
    Act { act: Act },
    Screenshot,
    FullPage { most: usize },
    Field { target: String },
    Showing,
    Viewport { size: Option<(u32, u32)> },
    PageSize,
    PersonInput { input: BrowserInput },
    Close { wipe: bool },
    Taken { taken: bool },
    Name { name: String },
}

/// A line from the desk to the helper.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Down {
    Call {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        persona: String,
        #[serde(flatten)]
        call: Box<Call>,
    },
    /// The gate's word on the load `ask` named.
    Answer { ask: u64, admits: bool },
}

/// A line from the helper to the desk.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Up {
    /// The helper's browser process and its private X display are up.
    Ready { pid: u32, display: u32 },
    Reply {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// May a frame of a teammate's page load `url`? ([`super::Gate::admits_frame`])
    Ask { ask: u64, url: String },
    State {
        persona: String,
        #[serde(flatten)]
        state: State,
    },
}

/// Where a teammate's browser is, as the trait's quick reads answer it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub current: Option<String>,
    pub needs: Option<String>,
    pub size: Option<(u32, u32)>,
}

/// A picture as a line carries it.
pub(crate) fn picture_out(png: &[u8]) -> Value {
    Value::String(STANDARD.encode(png))
}

pub(crate) fn picture_in(value: &Value) -> Result<Vec<u8>, String> {
    value
        .as_str()
        .and_then(|text| STANDARD.decode(text).ok())
        .ok_or_else(|| "The browser's picture did not arrive whole.".to_string())
}

/// One JSON line.
pub(crate) fn line(value: &impl Serialize) -> String {
    let mut text = serde_json::to_string(value).expect("a pipe line always serializes");
    text.push('\n');
    text
}

/// What goes up the pipe, in order: a line as it is, or a teammate whose
/// browser may have moved, whose [`State`] is read at the moment it is
/// written, after everything before it.
enum Outgoing {
    Line(Up),
    Moved(String),
}

/// The helper's link back to the desk, for its browser: the gate's answer
/// on a load, and word that a teammate's page moved on its own.
#[derive(Clone)]
pub struct Link {
    up: mpsc::UnboundedSender<Outgoing>,
    asks: Arc<Mutex<HashMap<u64, oneshot::Sender<bool>>>>,
    next: Arc<AtomicU64>,
}

/// What [`Link::new`] queues for [`answer`] to write.
pub struct Outbox(mpsc::UnboundedReceiver<Outgoing>);

impl Link {
    pub fn new() -> (Self, Outbox) {
        let (up, queued) = mpsc::unbounded_channel();
        (
            Self {
                up,
                asks: Arc::default(),
                next: Arc::new(AtomicU64::new(1)),
            },
            Outbox(queued),
        )
    }

    /// Whether the desk's gate lets a frame of a page load `url`. With the
    /// desk gone, nothing loads.
    pub async fn admits_frame(&self, url: &str) -> bool {
        let ask = self.next.fetch_add(1, Ordering::SeqCst);
        let (answer, answered) = oneshot::channel();
        self.asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ask, answer);
        let sent = self.up.send(Outgoing::Line(Up::Ask {
            ask,
            url: url.to_string(),
        }));
        if sent.is_err() {
            return false;
        }
        answered.await.unwrap_or(false)
    }

    /// A teammate's page moved without a call (a load it started itself, a
    /// refused certificate): the desk is told where it is now.
    pub fn moved(&self, persona_id: &str) {
        let _ = self.up.send(Outgoing::Moved(persona_id.to_string()));
    }

    fn answered(&self, ask: u64, admits: bool) {
        if let Some(waiting) = self
            .asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&ask)
        {
            let _ = waiting.send(admits);
        }
    }
}

fn state_of(browser: &dyn DeskBrowser, persona_id: &str) -> State {
    State {
        current: browser.current(persona_id),
        needs: browser.needs(persona_id),
        size: browser.size(persona_id),
    }
}

/// The helper's end: says `ready`, then answers the desk's calls on `stream`
/// with `browser`, each as it comes, until the desk hangs up. Every reply
/// goes after the teammate's [`State`] as it stands once the call is done.
pub async fn answer(
    stream: UnixStream,
    link: Link,
    outbox: Outbox,
    browser: Arc<dyn DeskBrowser>,
    ready: Up,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    write.write_all(line(&ready).as_bytes()).await?;
    let writer = {
        let browser = browser.clone();
        let mut outbox = outbox.0;
        tokio::spawn(async move {
            while let Some(outgoing) = outbox.recv().await {
                let up = match outgoing {
                    Outgoing::Line(up) => up,
                    Outgoing::Moved(persona) => Up::State {
                        state: state_of(browser.as_ref(), &persona),
                        persona,
                    },
                };
                if write.write_all(line(&up).as_bytes()).await.is_err() {
                    break;
                }
            }
        })
    };
    let mut lines = BufReader::new(read).lines();
    while let Some(text) = lines.next_line().await? {
        let Ok(down) = serde_json::from_str::<Down>(&text) else {
            eprintln!("[browser] the desk sent a line this helper does not read");
            continue;
        };
        match down {
            Down::Answer { ask, admits } => link.answered(ask, admits),
            Down::Call {
                id: None,
                persona,
                call,
            } => match *call {
                Call::Taken { taken } => browser.taken(&persona, taken),
                Call::Name { name } => browser.name_teammate(&persona, &name),
                _ => eprintln!("[browser] the desk sent a call with no id"),
            },
            Down::Call {
                id: Some(id),
                persona,
                call,
            } => {
                let (browser, link) = (browser.clone(), link.clone());
                tokio::spawn(async move {
                    let done = called(browser.as_ref(), &persona, *call).await;
                    link.moved(&persona);
                    let (ok, error) = match done {
                        Ok(value) => (Some(value), None),
                        Err(said) => (None, Some(said)),
                    };
                    let _ = link.up.send(Outgoing::Line(Up::Reply { id, ok, error }));
                });
            }
        }
    }
    writer.abort();
    Ok(())
}

/// One call, made of `browser` and answered as its reply carries it.
async fn called(browser: &dyn DeskBrowser, persona: &str, call: Call) -> Result<Value, String> {
    Ok(match call {
        Call::Open { url } => {
            let url =
                url::Url::parse(&url).map_err(|_| "That is not a web address.".to_string())?;
            value(&browser.open(persona, &url).await?)
        }
        Call::Step { step } => value(&browser.step(persona, step).await?),
        Call::Read { reading } => Value::String(browser.read(persona, reading).await?),
        Call::Act { act } => value(&browser.act(persona, &act).await?),
        Call::Screenshot => picture_out(&browser.screenshot(persona).await?),
        Call::FullPage { most } => {
            let page = browser.full_page(persona, most).await?;
            json!({
                "pictures": page.pictures.iter().map(|png| picture_out(png)).collect::<Vec<_>>(),
                "more": page.more,
            })
        }
        Call::Field { target } => value(&browser.field(persona, &target).await?),
        Call::Showing => value(&browser.showing(persona).await?),
        Call::Viewport { size } => value(&browser.viewport(persona, size).await?),
        Call::PageSize => value(&browser.page_size(persona).await),
        Call::PersonInput { input } => {
            browser.person_input(persona, &input).await?;
            Value::Null
        }
        Call::Close { wipe } => {
            browser.close(persona, wipe).await?;
            Value::Null
        }
        Call::Taken { taken } => {
            browser.taken(persona, taken);
            Value::Null
        }
        Call::Name { name } => {
            browser.name_teammate(persona, &name);
            Value::Null
        }
    })
}

/// An answer as its reply carries it.
fn value(answer: &impl Serialize) -> Value {
    serde_json::to_value(answer).expect("a browser answer always serializes")
}
