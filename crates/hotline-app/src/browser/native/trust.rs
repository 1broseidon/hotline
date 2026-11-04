//! What a teammate's page does when a load fails before it starts, and the
//! one way past a certificate WebKit refuses (docs/security.md, "A
//! teammate's browser").
//!
//! wry's navigation delegate answers neither WebKit's certificate challenge
//! nor a load that failed to start, so WebKit refuses a bad certificate and
//! the page stays, without a word, on the one before. A delegate of the
//! desk's own goes in front of wry's on each teammate page: it forwards
//! everything else to wry's, says why a load failed, and lets a refused
//! certificate through only when the person chose Visit anyway for that
//! very certificate, by the SHA-256 of its leaf, for that teammate, host
//! and port, until the desk stops. Only the bar, while the person has
//! control, adds one; the teammate never can.

use super::{Loads, Tab};
use block2::DynBlock;
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_foundation::{
    NSError, NSString, NSURLAuthenticationChallenge, NSURLAuthenticationMethodServerTrust,
    NSURLCredential, NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{WKNavigation, WKNavigationDelegate, WKWebView};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, PoisonError};

/// The certificates the person chose to visit anyway: teammate, host:port
/// and the leaf's SHA-256, for the desk's run.
#[derive(Default)]
pub(super) struct Exceptions(Mutex<HashSet<(String, String, String)>>);

impl Exceptions {
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

/// What the delegate keeps per page, beside the tab.
#[derive(Default)]
pub(super) struct Seen {
    /// The leaf fingerprint each host:port last offered.
    certificates: Mutex<HashMap<String, String>>,
    pub(super) untrusted: Mutex<Option<Untrusted>>,
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
    fn SecCertificateCopyData(certificate: *const c_void) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(object: *const c_void);
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

/// `host:port` as the gate and the bar key it.
fn origin_of(url: &str) -> Option<String> {
    tauri::Url::parse(url)
        .ok()
        .as_ref()
        .and_then(hotline_core::browser::origin_key)
}

pub(super) struct Ivars {
    inner: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
    persona_id: String,
    tab: Arc<Tab>,
    exceptions: Arc<Exceptions>,
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
                .insert(origin.clone(), print.clone());
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
            self.note_failure(error);
        }
    }
);

impl TrustDoor {
    fn note_failure(&self, error: &NSError) {
        let domain = error.domain().to_string();
        let Some((reason, label)) = failure(
            &domain,
            error.code(),
            &error.localizedDescription().to_string(),
        ) else {
            return;
        };
        let url = error
            .userInfo()
            .objectForKey(&NSString::from_str("NSErrorFailingURLStringKey"))
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string())
            .unwrap_or_default();
        let origin = origin_of(&url).unwrap_or_else(|| url.clone());
        let tab = &self.ivars().tab;
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
        let past = if label.is_some() {
            " Only the person can visit it anyway: they open your browser's pane, take control and choose Visit anyway. Ask them if the task needs it; you cannot do it yourself."
        } else {
            ""
        };
        let message = format!("{origin} did not open: {reason}.{still}{past}");
        *tab.seen
            .untrusted
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = label.map(|label| {
            let fingerprint = tab
                .seen
                .certificates
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&origin)
                .cloned();
            Untrusted {
                url: url.clone(),
                origin: origin.clone(),
                fingerprint,
                label,
            }
        });
        tab.loads
            .send_modify(|loads: &mut Loads| loads.failed = Some(message));
    }
}

thread_local! {
    /// WebKit holds a navigation delegate weakly: the desk's are kept here,
    /// on the main thread, for the app's run.
    static DOORS: RefCell<Vec<Retained<TrustDoor>>> = const { RefCell::new(Vec::new()) };
}

/// Puts the desk's delegate in front of wry's on a teammate's page.
pub(super) fn attach(
    view: *mut c_void,
    persona_id: String,
    tab: Arc<Tab>,
    exceptions: Arc<Exceptions>,
) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: `with_webview` hands the live WKWebView of the page, on the
    // main thread.
    let view = unsafe { &*view.cast::<WKWebView>() };
    // SAFETY: as above.
    let Some(inner) = (unsafe { view.navigationDelegate() }) else {
        return;
    };
    let door = TrustDoor::alloc(main).set_ivars(Ivars {
        inner,
        persona_id,
        tab,
        exceptions,
    });
    // SAFETY: allocated with its ivars set; NSObject's init returns it.
    let door: Retained<TrustDoor> = unsafe { msg_send![super(door), init] };
    // SAFETY: the door answers WKNavigationDelegate and forwards the rest.
    unsafe { view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*door))) };
    DOORS.with(|doors| doors.borrow_mut().push(door));
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
}
