//! The `browser` tool on the room's side: what a teammate asks of its own
//! browser, the guard card a local address raises, and the person's answer
//! to it. The browser itself is the app's ([`crate::browser::DeskBrowser`]).

use super::*;
use crate::browser::{
    self as desk_browser, Act, DeskBrowser, Gate, LoginPart, Page, Reading, Step,
};
use crate::contract::{BrowserPhase, BrowserSnapshot, BrowserView, PermissionOption};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::PoisonError;
use tokio::sync::oneshot;

pub(super) const BROWSER_REQUEST_PREFIX: &str = "browser:";
const ALLOW: &str = "allow";
const ALLOW_ALWAYS: &str = "allow_always";
const DENY: &str = "deny";

/// The widest snapshot the PIP and the phone are sent.
const SNAPSHOT_WIDTH: u32 = 960;

/// What the person is told the teammate's acts answer while they have its
/// browser.
const TAKEN: &str = "The person has taken over your browser and is using it now. Wait for them to hand it back: you can still read the page with text, links and screenshot, but navigating, acting and a full-page screenshot, which scrolls, wait until then.";

/// A teammate's desk browser as the room shows it.
#[derive(Clone)]
pub(crate) struct Viewed {
    view: BrowserView,
    jpeg: Option<Vec<u8>>,
}

/// A PNG of the page as a JPEG at most [`SNAPSHOT_WIDTH`] wide.
fn snapshot_jpeg(png: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(png).ok()?;
    let image = if image.width() > SNAPSHOT_WIDTH {
        let height = image.height() * SNAPSHOT_WIDTH / image.width();
        image.resize(
            SNAPSHOT_WIDTH,
            height,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 72)
        .encode_image(&image.to_rgb8())
        .ok()?;
    Some(jpeg)
}

/// The most pictures a full-page look answers: enough for most articles,
/// few enough that one call stays a glance.
const FULL_PAGE_PICTURES: usize = 4;

/// What a browser call answers: words, or pictures of the page, top first.
pub(crate) enum BrowserAnswer {
    Text(String),
    /// Base64 PNGs, and whether the page goes on below the last.
    Pictures {
        pngs: Vec<String>,
        more: bool,
    },
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
    asking: Asking,
    title: String,
    sender: oneshot::Sender<Decision>,
}

/// What a browser card asks the person.
#[derive(Clone)]
enum Asking {
    /// To open a local address, by its host and port.
    Local { origin: String },
    /// To use an ask-me key on a site.
    Use,
}

impl Asking {
    fn options(&self) -> Vec<PermissionOption> {
        match self {
            Self::Local { origin } => options(origin),
            Self::Use => vec![
                PermissionOption {
                    option_id: APPROVE.to_string(),
                    name: "Approve".to_string(),
                    kind: Some("allow_once".to_string()),
                },
                PermissionOption {
                    option_id: ALLOW.to_string(),
                    name: "Allow for 10 minutes".to_string(),
                    kind: Some("allow_always".to_string()),
                },
                PermissionOption {
                    option_id: DENY.to_string(),
                    name: "Deny".to_string(),
                    kind: Some("reject_once".to_string()),
                },
            ],
        }
    }

    /// The answer an option id gives, and its name on the decided card.
    fn decide(&self, option_id: &str) -> Option<(Decision, String)> {
        match (self, option_id) {
            (Self::Local { .. }, ALLOW) => Some((Decision::Allow, "Allow".into())),
            (Self::Local { origin }, ALLOW_ALWAYS) => {
                Some((Decision::Always, format!("Always allow {origin}")))
            }
            (Self::Use, APPROVE) => Some((Decision::Allow, "Approve".into())),
            (Self::Use, ALLOW) => Some((Decision::Always, "Allow for 10 minutes".into())),
            (_, DENY) => Some((Decision::Deny, "Deny".into())),
            _ => None,
        }
    }
}

/// How long an approved use lets the rest of its sign-in fill, and how long
/// "Allow for 10 minutes" lets the key fill on that site.
const APPROVED_MS: i64 = 60_000;
const ALLOWED_MS: i64 = 10 * 60_000;
const APPROVE: &str = "approve";

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

/// One part of a read, `part` from 1, of at most
/// [`desk_browser::READ_LIMIT`] characters each, cut between lines (a line
/// longer than a part is cut inside it). A read that fits is answered
/// whole; a longer one says which part this is and how to ask for the next.
fn part_of(text: &str, part: usize) -> Result<String, String> {
    let limit = desk_browser::READ_LIMIT;
    let mut parts: Vec<String> = vec![String::new()];
    for line in text.split_inclusive('\n') {
        let mut line = line;
        while !line.is_empty() {
            let last = parts.last_mut().expect("one part at least");
            let room = limit.saturating_sub(last.chars().count());
            let length = line.chars().count();
            if length <= room {
                last.push_str(line);
                break;
            }
            if !last.is_empty() && length <= limit {
                parts.push(String::new());
                continue;
            }
            let cut = line
                .char_indices()
                .nth(room.max(1))
                .map_or(line.len(), |(at, _)| at);
            last.push_str(&line[..cut]);
            line = &line[cut..];
            parts.push(String::new());
        }
    }
    parts.retain(|part| !part.is_empty());
    let count = parts.len().max(1);
    if part == 0 || part > count {
        return Err(format!(
            "This page has {count} part{}: ask for part 1 to {count}.",
            if count == 1 { "" } else { "s" }
        ));
    }
    let body = parts.get(part - 1).cloned().unwrap_or_default();
    if count == 1 {
        return Ok(body);
    }
    let next = if part < count {
        format!(" Read part {} with `part: {}`.", part + 1, part + 1)
    } else {
        " That is the end of the page.".to_string()
    };
    Ok(format!(
        "[Part {part} of {count} of a long page.{next}]\n\n{}",
        body.trim_end()
    ))
}

/// What the browser answers when the teammate's secrets cannot be read to
/// hide them.
const UNREAD: &str = "Your secrets could not be read from the vault to keep them out of what your browser answers, so it answers nothing until they can.";

/// What a picture is refused for while a secret shows on the page.
const SHOWN: &str = "A secret of yours shows on this page outside a masked password field (a one-time code in its field, or a value the page shows back), so no picture of it is taken. Submit or clear that field, or leave the page, and look again; text and links still read the page with the value hidden.";

/// The values of a teammate's granted secrets that nothing its browser
/// answers may carry, each with the name it is hidden as: a login's
/// password, its TOTP seed and the codes that seed makes now, a variable's
/// value and a passkey's private key, as the vault supplies them. A login's
/// username is not one: sites show it back as who is signed in. It has no
/// debug form.
#[derive(Default)]
pub(crate) struct Hidden {
    /// Longest first, so a value inside another is hidden as the longer.
    values: Vec<(String, String)>,
}

impl Hidden {
    /// The values in `stored`, with the codes a seed makes at `now`, the
    /// step before and the step after.
    fn of(
        stored: std::collections::BTreeMap<String, crate::vault::StoredSecret>,
        now: u64,
    ) -> Self {
        use crate::vault::StoredSecret;
        let mut values = Vec::new();
        for (name, secret) in stored {
            let hidden: Vec<String> = match secret {
                StoredSecret::Variable { value } => vec![value],
                StoredSecret::Login { password, totp, .. } => {
                    let codes: Vec<String> = totp
                        .iter()
                        .flat_map(|seed| {
                            [now.saturating_sub(30), now, now + 30]
                                .into_iter()
                                .filter_map(|at| desk_browser::totp_code(seed, at))
                        })
                        .collect();
                    std::iter::once(password).chain(totp).chain(codes).collect()
                }
                StoredSecret::Passkey { private_key, .. } => vec![private_key],
            };
            values.extend(
                hidden
                    .into_iter()
                    .filter(|value| value.chars().count() >= 4)
                    .map(|value| (value, name.clone())),
            );
        }
        values.sort_by_key(|(value, _)| std::cmp::Reverse(value.len()));
        values.dedup_by(|a, b| a.0 == b.0);
        Self { values }
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Whether `text` carries any value, as it is or written into a link.
    fn found_in(&self, text: &str) -> bool {
        self.hide(text) != text
    }

    /// `text` with every value replaced by `[redacted NAME]`. A value in a
    /// link is percent-encoded, a space as `+` or `%20`, so a word with
    /// `%` or `+` in it is decoded too, and one that carries a value is
    /// answered decoded with it hidden.
    pub(crate) fn hide(&self, text: &str) -> String {
        if self.values.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while !rest.is_empty() {
            let word = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            out.push_str(&rest[..word]);
            rest = &rest[word..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push_str(&self.hide_word(&rest[..end]));
            rest = &rest[end..];
        }
        // A value with a space in it spans words.
        self.hide_plain(&out)
    }

    fn hide_word(&self, word: &str) -> String {
        let plain = self.hide_plain(word);
        if !(plain.contains('%') || plain.contains('+')) {
            return plain;
        }
        for plus_is_space in [true, false] {
            let decoded = percent_decoded(&plain, plus_is_space);
            let hidden = self.hide_plain(&decoded);
            if hidden != decoded {
                return hidden;
            }
        }
        plain
    }

    fn hide_plain(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (value, name) in &self.values {
            if text.contains(value.as_str()) {
                text = text.replace(value.as_str(), &format!("[redacted {name}]"));
            }
        }
        text
    }
}

/// `word` with each `%XX` turned back into its byte, and `+` into a space
/// when `plus_is_space`, read as UTF-8 with anything else replaced.
fn percent_decoded(word: &str, plus_is_space: bool) -> String {
    let bytes = word.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (byte, hex) {
            (b'%', Some(decoded)) => {
                out.push(decoded);
                at += 3;
            }
            (b'+', _) if plus_is_space => {
                out.push(b' ');
                at += 1;
            }
            _ => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether any of the hidden values shows on the page now, as a picture
/// would show it.
async fn secret_shows(
    persona_id: &str,
    browser: &dyn DeskBrowser,
    hidden: &Hidden,
) -> Result<bool, String> {
    if hidden.is_empty() {
        return Ok(false);
    }
    let showing = browser.showing(persona_id).await?;
    Ok(hidden.found_in(&showing.text) || showing.fields.iter().any(|value| hidden.found_in(value)))
}

/// `picture`, taken only while no secret shows on the page, and kept only
/// if none showed by the time it was taken either.
async fn shown_clear<T>(
    persona_id: &str,
    browser: &dyn DeskBrowser,
    hidden: &Hidden,
    picture: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if secret_shows(persona_id, browser, hidden).await? {
        return Err(SHOWN.into());
    }
    let taken = picture.await?;
    if secret_shows(persona_id, browser, hidden).await? {
        return Err(SHOWN.into());
    }
    Ok(taken)
}

/// After the page said a `wait_for`'s `find` is on it: whether it still is
/// once the secrets' values are hidden from the page's text. When it is
/// not, it was found only inside a value, and the wait answers as one that
/// ran out, at the time it would have, so neither its answer nor how long
/// it took tells the agent anything of the value.
async fn found_unhidden(
    persona_id: &str,
    browser: &dyn DeskBrowser,
    hidden: &Hidden,
    find: &str,
    timeout_ms: u64,
    started: std::time::Instant,
) -> Result<(), String> {
    if hidden.is_empty() {
        return Ok(());
    }
    let text = browser.showing(persona_id).await?.text;
    if hidden.hide(&text).contains(find) {
        return Ok(());
    }
    let timeout = std::time::Duration::from_millis(timeout_ms);
    tokio::time::sleep(timeout.saturating_sub(started.elapsed())).await;
    Err(format!(
        "Still not on the page after {} seconds.",
        (timeout_ms + 500) / 1000
    ))
}

/// An act's arguments, checked before the page is touched.
fn act_of(action: &str, arguments: &Value) -> Result<Act, String> {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let target = || {
        text("ref")
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("{action} needs the `ref` of an element from a text read."))
    };
    Ok(match action {
        "click_ref" => Act::Click { target: target()? },
        "hover" => Act::Hover { target: target()? },
        "fill" => {
            if arguments.get("text").is_some() && arguments.get("secret").is_some() {
                return Err("fill takes `text` or `secret`, not both.".into());
            }
            Act::Fill {
                target: target()?,
                text: text("text")
                    .ok_or("fill needs `text`: an empty string clears the field.")?
                    .to_string(),
            }
        }
        "select" => {
            let values: Vec<String> = match (arguments.get("values"), text("value")) {
                (Some(Value::Array(values)), _) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                (_, Some(value)) => vec![value.to_string()],
                _ => Vec::new(),
            };
            if values.is_empty() {
                return Err("select needs a `value`, or `values` for a multiple select.".into());
            }
            Act::Select {
                target: target()?,
                values,
            }
        }
        "check" => Act::Check {
            target: target()?,
            on: arguments.get("uncheck").and_then(Value::as_bool) != Some(true),
        },
        "press" => {
            let key = text("key").map(str::trim).unwrap_or_default();
            if !(desk_browser::KEYS.contains(&key) || key.chars().count() == 1) {
                return Err(format!(
                    "press needs a `key`: one character, or one of {}.",
                    desk_browser::KEYS.join(", ")
                ));
            }
            Act::Press {
                key: key.to_string(),
            }
        }
        "scroll" => Act::Scroll {
            target: text("ref").map(str::to_string),
            dy: arguments.get("dy").and_then(Value::as_i64),
        },
        "wait_for" => {
            let find = text("text").map(str::to_string);
            let target = text("ref").map(str::to_string);
            if find.is_none() && target.is_none() {
                return Err("wait_for needs `text` to find, or a `ref` to see.".into());
            }
            Act::WaitFor {
                text: find,
                target,
                timeout_ms: arguments
                    .get("timeout_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(10_000)
                    .min(desk_browser::WAIT_LIMIT_MS),
            }
        }
        other => return Err(format!("No act {other}.")),
    })
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

    pub(super) fn desk_browser(&self) -> Option<Arc<dyn DeskBrowser>> {
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

    /// The desk browser, while this teammate is offered it: where a cookie
    /// lent to the teammate goes.
    pub(super) fn lending_browser(&self, persona_id: &str) -> Option<Arc<dyn DeskBrowser>> {
        if self.offers_browser(persona_id) {
            self.desk_browser()
        } else {
            None
        }
    }

    /// The site this teammate's desk browser is on, by its registrable
    /// domain, as a cookie offer names it.
    pub(super) fn desk_browser_site(&self, persona_id: &str) -> Option<String> {
        let page = self.lending_browser(persona_id)?.current(persona_id)?;
        crate::computer::cookies::lending::site_for_origin(&page)
    }

    /// The page again after cookies were lent or taken back, so it shows the
    /// sign-in the teammate has now; the PIP follows.
    pub(super) async fn browser_reload(&self, persona_id: &str) {
        let Some(browser) = self.desk_browser() else {
            return;
        };
        if browser.current(persona_id).is_some()
            && browser.step(persona_id, Step::Reload).await.is_ok()
        {
            self.browser_moved(persona_id, browser.as_ref()).await;
        }
    }

    /// The page in this teammate's desk browser, as a PNG, for `send_file`:
    /// refused, as the teammate's own look is, while a secret shows on it.
    pub(crate) async fn browser_screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String> {
        if !self.offers_browser(persona_id) {
            return Err("You have no desk browser, so there is no browser page to send.".into());
        }
        let browser = self.desk_browser().ok_or("The desk browser has closed.")?;
        let hidden = self.hidden(persona_id).await?;
        let png = shown_clear(persona_id, browser.as_ref(), &hidden, async {
            browser.screenshot(persona_id).await
        })
        .await;
        png.map_err(|error| hidden.hide(&error))
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
        // A full-page look scrolls the page, so it waits, as an act does,
        // while the person is using it.
        let full_page = arguments.get("full_page").and_then(Value::as_bool) == Some(true);
        let reads = matches!(action, "text" | "links") || (action == "screenshot" && !full_page);
        if let Ok(persona) = self.persona(persona_id) {
            browser.name_teammate(persona_id, &persona.name);
        }
        if !reads && self.browser_phase(persona_id) == Some(BrowserPhase::Person) {
            return Err(TAKEN.into());
        }
        let hidden = self.hidden(persona_id).await?;
        let answer = self
            .browser_do(persona_id, action, arguments, browser.as_ref(), &hidden)
            .await;
        // A move that failed still moved the page, to a warning the person
        // may be needed for: the PIP follows it too, once there is a page.
        if !reads && (answer.is_ok() || browser.current(persona_id).is_some()) {
            self.browser_moved(persona_id, browser.as_ref()).await;
        }
        // Every word the agent is answered has the secrets' values hidden:
        // the page line's address and title, an act's note, an error. A
        // read hid them before it was cut into parts.
        match answer {
            Ok(BrowserAnswer::Text(text)) if !matches!(action, "text" | "links") => {
                Ok(BrowserAnswer::Text(hidden.hide(&text)))
            }
            Err(error) => Err(hidden.hide(&error)),
            answer => answer,
        }
    }

    async fn browser_do(
        &self,
        persona_id: &str,
        action: &str,
        arguments: &Value,
        browser: &dyn DeskBrowser,
        hidden: &Hidden,
    ) -> Result<BrowserAnswer, String> {
        if action == "fill"
            && let Some(secret) = arguments.get("secret").and_then(Value::as_str)
        {
            return self
                .fill_secret(persona_id, secret, arguments, browser)
                .await;
        }
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
                self.guard_local(persona_id, &url, browser).await?;
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
            "text" | "links" => {
                let reading = if action == "text" {
                    Reading::Text
                } else {
                    Reading::Links
                };
                let part = arguments
                    .get("part")
                    .and_then(Value::as_u64)
                    .map_or(1, |part| usize::try_from(part).unwrap_or(usize::MAX));
                let read = hidden.hide(&browser.read(persona_id, reading).await?);
                Ok(BrowserAnswer::Text(part_of(&read, part)?))
            }
            "click_ref" | "fill" | "select" | "check" | "hover" | "press" | "scroll"
            | "wait_for" => {
                let act = act_of(action, arguments)?;
                let started = std::time::Instant::now();
                let acted = browser.act(persona_id, &act).await?;
                if let Act::WaitFor {
                    text: Some(find),
                    timeout_ms,
                    ..
                } = &act
                {
                    found_unhidden(persona_id, browser, hidden, find, *timeout_ms, started).await?;
                }
                let note = acted
                    .note
                    .map(|note| format!("{note}\n"))
                    .unwrap_or_default();
                Ok(BrowserAnswer::Text(format!(
                    "{note}{}\nRead the page again with text to see what changed; refs you have still mean the same elements, and one whose element is gone will say so.",
                    page_line(&acted.page)
                )))
            }
            "screenshot" if arguments.get("full_page").and_then(Value::as_bool) == Some(true) => {
                let page = shown_clear(persona_id, browser, hidden, async {
                    browser.full_page(persona_id, FULL_PAGE_PICTURES).await
                })
                .await?;
                Ok(BrowserAnswer::Pictures {
                    pngs: page
                        .pictures
                        .iter()
                        .map(|png| STANDARD.encode(png))
                        .collect(),
                    more: page.more,
                })
            }
            "screenshot" => {
                let png = shown_clear(persona_id, browser, hidden, async {
                    browser.screenshot(persona_id).await
                })
                .await?;
                Ok(BrowserAnswer::Pictures {
                    pngs: vec![STANDARD.encode(png)],
                    more: false,
                })
            }
            other => Err(format!(
                "The desk browser has no action '{other}'. It can navigate, text, links, screenshot, back, forward, reload, click_ref, fill, select, check, hover, press, scroll and wait_for."
            )),
        }
    }

    /// `fill` with a stored secret: `NAME.username`, `NAME.password` or
    /// `NAME.code`, typed by the desk into a field fit for that part, on a
    /// page of the login's own sites, and never in the answer.
    async fn fill_secret(
        &self,
        persona_id: &str,
        secret: &str,
        arguments: &Value,
        browser: &dyn DeskBrowser,
    ) -> Result<BrowserAnswer, String> {
        if arguments.get("text").is_some() {
            return Err("fill takes `text` or `secret`, not both.".into());
        }
        let target = arguments
            .get("ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .ok_or("fill needs the `ref` of a field from a text read.")?
            .to_string();
        let (name, part) = self.granted_login(persona_id, secret)?;
        // The field first, so a refused one raises no card for the person.
        let field = browser.field(persona_id, &target).await?;
        if !field.takes(part) {
            return Err(format!(
                "{target} is {}, and {secret} goes only into {}. Nothing was typed.",
                field.what(),
                part.goes_into()
            ));
        }
        let value = self.secret_value(persona_id, name, part, browser).await?;
        let acted = browser
            .act(
                persona_id,
                &Act::FillSecret {
                    target,
                    text: value,
                    part,
                    field,
                },
            )
            .await?;
        Ok(BrowserAnswer::Text(format!(
            "Typed {secret} into the field; its value is never shown to you.\n{}",
            page_line(&acted.page)
        )))
    }

    /// The login `NAME.field` names and the part it asks for, once the
    /// teammate is granted `NAME`.
    fn granted_login<'a>(
        &self,
        persona_id: &str,
        secret: &'a str,
    ) -> Result<(&'a str, LoginPart), String> {
        let (name, part) = secret
            .rsplit_once('.')
            .and_then(|(name, field)| Some((name, LoginPart::of(field)?)))
            .ok_or("secret is NAME.username, NAME.password or NAME.code.")?;
        let persona = self.persona(persona_id)?;
        if !persona
            .secrets
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|granted| granted == name)
        {
            return Err(format!(
                "{name} is not one of your secrets. Ask the person to give it to you, or to sign in themselves."
            ));
        }
        Ok((name, part))
    }

    /// The value of `part` of the login `name` on the browser's page now,
    /// once its sites and, for an ask-me key, the person, allow it.
    async fn secret_value(
        &self,
        persona_id: &str,
        name: &str,
        part: LoginPart,
        browser: &dyn DeskBrowser,
    ) -> Result<String, String> {
        use crate::vault::StoredSecret;
        let persona = self.persona(persona_id)?;
        let vault = self
            .vault
            .clone()
            .ok_or("This desk has no secrets store.")?;
        let names = vec![name.to_string()];
        let (mut values, _) =
            tokio::task::spawn_blocking(move || vault.shared_secret_values(&names))
                .await
                .map_err(|_| "The secrets store could not be read.".to_string())?
                .map_err(|error| error.to_string())?;
        let stored = values
            .remove(name)
            .ok_or_else(|| format!("{name} is not stored any more."))?;
        let StoredSecret::Login {
            r#use,
            sites,
            username,
            password,
            totp,
        } = stored
        else {
            return Err(match stored {
                StoredSecret::Passkey { .. } => format!(
                    "{name} is a passkey: it signs in by itself when the site asks for one."
                ),
                _ => format!("{name} is a variable, which goes in a shell, never into a page."),
            });
        };
        let page = browser
            .current(persona_id)
            .and_then(|page| url::Url::parse(&page).ok())
            .ok_or("Your browser has no page open.")?;
        if !desk_browser::login_fits(&sites, &page) {
            return Err(format!(
                "{name} is for {}, and this page is {}. Nothing was typed.",
                sites.join(", "),
                page.origin().ascii_serialization()
            ));
        }
        let (value, field) = match part {
            LoginPart::Username => (username, "username"),
            LoginPart::Password => (password, "password"),
            LoginPart::Code => {
                let seed = totp.ok_or_else(|| format!("{name} has no code."))?;
                let now = u64::try_from(now_ms() / 1000).unwrap_or_default();
                let code = desk_browser::totp_code(&seed, now)
                    .ok_or_else(|| format!("{name}'s code could not be made."))?;
                (code, "code")
            }
        };
        if r#use == crate::contract::SecretUse::Ask {
            let doing = format!("type its {field} on {page}");
            self.ask_for_key(persona_id, &persona.name, name, &doing, &page)
                .await?;
        }
        Ok(value)
    }

    /// An ask-me key is used only once the person approves this use, or has
    /// allowed it on this site for a while. `doing` says what the use is:
    /// "type its password on …", "sign in with it on …".
    pub(super) async fn ask_for_key(
        &self,
        persona_id: &str,
        persona_name: &str,
        name: &str,
        doing: &str,
        page: &url::Url,
    ) -> Result<(), String> {
        let site = page.origin().ascii_serialization();
        let key = (persona_id.to_string(), name.to_string(), site.clone());
        if lock(&self.browser_key_allows)
            .get(&key)
            .is_some_and(|until| *until > now_ms())
        {
            return Ok(());
        }
        let title = format!(
            "{persona_name} wants to use {name} on {site}.\n\nIt would {doing} in its browser. You never see the value, and neither does {persona_name}."
        );
        let decision = self
            .ask_in_browser(persona_id, Asking::Use, title)
            .await
            .map_err(|()| {
                format!("Nobody answered the request to use {name}, so it was not typed.")
            })?;
        let until = match decision {
            Decision::Allow => now_ms() + APPROVED_MS,
            Decision::Always => now_ms() + ALLOWED_MS,
            Decision::Deny => {
                return Err(format!(
                    "The person denied using {name} here. Do not ask again for it in this task."
                ));
            }
        };
        lock(&self.browser_key_allows).insert(key, until);
        Ok(())
    }

    /// The values of the teammate's granted secrets, to hide from what its
    /// browser answers, read from the vault off the async threads, since
    /// a keychain read blocks. They are read afresh each call, so a value
    /// changed or a grant taken back is hidden or let go at once. A vault
    /// that cannot be read answers an error: the browser answers nothing it
    /// could not hide secrets from.
    pub(super) async fn hidden(&self, persona_id: &str) -> Result<Hidden, String> {
        let Some(vault) = self.vault.clone() else {
            return Ok(Hidden::default());
        };
        let names = self
            .persona(persona_id)
            .ok()
            .and_then(|persona| persona.secrets)
            .unwrap_or_default();
        if names.is_empty() {
            return Ok(Hidden::default());
        }
        let read = tokio::task::spawn_blocking(move || vault.shared_secret_values(&names))
            .await
            .map_err(|_| UNREAD.to_string())?;
        let (values, _) = read.map_err(|error| format!("{UNREAD} {error}"))?;
        let now = u64::try_from(now_ms() / 1000).unwrap_or_default();
        Ok(Hidden::of(values, now))
    }

    fn browser_phase(&self, persona_id: &str) -> Option<BrowserPhase> {
        lock(&self.browser_views)
            .get(persona_id)
            .map(|viewed| viewed.view.phase)
    }

    /// The teammate's browser as its session info carries it.
    pub fn browser_view(&self, persona_id: &str) -> Option<BrowserView> {
        lock(&self.browser_views)
            .get(persona_id)
            .map(|viewed| viewed.view.clone())
    }

    /// The last snapshot of the teammate's browser, for the PIP and phone.
    pub fn browser_snapshot(&self, persona_id: &str) -> Option<BrowserSnapshot> {
        let views = lock(&self.browser_views);
        let viewed = views.get(persona_id)?;
        Some(BrowserSnapshot {
            jpeg: STANDARD.encode(viewed.jpeg.as_ref()?),
            at: viewed.view.snapshot_at?,
            url: viewed.view.url.clone(),
            title: viewed.view.title.clone(),
        })
    }

    /// After the teammate moved its browser: where it is now, and a fresh
    /// snapshot, told to whoever watches the teammate's session.
    async fn browser_moved(&self, persona_id: &str, browser: &dyn DeskBrowser) {
        let png = browser.screenshot(persona_id).await.ok();
        let title = browser
            .read(persona_id, Reading::Title)
            .await
            .unwrap_or_default();
        let url = browser.current(persona_id).unwrap_or_default();
        let needs = browser.needs(persona_id);
        let jpeg = png.as_deref().and_then(snapshot_jpeg);
        let now = now_ms();
        {
            let mut views = lock(&self.browser_views);
            let phase = views
                .get(persona_id)
                .map_or(BrowserPhase::Teammate, |viewed| viewed.view.phase);
            let old = views.remove(persona_id);
            let (jpeg, snapshot_at) = match jpeg {
                Some(jpeg) => (Some(jpeg), Some(now)),
                None => old
                    .map(|old| (old.jpeg, old.view.snapshot_at))
                    .unwrap_or_default(),
            };
            views.insert(
                persona_id.to_string(),
                Viewed {
                    view: BrowserView {
                        phase,
                        url,
                        title,
                        active_at: now,
                        snapshot_at,
                        needs,
                    },
                    jpeg,
                },
            );
        }
        let _ = self.info_changes.send(self.info(persona_id));
    }

    /// The person typed `url` into the teammate's browser bar with control:
    /// a local address is theirs to open, so it is allowed for that teammate
    /// and that host and port, as Allow on a card would, with no card. The
    /// desk's own ports stay shut, to the person too: whatever page is left
    /// open there, the teammate reads.
    pub async fn browser_person_opens(
        &self,
        persona_id: &str,
        url: &url::Url,
    ) -> Result<(), String> {
        if !desk_browser::reaches_local(url).await {
            return Ok(());
        }
        let origin = desk_browser::origin_key(url).ok_or("That address has no host and port.")?;
        let port = url.port_or_known_default().unwrap_or(0);
        if self.browser_gate.never(port) || self.computers.live_ports().contains(&port) {
            return Err(format!(
                "{origin} is one of Hotline's own ports, which a teammate's browser never opens."
            ));
        }
        self.browser_gate.allow(persona_id, &origin);
        Ok(())
    }

    /// The person moved the teammate's browser from its pane: the PIP
    /// follows, as it does the teammate's moves.
    pub async fn browser_person_moved(&self, persona_id: &str) {
        let Some(browser) = self.desk_browser() else {
            return;
        };
        if browser.current(persona_id).is_some() {
            self.browser_moved(persona_id, browser.as_ref()).await;
        }
    }

    /// The person takes the teammate's browser over, or hands it back.
    pub async fn browser_take(&self, persona_id: &str, taken: bool) -> Result<(), String> {
        let browser = self.desk_browser().ok_or("This desk has no browser.")?;
        if browser.current(persona_id).is_none() {
            return Err("That teammate's browser has no page open.".into());
        }
        {
            let mut views = lock(&self.browser_views);
            let Some(viewed) = views.get_mut(persona_id) else {
                return Err("That teammate's browser has no page open.".into());
            };
            viewed.view.phase = if taken {
                BrowserPhase::Person
            } else {
                BrowserPhase::Teammate
            };
        }
        // What the person did while they had it is the page now.
        self.browser_moved(persona_id, browser.as_ref()).await;
        Ok(())
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
        let decision = self
            .ask_in_browser(
                persona_id,
                Asking::Local {
                    origin: origin.clone(),
                },
                title,
            )
            .await
            .map_err(|()| {
                format!("Nobody answered the request to open {origin}, so it was not opened.")
            })?;
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

    /// Raises a browser card on the teammate's chat and waits for the
    /// person's answer; an error when it was never answered.
    async fn ask_in_browser(
        &self,
        persona_id: &str,
        asking: Asking,
        title: String,
    ) -> Result<Decision, ()> {
        let request_id = format!("{BROWSER_REQUEST_PREFIX}{}", new_id());
        let (sender, answered) = oneshot::channel();
        let options = asking.options();
        lock(&self.browser_waits).insert(
            request_id.clone(),
            BrowserWait {
                persona_id: persona_id.to_string(),
                asking,
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
                options,
                decision: None,
                decided_option_name: None,
            },
        );
        match tokio::time::timeout(HUMAN_DEADLINE, answered).await {
            Ok(Ok(decision)) => Ok(decision),
            _ => Err(()),
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
        let Some((decision, name)) = wait.asking.decide(option_id) else {
            self.write_browser_decision(request_id, &wait, "expired", "Invalid answer");
            let _ = wait.sender.send(Decision::Deny);
            return Err("That card has no such answer.".into());
        };
        if decision == Decision::Always
            && let Asking::Local { origin } = &wait.asking
            && let Err(error) = self
                .browser_gate
                .allow_always(&self.log, &wait.persona_id, origin)
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
                options: wait.asking.options(),
                decision: Some(decision.to_string()),
                decided_option_name: Some(option_name.to_string()),
            },
        );
    }
}

#[cfg(test)]
mod parts {
    use super::part_of;
    use crate::browser::READ_LIMIT;

    #[test]
    fn a_short_page_is_one_answer_and_a_long_one_comes_in_parts_cut_between_lines() {
        assert_eq!(part_of("Page: A\nhello\n", 1).unwrap(), "Page: A\nhello\n");
        assert!(part_of("Page: A\n", 2).unwrap_err().contains("1 part"));

        let line = format!("{}\n", "x".repeat(99));
        let page = line.repeat(READ_LIMIT / 100 * 2 + 10);
        let first = part_of(&page, 1).unwrap();
        assert!(
            first.starts_with("[Part 1 of 3 of a long page. Read part 2 with `part: 2`.]"),
            "{}",
            &first[..80]
        );
        assert!(first.chars().count() <= READ_LIMIT + 100);
        let body = first.split_once("\n\n").unwrap().1;
        assert!(
            body.lines().all(|line| line.len() == 99),
            "cut between lines"
        );
        let last = part_of(&page, 3).unwrap();
        assert!(last.contains("That is the end of the page."));
        let whole: usize = (1..=3)
            .map(|part| {
                part_of(&page, part)
                    .unwrap()
                    .split_once("\n\n")
                    .unwrap()
                    .1
                    .lines()
                    .count()
            })
            .sum();
        assert_eq!(whole, page.lines().count(), "every line is in some part");
        assert!(part_of(&page, 4).unwrap_err().contains("3 parts"));

        let huge = "y".repeat(READ_LIMIT * 2 + 5);
        assert_eq!(
            (1..=3)
                .map(|part| part_of(&huge, part)
                    .unwrap()
                    .split_once("\n\n")
                    .unwrap()
                    .1
                    .len())
                .sum::<usize>(),
            huge.len(),
            "a line longer than a part is cut inside it"
        );
    }
}

#[cfg(test)]
mod hiding {
    use super::Hidden;
    use crate::vault::StoredSecret;

    fn hidden() -> Hidden {
        let stored = [
            (
                "GH".to_string(),
                StoredSecret::Login {
                    r#use: crate::contract::SecretUse::Auto,
                    sites: vec!["https://github.com".into()],
                    username: "ada".into(),
                    password: "p@ss word+1/é".into(),
                    totp: Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()),
                },
            ),
            (
                "NPM".to_string(),
                StoredSecret::Variable {
                    value: "npm_abc".into(),
                },
            ),
            (
                "SHORT".to_string(),
                StoredSecret::Variable {
                    value: "abc".into(),
                },
            ),
        ];
        Hidden::of(stored.into_iter().collect(), 59)
    }

    #[test]
    fn a_value_is_hidden_as_it_is_and_as_a_link_writes_it() {
        let hidden = hidden();
        for (text, shown) in [
            ("token p@ss word+1/é here", "token [redacted GH] here"),
            (
                "URL: https://github.com/search?q=p%40ss+word%2B1%2F%C3%A9&type=code",
                "URL: https://github.com/search?q=[redacted GH]&type=code",
            ),
            (
                "URL: https://github.com/search?q=p%40ss%20word%2b1%2f%c3%a9",
                "URL: https://github.com/search?q=[redacted GH]",
            ),
            ("Your code is 287082.", "Your code is [redacted GH]."),
            ("npm_abc and npm_abc", "[redacted NPM] and [redacted NPM]"),
            (
                "abc stays: too short to tell",
                "abc stays: too short to tell",
            ),
            ("Signed in as ada", "Signed in as ada"),
            ("a link of its own: /a%20b+c", "a link of its own: /a%20b+c"),
        ] {
            assert_eq!(hidden.hide(text), shown, "{text}");
        }
        assert!(hidden.found_in("x?seed=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"));
        assert!(!hidden.found_in("nothing here"));
        assert!(!Hidden::default().found_in("p@ss word+1/é"));
    }
}
