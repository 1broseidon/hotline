//! The `browser` tool on the room's side: what a teammate asks of its own
//! browser, the guard card a local address raises, and the person's answer
//! to it. The browser itself is the app's ([`crate::browser::DeskBrowser`]).

use super::*;
use crate::browser::{self as desk_browser, Act, DeskBrowser, Gate, Page, Reading, Step};
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

    /// The page in this teammate's desk browser, as a PNG, for `send_file`.
    pub(crate) async fn browser_screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String> {
        if !self.offers_browser(persona_id) {
            return Err("You have no desk browser, so there is no browser page to send.".into());
        }
        self.desk_browser()
            .ok_or("The desk browser has closed.")?
            .screenshot(persona_id)
            .await
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
        let answer = self
            .browser_do(persona_id, action, arguments, browser.as_ref())
            .await;
        // A move that failed still moved the page, to a warning the person
        // may be needed for: the PIP follows it too, once there is a page.
        if !reads && (answer.is_ok() || browser.current(persona_id).is_some()) {
            self.browser_moved(persona_id, browser.as_ref()).await;
        }
        answer
    }

    async fn browser_do(
        &self,
        persona_id: &str,
        action: &str,
        arguments: &Value,
        browser: &dyn DeskBrowser,
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
            "text" => Ok(BrowserAnswer::Text(capped(self.redacted(
                persona_id,
                browser.read(persona_id, Reading::Text).await?,
            )))),
            "links" => Ok(BrowserAnswer::Text(capped(self.redacted(
                persona_id,
                browser.read(persona_id, Reading::Links).await?,
            )))),
            "click_ref" | "fill" | "select" | "check" | "hover" | "press" | "scroll"
            | "wait_for" => {
                let act = act_of(action, arguments)?;
                let acted = browser.act(persona_id, &act).await?;
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
                let page = browser.full_page(persona_id, FULL_PAGE_PICTURES).await?;
                Ok(BrowserAnswer::Pictures {
                    pngs: page
                        .pictures
                        .iter()
                        .map(|png| STANDARD.encode(png))
                        .collect(),
                    more: page.more,
                })
            }
            "screenshot" => Ok(BrowserAnswer::Pictures {
                pngs: vec![STANDARD.encode(browser.screenshot(persona_id).await?)],
                more: false,
            }),
            other => Err(format!(
                "The desk browser has no action '{other}'. It can navigate, text, links, screenshot, back, forward, reload, click_ref, fill, select, check, hover, press, scroll and wait_for."
            )),
        }
    }

    /// `fill` with a stored secret: `NAME.username`, `NAME.password` or
    /// `NAME.code`, typed by the desk on a page of the login's own sites,
    /// and never in the answer.
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
        let value = self.secret_value(persona_id, secret, browser).await?;
        let acted = browser
            .act(
                persona_id,
                &Act::Fill {
                    target,
                    text: value,
                },
            )
            .await?;
        Ok(BrowserAnswer::Text(format!(
            "Typed {secret} into the field; its value is never shown to you.\n{}",
            page_line(&acted.page)
        )))
    }

    /// The value `NAME.field` stands for on the browser's page now, once its
    /// grant, its sites and, for an ask-me key, the person, allow it.
    async fn secret_value(
        &self,
        persona_id: &str,
        secret: &str,
        browser: &dyn DeskBrowser,
    ) -> Result<String, String> {
        use crate::vault::StoredSecret;
        let (name, field) = secret
            .rsplit_once('.')
            .filter(|(_, field)| matches!(*field, "username" | "password" | "code"))
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
        let vault = self
            .vault
            .as_ref()
            .ok_or("This desk has no secrets store.")?;
        let (mut values, _) = vault
            .shared_secret_values(&[name.to_string()])
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
        let value = match field {
            "username" => username,
            "password" => password,
            _ => {
                let seed = totp.ok_or_else(|| format!("{name} has no code."))?;
                let now = u64::try_from(now_ms() / 1000).unwrap_or_default();
                desk_browser::totp_code(&seed, now)
                    .ok_or_else(|| format!("{name}'s code could not be made."))?
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

    /// `text` with every value of the teammate's granted secrets replaced by
    /// its name: a page that shows a key or a password back does not hand it
    /// to the agent.
    fn redacted(&self, persona_id: &str, text: String) -> String {
        use crate::vault::StoredSecret;
        let Some(vault) = &self.vault else {
            return text;
        };
        let names = self
            .persona(persona_id)
            .ok()
            .and_then(|persona| persona.secrets)
            .unwrap_or_default();
        if names.is_empty() {
            return text;
        }
        let Ok((values, _)) = vault.shared_secret_values(&names) else {
            return text;
        };
        let mut text = text;
        for (name, secret) in values {
            let hidden: Vec<String> = match secret {
                StoredSecret::Variable { value } => vec![value],
                StoredSecret::Login { password, totp, .. } => {
                    std::iter::once(password).chain(totp).collect()
                }
                StoredSecret::Passkey { private_key, .. } => vec![private_key],
            };
            for value in hidden
                .into_iter()
                .filter(|value| value.chars().count() >= 4)
            {
                text = text.replace(&value, &format!("[redacted {name}]"));
            }
        }
        text
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
