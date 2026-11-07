//! A teammate's desk browser as a live screen, through the sealed door at
//! `/v2/browser/{personaId}/ws` (task-390): the computer viewer's protocol
//! (`browser::screen`), so a viewer that draws a teammate's computer draws
//! its browser too, at the computer's speed. The browser sends only what
//! changed, as it changes; input goes up without waiting on a picture.
//!
//! The envelope is the computer door's: each plaintext message is
//! `{"type":"text","data":"…"}` or `{"type":"binary","data":"<base64>"}`,
//! and a viewer sends text envelopes only.
//!
//! The desk answers `ping` and `control` here and passes the rest on.
//! `control` is the browser's own Take control and Give it back
//! (`browser_take`), so the teammate's waiting, its session's phase and the
//! PIP follow it exactly as they follow the pane's bar or the phone's
//! take-over; a phase the person changes from elsewhere is told here within
//! a second. Only the owner drives, as only the owner takes a browser over
//! on the wire (`browserDrive`); a companion seat watches. The seat's input
//! reaches the page only while it is the one that took control and the
//! person still has the browser, and the browser checks that again. The
//! gate decides every load as ever: nothing here loads a page, and the
//! address bar, Back, Forward, Reload and Visit anyway stay wire commands.
//!
//! A seat that took control and vanishes hands the browser back after
//! [`GRACE`], unless a seat of the person's took it again meanwhile (a
//! viewer reconnecting takes it again as it comes back), so a teammate
//! never waits on a phone that went away.

use super::Remote;
use crate::browser::screen::{FromViewer, RELEASE};
use crate::contract::{BrowserPhase, BrowserView};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use hotline_room::room_handle::ScreenShown;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

/// How long a browser stays the person's after the seat driving it went
/// away, for that seat to come back.
pub(super) const GRACE: Duration = if cfg!(test) {
    Duration::from_millis(500)
} else {
    Duration::from_secs(10)
};
/// How often a viewer hears who has the browser and where it is, when that
/// changed.
const STATE_EVERY: Duration = Duration::from_secs(1);
/// How often, at most, the teammate's session picture (the PIP's) is taken
/// again while the person drives here and the page moves.
const PICTURE_EVERY: Duration = Duration::from_secs(1);
/// The longest viewer message the door reads: a paste of 1 MiB, quoted.
const VIEWER_MESSAGE_MAX: usize = 4 * 1024 * 1024;
const VIEW_ONLY: &str = "This device can watch this browser, not drive it.";

/// What one viewer message does, from what this seat may do (`owner`),
/// whether it took control (`driving`), and whether the person has the
/// browser (`held`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Act {
    Pong(u64),
    /// Take control: the browser's Take control.
    Take,
    /// Give it back: the browser's Give it back.
    GiveBack,
    /// To the browser's hands.
    Forward,
    /// Refused, with what the viewer is told.
    Refuse(&'static str),
    /// Nothing: a watching seat's input, or a release of what it never took.
    Watch,
}

pub(super) fn act(message: &FromViewer, owner: bool, driving: bool, held: bool) -> Act {
    match message {
        FromViewer::Ping { id } => Act::Pong(*id),
        FromViewer::Control { take: true } if owner => Act::Take,
        FromViewer::Control { take: true } => Act::Refuse(VIEW_ONLY),
        FromViewer::Control { take: false } if driving => Act::GiveBack,
        FromViewer::Control { take: false } => Act::Watch,
        _ if owner && driving && held => Act::Forward,
        FromViewer::Paste { .. } if !owner => Act::Refuse(VIEW_ONLY),
        FromViewer::Paste { .. } => Act::Refuse("Take control before pasting"),
        _ => Act::Watch,
    }
}

/// The `state` a viewer is told: the computer's fields, so its viewer reads
/// it as it reads a computer's (`holder` is `you` for the seat that took
/// control, `person` for the person anywhere else, `agent` for the
/// teammate), and the browser's own.
pub(super) fn state(view: Option<&BrowserView>, driving: bool) -> Value {
    let holder = match view.map(|view| view.phase) {
        None => "none",
        Some(BrowserPhase::Person) if driving => "you",
        Some(BrowserPhase::Person) => "person",
        Some(BrowserPhase::Teammate) => "agent",
    };
    json!({
        "t": "state",
        "holder": holder,
        "driving": driving,
        "running": 0,
        "completed": 0,
        "failed": 0,
        "browser": view.map(|view| json!({
            "phase": view.phase,
            "url": view.url,
            "title": view.title,
            "needs": view.needs,
        })),
    })
}

fn held(view: Option<&BrowserView>) -> bool {
    view.is_some_and(|view| view.phase == BrowserPhase::Person)
}

fn text(data: &str) -> Message {
    Message::text(json!({"type": "text", "data": data}).to_string())
}

fn error(said: &str, driving: bool) -> Message {
    text(&json!({"t": "error", "error": said, "driving": driving}).to_string())
}

/// One viewer of `persona`'s browser on a sealed channel, as `owner` or a
/// companion seat, until either side hangs up.
pub(super) async fn sealed_browser<S>(
    remote: Arc<Remote>,
    mut viewer: super::channel::Channel<S>,
    persona: &str,
    owner: bool,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        r#type: String,
        data: String,
    }
    let Ok(agents) = remote.agents() else {
        return;
    };
    let room = agents.room.clone();
    let mut screen = match room.browser_screen(persona).await {
        Ok(screen) => screen,
        Err(said) => {
            let _ = viewer.send(error(&said, false)).await;
            return;
        }
    };
    let mut driving = false;
    let mut told = String::new();
    let mut ticker = tokio::time::interval(STATE_EVERY);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The page moved since the session's picture was last taken.
    let mut moved = false;
    let mut pictured = Instant::now();
    let mut picturing: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let view = room.browser_view(persona);
                if driving && !held(view.as_ref()) {
                    // Handed back from elsewhere: this seat watches again.
                    driving = false;
                    remote.browser_driver(persona, false);
                    let _ = screen.input.send(RELEASE.to_string()).await;
                }
                let said = state(view.as_ref(), driving).to_string();
                if said != told {
                    if viewer.send(text(&said)).await.is_err() { break; }
                    told = said;
                }
                // The PIP and the phone's picture follow what the person
                // does here, as they followed the pictures a driving seat
                // polled for.
                if driving && moved && pictured.elapsed() >= PICTURE_EVERY
                    && picturing.as_ref().is_none_or(tokio::task::JoinHandle::is_finished)
                {
                    moved = false;
                    pictured = Instant::now();
                    let (room, persona) = (room.clone(), persona.to_string());
                    picturing = Some(tokio::spawn(async move {
                        let _ = room.browser_snapshot_fresh(&persona).await;
                    }));
                }
            }
            shown = screen.shown.recv() => {
                let message = match shown {
                    Some(ScreenShown::Frame(bytes)) => {
                        moved = true;
                        Message::text(json!({"type": "binary", "data": STANDARD.encode(bytes)}).to_string())
                    }
                    Some(ScreenShown::Text(said)) => text(&said),
                    // The browser stopped: the viewer opens again and finds
                    // the new one, or why there is none.
                    None => break,
                };
                if viewer.send(message).await.is_err() { break; }
            }
            input = viewer.next() => {
                let Some(Ok(Message::Text(input))) = input else {
                    if matches!(input, Some(Ok(Message::Ping(_) | Message::Pong(_)))) { continue; }
                    break;
                };
                let Ok(input) = serde_json::from_str::<Input>(&input) else { break; };
                if input.r#type != "text" || input.data.len() > VIEWER_MESSAGE_MAX { break; }
                let message = match FromViewer::parse(&input.data) {
                    Ok(message) => message,
                    Err(said) => {
                        if viewer.send(error(&said, driving)).await.is_err() { break; }
                        continue;
                    }
                };
                let view = room.browser_view(persona);
                match act(&message, owner, driving, held(view.as_ref())) {
                    Act::Pong(id) => {
                        if viewer.send(text(&json!({"t": "pong", "id": id}).to_string())).await.is_err() { break; }
                    }
                    Act::Take => match room.browser_take(persona, true).await {
                        Ok(view) => {
                            if !driving { remote.browser_driver(persona, true); }
                            driving = true;
                            let said = state(view.as_ref(), driving).to_string();
                            if viewer.send(text(&said)).await.is_err() { break; }
                            told = said;
                        }
                        Err(said) => {
                            if viewer.send(error(&said, driving)).await.is_err() { break; }
                        }
                    },
                    Act::GiveBack => {
                        driving = false;
                        remote.browser_driver(persona, false);
                        let _ = screen.input.send(RELEASE.to_string()).await;
                        let given = room.browser_take(persona, false).await;
                        let said = match given {
                            Ok(view) => state(view.as_ref(), false).to_string(),
                            Err(_) => state(room.browser_view(persona).as_ref(), false).to_string(),
                        };
                        if viewer.send(text(&said)).await.is_err() { break; }
                        told = said;
                    }
                    Act::Forward => {
                        if screen.input.send(input.data).await.is_err() { break; }
                    }
                    Act::Refuse(said) => {
                        if viewer.send(error(said, driving)).await.is_err() { break; }
                    }
                    Act::Watch => {}
                }
            }
        }
    }
    if driving {
        let _ = screen.input.send(RELEASE.to_string()).await;
        remote.browser_driver(persona, false);
        let (remote, persona) = (remote.clone(), persona.to_string());
        tokio::spawn(async move {
            tokio::time::sleep(GRACE).await;
            remote.browser_left(&persona).await;
        });
    }
}

impl Remote {
    /// A seat of the person's took control of `persona`'s browser here
    /// (`true`), or stopped driving it.
    pub(super) fn browser_driver(&self, persona: &str, driving: bool) {
        let mut drivers = super::lock(&self.browser_drivers);
        let count = drivers.entry(persona.to_string()).or_default();
        if driving {
            *count += 1;
        } else {
            *count = count.saturating_sub(1);
        }
        if *count == 0 {
            drivers.remove(persona);
        }
    }

    /// The seat driving `persona`'s browser went away a while ago: unless
    /// another took control here since, or it was handed back already, the
    /// browser goes back to the teammate.
    pub(super) async fn browser_left(&self, persona: &str) {
        if super::lock(&self.browser_drivers).contains_key(persona) {
            return;
        }
        let Ok(agents) = self.agents() else {
            return;
        };
        if held(agents.room.browser_view(persona).as_ref()) {
            let _ = agents.room.browser_take(persona, false).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(phase: BrowserPhase) -> BrowserView {
        BrowserView {
            phase,
            url: "https://example.com/".into(),
            title: "Example".into(),
            active_at: 1,
            snapshot_at: None,
            needs: None,
            viewport: None,
        }
    }

    #[test]
    fn control_is_take_control_and_give_it_back_for_the_owner_alone() {
        let take = FromViewer::Control { take: true };
        let give = FromViewer::Control { take: false };
        assert_eq!(act(&take, true, false, false), Act::Take);
        assert_eq!(
            act(&take, true, true, true),
            Act::Take,
            "again, as a reconnect does"
        );
        assert_eq!(act(&take, false, false, false), Act::Refuse(VIEW_ONLY));
        assert_eq!(act(&give, true, true, true), Act::GiveBack);
        assert_eq!(
            act(&give, true, false, true),
            Act::Watch,
            "a seat that never took it gives back nothing another seat holds"
        );
        assert_eq!(
            act(&FromViewer::Ping { id: 3 }, false, false, false),
            Act::Pong(3)
        );
    }

    #[test]
    fn input_reaches_the_page_only_from_the_owners_seat_that_took_it_while_the_person_has_it() {
        let moves = FromViewer::Move { x: 1, y: 2 };
        assert_eq!(act(&moves, true, true, true), Act::Forward);
        assert_eq!(act(&moves, true, false, true), Act::Watch, "watching");
        assert_eq!(
            act(&moves, true, true, false),
            Act::Watch,
            "handed back from elsewhere"
        );
        assert_eq!(act(&moves, false, true, true), Act::Watch, "a companion");
        let paste = FromViewer::Paste { text: "x".into() };
        assert_eq!(act(&paste, true, true, true), Act::Forward);
        assert_eq!(
            act(&paste, true, false, true),
            Act::Refuse("Take control before pasting")
        );
        assert_eq!(act(&paste, false, false, false), Act::Refuse(VIEW_ONLY));
    }

    #[test]
    fn the_state_names_the_holder_as_the_computers_viewer_reads_it() {
        let teammate = view(BrowserPhase::Teammate);
        let person = view(BrowserPhase::Person);
        assert_eq!(state(Some(&teammate), false)["holder"], "agent");
        assert_eq!(state(Some(&person), true)["holder"], "you");
        assert_eq!(state(Some(&person), false)["holder"], "person");
        assert_eq!(state(None, false)["holder"], "none");
        let said = state(Some(&person), true);
        assert_eq!(said["t"], "state");
        assert_eq!(said["driving"], true);
        assert_eq!(said["browser"]["phase"], "person");
        assert_eq!(said["browser"]["url"], "https://example.com/");
        assert_eq!(state(None, false)["browser"], Value::Null);
    }
}
