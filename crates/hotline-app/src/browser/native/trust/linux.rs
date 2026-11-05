//! The Linux side of a teammate page's trust (`trust.rs`). WebKitGTK asks
//! nothing while it loads: a refused certificate comes back as
//! `load-failed-with-tls-errors`, with the certificate and why, and any
//! other failure as `load-failed`. The desk answers both, keeps the
//! certificate on the main thread, and puts the warning in the page's
//! place. Visit anyway, from the bar while the person has control, lets
//! that very certificate through in the teammate's own web context
//! (`webkit_web_context_allow_tls_certificate_for_host`), and only when its
//! hash is the one the person was shown. WebKit keeps that in memory for
//! the run; the context is the teammate's alone, but WebKit pins it to the
//! host, not the host and port the desk's own exceptions name.

use super::{Exceptions, Tab, Untrusted, failed, origin_of, refused, tell};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use webkit2gtk::gio::{TlsCertificate, TlsCertificateFlags, prelude::TlsCertificateExt};
use webkit2gtk::glib::{self, translate::ToGlibPtr};
use webkit2gtk::{LoadEvent, WebContextExt, WebView, WebViewExt};

/// What a failed load leaves for its own `Finished`, which WebKitGTK sends
/// with the failing address (macOS sends none for a load that never
/// started): the address the page was still on, and a certificate's
/// refusal, both put back once the shared page-load handler has seen that
/// `Finished` and taken it for a page that loaded.
struct Failed {
    shown: String,
    untrusted: Option<Untrusted>,
}

thread_local! {
    static FAILED: RefCell<HashMap<usize, Failed>> = RefCell::default();
    /// The certificate each page was last refused, by its web view and
    /// host:port: the leaf's SHA-256 and the certificate itself, on the
    /// main thread where GTK keeps it.
    static OFFERED: RefCell<HashMap<(usize, String), (String, TlsCertificate)>> =
        RefCell::default();
}

fn key(view: &WebView) -> usize {
    glib::ObjectType::as_ptr(view) as usize
}

/// Why WebKitGTK refused a certificate, in words and as the bar's label.
fn refusal(errors: TlsCertificateFlags) -> (&'static str, &'static str) {
    if errors.contains(TlsCertificateFlags::EXPIRED) {
        (
            "its security certificate has expired",
            "Certificate expired",
        )
    } else if errors.contains(TlsCertificateFlags::NOT_ACTIVATED) {
        (
            "its security certificate is not valid yet",
            "Certificate not valid yet",
        )
    } else if errors.contains(TlsCertificateFlags::BAD_IDENTITY) {
        (
            "its security certificate is for a different name",
            "Wrong certificate name",
        )
    } else {
        (
            "its security certificate is not trusted",
            "Certificate not trusted",
        )
    }
}

/// Why a load failed to start, by its error's domain and code; `None` for
/// a load that was only superseded, stopped or refused by the gate.
fn failure(domain: &str, code: i32, described: &str) -> Option<String> {
    let reason = match (domain, code) {
        // Cancelled, and a load the gate's answer stopped.
        ("WebKitNetworkError", 302) | ("WebKitPolicyError", 102) | ("g-io-error-quark", 19) => {
            return None;
        }
        ("g-resolver-error-quark", _) | ("g-io-error-quark", 28) => "its name does not resolve",
        ("g-io-error-quark", 39) => "nothing answers there",
        ("g-io-error-quark", 24) => "it did not answer in time",
        ("g-io-error-quark", 37) => "its host cannot be reached",
        ("g-io-error-quark", 38) => "this computer is offline",
        ("g-tls-error-quark", _) => "a secure connection could not be made",
        _ => return Some(described.to_string()),
    };
    Some(reason.to_string())
}

/// The leaf certificate's SHA-256, in hex.
fn fingerprint(certificate: &TlsCertificate) -> Option<String> {
    let der = certificate.certificate()?;
    Some(hotline_core::browser::certificate_fingerprint(&der))
}

/// Notes what to put back after a failed load's own `Finished`.
fn keep(view: &WebView, tab: &Tab, untrusted: Option<Untrusted>) {
    let shown = tab
        .shown
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    FAILED.with(|failed| {
        failed
            .borrow_mut()
            .insert(key(view), Failed { shown, untrusted });
    });
}

/// Listens for the page's failed loads.
pub(in super::super) fn attach(
    view: WebView,
    _persona_id: String,
    tab: Arc<Tab>,
    _exceptions: Arc<Exceptions>,
    _gate: Arc<hotline_core::browser::Gate>,
) {
    {
        let tab = tab.clone();
        view.connect_load_failed_with_tls_errors(move |view, uri, certificate, errors| {
            let origin = origin_of(uri).unwrap_or_else(|| uri.to_string());
            let print = fingerprint(certificate);
            if let Some(print) = &print {
                OFFERED.with(|offered| {
                    offered.borrow_mut().insert(
                        (key(view), origin.clone()),
                        (print.clone(), certificate.clone()),
                    );
                });
            }
            let (reason, label) = refusal(errors);
            let untrusted = Untrusted {
                url: uri.to_string(),
                origin,
                fingerprint: print,
                label,
            };
            keep(view, &tab, Some(untrusted.clone()));
            let (page, message) = refused(&tab, untrusted, reason);
            // Not from inside the failed load's own signal.
            let view = view.clone();
            glib::idle_add_local_once(move || view.load_html(&page, None));
            tell(&tab, message);
            true
        });
    }
    {
        let tab = tab.clone();
        // Connected after wry's own, so it runs once wry's has seen the
        // failed load's `Finished`.
        view.connect_load_changed(move |view, event| {
            if event != LoadEvent::Finished {
                return;
            }
            let Some(failed) = FAILED.with(|failed| failed.borrow_mut().remove(&key(view))) else {
                return;
            };
            *tab.shown.lock().unwrap_or_else(PoisonError::into_inner) = failed.shown;
            if let Some(untrusted) = failed.untrusted {
                *tab.seen
                    .untrusted
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(untrusted);
            }
        });
    }
    view.connect_load_failed(move |view, _event, uri, error| {
        // SAFETY: a live GError, read and not kept.
        let code = unsafe { (*ToGlibPtr::<*const glib::ffi::GError>::to_glib_none(error).0).code };
        let Some(reason) = failure(error.domain().as_str(), code, error.message()) else {
            return false;
        };
        let url = if uri.is_empty() {
            tab.seen
                .attempted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        } else {
            uri.to_string()
        };
        let origin = origin_of(&url).unwrap_or_else(|| url.clone());
        keep(view, &tab, None);
        tell(&tab, failed(&tab, &origin, &reason));
        // Handled: the page stays on the one before, as on macOS, with no
        // error page of WebKit's in its place.
        true
    });
}

/// Lets go of what was kept for a page that is closing.
pub(in super::super) fn forget(view: WebView) {
    let key = key(&view);
    FAILED.with(|failed| failed.borrow_mut().remove(&key));
    OFFERED.with(|offered| offered.borrow_mut().retain(|(view, _), _| *view != key));
}

/// Visit anyway: lets the certificate this page was refused on `origin`
/// through in the teammate's own web context, if it is still the one with
/// `fingerprint`.
pub(in super::super) fn let_through(
    view: WebView,
    origin: &str,
    fingerprint: &str,
) -> Result<(), String> {
    let certificate = OFFERED
        .with(|offered| {
            offered
                .borrow()
                .get(&(key(&view), origin.to_string()))
                .filter(|(print, _)| print == fingerprint)
                .map(|(_, certificate)| certificate.clone())
        })
        .ok_or("Hotline did not see that site's certificate. Reload and try again.")?;
    let host = origin
        .rsplit_once(':')
        .map_or(origin, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    let context = view
        .context()
        .ok_or("The teammate's browser has no web context.")?;
    context.allow_tls_certificate_for_host(&certificate, host);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_load_says_why_and_a_stopped_one_says_nothing() {
        assert_eq!(failure("WebKitNetworkError", 302, ""), None);
        assert_eq!(failure("WebKitPolicyError", 102, ""), None);
        assert_eq!(failure("g-io-error-quark", 19, ""), None);
        assert_eq!(
            failure("g-resolver-error-quark", 0, "").as_deref(),
            Some("its name does not resolve")
        );
        assert_eq!(
            failure("g-io-error-quark", 39, "").as_deref(),
            Some("nothing answers there")
        );
        assert_eq!(
            failure("g-tls-error-quark", 3, "").as_deref(),
            Some("a secure connection could not be made")
        );
        assert_eq!(
            failure("WebKitPolicyError", 101, "Cannot show URI").as_deref(),
            Some("Cannot show URI")
        );
    }

    #[test]
    fn a_refused_certificate_says_what_webkit_found() {
        assert_eq!(
            refusal(TlsCertificateFlags::EXPIRED | TlsCertificateFlags::UNKNOWN_CA).1,
            "Certificate expired"
        );
        assert_eq!(
            refusal(TlsCertificateFlags::NOT_ACTIVATED).1,
            "Certificate not valid yet"
        );
        assert_eq!(
            refusal(TlsCertificateFlags::BAD_IDENTITY).1,
            "Wrong certificate name"
        );
        assert_eq!(
            refusal(TlsCertificateFlags::UNKNOWN_CA).1,
            "Certificate not trusted"
        );
    }

    #[test]
    fn a_failed_loads_domain_is_the_one_its_error_names() {
        let error = glib::Error::new(webkit2gtk::gio::IOErrorEnum::ConnectionRefused, "refused");
        // SAFETY: a live GError, read and not kept.
        let code = unsafe { (*ToGlibPtr::<*const glib::ffi::GError>::to_glib_none(&error).0).code };
        assert_eq!(
            failure(error.domain().as_str(), code, error.message()).as_deref(),
            Some("nothing answers there")
        );
    }
}
