//! How a teammate's page starts: made on `about:blank` with every load
//! refused, then sent to its first address once the desk's delegate is in
//! front of wry's (`trust.rs`), so even the first load says why it failed.
//! What it may load after that is the gate's question (docs/security.md,
//! "A teammate's browser"): any web page but the desk's own ports.

use objc2::Message;
use objc2::rc::Retained;
use objc2_foundation::{NSString, NSURL, NSURLRequest};
use objc2_web_kit::WKWebView;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Url;

/// The page `with_webview` hands over, as WebKit's type.
///
/// SAFETY: the caller is `with_webview`'s closure, on the main thread,
/// where `view` is the live `WKWebView` of a Tauri window.
unsafe fn web_view(view: *mut c_void) -> Retained<WKWebView> {
    // SAFETY: as the function's own contract above.
    unsafe { &*view.cast::<WKWebView>() }.retain()
}

/// Lets the page's loads through (`started`) and sends it to `url`.
pub(super) fn start(view: *mut c_void, url: Url, started: Arc<AtomicBool>) {
    // SAFETY: see `web_view`.
    let view = unsafe { web_view(view) };
    started.store(true, Ordering::Release);
    if let Some(address) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
        // SAFETY: WebKit on its own thread; the address is live.
        let _ = unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&address)) };
    }
}
