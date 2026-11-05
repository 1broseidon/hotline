//! What a teammate's page does when a load fails before it starts, and the
//! one way past a certificate WebKit refuses (docs/security.md, "A
//! teammate's browser").
//!
//! wry answers neither a certificate WebKit refuses nor a load that failed
//! to start, so the page would stay, without a word, on the one before. The
//! desk listens for both on each teammate page, on each platform's terms
//! (`trust/apple.rs`, `trust/linux.rs`): it says why a load failed, puts a
//! warning in the page's place, and lets a refused certificate through only
//! when the person chose Visit anyway for that very certificate, by the
//! SHA-256 of its leaf, for that teammate, host and port, until the desk
//! stops. Only the bar, while the person has control, adds one; the
//! teammate never can.

use super::{Loads, Tab};
use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "macos")]
pub(super) use apple::{attach, forget};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(super) use linux::{attach, forget, let_through};

/// The certificates the person chose to visit anyway: teammate, host:port
/// and the leaf's SHA-256, for the desk's run.
#[derive(Default)]
pub(super) struct Exceptions(Mutex<HashSet<(String, String, String)>>);

impl Exceptions {
    #[cfg_attr(
        not(target_os = "macos"),
        allow(
            dead_code,
            reason = "WebKitGTK is let through at Visit anyway, never asked at load time"
        )
    )]
    fn holds(&self, persona_id: &str, origin: &str, fingerprint: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(
                persona_id.to_string(),
                origin.to_string(),
                fingerprint.to_string(),
            ))
    }

    pub(super) fn add(&self, persona_id: &str, origin: &str, fingerprint: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((
                persona_id.to_string(),
                origin.to_string(),
                fingerprint.to_string(),
            ));
    }

    /// A teammate's browser closed: the certificates let through for it go.
    pub(super) fn forget(&self, persona_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(persona, _, _)| persona != persona_id);
    }

    /// Whether the person let any certificate through on `origin`.
    pub(super) fn any_on(&self, persona_id: &str, origin: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(persona, held, _)| persona == persona_id && held == origin)
    }
}

/// A load refused for its certificate, as the bar offers it.
#[derive(Clone)]
pub(super) struct Untrusted {
    pub(super) url: String,
    pub(super) origin: String,
    /// The leaf certificate's SHA-256, when the challenge was seen.
    pub(super) fingerprint: Option<String>,
    /// Short, for the bar: "Certificate expired".
    pub(super) label: &'static str,
}

/// What the desk keeps per page, beside the tab.
#[derive(Default)]
pub(super) struct Seen {
    /// The certificate each host:port last offered, as macOS hands it to
    /// the delegate. WebKitGTK hands it with the failure instead, and the
    /// GTK side keeps it on the main thread (`trust/linux.rs`).
    #[cfg(target_os = "macos")]
    certificates: Mutex<std::collections::HashMap<String, apple::Offered>>,
    pub(super) untrusted: Mutex<Option<Untrusted>>,
    /// The last top-level address the gate let the page go to, for a
    /// failure whose error does not name it.
    pub(super) attempted: Mutex<String>,
}

impl Seen {
    pub(super) fn untrusted(&self) -> Option<Untrusted> {
        self.untrusted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn clear(&self) {
        *self
            .untrusted
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The warning a refused certificate leaves in the page's place.
fn warning_page(origin: &str, reason: &str) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Not private</title><style>
html{{color-scheme:dark}}body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#16181d;color:#e8e6e1;font:15px/1.5 -apple-system,system-ui,sans-serif}}
main{{max-width:440px;padding:32px}}h1{{font-size:22px;font-weight:600;margin:0 0 12px;color:#f2c46d}}p{{margin:0 0 10px;color:#b9b6ae}}b{{color:#e8e6e1;font-weight:600}}
</style></head><body><main><h1>This connection is not private</h1>
<p><b>{origin}</b> did not open: {reason}.</p>
<p>The teammate cannot go past this. If you trust this site, take control above and choose Visit anyway.</p></main></body></html>"#,
        origin = escape(origin),
        reason = escape(reason),
    )
}

/// `host:port` as the gate and the bar key it.
fn origin_of(url: &str) -> Option<String> {
    tauri::Url::parse(url)
        .ok()
        .as_ref()
        .and_then(hotline_core::browser::origin_key)
}

/// Keeps a load refused for its certificate for the bar, and answers the
/// warning page to put in the page's place and what the call is told.
fn refused(tab: &Tab, untrusted: Untrusted, reason: &str) -> (String, String) {
    let origin = untrusted.origin.clone();
    *tab.seen
        .untrusted
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(untrusted);
    (
        warning_page(&origin, reason),
        format!(
            "{origin} did not open: {reason}. Your browser shows a warning in its place. Only the person can visit it anyway: they open your browser's pane, take control and choose Visit anyway. Ask them if the task needs it; you cannot do it yourself."
        ),
    )
}

/// What the call is told of a load that failed for any other reason.
fn failed(tab: &Tab, origin: &str, reason: &str) -> String {
    let shown = tab
        .shown
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let still = if shown.is_empty() || shown == "about:blank" {
        String::new()
    } else {
        format!(" The browser is still on {shown}.")
    };
    format!("{origin} did not open: {reason}.{still}")
}

/// Answers the call waiting on the page's load with `message`.
fn tell(tab: &Tab, message: String) {
    tab.loads
        .send_modify(|loads: &mut Loads| loads.failed = Some(message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_page_escapes_what_it_names() {
        let page = warning_page("<x>:443", "a \"b\" & c");
        assert!(page.contains("&lt;x&gt;:443"));
        assert!(page.contains("a &quot;b&quot; &amp; c"));
        assert!(!page.contains("<x>"));
    }

    #[test]
    fn a_certificate_is_let_through_only_for_its_teammate_origin_and_hash() {
        let exceptions = Exceptions::default();
        exceptions.add("quill", "fw.example:8444", "ab");
        assert!(exceptions.holds("quill", "fw.example:8444", "ab"));
        assert!(!exceptions.holds("quill", "fw.example:8444", "cd"));
        assert!(!exceptions.holds("kit", "fw.example:8444", "ab"));
        assert!(!exceptions.holds("quill", "fw.example:443", "ab"));
        assert!(exceptions.any_on("quill", "fw.example:8444"));
        assert!(!exceptions.any_on("kit", "fw.example:8444"));
    }

    #[test]
    fn an_origin_is_the_host_and_port_of_its_address() {
        assert_eq!(
            origin_of("https://FW.example:8444/x").as_deref(),
            Some("fw.example:8444")
        );
        assert_eq!(
            origin_of("https://example.com/").as_deref(),
            Some("example.com:443")
        );
        assert_eq!(origin_of("not a url"), None);
    }
}
