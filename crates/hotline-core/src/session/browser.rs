//! The `browser` tool on the room's side: what a teammate asks of its own
//! browser, the guard card a local address raises, and the person's answer
//! to it. The browser itself is the app's ([`crate::browser::DeskBrowser`]).

use super::*;
use crate::browser::{self as desk_browser, DeskBrowser, Gate, Page, Reading, Step};
use crate::contract::PermissionOption;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::PoisonError;
use tokio::sync::oneshot;

pub(super) const BROWSER_REQUEST_PREFIX: &str = "browser:";
const ALLOW: &str = "allow";
const ALLOW_ALWAYS: &str = "allow_always";
const DENY: &str = "deny";

/// What a browser call answers: words, or a picture of the page.
pub(crate) enum BrowserAnswer {
    Text(String),
    Png(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Decision {
    Allow,
    Always,
    Deny,
}

/// One guard card waiting for the person.
pub(crate) struct BrowserWait {
    persona_id: String,
    origin: String,
    title: String,
    sender: oneshot::Sender<Decision>,
}

/// Takes the wait off the room if the asking call goes away first, so the
/// card says it expired instead of offering an answer nobody hears.
struct WaitGuard<'a> {
    room: &'a Room,
    request_id: String,
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        if let Some(wait) = lock(&self.room.browser_waits).remove(&self.request_id) {
            self.room.write_browser_decision(
                &self.request_id,
                &wait,
                "expired",
                "Expired unanswered",
            );
        }
    }
}

fn options(origin: &str) -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            option_id: ALLOW.to_string(),
            name: "Allow".to_string(),
            kind: Some("allow_once".to_string()),
        },
        PermissionOption {
            option_id: ALLOW_ALWAYS.to_string(),
            name: format!("Always allow {origin}"),
            kind: Some("allow_always".to_string()),
        },
        PermissionOption {
            option_id: DENY.to_string(),
            name: "Deny".to_string(),
            kind: Some("reject_once".to_string()),
        },
    ]
}

fn page_line(page: &Page) -> String {
    let title = if page.title.trim().is_empty() {
        "(no title)"
    } else {
        page.title.trim()
    };
    let loading = if page.loaded {
        ""
    } else {
        "\nThe page was still loading; read it again in a moment if it looks incomplete."
    };
    format!("Page: {title}\nURL: {}{loading}", page.url)
}

/// Cuts a read to [`desk_browser::READ_LIMIT`] characters, saying so.
fn capped(text: String) -> String {
    match text.char_indices().nth(desk_browser::READ_LIMIT) {
        Some((cut, _)) => format!(
            "{}\n\n[Cut at {} characters: the page goes on.]",
            &text[..cut],
            desk_browser::READ_LIMIT
        ),
        None => text,
    }
}

impl Room {
    /// The app's browser, with the desk's own ports the guard never offers.
    pub fn install_browser(&self, browser: Arc<dyn DeskBrowser>, never_ports: &[u16]) {
        self.browser_gate.load(&self.log, never_ports);
        *self.browser.write().unwrap_or_else(PoisonError::into_inner) = Some(browser);
    }

    /// The guard the app asks before every top-level load.
    pub fn browser_gate(&self) -> Arc<Gate> {
        self.browser_gate.clone()
    }

    fn desk_browser(&self) -> Option<Arc<dyn DeskBrowser>> {
        self.browser
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether this teammate is offered the desk's browser: the app put one
    /// in, and the teammate has no computer, whose browser is the full one.
    pub(crate) fn offers_browser(&self, persona_id: &str) -> bool {
        self.desk_browser().is_some()
            && self.persona(persona_id).is_ok_and(|persona| {
                !persona
                    .computer
                    .as_ref()
                    .is_some_and(|computer| computer.enabled)
            })
    }

    /// One `browser` call.
    pub(crate) async fn browser_call(
        &self,
        persona_id: &str,
        arguments: &Value,
    ) -> Result<BrowserAnswer, String> {
        if !self.offers_browser(persona_id) {
            return Err("You have no desk browser: either Hotline is running without its window, or you have a computer, whose browser is your own.".into());
        }
        let browser = self.desk_browser().ok_or("The desk browser has closed.")?;
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or("browser needs an `action`.")?;
        match action {
            "navigate" => {
                let link = arguments
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .ok_or("navigate needs a `url`.")?;
                let url = url::Url::parse(link)
                    .ok()
                    .filter(|url| matches!(url.scheme(), "http" | "https"))
                    .ok_or("navigate needs a full http or https link.")?;
                self.guard_local(persona_id, &url, browser.as_ref()).await?;
                let page = browser.open(persona_id, &url).await?;
                Ok(BrowserAnswer::Text(page_line(&page)))
            }
            "back" | "forward" | "reload" => {
                let step = match action {
                    "back" => Step::Back,
                    "forward" => Step::Forward,
                    _ => Step::Reload,
                };
                let page = browser.step(persona_id, step).await?;
                Ok(BrowserAnswer::Text(page_line(&page)))
            }
            "text" => Ok(BrowserAnswer::Text(capped(
                browser.read(persona_id, Reading::Text).await?,
            ))),
            "links" => Ok(BrowserAnswer::Text(capped(
                browser.read(persona_id, Reading::Links).await?,
            ))),
            "screenshot" => Ok(BrowserAnswer::Png(
                STANDARD.encode(browser.screenshot(persona_id).await?),
            )),
            other => Err(format!(
                "The desk browser has no action '{other}'. It can navigate, text, links, screenshot, back, forward and reload."
            )),
        }
    }

    /// Lets a local address through only once the person allowed its host
    /// and port for this teammate; asks them on a card when they have not.
    async fn guard_local(
        &self,
        persona_id: &str,
        url: &url::Url,
        browser: &dyn DeskBrowser,
    ) -> Result<(), String> {
        if !desk_browser::reaches_local(url).await {
            return Ok(());
        }
        let origin = desk_browser::origin_key(url).ok_or("That link has no host and port.")?;
        let port = url.port_or_known_default().unwrap_or(0);
        if self.browser_gate.never(port) || self.computers.live_ports().contains(&port) {
            return Err(format!(
                "{origin} is one of Hotline's own ports, which a teammate's browser never opens."
            ));
        }
        if self.browser_gate.granted(persona_id, &origin) {
            return Ok(());
        }
        let persona = self.persona(persona_id)?;
        let was_on = browser
            .current(persona_id)
            .filter(|page| page != "about:blank")
            .map(|page| format!("\n\nIts browser was on {page}."))
            .unwrap_or_default();
        let title = format!(
            "{} wants to open {origin} in its browser.\n\nThat address is on this computer or your local network.{was_on}",
            persona.name
        );
        let request_id = format!("{BROWSER_REQUEST_PREFIX}{}", new_id());
        let (sender, answered) = oneshot::channel();
        lock(&self.browser_waits).insert(
            request_id.clone(),
            BrowserWait {
                persona_id: persona_id.to_string(),
                origin: origin.clone(),
                title: title.clone(),
                sender,
            },
        );
        let _guard = WaitGuard {
            room: self,
            request_id: request_id.clone(),
        };
        self.write_home(
            persona_id,
            &ThreadId::dm(persona_id),
            &TranscriptEvent::Permission {
                from: None,
                id: format!("perm:{request_id}"),
                ts: now_ms(),
                request_id: request_id.clone(),
                title,
                options: options(&origin),
                decision: None,
                decided_option_name: None,
            },
        );
        let decision = match tokio::time::timeout(HUMAN_DEADLINE, answered).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => return Err("The request to open that page was cancelled.".into()),
            Err(_) => {
                return Err(format!(
                    "Nobody answered the request to open {origin}, so it was not opened."
                ));
            }
        };
        match decision {
            Decision::Allow => {
                self.browser_gate.allow(persona_id, &origin);
                Ok(())
            }
            Decision::Always => Ok(()),
            Decision::Deny => Err(format!(
                "The person denied opening {origin} in your browser. Do not ask again for it in this task."
            )),
        }
    }

    /// The person's answer to a guard card.
    pub(super) fn answer_browser_card(
        &self,
        persona_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> Result<(), String> {
        let wait = {
            let mut waits = lock(&self.browser_waits);
            match waits.get(request_id) {
                Some(wait) if wait.persona_id == persona_id => waits.remove(request_id),
                _ => None,
            }
        }
        .ok_or("That request is no longer waiting for an answer.")?;
        let (decision, name) = match option_id {
            ALLOW => (Decision::Allow, "Allow".to_string()),
            ALLOW_ALWAYS => (Decision::Always, format!("Always allow {}", wait.origin)),
            DENY => (Decision::Deny, "Deny".to_string()),
            _ => {
                self.write_browser_decision(request_id, &wait, "expired", "Invalid answer");
                let _ = wait.sender.send(Decision::Deny);
                return Err("That card has no such answer.".into());
            }
        };
        if decision == Decision::Always
            && let Err(error) =
                self.browser_gate
                    .allow_always(&self.log, &wait.persona_id, &wait.origin)
        {
            self.write_browser_decision(request_id, &wait, "expired", "Could not be saved");
            let _ = wait.sender.send(Decision::Deny);
            return Err(error);
        }
        self.write_browser_decision(request_id, &wait, option_id, &name);
        let _ = wait.sender.send(decision);
        Ok(())
    }

    fn write_browser_decision(
        &self,
        request_id: &str,
        wait: &BrowserWait,
        decision: &str,
        option_name: &str,
    ) {
        self.write(
            &wait.persona_id,
            &TranscriptEvent::Permission {
                from: None,
                id: format!("perm:{request_id}"),
                ts: now_ms(),
                request_id: request_id.to_string(),
                title: wait.title.clone(),
                options: options(&wait.origin),
                decision: Some(decision.to_string()),
                decided_option_name: Some(option_name.to_string()),
            },
        );
    }
}
