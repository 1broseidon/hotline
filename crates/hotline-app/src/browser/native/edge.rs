//! What the Windows side of the desk browser decides of WebView2's answers,
//! in plain values, so it is built and tested on every platform and not
//! only where WebView2 runs (`win.rs`, `trust/win.rs`,
//! `asks/win.rs`, the cookies' Windows side).
//!
//! WebView2 is Chromium, and the desk speaks to it in two ways: its COM
//! events (a load, a certificate, a permission) and the DevTools protocol
//! (the desk's script world, input and the file chooser), whose answers are
//! JSON. Everything here turns one of those into what the desk does next.

#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "WebView2 runs on Windows only; the decisions are tested everywhere"
    )
)]

use serde_json::{Value, json};

/// The WebView2 environment's browser arguments for a teammate's store:
/// wry's own three (no mini menu, no PDF menu, no SmartScreen lookups, as
/// Hotline's window has), and Chromium's occlusion and backgrounding left
/// off, so a page in a window nobody sees still lays out, draws and runs its
/// timers at full speed, as WebKit's offscreen pages do on macOS and Linux.
/// Every web view on one data directory must be made with the same
/// arguments, so the page, its popup and a store opened for its cookies all
/// use these.
pub(super) const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding";

/// The desk's scripts are bodies of an async function, as WebKit's
/// `callAsyncJavaScript` takes them: here they are wrapped in one and
/// called, and the protocol awaits its promise.
pub(super) fn wrap(body: &str) -> String {
    format!("(async () => {{\n{body}\n}})()")
}

/// A DevTools protocol call's answer: its result object, or what went wrong
/// in words. A failed call answers `{"code": …, "message": …}`.
pub(super) fn devtools_answer(succeeded: bool, json: &str) -> Result<Value, String> {
    let value: Option<Value> = serde_json::from_str(json).ok();
    if succeeded {
        return value.ok_or_else(|| "The browser's answer was unreadable.".to_string());
    }
    Err(value
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map_or_else(|| "WebView2 gave no reason.".to_string(), str::to_string))
}

/// The main frame's id, from `Page.getFrameTree`.
pub(super) fn main_frame(tree: &Value) -> Result<String, String> {
    tree.pointer("/frameTree/frame/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "The page has no frame to read.".to_string())
}

/// The desk's world in the main frame, from `Page.createIsolatedWorld`.
pub(super) fn world_of(made: &Value) -> Result<i64, String> {
    made.get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or_else(|| "The page gave the desk no world to read it in.".to_string())
}

/// What `Runtime.evaluate` answered: the script's string, or the
/// exception it threw, its first line only, as WebKit says it on the other
/// platforms (the stack below it names only the desk's own script).
pub(super) fn evaluated(answer: &Value) -> Result<String, String> {
    if let Some(thrown) = answer.get("exceptionDetails") {
        let said = thrown
            .pointer("/exception/description")
            .or_else(|| thrown.get("text"))
            .and_then(Value::as_str)
            .and_then(|said| said.lines().next())
            .filter(|said| !said.trim().is_empty())
            .unwrap_or("A JavaScript exception occurred");
        return Err(said.to_string());
    }
    answer
        .pointer("/result/value")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "The page answered nothing.".to_string())
}

/// A size in CSS pixels as the physical pixels of a window at `dpi`, as
/// WebView2's controller bounds are given (96 is 100%); an unreadable DPI
/// counts as 96.
pub(super) fn physical(width: u32, height: u32, dpi: u32) -> (i32, i32) {
    let dpi = if dpi == 0 { 96 } else { dpi };
    let scale = |css: u32| {
        let pixels = (u64::from(css) * u64::from(dpi) + 48) / 96;
        i32::try_from(pixels).unwrap_or(i32::MAX)
    };
    (scale(width), scale(height))
}

/// The protocol's events for a click at a point in the page's viewport, in
/// CSS pixels: the pointer moves there, presses and lets go.
pub(super) fn click_events(x: f64, y: f64) -> Vec<Value> {
    vec![
        json!({ "type": "mouseMoved", "x": x, "y": y }),
        json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }),
        json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1 }),
    ]
}

/// The protocol's event for the pointer resting at a point.
pub(super) fn hover_events(x: f64, y: f64) -> Vec<Value> {
    vec![json!({ "type": "mouseMoved", "x": x, "y": y })]
}

/// The protocol's two events for one key the tool names, or one character,
/// as Chromium's own keyboard sends them: the key, its code, the Windows
/// virtual key, the text it types, and Shift. None for anything else.
pub(super) fn key_events(key: &str) -> Option<Vec<Value>> {
    let (name, code, virtual_key, text, shift) = match key {
        "Enter" => ("Enter", "Enter", 13, "\r".to_string(), false),
        "Tab" => ("Tab", "Tab", 9, String::new(), false),
        "Shift+Tab" => ("Tab", "Tab", 9, String::new(), true),
        "Escape" => ("Escape", "Escape", 27, String::new(), false),
        "Backspace" => ("Backspace", "Backspace", 8, String::new(), false),
        "Delete" => ("Delete", "Delete", 46, String::new(), false),
        "Space" => (" ", "Space", 32, " ".to_string(), false),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38, String::new(), false),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40, String::new(), false),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37, String::new(), false),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39, String::new(), false),
        "Home" => ("Home", "Home", 36, String::new(), false),
        "End" => ("End", "End", 35, String::new(), false),
        "PageUp" => ("PageUp", "PageUp", 33, String::new(), false),
        "PageDown" => ("PageDown", "PageDown", 34, String::new(), false),
        other => {
            let mut chars = other.chars();
            let (Some(only), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let virtual_key = if only.is_ascii_alphanumeric() {
                u32::from(only.to_ascii_uppercase())
            } else {
                0
            };
            let down = json!({
                "type": "keyDown", "key": other, "text": other, "unmodifiedText": other,
                "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
            });
            let up = json!({
                "type": "keyUp", "key": other,
                "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
            });
            return Some(vec![down, up]);
        }
    };
    let modifiers = if shift { 8 } else { 0 };
    let down = json!({
        // A key that types nothing goes down raw, as Chromium's own does.
        "type": if text.is_empty() { "rawKeyDown" } else { "keyDown" },
        "key": name, "code": code, "text": text, "unmodifiedText": text,
        "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
        "modifiers": modifiers,
    });
    let up = json!({
        "type": "keyUp", "key": name, "code": code,
        "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
        "modifiers": modifiers,
    });
    Some(vec![down, up])
}

/// How a top-level load ended, by `NavigationCompleted`'s success, its
/// `COREWEBVIEW2_WEB_ERROR_STATUS` and the HTTP status the server answered.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// A page is up: the site's own, an error page a server sent (a 404),
    /// or what a page stopped with `window.stop()` left.
    Loaded,
    /// The load was cancelled: the gate refused it, or another load took
    /// its place. Nothing to say.
    Stopped,
    /// It failed to load, and why.
    Failed(&'static str),
}

pub(super) fn outcome(succeeded: bool, status: i32, http: Option<i32>) -> Outcome {
    if succeeded {
        return Outcome::Loaded;
    }
    let reason = match status {
        14 => return Outcome::Stopped,
        _ if http.is_some_and(|code| code >= 400) => return Outcome::Loaded,
        13 => "its name does not resolve",
        12 => "nothing answers there",
        7 => "it did not answer in time",
        6 => "its host cannot be reached",
        11 => "this computer is offline",
        9 | 10 => "the connection was dropped",
        8 => "its server's answer could not be read",
        15 => "a redirect it sent could not be followed",
        1..=5 => "a secure connection could not be made",
        16 => "the browser could not load it",
        _ => return Outcome::Loaded,
    };
    Outcome::Failed(reason)
}

/// Why WebView2 refused a server's certificate, in words and as the bar's
/// label, by `ServerCertificateErrorDetected`'s status; None for one the
/// person may not visit past: a revoked certificate, as Edge itself offers
/// no way past one, and a client certificate's error.
pub(super) fn refusal(status: i32) -> Option<(&'static str, &'static str)> {
    match status {
        2 => Some((
            "its security certificate has expired or is not valid yet",
            "Certificate expired",
        )),
        1 => Some((
            "its security certificate is for a different name",
            "Wrong certificate name",
        )),
        5 => Some((
            "its security certificate is not trusted",
            "Certificate not trusted",
        )),
        _ => None,
    }
}

/// Why a certificate the person may not visit past was refused.
pub(super) fn unrefusable(status: i32) -> &'static str {
    match status {
        4 => "its security certificate has been revoked",
        _ => "a secure connection could not be made",
    }
}

/// A certificate's DER bytes from WebView2's PEM: what is between its
/// BEGIN and END lines, in base64.
pub(super) fn der_of_pem(pem: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let body: String = pem
        .lines()
        .map(str::trim)
        .skip_while(|line| !line.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()
        .filter(|der| !der.is_empty())
}

/// Whether a load WebView2 names is the page's own top-level load the gate
/// let through (`attempted`), and not a picture, script or frame inside it:
/// one address, however each was spelled.
pub(super) fn same_address(named: &str, attempted: &str) -> bool {
    match (tauri::Url::parse(named), tauri::Url::parse(attempted)) {
        (Ok(mut named), Ok(mut attempted)) => {
            named.set_fragment(None);
            attempted.set_fragment(None);
            named == attempted
        }
        _ => false,
    }
}

/// How a lent cookie is sent, as Chromium takes it. A cookie the person's
/// browser sent everywhere but not only over TLS would be dropped by
/// Chromium as `SameSite=None`, so it goes as Lax, Chromium's own default
/// for a cookie that names none.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SameSite {
    None,
    Lax,
    Strict,
}

pub(super) fn same_site(named: Option<&str>, secure: bool) -> SameSite {
    match named {
        Some("Strict") => SameSite::Strict,
        Some("Lax") => SameSite::Lax,
        _ if secure => SameSite::None,
        _ => SameSite::Lax,
    }
}

/// What the page's world hears for `navigator.credentials` on Windows: a
/// passkey call is refused at once, as a browser with no authenticator
/// refuses it. WebView2's own WebAuthn would reach the person's Windows
/// Hello and security keys, which are not the teammate's; the desk has
/// already put a virtual authenticator environment with no authenticators
/// in front of them (`WebAuthn.enable`), so a page that gets past this
/// script reaches nothing either, and this only spares it the wait.
pub(super) const NO_PASSKEYS: &str = r#"(() => {
  const credentials = navigator.credentials;
  if (!credentials) return;
  const refused = () => Promise.reject(new DOMException("Passkeys are not available in this browser.", "NotAllowedError"));
  for (const name of ["get", "create"]) {
    const own = credentials[name];
    if (typeof own !== "function") continue;
    Object.defineProperty(credentials, name, {
      configurable: true,
      writable: true,
      value: function (options) {
        return options && options.publicKey ? refused() : own.call(this, options);
      },
    });
  }
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_teammates_store_keeps_wrys_arguments_and_never_backgrounds_its_page() {
        for wry in ["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"] {
            assert!(BROWSER_ARGS.contains(wry), "{wry}");
        }
        assert!(BROWSER_ARGS.contains("CalculateNativeWinOcclusion"));
        assert!(BROWSER_ARGS.contains("--disable-backgrounding-occluded-windows"));
        assert_eq!(BROWSER_ARGS.matches("--disable-features").count(), 1);
    }

    #[test]
    fn a_script_runs_as_an_async_functions_body() {
        assert_eq!(wrap("return '1';"), "(async () => {\nreturn '1';\n})()");
    }

    #[test]
    fn a_devtools_answer_is_its_result_or_its_message() {
        assert_eq!(
            devtools_answer(true, r#"{"executionContextId":7}"#).unwrap()["executionContextId"],
            7
        );
        assert_eq!(
            devtools_answer(
                false,
                r#"{"code":-32000,"message":"Cannot find context with specified id"}"#
            ),
            Err("Cannot find context with specified id".to_string())
        );
        assert_eq!(
            devtools_answer(false, ""),
            Err("WebView2 gave no reason.".to_string())
        );
        assert!(devtools_answer(true, "not json").is_err());
    }

    #[test]
    fn the_desks_world_is_made_in_the_main_frame() {
        let tree = json!({ "frameTree": { "frame": { "id": "F1", "url": "https://a.test/" }, "childFrames": [] } });
        assert_eq!(main_frame(&tree), Ok("F1".to_string()));
        assert!(main_frame(&json!({})).is_err());
        assert_eq!(world_of(&json!({ "executionContextId": 12 })), Ok(12));
        assert!(world_of(&json!({})).is_err());
    }

    #[test]
    fn an_evaluation_answers_its_string_or_what_it_threw() {
        assert_eq!(
            evaluated(&json!({ "result": { "type": "string", "value": "{\"a\":1}" } })),
            Ok("{\"a\":1}".to_string())
        );
        assert_eq!(
            evaluated(&json!({
                "result": { "type": "object" },
                "exceptionDetails": { "text": "Uncaught", "exception": { "description": "TypeError: x is not a function\n    at <anonymous>:3:1" } }
            })),
            Err("TypeError: x is not a function".to_string())
        );
        assert_eq!(
            evaluated(&json!({ "result": { "type": "undefined" } })),
            Err("The page answered nothing.".to_string())
        );
    }

    #[test]
    fn a_viewport_is_its_css_size_in_the_windows_own_pixels() {
        assert_eq!(physical(390, 844, 96), (390, 844));
        assert_eq!(physical(390, 844, 144), (585, 1266));
        assert_eq!(physical(1280, 800, 120), (1600, 1000));
        assert_eq!(physical(390, 844, 0), (390, 844));
    }

    #[test]
    fn a_click_moves_presses_and_lets_go_where_it_was_asked() {
        let events = click_events(10.5, 20.0);
        let kinds: Vec<_> = events.iter().map(|event| event["type"].clone()).collect();
        assert_eq!(kinds, ["mouseMoved", "mousePressed", "mouseReleased"]);
        assert!(
            events
                .iter()
                .all(|event| event["x"] == 10.5 && event["y"] == 20.0)
        );
        assert_eq!(events[1]["button"], "left");
        assert_eq!(hover_events(1.0, 2.0)[0]["type"], "mouseMoved");
    }

    #[test]
    fn every_key_the_tool_names_has_chromiums_key() {
        for key in hotline_core::browser::KEYS {
            let events = key_events(key).unwrap_or_else(|| panic!("{key}"));
            assert_eq!(events.len(), 2, "{key}");
            assert_eq!(events[1]["type"], "keyUp", "{key}");
            assert_ne!(events[0]["windowsVirtualKeyCode"], 0, "{key}");
        }
        let enter = key_events("Enter").unwrap();
        assert_eq!(enter[0]["type"], "keyDown");
        assert_eq!(enter[0]["text"], "\r");
        let back = key_events("Shift+Tab").unwrap();
        assert_eq!(back[0]["type"], "rawKeyDown");
        assert_eq!(back[0]["modifiers"], 8);
        let a = key_events("a").unwrap();
        assert_eq!(a[0]["text"], "a");
        assert_eq!(a[0]["windowsVirtualKeyCode"], u32::from(b'A'));
        assert_eq!(key_events("Hyper+Q"), None);
        assert_eq!(key_events(""), None);
    }

    #[test]
    fn a_load_that_failed_says_why_and_one_cancelled_says_nothing() {
        assert_eq!(outcome(true, 0, Some(200)), Outcome::Loaded);
        assert_eq!(outcome(false, 14, None), Outcome::Stopped);
        assert_eq!(
            outcome(false, 13, None),
            Outcome::Failed("its name does not resolve")
        );
        assert_eq!(
            outcome(false, 12, None),
            Outcome::Failed("nothing answers there")
        );
        assert_eq!(
            outcome(false, 2, None),
            Outcome::Failed("a secure connection could not be made")
        );
        // A server's own 404 is a page, as WebKit counts it.
        assert_eq!(outcome(false, 0, Some(404)), Outcome::Loaded);
        assert_eq!(outcome(false, 0, None), Outcome::Loaded);
    }

    #[test]
    fn only_a_certificate_edge_would_let_past_is_offered_to_the_person() {
        assert_eq!(
            refusal(2).map(|(_, label)| label),
            Some("Certificate expired")
        );
        assert_eq!(
            refusal(1).map(|(_, label)| label),
            Some("Wrong certificate name")
        );
        assert_eq!(
            refusal(5).map(|(_, label)| label),
            Some("Certificate not trusted")
        );
        assert_eq!(refusal(4), None, "revoked");
        assert_eq!(refusal(3), None, "a client certificate's error");
        assert_eq!(unrefusable(4), "its security certificate has been revoked");
    }

    #[test]
    fn a_pem_certificate_is_its_der_bytes() {
        let der = [0x30u8, 0x82, 0x01, 0x0a, 0xde, 0xad, 0xbe, 0xef];
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\r\n{}\r\n-----END CERTIFICATE-----\r\n",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, der)
        );
        assert_eq!(der_of_pem(&pem).as_deref(), Some(&der[..]));
        assert_eq!(
            hotline_core::browser::certificate_fingerprint(&der_of_pem(&pem).unwrap()),
            hotline_core::browser::certificate_fingerprint(&der)
        );
        assert_eq!(der_of_pem("no certificate"), None);
    }

    #[test]
    fn a_certificate_error_is_the_pages_only_for_its_own_address() {
        assert!(same_address(
            "https://expired.badssl.com",
            "https://expired.badssl.com/"
        ));
        assert!(same_address("https://a.test/x#top", "https://a.test/x"));
        assert!(!same_address("https://cdn.a.test/x.js", "https://a.test/"));
        assert!(!same_address("https://a.test/", ""));
    }

    #[test]
    fn a_lent_cookie_without_tls_is_never_sent_as_samesite_none() {
        assert_eq!(same_site(Some("Strict"), false), SameSite::Strict);
        assert_eq!(same_site(Some("Lax"), true), SameSite::Lax);
        assert_eq!(same_site(Some("None"), true), SameSite::None);
        assert_eq!(same_site(None, true), SameSite::None);
        assert_eq!(same_site(Some("None"), false), SameSite::Lax);
        assert_eq!(same_site(None, false), SameSite::Lax);
    }

    #[test]
    fn a_passkey_call_is_refused_in_the_page_and_nothing_else_is() {
        assert!(NO_PASSKEYS.contains("options.publicKey ? refused()"));
        assert!(NO_PASSKEYS.contains("\"NotAllowedError\""));
        assert!(NO_PASSKEYS.contains("own.call(this, options)"));
    }
}
