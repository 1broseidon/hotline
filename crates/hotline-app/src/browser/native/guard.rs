//! What keeps a teammate's page off this machine and the local network
//! past the gate's check of each top-level load by its name
//! (docs/security.md, "A teammate's browser"):
//!
//! - a content rule list on every page that blocks each load a page makes
//!   (a script, a picture, a frame, a fetch, a WebSocket) to a local address
//!   by its name, unless the page itself is on one, which only the person's
//!   answer lets it be;
//! - a look at what a top-level load's name resolves to, once the gate let
//!   it through, that stops the load and leaves the page blank when it is
//!   local and the person did not allow it, so a public page cannot
//!   redirect to a public name pointed at this machine.
//!
//! The page loads nothing until its rule list is in place: it is made on
//! `about:blank` with every load refused, and sent to its first address
//! once the list is attached.

use super::Tab;
use block2::RcBlock;
use hotline_core::browser::Gate;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_foundation::{NSError, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKWebView};
use std::cell::{Cell, OnceCell};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Url};

/// The rule list's name in WebKit's store.
const IDENTIFIER: &str = "hotline-desk-browser-local";

/// Any URL's scheme and user, before its host.
const BEFORE_HOST: &str = r"^[a-z][a-z0-9+.-]*://([^/@]*@)?";

/// Local hosts, as WebKit's rule regexes: they have no `|` and no counted
/// repeats, so a range is spelled as several. Each one ends where the host
/// does, so `10.1.2.3.example.com` is not one. WebKit has already written
/// a numeric host as four decimal octets, and IPv6 in its shortest form.
const LOCAL_HOSTS: [&str; 19] = [
    r"localhost\.?[:/]",
    r"[^/:@]*\.localhost\.?[:/]",
    r"[^/:@]*\.local\.?[:/]",
    r"0\.0\.0\.0[:/]",
    r"127\.[0-9]+\.[0-9]+\.[0-9]+[:/]",
    r"10\.[0-9]+\.[0-9]+\.[0-9]+[:/]",
    r"172\.1[6-9]\.[0-9]+\.[0-9]+[:/]",
    r"172\.2[0-9]\.[0-9]+\.[0-9]+[:/]",
    r"172\.3[01]\.[0-9]+\.[0-9]+[:/]",
    r"192\.168\.[0-9]+\.[0-9]+[:/]",
    r"169\.254\.[0-9]+\.[0-9]+[:/]",
    r"100\.6[4-9]\.[0-9]+\.[0-9]+[:/]",
    r"100\.[7-9][0-9]\.[0-9]+\.[0-9]+[:/]",
    r"100\.1[01][0-9]\.[0-9]+\.[0-9]+[:/]",
    r"100\.12[0-7]\.[0-9]+\.[0-9]+[:/]",
    r"\[::1?\]",
    r"\[f[cd][0-9a-f]*:",
    r"\[fe[89ab][0-9a-f]*:",
    r"\[::ffff:",
];

/// The rule list: one rule a local host, each blocking every load to it
/// unless the page on top is itself on a local host.
fn rules() -> String {
    let local: Vec<String> = LOCAL_HOSTS
        .iter()
        .map(|host| format!("{BEFORE_HOST}{host}"))
        .collect();
    let rules: Vec<serde_json::Value> = local
        .iter()
        .map(|filter| {
            serde_json::json!({
                "trigger": { "url-filter": filter, "unless-top-url": local },
                "action": { "type": "block" },
            })
        })
        .collect();
    serde_json::Value::Array(rules).to_string()
}

thread_local! {
    /// Compiled once, on the main thread, for every teammate's page.
    static COMPILED: OnceCell<Retained<WKContentRuleList>> = const { OnceCell::new() };
}

/// The page `with_webview` hands over, as WebKit's type.
///
/// SAFETY: the caller is `with_webview`'s closure, on the main thread,
/// where `view` is the live `WKWebView` of a Tauri window.
unsafe fn web_view(view: *mut c_void) -> Retained<WKWebView> {
    // SAFETY: as the function's own contract above.
    unsafe { &*view.cast::<WKWebView>() }.retain()
}

/// Attaches the rule list to a page made on `about:blank`, then lets loads
/// through (`guarded`) and sends it to `url`. If the list cannot be made,
/// the page stays blank and `refused` says so.
pub(super) fn attach(
    view: *mut c_void,
    url: Url,
    guarded: Arc<AtomicBool>,
    refused: impl FnOnce() + 'static,
) {
    let Some(main) = MainThreadMarker::new() else {
        return refused();
    };
    // SAFETY: see `web_view`.
    let view = unsafe { web_view(view) };
    let go = move |list: &WKContentRuleList| {
        // SAFETY: WebKit on its own thread; the list and the address are live.
        unsafe {
            view.configuration()
                .userContentController()
                .addContentRuleList(list);
        }
        guarded.store(true, Ordering::Release);
        if let Some(address) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
            // SAFETY: as above.
            let _ = unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&address)) };
        }
    };
    if let Some(list) = COMPILED.with(|compiled| compiled.get().cloned()) {
        return go(&list);
    }
    // SAFETY: the default store, asked on the main thread.
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(main) }) else {
        eprintln!("[browser] WebKit has no content rule store");
        return refused();
    };
    let once = Cell::new(Some((go, refused)));
    let done = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
        let Some((go, refused)) = once.take() else {
            return;
        };
        // SAFETY: WebKit hands a valid list or null.
        let Some(list) = (unsafe { list.as_ref() }) else {
            // SAFETY: WebKit hands a valid error or null.
            let reason = unsafe { error.as_ref() }.map_or_else(String::new, |error| {
                error.localizedDescription().to_string()
            });
            eprintln!("[browser] the local-address rule list did not compile: {reason}");
            return refused();
        };
        COMPILED.with(|compiled| {
            compiled.get_or_init(|| list.retain());
        });
        go(list);
    });
    // SAFETY: a name, the list's JSON, and a block WebKit calls once on
    // the main thread.
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str(IDENTIFIER)),
            Some(&NSString::from_str(&rules())),
            Some(&done),
        );
    }
}

/// Once the gate let a top-level load through by its name, resolves the
/// name off the main thread and, when it is local and not allowed for this
/// teammate, records the load as refused and stops it.
pub(super) fn check_landing(
    app: &AppHandle,
    gate: &Arc<Gate>,
    persona_id: &str,
    tab: &Arc<Tab>,
    url: &Url,
) {
    let (app, gate, persona_id, tab, url) = (
        app.clone(),
        gate.clone(),
        persona_id.to_string(),
        tab.clone(),
        url.clone(),
    );
    tauri::async_runtime::spawn(async move {
        if gate.admits_landing(&persona_id, &url).await {
            return;
        }
        tab.loads
            .send_modify(|loads| loads.blocked = Some(url.to_string()));
        let Some(page) = app.get_webview(&tab.page) else {
            return;
        };
        let host = url.host_str().unwrap_or_default().to_string();
        let _ = page.with_webview(move |view| stop(view.inner(), &host));
    });
}

/// Stops the page's load and, if it already shows `host`, blanks it.
fn stop(view: *mut c_void, host: &str) {
    // SAFETY: see `web_view`.
    let view = unsafe { web_view(view) };
    // SAFETY: plain WebKit navigation on its own thread.
    unsafe { view.stopLoading() };
    // SAFETY: as above.
    let shown = unsafe { view.URL() }
        .and_then(|url| url.host())
        .is_some_and(|shown| shown.to_string().eq_ignore_ascii_case(host));
    if shown && let Some(blank) = NSURL::URLWithString(&NSString::from_str("about:blank")) {
        // SAFETY: as above.
        let _ = unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&blank)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether some rule's filter matches `url`, as WebKit's search does.
    fn blocked(url: &str) -> bool {
        LOCAL_HOSTS.iter().any(|host| {
            regex::RegexBuilder::new(&format!("{BEFORE_HOST}{host}"))
                .case_insensitive(true)
                .build()
                .unwrap()
                .is_match(url)
        })
    }

    #[test]
    fn the_rules_name_every_local_host_and_no_public_one() {
        for local in [
            "http://localhost:2322/api",
            "http://LOCALHOST/",
            "http://localhost./",
            "ws://app.localhost:5173/socket",
            "https://printer.local/status",
            "http://user:pw@127.0.0.1:8080/",
            "http://127.1.2.3/",
            "http://0.0.0.0:3000/",
            "http://10.0.0.5/",
            "http://172.16.0.1/",
            "http://172.31.255.255/",
            "http://192.168.1.20/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://100.127.0.1/",
            "http://[::1]:8080/",
            "http://[::]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:7f00:1]/",
        ] {
            assert!(blocked(local), "{local}");
        }
        for public in [
            "https://example.com/",
            "https://localhost.example.com/",
            "https://example.com/localhost:2322/",
            "https://example.com/?next=http://127.0.0.1/",
            "http://10.1.2.3.example.com/",
            "http://172.32.0.1/",
            "http://172.15.0.1/",
            "http://100.128.0.1/",
            "http://100.63.0.1/",
            "http://8.8.8.8/",
            "http://[2606:4700::1111]/",
            "https://notlocal.com/",
        ] {
            assert!(!blocked(public), "{public}");
        }
    }

    #[test]
    fn each_rule_spares_a_page_on_a_local_host() {
        let rules: serde_json::Value = serde_json::from_str(&rules()).unwrap();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules.len(), LOCAL_HOSTS.len());
        for rule in rules {
            assert_eq!(rule["action"]["type"], "block");
            assert_eq!(
                rule["trigger"]["unless-top-url"].as_array().unwrap().len(),
                LOCAL_HOSTS.len()
            );
        }
    }
}
