//! Passkeys in a teammate's desk browser, on WebKit's side. `passkeys.js`
//! runs in the page's world in every frame, before the page's own scripts,
//! and turns `navigator.credentials.get` and `create` into messages to
//! [`PasskeyDoor`], a script message handler with a reply. The door takes
//! the caller's origin only from `WKScriptMessage.frameInfo.securityOrigin`,
//! and the page's from the web view's own address: never from the message,
//! which the page wrote. It hands the call to the core
//! ([`Desk::browser_passkey`]), which checks the origin against the relying
//! party, asks the person where it must, and signs with the teammate's own
//! passkey from the vault; the page hears the signed answer, or a refusal
//! with a `DOMException`'s name. A private key never enters the page.

use block2::{DynBlock, RcBlock};
use hotline_core::browser::authenticator::Request;
use hotline_core::desk::Desk;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKContentWorld, WKScriptMessage, WKScriptMessageHandlerWithReply, WKUserContentController,
    WKWebView,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::{Manager, Url, Webview};

/// The script that answers WebAuthn calls through the desk; the page's
/// builder adds it for all frames.
pub(super) const SCRIPT: &str = include_str!("passkeys.js");

/// The handler's name under `window.webkit.messageHandlers`.
const HANDLER: &str = "hotlinePasskeys";

/// Whose browser a door is in, and the app that reaches the desk.
pub(super) struct Ivars {
    app: tauri::AppHandle,
    persona_id: String,
}

/// What a refused call tells the page: a `DOMException`'s name and a
/// sentence.
type Refused = (&'static str, String);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct PasskeyDoor;

    unsafe impl NSObjectProtocol for PasskeyDoor {}

    unsafe impl WKScriptMessageHandlerWithReply for PasskeyDoor {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        fn did_receive(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
        ) {
            let reply = OnMain(reply.copy());
            let asked = asked(message);
            let app = self.ivars().app.clone();
            let persona_id = self.ivars().persona_id.clone();
            tauri::async_runtime::spawn(async move {
                let answer = match (asked, app.try_state::<Arc<Desk>>()) {
                    (Ok(request), Some(desk)) => desk
                        .browser_passkey(&persona_id, &request)
                        .await
                        .map_err(|refusal| (refusal.name, refusal.message)),
                    (Err(refused), _) => Err(refused),
                    (_, None) => Err(("NotAllowedError", "The desk is not open.".into())),
                };
                let text = match answer {
                    Ok(credential) => json!({ "credential": credential }),
                    Err((name, message)) => {
                        json!({ "error": { "name": name, "message": message } })
                    }
                }
                .to_string();
                let _ = app.run_on_main_thread(move || reply.answer(&text));
            });
        }
    }
);

/// WebKit's reply block, carried to the desk's answer and back: it is only
/// ever called on the main thread, through `run_on_main_thread`.
struct OnMain(RcBlock<dyn Fn(*mut AnyObject, *mut NSString)>);

// SAFETY: the block is called only on the main thread (see above); WebKit's
// reply blocks are reference counted with atomic operations, so the copy
// may be released wherever the closure carrying it ends.
unsafe impl Send for OnMain {}

impl OnMain {
    /// Resolves the page's promise with `text`, a JSON string.
    fn answer(self, text: &str) {
        let text = NSString::from_str(text);
        let value = Retained::as_ptr(&text).cast_mut().cast::<AnyObject>();
        self.0.call((value, std::ptr::null_mut()));
    }
}

/// The origin WebKit reports for a frame, from its security origin's parts;
/// `None` for an opaque one, such as a sandboxed frame's.
fn origin_of(protocol: &str, host: &str, port: isize) -> Option<String> {
    if protocol.is_empty() || host.is_empty() {
        return None;
    }
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port = if port > 0 {
        format!(":{port}")
    } else {
        String::new()
    };
    let origin = Url::parse(&format!("{protocol}://{host}{port}/"))
        .ok()?
        .origin()
        .ascii_serialization();
    (origin != "null").then_some(origin)
}

/// The call a message carries, with the origins WebKit reports for it.
fn asked(message: &WKScriptMessage) -> Result<Request, Refused> {
    let refused = |text: &str| ("NotAllowedError", text.to_string());
    // SAFETY: WebKit's own accessors on the message it handed over, on the
    // main thread the handler is called on.
    let (body, origin, page) = unsafe {
        let body = message
            .body()
            .downcast::<NSString>()
            .ok()
            .map(|body| body.to_string());
        let frame = message.frameInfo().securityOrigin();
        let origin = origin_of(
            &frame.protocol().to_string(),
            &frame.host().to_string(),
            frame.port(),
        );
        let page = message
            .webView()
            .and_then(|view| view.URL())
            .and_then(|url| url.absoluteString())
            .map(|url| url.to_string());
        (body, origin, page)
    };
    let origin = origin.ok_or_else(|| refused("This frame has no origin to use a passkey for."))?;
    let top_origin = page
        .and_then(|page| Url::parse(&page).ok())
        .map(|page| page.origin().ascii_serialization())
        .ok_or_else(|| refused("The page has no address."))?;
    let body: Value = body
        .and_then(|body| serde_json::from_str(&body).ok())
        .ok_or_else(|| refused("The passkey request could not be read."))?;
    let create = match body["kind"].as_str() {
        Some("get") => false,
        Some("create") => true,
        _ => return Err(refused("The passkey request could not be read.")),
    };
    Ok(Request {
        create,
        origin,
        top_origin,
        options: body["options"].clone(),
    })
}

/// Puts the door on `page`'s web view, in the page's world, for
/// `persona_id`. The script finds it when a page first asks.
pub(super) fn attach(page: &Webview, persona_id: &str) {
    let app = page.app_handle().clone();
    let persona_id = persona_id.to_string();
    let _ = page.with_webview(move |view| {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        // SAFETY: `with_webview` hands the live WKWebView of the page, on
        // the main thread.
        let view = unsafe { &*view.inner().cast::<WKWebView>() };
        let door = PasskeyDoor::alloc(main).set_ivars(Ivars { app, persona_id });
        // SAFETY: the door was just allocated with its ivars set, and
        // NSObject's init takes nothing and returns that same object.
        let door: Retained<PasskeyDoor> = unsafe { msg_send![super(door), init] };
        // SAFETY: the web view's own controller, which keeps the door for
        // as long as the web view lives.
        unsafe {
            view.configuration()
                .userContentController()
                .addScriptMessageHandlerWithReply_contentWorld_name(
                    ProtocolObject::from_ref(&*door),
                    &WKContentWorld::pageWorld(main),
                    &NSString::from_str(HANDLER),
                );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    #[test]
    fn a_frames_origin_is_built_from_what_webkit_reports() {
        assert_eq!(
            origin_of("https", "github.com", 0).as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin_of("https", "github.com", 443).as_deref(),
            Some("https://github.com"),
            "a default port is not written"
        );
        assert_eq!(
            origin_of("http", "localhost", 8123).as_deref(),
            Some("http://localhost:8123")
        );
        assert_eq!(
            origin_of("https", "::1", 8443).as_deref(),
            Some("https://[::1]:8443")
        );
        assert_eq!(origin_of("", "", 0), None, "an opaque origin has none");
        assert_eq!(origin_of("file", "", 0), None);
    }
}
