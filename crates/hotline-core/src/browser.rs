//! A teammate's own browser on the desk: a web view the desktop app keeps for
//! each teammate that has no computer, so a quick look at a page needs no
//! container, MCP server or Playwright. The core owns what the agent may ask
//! of it (the `browser` tool) and which addresses it may load; the app owns
//! the web view and implements [`DeskBrowser`]. A desk with no window, such
//! as `hotline serve`, installs none, and no teammate is offered the tool.
//!
//! Local addresses are guarded (docs/security.md, "A teammate's browser"):
//! a page on this machine or the local network opens only once the person
//! allowed that host and port for that teammate, on a card that says who
//! wants to open what. The desk's own ports are never offered. A page a
//! public site sends the browser to on its own is held to the same answers
//! by [`Gate::admits`], which the app asks before every top-level load.

use crate::log::{Log, StreamId};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{PoisonError, RwLock};
use url::{Host, Url};

/// The most a page's text or links may be before the answer is cut, in
/// characters. A page longer than this says where it was cut.
pub const READ_LIMIT: usize = 60_000;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Click {
        target: String,
    },
    /// Types `text` into a field, replacing what it held; empty clears it.
    Fill {
        target: String,
        text: String,
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
    /// The page the teammate's browser is on, when it has one.
    fn current(&self, persona_id: &str) -> Option<String>;
}

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

    /// Whether the browser may load `url` as its page. The app asks this
    /// before every top-level load, the agent's and the page's own, so a
    /// public page cannot send it to a local address the person has not
    /// allowed. Only web pages and the blank page load.
    pub fn admits(&self, persona_id: &str, url: &Url) -> bool {
        match url.scheme() {
            "about" => return url.as_str() == "about:blank",
            "http" | "https" => {}
            _ => return false,
        }
        if !names_local(url) {
            return true;
        }
        let Some(port) = url.port_or_known_default() else {
            return false;
        };
        !self.never(port) && origin_key(url).is_some_and(|origin| self.granted(persona_id, &origin))
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
    fn the_gate_admits_public_pages_and_only_the_local_ones_allowed() {
        let gate = Gate::default();
        gate.never.write().unwrap().insert(4000);
        assert!(gate.admits("kit", &url("https://example.com/")));
        assert!(gate.admits("kit", &url("about:blank")));
        assert!(!gate.admits("kit", &url("file:///etc/passwd")));
        assert!(!gate.admits("kit", &url("tauri://localhost/")));
        assert!(!gate.admits("kit", &url("http://localhost:2322/")));
        gate.allow("kit", "localhost:2322");
        assert!(gate.admits("kit", &url("http://localhost:2322/deep/page")));
        assert!(
            !gate.admits("goldie", &url("http://localhost:2322/")),
            "one teammate's answer is not another's"
        );
        assert!(
            !gate.admits("kit", &url("http://localhost:5432/")),
            "an answer is for one port"
        );
        gate.allow("kit", "localhost:4000");
        assert!(
            !gate.admits("kit", &url("http://localhost:4000/")),
            "a never port stays shut whatever was answered"
        );
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
