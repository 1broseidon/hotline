//! A teammate's own browser on the desk: a web view the desktop app keeps for
//! each teammate that has no computer, so a quick look at a page needs no
//! container, MCP server or Playwright. The core owns what the agent may ask
//! of it (the `browser` tool) and which addresses it may load; the app owns
//! the web view and implements [`DeskBrowser`]. A desk with no window, such
//! as `hotline serve`, installs none, and no teammate is offered the tool.
//!
//! Local addresses are guarded where the teammate asks for them
//! (docs/security.md, "A teammate's browser"): its `navigate` to a page on
//! this machine or the local network opens only once the person allowed
//! that host and port for that teammate, on a card that says who wants to
//! open what. Once a page is open the browser is a browser: its redirects,
//! sign-in round trips and the pages it loads go where they go. Only the
//! desk's own ports never open, by the teammate, the page or the person
//! ([`Gate::admits`]).

pub mod authenticator;

use crate::log::{Log, StreamId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{PoisonError, RwLock};
use url::{Host, Url};

/// The most one answer of a page's text or links may be, in characters: a
/// longer page comes in parts, each asked for by number. Kept under where
/// an agent's harness moves a long tool answer out to a file.
pub const READ_LIMIT: usize = 20_000;

/// Ollama's own port. A teammate's browser has no business with the
/// desk's model server, whatever the person answers.
const OLLAMA_PORT: u16 = 11434;

/// The room event that records an "Always allow" answer.
const ALLOWED: &str = "browser_local_allowed";

/// Where a teammate's browser is after a call that moved it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    /// False when the page was still loading when the call stopped waiting.
    pub loaded: bool,
}

/// A move through the browser's history, or the same page again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
    Reload,
}

/// What a read of the page returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// The page as lines a model can read: headings, text, links, buttons and
    /// fields, each control with a ref.
    Text,
    /// Every link on the page, with its text and where it goes.
    Links,
    /// The page's title alone.
    Title,
}

/// One act on the page, on a ref from the latest text read where it needs
/// one. Refs are the `e12` marks a text read puts on links and controls.
/// Its debug form never shows what a fill types: it may be a password.
#[derive(Clone, PartialEq, Eq)]
pub enum Act {
    Click {
        target: String,
    },
    /// Types `text` into a field, replacing what it held; empty clears it.
    Fill {
        target: String,
        text: String,
    },
    /// Types a login's `part` into a field the core found fit to take it
    /// ([`Field::takes`]). `field` is what the page said the field was, and
    /// the page types nothing if the field is no longer that.
    FillSecret {
        target: String,
        text: String,
        part: LoginPart,
        field: Field,
    },
    Select {
        target: String,
        values: Vec<String>,
    },
    Check {
        target: String,
        on: bool,
    },
    Hover {
        target: String,
    },
    /// One key, by name (`Enter`, `Tab`, `Escape`, `ArrowDown`…) or a single
    /// character, to whatever has focus.
    Press {
        key: String,
    },
    /// To a ref, or down the page by `dy` pixels (up when negative), or a
    /// screenful when neither is given.
    Scroll {
        target: Option<String>,
        dy: Option<i64>,
    },
    /// Until `text` is on the page or `target` is visible.
    WaitFor {
        text: Option<String>,
        target: Option<String>,
        timeout_ms: u64,
    },
}

impl std::fmt::Debug for Act {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fill { target, .. } => f
                .debug_struct("Fill")
                .field("target", target)
                .finish_non_exhaustive(),
            Self::FillSecret {
                target,
                part,
                field,
                ..
            } => f
                .debug_struct("FillSecret")
                .field("target", target)
                .field("part", part)
                .field("field", field)
                .finish_non_exhaustive(),
            Self::Click { target } => f.debug_struct("Click").field("target", target).finish(),
            Self::Select { target, values } => f
                .debug_struct("Select")
                .field("target", target)
                .field("values", values)
                .finish(),
            Self::Check { target, on } => f
                .debug_struct("Check")
                .field("target", target)
                .field("on", on)
                .finish(),
            Self::Hover { target } => f.debug_struct("Hover").field("target", target).finish(),
            Self::Press { key } => f.debug_struct("Press").field("key", key).finish(),
            Self::Scroll { target, dy } => f
                .debug_struct("Scroll")
                .field("target", target)
                .field("dy", dy)
                .finish(),
            Self::WaitFor {
                text,
                target,
                timeout_ms,
            } => f
                .debug_struct("WaitFor")
                .field("text", text)
                .field("target", target)
                .field("timeout_ms", timeout_ms)
                .finish(),
        }
    }
}

/// Whether a login stored for `sites` may be typed on `page`: same scheme
/// and port as one of its sites, on its host or a subdomain of it, as the
/// computer decides (docs/security.md). A site is `https://` unless it is
/// localhost.
pub fn login_fits(sites: &[String], page: &Url) -> bool {
    let Some(host) = page.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    sites.iter().any(|site| {
        let Ok(site) = Url::parse(site) else {
            return false;
        };
        let Some(site_host) = site.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        site.scheme() == page.scheme()
            && site.port_or_known_default() == page.port_or_known_default()
            && (host == site_host || host.ends_with(&format!(".{site_host}")))
    })
}

/// The part of a login a `fill` with `secret` types: `NAME.username`,
/// `NAME.password` or `NAME.code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginPart {
    Username,
    Password,
    Code,
}

impl LoginPart {
    /// The part a secret's `.field` names.
    pub fn of(field: &str) -> Option<Self> {
        match field {
            "username" => Some(Self::Username),
            "password" => Some(Self::Password),
            "code" => Some(Self::Code),
            _ => None,
        }
    }

    /// The fields this part goes into, as a refusal says it.
    pub fn goes_into(self) -> &'static str {
        match self {
            Self::Username => "a username or email field",
            Self::Password => "a password field",
            Self::Code => "a one-time-code field or a password field",
        }
    }
}

/// A field on the page as the desk reads it before it types a login's part
/// into it (`browser/field.js` in the app): the facts [`Field::takes`]
/// decides on, and the page checks again as it types. Words are
/// lowercased.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Field {
    /// `input`, `textarea`, `select`, `editable` for a contenteditable
    /// element, or another element's tag.
    pub tag: String,
    /// An input's type as the browser reads it (`text` when it has none).
    #[serde(rename = "type")]
    pub kind: String,
    pub autocomplete: String,
    pub inputmode: String,
    pub role: String,
    /// Its name, id, label, placeholder and ARIA label, joined.
    pub words: String,
    /// It is inside a search landmark or a search form.
    pub in_search: bool,
    /// Its form has a password field.
    pub with_password: bool,
}

/// What the words of a field say when it asks for a one-time code: one of
/// these runs of letters, with its separators taken out...
const CODE_WORDS: [&str; 9] = [
    "onetime",
    "totp",
    "twofactor",
    "2fa",
    "passcode",
    "verification",
    "verify",
    "authenticat",
    "authcode",
];
/// ...or a word that is, starts or ends with one of these...
const CODE_ENDS: [&str; 3] = ["otp", "mfa", "code"];
/// ...unless it names one of these other codes.
const NOT_CODE_WORDS: [&str; 15] = [
    "zip", "postal", "postcode", "promo", "coupon", "discount", "gift", "voucher", "referral",
    "invite", "country", "area", "card", "cvv", "cvc",
];

impl Field {
    /// The purpose its `autocomplete` names, past section, address and
    /// contact-kind words, `on`, `off` and `webauthn`.
    fn purpose(&self) -> Option<&str> {
        self.autocomplete.split_ascii_whitespace().rfind(|token| {
            !(token.starts_with("section-")
                || matches!(
                    *token,
                    "shipping"
                        | "billing"
                        | "home"
                        | "work"
                        | "mobile"
                        | "fax"
                        | "pager"
                        | "on"
                        | "off"
                        | "webauthn"
                ))
        })
    }

    /// Its words, lowercased, one at a time.
    fn word_list(&self) -> Vec<String> {
        self.words
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// A search box: its type or role, a search landmark or form around
    /// it, or its words (`search`, `q`, `query`).
    fn search(&self) -> bool {
        self.kind == "search"
            || self.role == "searchbox"
            || self.in_search
            || self
                .word_list()
                .iter()
                .any(|word| word == "q" || word == "query" || word.contains("search"))
    }

    /// Its words say it asks for a one-time code.
    fn named_like_code(&self) -> bool {
        let flat: String = self.word_list().concat();
        if NOT_CODE_WORDS.iter().any(|word| flat.contains(word)) {
            return false;
        }
        CODE_WORDS.iter().any(|word| flat.contains(word))
            || self.word_list().iter().any(|word| {
                CODE_ENDS
                    .iter()
                    .any(|end| word.starts_with(end) || word.ends_with(end))
            })
    }

    /// A one-time-code field: a text, tel or number input that says so by
    /// `autocomplete="one-time-code"`, or, with no other purpose, takes
    /// digits (`inputmode="numeric"`, or type tel or number) and is named
    /// or labelled like a code.
    fn one_time_code(&self) -> bool {
        if !matches!(self.kind.as_str(), "text" | "tel" | "number") {
            return false;
        }
        match self.purpose() {
            Some(purpose) => purpose == "one-time-code",
            None => {
                (self.inputmode == "numeric" || matches!(self.kind.as_str(), "tel" | "number"))
                    && self.named_like_code()
            }
        }
    }

    /// A username field: `autocomplete` username or email on a text, email
    /// or tel input, an email input, or a text input in a form with a
    /// password field, with no other purpose.
    fn username(&self) -> bool {
        match self.purpose() {
            Some("username" | "email") => matches!(self.kind.as_str(), "text" | "email" | "tel"),
            Some(_) => false,
            None => self.kind == "email" || (self.kind == "text" && self.with_password),
        }
    }

    /// Whether this field may take `part` of a login. Only an input does,
    /// never a search box: a password goes into a password field only, a
    /// code into a one-time-code field or a password field, and a username
    /// into a username or email field (docs/security.md, "Logins in the
    /// desk browser").
    pub fn takes(&self, part: LoginPart) -> bool {
        if self.tag != "input" || self.search() {
            return false;
        }
        match part {
            LoginPart::Password => self.kind == "password",
            LoginPart::Code => self.kind == "password" || self.one_time_code(),
            LoginPart::Username => self.username(),
        }
    }

    /// What it is, as a refusal names it.
    pub fn what(&self) -> String {
        match self.tag.as_str() {
            "input" if self.search() => "a search field".into(),
            "input" => match self.kind.as_str() {
                "password" => "a password field".into(),
                "email" => "an email field".into(),
                "text" => "a text field".into(),
                other => format!("a {other} field"),
            },
            "textarea" => "a text area".into(),
            "editable" => "an editable part of the page".into(),
            "select" => "a select".into(),
            _ => "not a field".into(),
        }
    }
}

/// What a picture of the page could show, as words: its rendered text, in
/// open shadow roots and frames of its own origin too, and the value of
/// every field but a masked password field. The core looks for a secret's
/// value in it before and after a screenshot. It has no debug form: it may
/// hold one.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Showing {
    pub text: String,
    pub fields: Vec<String>,
}

/// The six-digit code a base32 TOTP seed gives at `unix_seconds`
/// (RFC 6238: SHA-1, 30-second steps).
pub fn totp_code(seed: &str, unix_seconds: u64) -> Option<String> {
    use hmac::{Hmac, Mac};
    let cleaned: String = seed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect::<String>()
        .to_ascii_uppercase();
    let key = data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .ok()?;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    mac.update(&(unix_seconds / 30).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 0x0f);
    let value = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    Some(format!("{:06}", value % 1_000_000))
}

/// The longest a `wait_for` may wait.
pub const WAIT_LIMIT_MS: u64 = 15_000;

/// The keys `press` knows by name, besides a single character.
pub const KEYS: [&str; 15] = [
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "Space",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Shift+Tab",
];

/// What an act left: the page it is on, and anything worth saying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acted {
    pub page: Page,
    pub note: Option<String>,
}

/// The whole page as `screenshot` with `full_page` looks at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullPage {
    /// PNGs from the top of the page down, one viewport tall each.
    pub pictures: Vec<Vec<u8>>,
    /// The page goes on below the last picture.
    pub more: bool,
}

/// The desktop app's web views, one per teammate, driven by the core.
#[async_trait]
pub trait DeskBrowser: Send + Sync {
    /// Loads `url` as the page, making the teammate's browser if it has none.
    /// The core has already checked the address against the guard.
    async fn open(&self, persona_id: &str, url: &Url) -> Result<Page, String>;
    async fn step(&self, persona_id: &str, step: Step) -> Result<Page, String>;
    async fn read(&self, persona_id: &str, reading: Reading) -> Result<String, String>;
    /// Does `act`, and waits for a load it starts. The core has checked its
    /// shape; the app checks the ref is still on the page.
    async fn act(&self, persona_id: &str, act: &Act) -> Result<Acted, String>;
    /// The visible part of the page as a PNG.
    async fn screenshot(&self, persona_id: &str) -> Result<Vec<u8>, String>;
    /// The page from the top, a viewport at a time, as at most `most`
    /// pictures, with its scroll put back where it was. A browser that
    /// cannot scroll and look answers the visible part alone.
    async fn full_page(&self, persona_id: &str, _most: usize) -> Result<FullPage, String> {
        Ok(FullPage {
            pictures: vec![self.screenshot(persona_id).await?],
            more: false,
        })
    }
    /// What the field `target`, a ref from the latest text read, is, as a
    /// login's part is checked against it before it is typed. A browser
    /// that cannot say types no secret.
    async fn field(&self, _persona_id: &str, target: &str) -> Result<Field, String> {
        Err(format!(
            "This browser cannot tell what {target} is, so it types no secret into it."
        ))
    }
    /// What a picture of the page could show, as words, for the core to
    /// look for a secret in before it hands one over. A browser that cannot
    /// say takes no picture while there is a secret to look for.
    async fn showing(&self, _persona_id: &str) -> Result<Showing, String> {
        Err("This browser cannot check its page for your secrets, so it takes no picture while you hold any.".into())
    }
    /// The page the teammate's browser is on, when it has one.
    fn current(&self, persona_id: &str) -> Option<String>;
    /// What the teammate's browser waits on the person for, in a few words
    /// ("Certificate expired"), as the PIP marks it; `None` while nothing.
    fn needs(&self, _persona_id: &str) -> Option<String> {
        None
    }
    /// The teammate's name, as its pointer over the page is tagged; the
    /// core says it before each call.
    fn name_teammate(&self, _persona_id: &str, _name: &str) {}
    /// Puts the person's cookies for one site, lent on a cookie offer, into
    /// the teammate's own web storage. Each is `{name, value, domain, path,
    /// expires, httpOnly, secure, sameSite}`; the values go to WebKit and
    /// nowhere else, so an error never repeats one.
    async fn lend_cookies(&self, _persona_id: &str, _cookies: &[Value]) -> Result<(), String> {
        Err(NO_COOKIES.into())
    }
    /// Deletes every cookie on `site`, a registrable domain, and its
    /// subdomains from the teammate's web storage, whether or not its
    /// browser is open.
    async fn forget_cookies(&self, _persona_id: &str, _site: &str) -> Result<(), String> {
        Err(NO_COOKIES.into())
    }
}

const NO_COOKIES: &str = "This desk browser cannot hold lent cookies.";

/// Whether an address is this machine or the local network: loopback,
/// private ranges, link-local, carrier-grade NAT, unique-local IPv6 and the
/// unspecified address, including IPv4 written as IPv6.
pub fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (a == 100 && (b & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_local_ip(IpAddr::V4(v4)))
        }
    }
}

/// Whether a link names a local address on its face, without asking DNS:
/// `localhost` and its subdomains, `.local` names and local IP literals.
pub fn names_local(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            #[allow(
                clippy::case_sensitive_file_extension_comparisons,
                reason = "the domain is lowercased above"
            )]
            let local = domain == "localhost"
                || domain.ends_with(".localhost")
                || domain.ends_with(".local");
            local
        }
        Some(Host::Ipv4(ip)) => is_local_ip(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => is_local_ip(IpAddr::V6(ip)),
        None => false,
    }
}

/// Whether a link reaches a local address: by its name, or by what its name
/// resolves to now, so a public name pointed at this machine is caught too.
/// A name that does not resolve is not local; the browser fails to load it.
pub async fn reaches_local(url: &Url) -> bool {
    if names_local(url) {
        return true;
    }
    let (Some(Host::Domain(domain)), Some(port)) = (url.host(), url.port_or_known_default()) else {
        return false;
    };
    match tokio::net::lookup_host((domain, port)).await {
        Ok(addresses) => addresses
            .into_iter()
            .any(|address| is_local_ip(address.ip())),
        Err(_) => false,
    }
}

/// A certificate's SHA-256, in lowercase hex, as the desk browser pins the
/// one the person chose to visit anyway.
pub fn certificate_fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The host and port a guard answer is for, as the card shows it:
/// `localhost:2322`, `[::1]:8080`, `192.168.1.20:80`.
pub fn origin_key(url: &Url) -> Option<String> {
    let host = match url.host()? {
        Host::Domain(domain) => domain.trim_end_matches('.').to_ascii_lowercase(),
        Host::Ipv4(ip) => ip.to_string(),
        Host::Ipv6(ip) => format!("[{ip}]"),
    };
    Some(format!("{host}:{}", url.port_or_known_default()?))
}

/// Which local addresses each teammate's browser may load: the person's
/// answers, held for the desk's run ("Allow") or in the room for good
/// ("Always allow"), and the ports never offered.
#[derive(Default)]
pub struct Gate {
    never: RwLock<HashSet<u16>>,
    granted: RwLock<HashSet<(String, String)>>,
}

impl Gate {
    /// Reads the standing answers from the room and takes `never` as the
    /// desk's own ports, with Ollama's.
    pub fn load(&self, log: &Log, never: &[u16]) {
        {
            let mut ports = self.never.write().unwrap_or_else(PoisonError::into_inner);
            ports.insert(OLLAMA_PORT);
            ports.extend(never.iter().copied());
        }
        let mut granted = self.granted.write().unwrap_or_else(PoisonError::into_inner);
        for event in log.load(&StreamId::Room) {
            if event["kind"] == ALLOWED
                && event["deleted"] != true
                && let (Some(persona), Some(origin)) =
                    (event["persona"].as_str(), event["origin"].as_str())
            {
                granted.insert((persona.to_string(), origin.to_string()));
            }
        }
    }

    /// Whether `port` is one the guard never offers.
    pub fn never(&self, port: u16) -> bool {
        self.never
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&port)
    }

    pub fn granted(&self, persona_id: &str, origin: &str) -> bool {
        self.granted
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(persona_id.to_string(), origin.to_string()))
    }

    /// "Allow": until the desk stops.
    pub fn allow(&self, persona_id: &str, origin: &str) {
        self.granted
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((persona_id.to_string(), origin.to_string()));
    }

    /// "Always allow": recorded in the room, then held.
    pub fn allow_always(&self, log: &Log, persona_id: &str, origin: &str) -> Result<(), String> {
        log.append(&StreamId::Room, &allowed_event(persona_id, origin))
            .map_err(|error| error.to_string())?;
        self.allow(persona_id, origin);
        Ok(())
    }

    /// Whether the browser may load `url` as its page at all. The app asks
    /// this before every top-level load: web pages and the blank page load,
    /// anywhere but the desk's own ports on this machine or the network.
    /// Which local addresses the teammate may ask for is the `navigate`
    /// card's question, not this one's.
    pub fn admits(&self, url: &Url) -> bool {
        match url.scheme() {
            "about" => return url.as_str() == "about:blank",
            "http" | "https" => {}
            _ => return false,
        }
        !(names_local(url)
            && url
                .port_or_known_default()
                .is_some_and(|port| self.never(port)))
    }
}

/// The id of a teammate's own web storage, so its cookies and sign-ins are
/// its own: never Hotline's window's, never another teammate's.
pub fn store_id(persona_id: &str) -> [u8; 16] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("hotline-browser:{persona_id}").as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

fn allowed_event(persona_id: &str, origin: &str) -> Value {
    json!({
        "kind": ALLOWED,
        "id": format!("browser-local:{persona_id}:{origin}"),
        "persona": persona_id,
        "origin": origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn local_addresses_are_this_machine_and_the_local_network() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_local_ip(local.parse().unwrap()), "{local}");
        }
        for public in ["8.8.8.8", "1.1.1.1", "100.128.0.1", "2606:4700::1111"] {
            assert!(!is_local_ip(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn a_link_names_a_local_address_by_localhost_dot_local_or_an_ip() {
        assert!(names_local(&url("http://localhost:2322/")));
        assert!(names_local(&url("http://app.localhost/")));
        assert!(names_local(&url("http://printer.local/")));
        assert!(names_local(&url("http://192.168.1.20/")));
        assert!(names_local(&url("http://[::1]:8080/")));
        assert!(!names_local(&url("https://example.com/")));
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_this_machine_reaches_a_local_address() {
        // `localhost.` with its trailing dot resolves through the system,
        // and the IP literal never asks DNS at all.
        assert!(reaches_local(&url("http://127.0.0.1:2322/")).await);
        assert!(reaches_local(&url("http://localhost.:2322/")).await);
    }

    #[test]
    fn an_origin_key_is_the_host_and_port_the_card_shows() {
        assert_eq!(
            origin_key(&url("http://localhost:2322/a")).as_deref(),
            Some("localhost:2322")
        );
        assert_eq!(
            origin_key(&url("http://192.168.1.20/")).as_deref(),
            Some("192.168.1.20:80")
        );
        assert_eq!(
            origin_key(&url("http://[::1]:8080/")).as_deref(),
            Some("[::1]:8080")
        );
    }

    #[test]
    fn the_gate_admits_every_web_page_but_the_desks_own_ports() {
        let gate = Gate::default();
        gate.never.write().unwrap().insert(4000);
        assert!(gate.admits(&url("https://example.com/")));
        assert!(gate.admits(&url("about:blank")));
        assert!(gate.admits(&url("http://localhost:2322/")));
        assert!(gate.admits(&url("https://pm.dfam.example:8443/sso")));
        assert!(gate.admits(&url("http://192.168.1.20/")));
        assert!(!gate.admits(&url("http://localhost:4000/")));
        assert!(!gate.admits(&url("http://127.0.0.1:4000/door")));
        assert!(!gate.admits(&url("file:///etc/passwd")));
        assert!(!gate.admits(&url("tauri://localhost/")));
    }

    #[test]
    fn a_login_fits_its_own_sites_and_their_subdomains_only() {
        let sites = vec![
            "https://github.com".to_string(),
            "http://localhost:3000".to_string(),
        ];
        assert!(login_fits(&sites, &url("https://github.com/login")));
        assert!(login_fits(&sites, &url("https://gist.github.com/")));
        assert!(login_fits(&sites, &url("http://localhost:3000/signin")));
        assert!(
            !login_fits(&sites, &url("http://github.com/login")),
            "scheme"
        );
        assert!(
            !login_fits(&sites, &url("https://github.com.evil.dev/")),
            "suffix trick"
        );
        assert!(
            !login_fits(&sites, &url("https://evilgithub.com/")),
            "not a subdomain"
        );
        assert!(!login_fits(&sites, &url("http://localhost:3001/")), "port");
    }

    #[test]
    fn a_certificate_fingerprint_is_its_sha_256() {
        assert_eq!(
            certificate_fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_totp_code_is_rfc_6238() {
        // RFC 6238's SHA-1 test seed, "12345678901234567890", in base32.
        let seed = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        assert_eq!(totp_code(seed, 59).as_deref(), Some("287082"));
        assert_eq!(totp_code(seed, 1_111_111_109).as_deref(), Some("081804"));
        assert_eq!(totp_code("not base32!", 59), None);
    }

    fn input(kind: &str, autocomplete: &str, words: &str) -> Field {
        Field {
            tag: "input".into(),
            kind: kind.into(),
            autocomplete: autocomplete.into(),
            words: words.into(),
            ..Field::default()
        }
    }

    #[test]
    fn a_login_part_goes_only_into_a_field_made_for_it() {
        use LoginPart::{Code, Password, Username};
        let takes = |field: &Field| {
            [Username, Password, Code]
                .into_iter()
                .filter(|part| field.takes(*part))
                .collect::<Vec<_>>()
        };
        let numeric = |kind: &str, words: &str| Field {
            inputmode: "numeric".into(),
            ..input(kind, "", words)
        };
        let in_form = |field: Field| Field {
            with_password: true,
            ..field
        };
        for (field, parts, why) in [
            (
                input("password", "current-password", "password"),
                vec![Password, Code],
                "a password field",
            ),
            (input("password", "", ""), vec![Password, Code], "any one"),
            (
                input("text", "one-time-code", ""),
                vec![Code],
                "autocomplete says one-time code",
            ),
            (
                input("tel", "section-mfa one-time-code", ""),
                vec![Code],
                "past a section",
            ),
            (numeric("text", "app_otp"), vec![Code], "numeric, named otp"),
            (
                numeric("text", "user[otp_attempt]"),
                vec![Code],
                "a word starting with otp",
            ),
            (
                numeric("text", "verification code"),
                vec![Code],
                "labelled a code",
            ),
            (input("tel", "", "totpPin"), vec![Code], "tel, named totp"),
            (
                numeric("text", "zip code"),
                vec![],
                "a postal code is not one",
            ),
            (
                Field {
                    inputmode: "numeric".into(),
                    ..input("text", "postal-code", "code")
                },
                vec![],
                "nor a field whose autocomplete is another purpose",
            ),
            (numeric("text", "amount"), vec![], "digits alone"),
            (
                input("text", "", "app_otp"),
                vec![],
                "a code's name on a field that takes any text",
            ),
            (
                input("text", "username", "login_field"),
                vec![Username],
                "autocomplete username",
            ),
            (
                input("text", "username webauthn", ""),
                vec![Username],
                "past webauthn",
            ),
            (
                input("email", "", "email"),
                vec![Username],
                "an email field",
            ),
            (
                input("tel", "username", "phone"),
                vec![Username],
                "a phone number as the username",
            ),
            (
                in_form(input("text", "", "login")),
                vec![Username],
                "a text field in a form with a password field",
            ),
            (
                input("text", "", "login"),
                vec![],
                "a text field in no sign-in form",
            ),
            (
                in_form(input("text", "street-address", "address")),
                vec![],
                "a field for something else in that form",
            ),
            (in_form(input("search", "", "")), vec![], "a search field"),
            (
                in_form(input("text", "username", "Search or jump to…")),
                vec![],
                "worded like search",
            ),
            (in_form(input("text", "", "q")), vec![], "named q"),
            (
                Field {
                    in_search: true,
                    ..input("password", "", "")
                },
                vec![],
                "in a search landmark",
            ),
            (
                Field {
                    role: "searchbox".into(),
                    ..input("email", "", "")
                },
                vec![],
                "a searchbox",
            ),
            (
                Field {
                    tag: "textarea".into(),
                    ..in_form(input("", "username", ""))
                },
                vec![],
                "a text area",
            ),
            (
                Field {
                    tag: "editable".into(),
                    ..in_form(input("", "", ""))
                },
                vec![],
                "an editable part of the page",
            ),
        ] {
            assert_eq!(takes(&field), parts, "{why}: {field:?}");
        }
    }

    #[test]
    fn a_secrets_fill_never_shows_its_text_in_debug() {
        let act = Act::FillSecret {
            target: "e3".into(),
            text: "hunter22".into(),
            part: LoginPart::Password,
            field: input("password", "", ""),
        };
        assert!(!format!("{act:?}").contains("hunter22"));
    }

    #[test]
    fn a_fill_never_shows_its_text_in_debug() {
        let act = Act::Fill {
            target: "e3".into(),
            text: "hunter2".into(),
        };
        assert!(!format!("{act:?}").contains("hunter2"));
    }

    #[test]
    fn always_allow_is_kept_in_the_room_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let log = Log::open(dir.path());
        Gate::default()
            .allow_always(&log, "kit", "localhost:2322")
            .unwrap();
        let gate = Gate::default();
        gate.load(&log, &[7000]);
        assert!(gate.granted("kit", "localhost:2322"));
        assert!(gate.never(7000));
        assert!(gate.never(OLLAMA_PORT));
    }
}
