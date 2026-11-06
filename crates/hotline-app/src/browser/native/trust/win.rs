//! The Windows side of a teammate page's trust and loads (`trust.rs`).
//!
//! WebView2 tells a page's own loads from its frames': `NavigationStarting`
//! is the top of the window, which wry hands to the gate's page rule
//! (`Gate::admits`, through `browser.rs`'s `navigating`), and
//! `FrameNavigationStarting` is a frame inside it, which the desk judges by
//! the frame rule here (`Gate::admits_frame`) and which never fails a call.
//! A launch of another app for a scheme that is not the web's is cancelled
//! besides, though the gate has refused that load already.
//!
//! A certificate WebView2 refuses comes to `ServerCertificateErrorDetected`,
//! for the page and for anything it loads. Without an answer WebView2 would
//! show Edge's own warning, whose "Continue to the site" a teammate's click
//! could follow; so every one is answered. The certificate the person chose
//! to visit anyway, by its leaf's SHA-256, for that teammate and host and
//! port, is let through (`ALWAYS_ALLOW`, which WebView2 keeps for that host
//! and certificate for the browser's run, so, as on Linux, the same
//! certificate on another port of that host is accepted too); every other
//! one is cancelled, and when it was the page's own load the desk puts its
//! warning in the page's place and tells the call why, as on macOS. A
//! revoked certificate is cancelled with no Visit anyway, as Edge offers
//! none.
//!
//! The page's loads are counted here too (`watch_loads`), not by wry's page
//! load handler: one `NavigationCompleted` says both that a load ended and
//! whether it failed, and the call waiting on it must hear the failure
//! before it hears the load end.

use super::{Exceptions, Tab, Untrusted, origin_of, refused, tell};
use crate::browser::native::win::{View, devtools, webview};
use crate::browser::native::{edge, page_loaded};
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use tauri::Url;
use tauri::webview::PageLoadEvent;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, COREWEBVIEW2_WEB_ERROR_STATUS,
    ICoreWebView2, ICoreWebView2_14, ICoreWebView2_18, ICoreWebView2NavigationCompletedEventArgs2,
};
use webview2_com::{
    ClearServerCertificateErrorActionsCompletedHandler, LaunchingExternalUriSchemeEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    ServerCertificateErrorDetectedEventHandler, take_pwstr,
};
use windows::core::{BOOL, HSTRING, Interface, PWSTR};

/// What a page's refused certificate leaves for the end of its load: the
/// warning to put in the page's place, or nothing to put there (a revoked
/// certificate's); then, while the blank page the warning goes into loads,
/// the warning itself. Either way the call was told already.
enum Refused {
    Warning(String),
    Told,
    Showing(String),
}

thread_local! {
    static REFUSED: RefCell<HashMap<usize, Refused>> = RefCell::default();
}

fn key(view: &View) -> usize {
    view.as_raw() as usize
}

/// The SHA-256 of a refused certificate's leaf.
fn fingerprint(
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ServerCertificateErrorDetectedEventArgs,
) -> Option<String> {
    // SAFETY: WebView2's own getters, inside its event, on the main thread.
    let pem = unsafe {
        let certificate = args.ServerCertificate().ok()?;
        let mut pem = PWSTR::null();
        certificate.ToPemEncoding(&raw mut pem).ok()?;
        take_pwstr(pem)
    };
    edge::der_of_pem(&pem).map(|der| hotline_core::browser::certificate_fingerprint(&der))
}

/// Listens for the page's frames, its refused certificates and its
/// launches of other apps.
pub(in super::super) fn attach(
    view: View,
    persona_id: String,
    tab: Arc<Tab>,
    exceptions: Arc<Exceptions>,
    gate: Arc<hotline_core::browser::Gate>,
) {
    let Some(page) = webview(&view) else {
        return;
    };
    let key = key(&view);
    let mut token = 0;
    // SAFETY: WebView2's events, on the main thread; each handler lives as
    // long as the web view.
    unsafe {
        let _ = page.add_FrameNavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut uri = PWSTR::null();
                args.Uri(&raw mut uri)?;
                let uri = take_pwstr(uri);
                let admitted = Url::parse(&uri).is_ok_and(|url| gate.admits_frame(&url));
                args.SetCancel(!admitted)
            })),
            &raw mut token,
        );
        if let Ok(page) = page.cast::<ICoreWebView2_18>() {
            let _ = page.add_LaunchingExternalUriScheme(
                &LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &raw mut token,
            );
        }
        if let Ok(page) = page.cast::<ICoreWebView2_14>() {
            let _ = page.add_ServerCertificateErrorDetected(
                &ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    let mut uri = PWSTR::null();
                    args.RequestUri(&raw mut uri)?;
                    let uri = take_pwstr(uri);
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.ErrorStatus(&raw mut status)?;
                    let print = fingerprint(&args);
                    let origin = origin_of(&uri).unwrap_or_else(|| uri.clone());
                    if let Some(print) = &print
                        && exceptions.holds(&persona_id, &origin, print)
                    {
                        return args
                            .SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW);
                    }
                    args.SetAction(COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL)?;
                    let attempted = tab
                        .seen
                        .attempted
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    // A picture, script or frame of the page: refused, and
                    // the page goes on without it.
                    if !edge::same_address(&uri, &attempted) {
                        return Ok(());
                    }
                    let left = if let Some((reason, label)) = edge::refusal(status.0) {
                        let untrusted = Untrusted {
                            url: uri,
                            origin,
                            fingerprint: print,
                            label,
                        };
                        let (warning, message) = refused(&tab, untrusted, reason);
                        tell(&tab, message);
                        Refused::Warning(warning)
                    } else {
                        let reason = edge::unrefusable(status.0);
                        tell(&tab, format!("{origin} did not open: {reason}."));
                        Refused::Told
                    };
                    REFUSED.with(|refused| refused.borrow_mut().insert(key, left));
                    Ok(())
                })),
                &raw mut token,
            );
        }
    }
}

/// Writes the warning into the blank page now up, through the protocol, as
/// the page's whole document: it has no script, and the page's own scripts
/// are gone with the load it replaced.
fn warn(page: &ICoreWebView2, warning: String) {
    let into = page.clone();
    devtools(page, "Page.getFrameTree", &json!({}), move |tree| {
        if let Ok(frame) = tree.and_then(|tree| edge::main_frame(&tree)) {
            devtools(
                &into,
                "Page.setDocumentContent",
                &json!({ "frameId": frame, "html": warning }),
                |_| {},
            );
        }
    });
}

/// The page's address now, as WebView2 has it.
fn source(page: &ICoreWebView2) -> String {
    let mut uri = PWSTR::null();
    // SAFETY: a plain getter on the main thread.
    if unsafe { page.Source(&raw mut uri) }.is_err() {
        return String::new();
    }
    take_pwstr(uri)
}

/// Counts the page's loads (`page`) or its popup's for the tab: one that
/// started when the gate let it through, one that ended when it ended, and
/// one that failed, said first.
pub(in super::super) fn watch_loads(view: View, tab: Arc<Tab>, page: bool) {
    let Some(web) = webview(&view) else {
        return;
    };
    let key = key(&view);
    let mut token = 0;
    // SAFETY: WebView2's events, on the main thread; each handler lives as
    // long as the web view. wry's own `NavigationStarting`, the gate's, was
    // added when the web view was made and has answered first.
    unsafe {
        let started = tab.clone();
        let _ = web.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut cancelled = BOOL::default();
                args.Cancel(&raw mut cancelled)?;
                if !cancelled.as_bool() {
                    started.loads.send_modify(|loads| loads.started += 1);
                }
                Ok(())
            })),
            &raw mut token,
        );
        let _ = web.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |web, args| {
                let (Some(web), Some(args)) = (web, args) else {
                    return Ok(());
                };
                let mut succeeded = BOOL::default();
                args.IsSuccess(&raw mut succeeded)?;
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&raw mut status)?;
                let http = args
                    .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
                    .ok()
                    .and_then(|args| {
                        let mut code = 0;
                        args.HttpStatusCode(&raw mut code).ok()?;
                        (code > 0).then_some(code)
                    });
                let outcome = edge::outcome(succeeded.as_bool(), status.0, http);
                let address = source(&web);
                let refusal = REFUSED.with(|refused| refused.borrow_mut().remove(&key));
                match (refusal, outcome) {
                    // The certificate's refusal was said; its warning goes
                    // into a blank page of the desk's, which the gate lets
                    // through as it lets every `about:blank` through. (A
                    // string loaded as a page would come to the gate as a
                    // `data:` address, which it refuses at the top.)
                    (Some(Refused::Warning(warning)), outcome) if outcome != edge::Outcome::Loaded => {
                        REFUSED.with(|refused| refused.borrow_mut().insert(key, Refused::Showing(warning)));
                        let _ = web.Navigate(&HSTRING::from("about:blank"));
                    }
                    (Some(Refused::Showing(warning)), edge::Outcome::Loaded) if address == "about:blank" => {
                        warn(&web, warning);
                        if let Ok(blank) = Url::parse("about:blank") {
                            page_loaded(&tab, page, PageLoadEvent::Finished, &blank);
                        }
                    }
                    (Some(Refused::Told), outcome) if outcome != edge::Outcome::Loaded => {}
                    (_, edge::Outcome::Stopped) => {}
                    (_, edge::Outcome::Loaded) => {
                        if let Ok(url) = Url::parse(&address) {
                            page_loaded(&tab, page, PageLoadEvent::Finished, &url);
                        }
                    }
                    (_, edge::Outcome::Failed(reason)) => {
                        let attempted = tab
                            .seen
                            .attempted
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clone();
                        let named = if attempted.is_empty() { &address } else { &attempted };
                        let origin = origin_of(named).unwrap_or_else(|| named.clone());
                        // Said before the load's end is counted, so the call
                        // waiting hears why and not that it loaded.
                        tell(
                            &tab,
                            format!("{origin} did not open: {reason}. Your browser shows its error page in its place."),
                        );
                        if let Ok(url) = Url::parse(&address) {
                            page_loaded(&tab, page, PageLoadEvent::Finished, &url);
                        }
                    }
                }
                Ok(())
            })),
            &raw mut token,
        );
    }
}

/// Lets go of what was kept for a page that is closing, and of the
/// certificates WebView2 was told to let through for it.
pub(in super::super) fn forget(view: View) {
    REFUSED.with(|refused| refused.borrow_mut().remove(&key(&view)));
    if let Some(page) = webview(&view).and_then(|page| page.cast::<ICoreWebView2_14>().ok()) {
        // SAFETY: on the main thread, with a handler WebView2 keeps.
        let _ = unsafe {
            page.ClearServerCertificateErrorActions(
                &ClearServerCertificateErrorActionsCompletedHandler::create(Box::new(|_| Ok(()))),
            )
        };
    }
}
