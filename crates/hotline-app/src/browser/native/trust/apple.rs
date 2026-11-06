//! The macOS side of a teammate page's trust (`trust.rs`): a navigation
//! delegate of the desk's own goes in front of wry's on each teammate page.
//! It forwards everything else to wry's, says why a load failed, answers
//! WebKit's certificate challenge with the certificate the person chose to
//! visit anyway, and only that one, and judges a frame's load itself.
//!
//! wry's navigation handler is asked for every frame's load and is told
//! only its address, so the gate's page rule would refuse a sign-in's
//! `about:srcdoc` frame, a payment's `blob:` one, and fail the teammate's
//! next call for it. Here WebKit says which frame a load is for: the page's
//! own (and a popup's first load) go on to wry's and the gate's page rule
//! ([`Gate::admits`]), and a frame inside the page is judged by the frame
//! rule ([`Gate::admits_frame`]) and never fails a call.

use super::{Exceptions, Tab, Untrusted, failed, origin_of, refused, tell};
use block2::DynBlock;
use hotline_core::browser::Gate;
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_foundation::{
    NSError, NSString, NSURL, NSURLAuthenticationChallenge, NSURLAuthenticationMethodServerTrust,
    NSURLCredential, NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKWebView,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::{Arc, PoisonError};

/// A SecTrust the desk keeps, released when dropped.
struct Kept(*mut SecTrust);

// SAFETY: a SecTrust is a CoreFoundation object, safe to retain, release
// and evaluate from any thread.
unsafe impl Send for Kept {}

impl Drop for Kept {
    fn drop(&mut self) {
        // SAFETY: retained once when kept.
        unsafe { CFRelease(self.0.cast()) };
    }
}

/// The certificate a host:port last offered.
pub(super) struct Offered {
    fingerprint: String,
    trust: Kept,
}

/// `SecTrustRef`, as Foundation's methods spell it.
#[repr(C)]
struct SecTrust {
    _private: [u8; 0],
}

// SAFETY: the encoding Foundation declares `serverTrust` and
// `credentialForTrust:` with.
unsafe impl RefEncode for SecTrust {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__SecTrust", &[]));
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTrustCopyCertificateChain(trust: *mut SecTrust) -> *const c_void;
    fn SecTrustEvaluateWithError(trust: *mut SecTrust, error: *mut *const c_void) -> bool;
    fn SecCertificateCopyData(certificate: *const c_void) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(object: *const c_void);
    fn CFRetain(object: *const c_void) -> *const c_void;
    fn CFErrorGetCode(error: *const c_void) -> isize;
}

/// Why the system refuses `trust`, in words and as the bar's label, by
/// evaluating it again: WebKit reports an expired certificate as merely
/// untrusted. Only after a load failed for its certificate.
fn verdict(trust: *mut SecTrust) -> (&'static str, &'static str) {
    let mut error: *const c_void = std::ptr::null();
    // SAFETY: a kept, retained SecTrust; the error is released here.
    let code = unsafe {
        if SecTrustEvaluateWithError(trust, &raw mut error) || error.is_null() {
            0
        } else {
            let code = CFErrorGetCode(error);
            CFRelease(error);
            code
        }
    };
    refusal(code)
}

/// [`verdict`] by the Security framework's code.
fn refusal(code: isize) -> (&'static str, &'static str) {
    match code {
        -67818 => (
            "its security certificate has expired",
            "Certificate expired",
        ),
        -67819 => (
            "its security certificate is not valid yet",
            "Certificate not valid yet",
        ),
        -67602 => (
            "its security certificate is for a different name",
            "Wrong certificate name",
        ),
        _ => (
            "its security certificate is not trusted",
            "Certificate not trusted",
        ),
    }
}

/// The leaf certificate's SHA-256, in hex.
fn fingerprint(trust: *mut SecTrust) -> Option<String> {
    // SAFETY: `trust` is the challenge's live SecTrust; every CF object
    // copied here is released before returning.
    unsafe {
        let chain = SecTrustCopyCertificateChain(trust);
        if chain.is_null() {
            return None;
        }
        let digest = (CFArrayGetCount(chain) > 0)
            .then(|| SecCertificateCopyData(CFArrayGetValueAtIndex(chain, 0)))
            .filter(|data| !data.is_null())
            .map(|data| {
                let bytes = std::slice::from_raw_parts(
                    CFDataGetBytePtr(data),
                    CFDataGetLength(data).max(0) as usize,
                );
                let digest = hotline_core::browser::certificate_fingerprint(bytes);
                CFRelease(data);
                digest
            });
        CFRelease(chain);
        digest
    }
}

/// Why a load failed to start, as NSURLErrorDomain's codes say it, and the
/// bar's label when it was the certificate; `None` for a load that was
/// only superseded or stopped.
fn failure(domain: &str, code: isize, described: &str) -> Option<(String, Option<&'static str>)> {
    if domain == "WebKitErrorDomain" && code == 102 {
        return None;
    }
    if domain != "NSURLErrorDomain" {
        return Some((described.to_string(), None));
    }
    let (reason, label) = match code {
        -999 => return None,
        -1201 => (
            "its security certificate has expired",
            Some("Certificate expired"),
        ),
        -1204 => (
            "its security certificate is not valid yet",
            Some("Certificate not valid yet"),
        ),
        -1202 | -1203 => (
            "its security certificate is not trusted",
            Some("Certificate not trusted"),
        ),
        -1200 | -1205 | -1206 => ("a secure connection could not be made", None),
        -1003 => ("its name does not resolve", None),
        -1004 => ("nothing answers there", None),
        -1001 => ("it did not answer in time", None),
        -1009 => ("this Mac is offline", None),
        _ => return Some((described.to_string(), None)),
    };
    Some((reason.to_string(), label))
}

pub(super) struct Ivars {
    inner: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    persona_id: String,
    tab: Arc<Tab>,
    exceptions: Arc<Exceptions>,
    gate: Arc<Gate>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct TrustDoor;

    impl TrustDoor {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: NSObject's own answer for this class.
            let ours: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            ours || self.ivars().inner.respondsToSelector(selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding(&self, _selector: Sel) -> *mut AnyObject {
            Retained::as_ptr(&self.ivars().inner) as *mut AnyObject
        }
    }

    unsafe impl NSObjectProtocol for TrustDoor {}

    unsafe impl WKNavigationDelegate for TrustDoor {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(
            &self,
            web_view: &WKWebView,
            action: &WKNavigationAction,
            done: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: WebKit's own action and its frame, read on the main
            // thread. No target frame is a new window's first load.
            let top = unsafe { action.targetFrame() }
                .is_none_or(|frame| unsafe { frame.isMainFrame() });
            if top {
                // SAFETY: wry's delegate answers this itself, asking the gate.
                unsafe {
                    self.ivars()
                        .inner
                        .webView_decidePolicyForNavigationAction_decisionHandler(web_view, action, done);
                };
                return;
            }
            done.call((if self.frame_admits(action) {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn challenged(
            &self,
            _web_view: &WKWebView,
            challenge: &NSURLAuthenticationChallenge,
            done: &DynBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>,
        ) {
            let default = || done.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut()));
            let space = challenge.protectionSpace();
            // SAFETY: Foundation's own constant, read as it is.
            let server_trust = unsafe { NSURLAuthenticationMethodServerTrust }.to_string();
            if space.authenticationMethod().to_string() != server_trust
            {
                return default();
            }
            // SAFETY: a server-trust protection space answers its SecTrust.
            let trust: *mut SecTrust = unsafe { msg_send![&*space, serverTrust] };
            if trust.is_null() {
                return default();
            }
            let Some(print) = fingerprint(trust) else {
                return default();
            };
            let host = space.host().to_string();
            let origin = format!(
                "{}:{}",
                host.trim_end_matches('.').to_ascii_lowercase(),
                space.port()
            );
            let ivars = self.ivars();
            ivars
                .tab
                .seen
                .certificates
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(
                    origin.clone(),
                    Offered {
                        fingerprint: print.clone(),
                        // SAFETY: retained here, released when dropped.
                        trust: Kept(unsafe { CFRetain(trust.cast()) }.cast_mut().cast()),
                    },
                );
            if !ivars.exceptions.holds(&ivars.persona_id, &origin, &print) {
                return default();
            }
            // SAFETY: a credential for the very trust WebKit asked about.
            let credential: Retained<NSURLCredential> =
                unsafe { msg_send![NSURLCredential::class(), credentialForTrust: trust] };
            done.call((
                NSURLSessionAuthChallengeDisposition::UseCredential,
                Retained::as_ptr(&credential) as *mut NSURLCredential,
            ));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn failed(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            let inner = &self.ivars().inner;
            if inner.respondsToSelector(sel!(webView:didFailProvisionalNavigation:withError:)) {
                // SAFETY: wry's delegate answers this itself.
                unsafe { inner.webView_didFailProvisionalNavigation_withError(web_view, navigation, error) };
            }
            self.note_failure(web_view, error);
        }
    }
);

impl TrustDoor {
    /// Whether a frame inside the page may load what `action` asks: by the
    /// gate's frame rule, and never as a download, which the page refuses.
    fn frame_admits(&self, action: &WKNavigationAction) -> bool {
        // SAFETY: plain getters on WebKit's own action, on the main thread;
        // `shouldPerformDownload` only where the action answers it (11.3+).
        let download = unsafe {
            action.respondsToSelector(sel!(shouldPerformDownload)) && action.shouldPerformDownload()
        };
        // SAFETY: as above.
        let address = unsafe { action.request() }
            .URL()
            .and_then(|url| url.absoluteString())
            .map(|url| url.to_string());
        !download
            && address
                .and_then(|address| tauri::Url::parse(&address).ok())
                .is_some_and(|url| self.ivars().gate.admits_frame(&url))
    }

    fn note_failure(&self, web_view: &WKWebView, error: &NSError) {
        let domain = error.domain().to_string();
        let Some((reason, label)) = failure(
            &domain,
            error.code(),
            &error.localizedDescription().to_string(),
        ) else {
            return;
        };
        let tab = &self.ivars().tab;
        let url = failing_url(error).unwrap_or_else(|| {
            tab.seen
                .attempted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
        let origin = origin_of(&url).unwrap_or_else(|| url.clone());
        let message = if label.is_some() {
            let (reason, label, fingerprint) = {
                let certificates = tab
                    .seen
                    .certificates
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                match certificates.get(&origin) {
                    Some(offered) => {
                        let (reason, label) = verdict(offered.trust.0);
                        (reason.to_string(), label, Some(offered.fingerprint.clone()))
                    }
                    None => (reason, label.unwrap_or("Certificate not trusted"), None),
                }
            };
            let (page, message) = refused(
                tab,
                Untrusted {
                    url,
                    origin,
                    fingerprint,
                    label,
                },
                &reason,
            );
            // SAFETY: the page's own web view, on the main thread.
            let _ = unsafe { web_view.loadHTMLString_baseURL(&NSString::from_str(&page), None) };
            message
        } else {
            failed(tab, &origin, &reason)
        };
        tell(tab, message);
    }
}

/// The address a failed load was for, as the error names it.
fn failing_url(error: &NSError) -> Option<String> {
    let info = error.userInfo();
    let named = info
        .objectForKey(&NSString::from_str("NSErrorFailingURLKey"))
        .and_then(|value| value.downcast::<NSURL>().ok())
        .and_then(|url| url.absoluteString())
        .map(|url| url.to_string());
    named
        .or_else(|| {
            info.objectForKey(&NSString::from_str("NSErrorFailingURLStringKey"))
                .and_then(|value| value.downcast::<NSString>().ok())
                .map(|value| value.to_string())
        })
        .filter(|url| !url.is_empty())
}

thread_local! {
    /// WebKit holds a navigation delegate weakly: the desk's are kept here,
    /// on the main thread, by their web view, until it closes.
    static DOORS: RefCell<std::collections::HashMap<usize, Retained<TrustDoor>>> =
        RefCell::default();
}

/// Lets go of a closing page's delegate.
pub(in super::super) fn forget(view: *mut c_void) {
    DOORS.with(|doors| doors.borrow_mut().remove(&(view as usize)));
}

/// Puts the desk's delegate in front of wry's on a teammate's page.
pub(in super::super) fn attach(
    view_ptr: *mut c_void,
    persona_id: String,
    tab: Arc<Tab>,
    exceptions: Arc<Exceptions>,
    gate: Arc<Gate>,
) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: `with_webview` hands the live WKWebView of the page, on the
    // main thread.
    let view = unsafe { &*view_ptr.cast::<WKWebView>() };
    // SAFETY: as above.
    let Some(inner) = (unsafe { view.navigationDelegate() }) else {
        return;
    };
    let door = TrustDoor::alloc(main).set_ivars(Ivars {
        inner,
        persona_id,
        tab,
        exceptions,
        gate,
    });
    // SAFETY: allocated with its ivars set; NSObject's init returns it.
    let door: Retained<TrustDoor> = unsafe { msg_send![super(door), init] };
    // SAFETY: the door answers WKNavigationDelegate and forwards the rest.
    unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*door))) };
    DOORS.with(|doors| doors.borrow_mut().insert(view_ptr as usize, door));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_load_says_why_and_only_a_certificate_can_be_visited_anyway() {
        assert_eq!(failure("NSURLErrorDomain", -999, ""), None);
        assert_eq!(failure("WebKitErrorDomain", 102, ""), None);
        assert_eq!(
            failure("NSURLErrorDomain", -1201, ""),
            Some((
                "its security certificate has expired".into(),
                Some("Certificate expired")
            ))
        );
        assert_eq!(
            failure("NSURLErrorDomain", -1202, ""),
            Some((
                "its security certificate is not trusted".into(),
                Some("Certificate not trusted")
            ))
        );
        assert_eq!(
            failure("NSURLErrorDomain", -1004, ""),
            Some(("nothing answers there".into(), None))
        );
        assert_eq!(
            failure("NSURLErrorDomain", -1200, ""),
            Some(("a secure connection could not be made".into(), None))
        );
        assert_eq!(
            failure("NSPOSIXErrorDomain", 1, "Operation not permitted"),
            Some(("Operation not permitted".into(), None))
        );
    }

    #[test]
    fn a_refused_certificate_says_what_the_system_found() {
        assert_eq!(refusal(-67818).1, "Certificate expired");
        assert_eq!(refusal(-67602).1, "Wrong certificate name");
        assert_eq!(refusal(-67843).1, "Certificate not trusted");
        assert_eq!(refusal(0).1, "Certificate not trusted");
    }
}
