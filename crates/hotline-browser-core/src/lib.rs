//! A teammate's own browser on the desk, as the desk and every browser it
//! drives share it: what the agent may ask of a browser and what it answers
//! ([`DeskBrowser`] and its types), which addresses a page may load
//! ([`Gate`]), where a teammate's web data is kept ([`store_dir`]), the
//! pipe the server's browser helper talks over ([`pipe`]), and a browser
//! as a live screen in the computer viewer's protocol ([`screen`]).
//!
//! It is its own crate so that helper (`crates/hotline-browser`) builds
//! without the core, whose speech engine would otherwise be linked into a
//! process that only draws web pages. `hotline-core` re-exports all of it
//! as `browser`, beside what only the core needs (the `browser` tool's
//! sizes, logins and passkeys, and the helper's supervisor), so
//! `hotline_core::browser::Gate` still holds. It stands on
//! `hotline-contract`, `hotline-support` (the ports this process listens
//! on) and `hotline-store` (the room, where an "Always allow" is kept), and
//! must never depend on `hotline-core`, Tauri or GTK.
//!
//! Local addresses are guarded where the teammate asks for them
//! (docs/security.md, "A teammate's browser"): its `navigate` to a page on
//! this machine or the local network opens only once the person allowed
//! that host and port for that teammate, on a card that says who wants to
//! open what. Once a page is open the browser is a browser: its redirects,
//! sign-in round trips, popups and the frames it loads go where they go.
//! Only the desk's own ports and schemes that are not the web's never open,
//! by the teammate, the page or the person ([`Gate::admits`] for a page,
//! [`Gate::admits_frame`] for a frame in one).

#[cfg(unix)]
pub mod pipe;
pub mod screen;

use async_trait::async_trait;
use hotline_contract::BrowserInput;
use hotline_store::log::{Log, StreamId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{PoisonError, RwLock};
use url::{Host, Url};

/// Ollama's own port. A teammate's browser has no business with the
/// desk's model server, whatever the person answers.
const OLLAMA_PORT: u16 = 11434;

/// The room event that records an "Always allow" answer.
const ALLOWED: &str = "browser_local_allowed";

/// Where a teammate's browser is after a call that moved it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub url: String,
    pub title: String,
    /// False when the page was still loading when the call stopped waiting.
    pub loaded: bool,
}

/// A move through the browser's history, or the same page again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Back,
    Forward,
    Reload,
}

/// What a read of the page returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
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
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Showing {
    pub text: String,
    pub fields: Vec<String>,
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Sizes the page to `size`, width by height in CSS pixels, so it lays
    /// out and is pictured as a screen that size would show it (a phone's
    /// 390 by 844); `None` puts back the desk's own. Answers the page.
    async fn viewport(&self, _persona_id: &str, _size: Option<(u32, u32)>) -> Result<Page, String> {
        Err("This browser cannot change its size.".into())
    }
    /// The size the page was set to with `viewport`, when not the desk's own.
    fn size(&self, _persona_id: &str) -> Option<(u32, u32)> {
        None
    }
    /// The visible page's size in CSS pixels, as a picture of it shows it
    /// scaled and a point of the person's input is given in.
    async fn page_size(&self, persona_id: &str) -> Option<(u32, u32)> {
        self.size(persona_id)
    }
    /// The person's own input from a seat away from the desk, on a browser
    /// they have taken over: a tap at a point, a scroll, text typed where
    /// the focus is, or one key. The core has checked its shape and that
    /// the person has the browser. A browser that cannot take it says so.
    async fn person_input(&self, _persona_id: &str, _input: &BrowserInput) -> Result<(), String> {
        Err("This desk browser takes no input from away from the desk.".into())
    }
    /// Visit anyway: lets through the certificate the teammate's last load
    /// was refused on, that very one by the SHA-256 of its leaf, for this
    /// teammate, host and port until the desk stops, and loads the page
    /// again. The core has checked that the person has the browser; only
    /// they ever do this, never the teammate. A browser that refuses no
    /// certificate has none to let through.
    async fn visit_anyway(&self, _persona_id: &str) -> Result<(), String> {
        Err("This desk browser has no refused certificate to visit past.".into())
    }
    /// The person took the teammate's browser over (`true`) or handed it
    /// back, from wherever they did: the pane's shield, which keeps a
    /// watcher's clicks off the page, goes or comes back with it.
    fn taken(&self, _persona_id: &str, _taken: bool) {}
    /// Whether this browser is a live screen ([`DeskBrowser::screen`]): the
    /// desk says `browserStream` in its hello when it is.
    fn streams(&self) -> bool {
        false
    }
    /// One viewer's live screen of the teammate's browser, in the computer
    /// viewer's protocol ([`screen`]): the changed rectangles as they are
    /// drawn, and the person's pointer and keys for its own hands, which
    /// the browser takes only while the person has it (`taken`). A browser
    /// that is not a screen, or a teammate with no page, says so.
    async fn screen(&self, _persona_id: &str) -> Result<screen::Link, String> {
        Err("This desk browser is not a live screen.".into())
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
    /// Puts the person's cookies for one site, shared on its ask, into
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
    /// Closes the teammate's browser: its window, its page, a popup it
    /// opened, and everything the app keeps for it, so nothing of it is
    /// left driving or drawn. With `wipe`, its web storage goes too, its
    /// cookies (lent ones included), sign-ins and cache, as for a teammate
    /// deleted; without it the storage stays for its next page, as for one
    /// whose computer was turned on and may be turned off again. A teammate
    /// with no browser open is closed already, and a wipe still empties its
    /// storage.
    async fn close(&self, persona_id: &str, wipe: bool) -> Result<(), String>;
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
/// ("Always allow"), and the desk's own ports, never offered.
#[derive(Default)]
pub struct Gate {
    /// The ports given at install: the Door's, the dev window's server's,
    /// Ollama's. Every listener of this process's is asked at each load
    /// besides ([`hotline_support::listening`]), and so are the running computers'.
    never: RwLock<HashSet<u16>>,
    /// The loopback ports this desk's running computers are published on,
    /// asked at each load.
    computers: RwLock<Option<LivePorts>>,
    granted: RwLock<HashSet<(String, String)>>,
}

/// What answers the ports the desk's running computers are published on.
type LivePorts = Box<dyn Fn() -> Vec<u16> + Send + Sync>;

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
        granted.extend(always_allowed(log));
    }

    /// A teammate's "Always allow" answers, as host:port, sorted.
    pub fn always(&self, log: &Log, persona_id: &str) -> Vec<String> {
        let mut origins: Vec<String> = always_allowed(log)
            .into_iter()
            .filter(|(persona, _)| persona == persona_id)
            .map(|(_, origin)| origin)
            .collect();
        origins.sort();
        origins
    }

    /// Takes one answer back, "Always allow" or this run's "Allow": struck
    /// from the room and from this run, so the teammate's next `navigate` to
    /// it asks again.
    pub fn disallow(&self, log: &Log, persona_id: &str, origin: &str) -> Result<(), String> {
        self.granted
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(persona_id.to_string(), origin.to_string()));
        if self.always(log, persona_id).iter().any(|one| one == origin) {
            let mut struck = allowed_event(persona_id, origin);
            struck["deleted"] = true.into();
            log.append(&StreamId::Room, &struck)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Asks `live` for the running computers' published ports at each load.
    pub fn watch_computers(&self, live: impl Fn() -> Vec<u16> + Send + Sync + 'static) {
        *self
            .computers
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(live));
    }

    /// Whether `port` is one of the desk's own now: one given at install,
    /// one this process listens on, or a running computer's.
    pub fn never(&self, port: u16) -> bool {
        self.never
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&port)
            || hotline_support::listening::held(port)
            || self
                .computers
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .is_some_and(|live| live().contains(&port))
    }

    /// Whether `url` may reach one of the desk's own listeners: it is on one
    /// of the desk's ports, and its host is this machine or the local network
    /// by its address, or is a name. A name counts whatever it resolves to
    /// now: only WebKit's own lookup, at connect time, says where it lands,
    /// and a name that answered a public address to a check made first can
    /// answer loopback to the load (DNS rebinding), so no lookup made here
    /// could clear it. A public address on one of those ports is someone
    /// else's server and opens.
    pub fn own(&self, url: &Url) -> bool {
        let Some(port) = url.port_or_known_default() else {
            return false;
        };
        match url.host() {
            Some(Host::Ipv4(ip)) => is_local_ip(IpAddr::V4(ip)) && self.never(port),
            Some(Host::Ipv6(ip)) => is_local_ip(IpAddr::V6(ip)) && self.never(port),
            Some(Host::Domain(_)) => self.never(port),
            None => false,
        }
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

    /// A deleted teammate's answers: gone from this run, and each "Always
    /// allow" it had is struck from the room, so none outlives it.
    pub fn forget(&self, log: &Log, persona_id: &str) {
        self.granted
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(persona, _)| persona != persona_id);
        for origin in &self.always(log, persona_id) {
            let mut struck = allowed_event(persona_id, origin);
            struck["deleted"] = true.into();
            let _ = log.append(&StreamId::Room, &struck);
        }
    }

    /// Whether the browser may load `url` as its page, the top of the
    /// window: the app asks this before every top-level load, the agent's,
    /// the page's own and the person's. Web pages and the blank page load,
    /// anywhere but the desk's own ports ([`Self::own`]). Which local
    /// addresses the teammate may ask for is the `navigate` card's question,
    /// not this one's.
    pub fn admits(&self, url: &Url) -> bool {
        match url.scheme() {
            "about" => url.as_str() == "about:blank",
            "http" | "https" => !self.own(url),
            _ => false,
        }
    }

    /// Whether a frame inside the page may load `url`. A frame is the page's
    /// business, as in any browser: a sign-in, a payment or an embed, and
    /// the inline documents a page builds for one (`about:srcdoc`, `data:`,
    /// `blob:`), all load. Only what could reach this machine's files or
    /// the desk itself is refused: any other scheme (`file:`, the shell's
    /// own `tauri:` and `ipc:`), and the desk's own ports.
    pub fn admits_frame(&self, url: &Url) -> bool {
        match url.scheme() {
            "about" => matches!(url.as_str(), "about:blank" | "about:srcdoc"),
            "data" | "blob" => true,
            _ => self.admits(url),
        }
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

/// A teammate's web data directory under the data directory `root`: its
/// cookies, storage and cache, named by [`store_id`] in hex, as the Linux
/// app names its own under Hotline's. The server's browser helper keeps
/// each teammate's here, and a deleted teammate's is removed from here.
pub fn store_dir(root: &std::path::Path, persona_id: &str) -> std::path::PathBuf {
    let id: String = store_id(persona_id)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    root.join("desk-browser").join(id)
}

/// Every "Always allow" still standing, read in order: an answer struck
/// later (taken back, or its teammate deleted) is gone, and one given again
/// after that stands again.
fn always_allowed(log: &Log) -> HashSet<(String, String)> {
    let mut standing = HashSet::new();
    for event in log.load(&StreamId::Room) {
        if event["kind"] != ALLOWED {
            continue;
        }
        let (Some(persona), Some(origin)) = (event["persona"].as_str(), event["origin"].as_str())
        else {
            continue;
        };
        let key = (persona.to_string(), origin.to_string());
        if event["deleted"] == true {
            standing.remove(&key);
        } else {
            standing.insert(key);
        }
    }
    standing
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
mod tests;
