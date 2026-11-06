//! How a teammate's page starts: made on `about:blank` with every load
//! refused, then sent to its first address once the desk listens for its
//! failures (`trust.rs`: a delegate in front of wry's on macOS, signal
//! handlers on WebKitGTK, WebView2's events on Windows), so even the first
//! load says why it failed.
//! What it may load after that is the gate's question (docs/security.md,
//! "A teammate's browser"): any web page but the desk's own ports.

#[cfg(target_os = "macos")]
pub(super) use apple::start;
#[cfg(target_os = "linux")]
pub(super) use linux::start;
#[cfg(windows)]
pub(super) use win::start;

#[cfg(target_os = "macos")]
mod apple {
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
    pub(in super::super) fn start(view: *mut c_void, url: Url, started: Arc<AtomicBool>) {
        // SAFETY: see `web_view`.
        let view = unsafe { web_view(view) };
        started.store(true, Ordering::Release);
        if let Some(address) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
            // SAFETY: WebKit on its own thread; the address is live.
            let _ = unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&address)) };
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use url::Url;
    use webkit2gtk::{WebView, WebViewExt};

    /// Lets the page's loads through (`started`) and sends it to `url`.
    pub(in super::super) fn start(view: WebView, url: Url, started: Arc<AtomicBool>) {
        started.store(true, Ordering::Release);
        view.load_uri(url.as_str());
    }
}

#[cfg(windows)]
mod win {
    use crate::browser::native::win::{View, webview};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::Url;
    use windows::core::HSTRING;

    /// Lets the page's loads through (`started`) and sends it to `url`.
    pub(in super::super) fn start(view: View, url: Url, started: Arc<AtomicBool>) {
        started.store(true, Ordering::Release);
        if let Some(page) = webview(&view) {
            // SAFETY: WebView2 on its own thread; the address is copied.
            let _ = unsafe { page.Navigate(&HSTRING::from(url.as_str())) };
        }
    }
}
