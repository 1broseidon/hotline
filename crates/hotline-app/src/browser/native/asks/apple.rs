//! The macOS side of a teammate page's asks (`asks.rs`): a UI delegate of the
//! desk's own goes in front of wry's on each teammate page. wry's grants
//! every microphone and camera request and runs a modal `NSOpenPanel` for
//! every file input; this one denies the first always and lets the second
//! through to wry's only while the person has control. Everything else,
//! the page's request for a window among it, goes on to wry's.

use super::{Ask, Tab, grants, opens_picker, refuse_picker};
use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSArray, NSURL};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKOpenPanelParameters, WKPermissionDecision, WKSecurityOrigin,
    WKUIDelegate, WKWebView,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;

/// What a capture request asks for.
fn media_ask(kind: WKMediaCaptureType) -> Ask {
    if kind == WKMediaCaptureType::Microphone {
        Ask::Microphone
    } else if kind == WKMediaCaptureType::Camera {
        Ask::Camera
    } else {
        Ask::Capture
    }
}

/// WebKit's answer to a capture request: never Prompt, which would put
/// WebKit's own question to whoever is at the Mac.
fn decision(ask: Ask) -> WKPermissionDecision {
    if grants(ask) {
        WKPermissionDecision::Grant
    } else {
        WKPermissionDecision::Deny
    }
}

pub(super) struct Ivars {
    /// wry's delegate, which makes the window a page asks for.
    inner: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
    tab: Arc<Tab>,
    person_has_it: Box<dyn Fn() -> bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct AskDoor;

    impl AskDoor {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: NSObject's own answer for this class.
            let ours: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            ours || self
                .ivars()
                .inner
                .as_ref()
                .is_some_and(|inner| inner.respondsToSelector(selector))
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding(&self, _selector: Sel) -> *mut AnyObject {
            self.ivars()
                .inner
                .as_ref()
                .map_or(std::ptr::null_mut(), |inner| Retained::as_ptr(inner) as *mut AnyObject)
        }
    }

    unsafe impl NSObjectProtocol for AskDoor {}

    unsafe impl WKUIDelegate for AskDoor {
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn capture(
            &self,
            _web_view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            decide: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            decide.call((decision(media_ask(kind)),));
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            web_view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            frame: &WKFrameInfo,
            done: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            let ivars = self.ivars();
            if !opens_picker((ivars.person_has_it)()) {
                refuse_picker(&ivars.tab);
                return done.call((std::ptr::null_mut(),));
            }
            match &ivars.inner {
                Some(inner)
                    if inner.respondsToSelector(sel!(
                        webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:
                    )) =>
                {
                    // SAFETY: wry's delegate answers this itself, on the main
                    // thread, with the arguments WebKit gave.
                    unsafe {
                        inner.webView_runOpenPanelWithParameters_initiatedByFrame_completionHandler(
                            web_view, parameters, frame, done,
                        );
                    }
                }
                _ => done.call((std::ptr::null_mut(),)),
            }
        }
    }
);

thread_local! {
    /// WebKit holds a UI delegate weakly: the desk's are kept here, on the
    /// main thread, for the app's run.
    static DOORS: RefCell<Vec<Retained<AskDoor>>> = const { RefCell::new(Vec::new()) };
}

/// Puts the desk's UI delegate in front of wry's on a teammate's page.
/// `person_has_it` says whether the person has control of it now.
pub(in super::super) fn attach(
    view: *mut c_void,
    tab: Arc<Tab>,
    person_has_it: impl Fn() -> bool + 'static,
) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: `with_webview` hands the live WKWebView of the page, on the
    // main thread.
    let view = unsafe { &*view.cast::<WKWebView>() };
    // SAFETY: as above.
    let inner = unsafe { view.UIDelegate() };
    let door = AskDoor::alloc(main).set_ivars(Ivars {
        inner,
        tab,
        person_has_it: Box::new(person_has_it),
    });
    // SAFETY: allocated with its ivars set; NSObject's init returns it.
    let door: Retained<AskDoor> = unsafe { msg_send![super(door), init] };
    // SAFETY: the door answers WKUIDelegate and forwards the rest.
    unsafe { view.setUIDelegate(Some(ProtocolObject::from_ref(&*door))) };
    DOORS.with(|doors| doors.borrow_mut().push(door));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_microphone_and_camera_are_always_denied_never_asked() {
        for kind in [
            WKMediaCaptureType::Microphone,
            WKMediaCaptureType::Camera,
            WKMediaCaptureType::CameraAndMicrophone,
            WKMediaCaptureType(7),
        ] {
            assert_eq!(decision(media_ask(kind)), WKPermissionDecision::Deny);
        }
        assert_eq!(media_ask(WKMediaCaptureType::Microphone), Ask::Microphone);
        assert_eq!(media_ask(WKMediaCaptureType::Camera), Ask::Camera);
    }
}
