//! WebKit itself, on the main thread `with_webview` runs on: the macOS side
//! of the desk browser's platform seam (`browser.rs`).

use super::Step;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventModifierFlags, NSEventType, NSImage,
    NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode,
};
use objc2_foundation::{NSDictionary, NSError, NSPoint, NSProcessInfo, NSString};
use objc2_web_kit::{WKContentWorld, WKUIDelegate, WKWebView};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::oneshot;

/// The content world the desk's scripts run in: the page's own
/// scripts never see its variables, and it never sees theirs.
const WORLD: &str = "hotline-desk-browser";

thread_local! {
    /// Held for the app's life: a named world lives only while it is
    /// retained, and its variables with it.
    static KEPT: std::cell::OnceCell<objc2::rc::Retained<WKContentWorld>> =
        const { std::cell::OnceCell::new() };
}

type Reply<T> = Arc<Mutex<Option<oneshot::Sender<Result<T, String>>>>>;

fn send<T>(reply: &Reply<T>, answer: Result<T, String>) {
    if let Some(sender) = reply.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = sender.send(answer);
    }
}

fn said(error: *mut NSError) -> String {
    // SAFETY: WebKit hands a valid NSError or null.
    unsafe { error.as_ref() }.map_or_else(
        || "WebKit gave no reason.".into(),
        |error| error.localizedDescription().to_string(),
    )
}

/// The web view `with_webview` hands over, as WebKit's type.
///
/// SAFETY: the caller is `with_webview`'s closure, on the main
/// thread, where `view` is the live `WKWebView` of a Tauri window.
unsafe fn view<'a>(view: *mut c_void) -> &'a WKWebView {
    // SAFETY: as the function's own contract above.
    unsafe { &*view.cast::<WKWebView>() }
}

/// Runs `body` as an async function in the desk's content world and
/// answers its string.
pub(super) fn call(view_ptr: *mut c_void, body: &str, reply: Reply<String>) {
    let Some(main) = MainThreadMarker::new() else {
        return send(
            &reply,
            Err("The browser was asked off the main thread.".into()),
        );
    };
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    let world = KEPT.with(|kept| {
        kept.get_or_init(|| {
            // SAFETY: a named world, retained here for the app's life.
            unsafe { WKContentWorld::worldWithName(&NSString::from_str(WORLD), main) }
        })
        .clone()
    });
    let body = NSString::from_str(body);
    let done = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        if !error.is_null() {
            return send(&reply, Err(said(error)));
        }
        // SAFETY: WebKit hands the script's value or null.
        let answer = unsafe { result.as_ref() }
            .and_then(|value| value.downcast_ref::<NSString>())
            .map(ToString::to_string)
            .ok_or_else(|| "The page answered nothing.".to_string());
        send(&reply, answer);
    });
    // SAFETY: no arguments, the main frame, and a block that lives
    // as long as WebKit keeps it.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &body,
            None,
            None,
            &world,
            Some(&done),
        );
    }
}

pub(super) fn step(view_ptr: *mut c_void, step: Step) {
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    // SAFETY: plain WebKit navigation on its own thread.
    unsafe {
        match step {
            Step::Back => {
                let _ = view.goBack();
            }
            Step::Forward => {
                let _ = view.goForward();
            }
            Step::Reload => {
                let _ = view.reload();
            }
        }
    }
}

/// What the desk sends the web view as the person's own input: a
/// click or a hover at a point in the page's CSS pixels, with the
/// page's viewport height, or one key.
pub(super) enum Input {
    Click(f64, f64, f64),
    Hover(f64, f64, f64),
    Key(String),
}

/// A named key as AppKit spells it: its characters, its virtual key
/// code and whether Shift is held.
fn key_of(key: &str) -> (String, u16, bool) {
    let function = |code: u32| char::from_u32(code).unwrap_or(' ').to_string();
    match key {
        "Enter" => ("\r".into(), 36, false),
        "Tab" => ("\t".into(), 48, false),
        "Shift+Tab" => ("\u{19}".into(), 48, true),
        "Escape" => ("\u{1b}".into(), 53, false),
        "Backspace" => ("\u{7f}".into(), 51, false),
        "Delete" => (function(0xF728), 117, false),
        "Space" => (" ".into(), 49, false),
        "ArrowUp" => (function(0xF700), 126, false),
        "ArrowDown" => (function(0xF701), 125, false),
        "ArrowLeft" => (function(0xF702), 123, false),
        "ArrowRight" => (function(0xF703), 124, false),
        "Home" => (function(0xF729), 115, false),
        "End" => (function(0xF72B), 119, false),
        "PageUp" => (function(0xF72C), 116, false),
        "PageDown" => (function(0xF72D), 121, false),
        other => (other.to_string(), 0, false),
    }
}

/// Puts a hidden browser window on screen, invisible, at the back and
/// out of the person's way: WebKit takes input only in a window that
/// is ordered in. Showing the pane makes it opaque again.
fn cloak(window: &NSWindow) {
    if window.isVisible() {
        return;
    }
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    window.orderBack(None);
}

/// Hides the pane again, as `cloak` keeps a never-shown one: the
/// teammate's browser goes on behind everything, invisible.
pub(super) fn conceal(view_ptr: *mut c_void) {
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    if let Some(window) = view.window() {
        window.setAlphaValue(0.0);
        window.setIgnoresMouseEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.orderBack(None);
    }
}

/// Shows the pane: the window opaque, in front, taking the person's
/// clicks, and the page shielded until they take it over.
pub(super) fn reveal(view_ptr: *mut c_void) {
    // SAFETY: see `view`.
    let page = unsafe { view(view_ptr) };
    if let Some(window) = page.window() {
        window.setAlphaValue(1.0);
        window.setIgnoresMouseEvents(false);
        window.setCollectionBehavior(NSWindowCollectionBehavior::Default);
        window.makeKeyAndOrderFront(None);
    }
    shield(view_ptr, true);
}

objc2::define_class!(
    /// A clear view that takes a watcher's mouse and scroll and does
    /// nothing with them. A plain NSView would hand them up to the
    /// window's own view, which passes them on to the page.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "HotlineBrowserShield"]
    struct Shield;

    impl Shield {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {}
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {}
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {}
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &NSEvent) {}
        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, _event: &NSEvent) {}
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, _event: &NSEvent) {}
        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, _event: &NSEvent) {}
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, _event: &NSEvent) {}
        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, _event: &NSEvent) {}
    }
);

thread_local! {
    /// The shield over each page a watcher sees, by its web view.
    static SHIELDS: std::cell::RefCell<std::collections::HashMap<usize, objc2::rc::Retained<NSView>>> =
        std::cell::RefCell::default();
}

/// A clear view over the page that takes a watcher's clicks and
/// scrolls, so watching never drives the page. The desk's own input
/// goes to the page directly and is not stopped by it.
pub(super) fn shield(view_ptr: *mut c_void, on: bool) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: see `view`.
    let page = unsafe { view(view_ptr) };
    SHIELDS.with(|shields| {
        let mut shields = shields.borrow_mut();
        let key = view_ptr as usize;
        if !on {
            if let Some(shield) = shields.remove(&key) {
                shield.removeFromSuperview();
            }
            return;
        }
        // Watching again: nothing the person types reaches the page.
        if let Some(window) = page.window() {
            window.makeFirstResponder(None);
        }
        if shields.contains_key(&key) {
            return;
        }
        // SAFETY: the page's own superview, on the main thread.
        let Some(parent) = (unsafe { page.superview() }) else {
            return;
        };
        let frame = page.frame();
        // SAFETY: NSView's designated initialiser, on a fresh Shield.
        let shield: objc2::rc::Retained<Shield> =
            unsafe { objc2::msg_send![Shield::alloc(main), initWithFrame: frame] };
        let shield = objc2::rc::Retained::into_super(shield);
        parent.addSubview_positioned_relativeTo(&shield, NSWindowOrderingMode::Above, Some(page));
        shields.insert(key, shield);
    });
}

/// Lets go of what the desk put over a page that is closing: its shield.
/// The hands, the trust delegate and the popup's door have their own.
pub(super) fn forget(view_ptr: *mut c_void) {
    shield(view_ptr, false);
    DOORS.with(|doors| doors.borrow_mut().remove(&(view_ptr as usize)));
}

pub(super) struct DoorIvars {
    inner: Retained<ProtocolObject<dyn WKUIDelegate>>,
    closed: Box<dyn Fn()>,
}

objc2::define_class!(
    /// A popup's UI delegate, in front of wry's: wry answers nothing when
    /// the popup's own script closes it (`window.close()`, as a sign-in
    /// popup does once it has answered its opener), so the desk does, and
    /// passes everything else to wry's.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DoorIvars]
    #[name = "HotlineBrowserPopupDoor"]
    struct PopupDoor;

    impl PopupDoor {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: NSObject's own answer for this class.
            let ours: bool = unsafe { objc2::msg_send![super(self), respondsToSelector: selector] };
            ours || self.ivars().inner.respondsToSelector(selector)
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding(&self, _selector: Sel) -> *mut AnyObject {
            Retained::as_ptr(&self.ivars().inner) as *mut AnyObject
        }
    }

    unsafe impl NSObjectProtocol for PopupDoor {}

    unsafe impl WKUIDelegate for PopupDoor {
        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _web_view: &WKWebView) {
            (self.ivars().closed)();
        }
    }
);

thread_local! {
    /// WebKit holds a UI delegate weakly: each popup's door is kept here,
    /// by its web view, until the popup closes.
    static DOORS: std::cell::RefCell<std::collections::HashMap<usize, Retained<PopupDoor>>> =
        std::cell::RefCell::default();
}

/// Puts the desk's door in front of wry's UI delegate on a popup, so its
/// own `window.close()` runs `closed`.
pub(super) fn watch_close(view_ptr: *mut c_void, closed: Box<dyn Fn()>) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    // SAFETY: a plain getter on the main thread.
    let Some(inner) = (unsafe { view.UIDelegate() }) else {
        return;
    };
    let door = PopupDoor::alloc(main).set_ivars(DoorIvars { inner, closed });
    // SAFETY: allocated with its ivars set; NSObject's init returns it.
    let door: Retained<PopupDoor> = unsafe { objc2::msg_send![super(door), init] };
    // SAFETY: the door answers WKUIDelegate and forwards the rest.
    unsafe { view.setUIDelegate(Some(ProtocolObject::from_ref(&*door))) };
    DOORS.with(|doors| doors.borrow_mut().insert(view_ptr as usize, door));
}

/// The page's size is its web view's frame, which the window lays out; a
/// WKWebView needs nothing more.
pub(super) fn fit(_view_ptr: *mut c_void, _size: Option<(u32, u32)>) {}

/// Keeps a page's shield over it after the window is resized.
pub(super) fn shield_follow(view_ptr: *mut c_void) {
    // SAFETY: see `view`.
    let page = unsafe { view(view_ptr) };
    SHIELDS.with(|shields| {
        if let Some(shield) = shields.borrow().get(&(view_ptr as usize)) {
            shield.setFrame(page.frame());
        }
    });
}

pub(super) fn input(view_ptr: *mut c_void, input: &Input) {
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    let Some(window) = view.window() else {
        return;
    };
    cloak(&window);
    let number = window.windowNumber();
    let time = NSProcessInfo::processInfo().systemUptime();
    // The page's viewport sits below the window's title bar inside
    // the view, so a point in it is lower in the view by the height
    // the bar hides.
    let at = |x: f64, y: f64, viewport: f64| {
        let height = view.bounds().size.height;
        let y = y + (height - viewport).max(0.0);
        let local = NSPoint::new(x, if view.isFlipped() { y } else { height - y });
        view.convertPoint_toView(local, None)
    };
    let mouse = |kind: NSEventType, point: NSPoint| {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            point,
            NSEventModifierFlags::empty(),
            time,
            number,
            None,
            0,
            1,
            1.0,
        )
    };
    match input {
        Input::Click(x, y, viewport) => {
            let point = at(*x, *y, *viewport);
            if let Some(moved) = mouse(NSEventType::MouseMoved, point) {
                view.mouseMoved(&moved);
            }
            if let (Some(down), Some(up)) = (
                mouse(NSEventType::LeftMouseDown, point),
                mouse(NSEventType::LeftMouseUp, point),
            ) {
                view.mouseDown(&down);
                view.mouseUp(&up);
            }
        }
        Input::Hover(x, y, viewport) => {
            if let Some(moved) = mouse(NSEventType::MouseMoved, at(*x, *y, *viewport)) {
                view.mouseMoved(&moved);
            }
        }
        Input::Key(key) => {
            let (characters, code, shift) = key_of(key);
            let flags = if shift {
                NSEventModifierFlags::Shift
            } else {
                NSEventModifierFlags::empty()
            };
            let characters = NSString::from_str(&characters);
            window.makeFirstResponder(Some(view));
            for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
                let Some(event) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind,
                    NSPoint::new(0.0, 0.0),
                    flags,
                    time,
                    number,
                    None,
                    &characters,
                    &characters,
                    false,
                    code,
                ) else {
                    continue;
                };
                if kind == NSEventType::KeyDown {
                    view.keyDown(&event);
                } else {
                    view.keyUp(&event);
                }
            }
        }
    }
}

/// The visible page as a PNG.
pub(super) fn snapshot(view_ptr: *mut c_void, reply: Reply<Vec<u8>>) {
    // SAFETY: see `view`.
    let view = unsafe { view(view_ptr) };
    let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        // SAFETY: WebKit hands a valid image or null.
        let answer = match unsafe { image.as_ref() } {
            Some(image) => png(image),
            None => Err(said(error)),
        };
        send(&reply, answer);
    });
    // SAFETY: the default configuration is the visible viewport.
    unsafe { view.takeSnapshotWithConfiguration_completionHandler(None, &done) };
}

fn png(image: &NSImage) -> Result<Vec<u8>, String> {
    let tiff = image
        .TIFFRepresentation()
        .ok_or("The page's picture could not be read.")?;
    let bitmap =
        NSBitmapImageRep::imageRepWithData(&tiff).ok_or("The page's picture could not be read.")?;
    // SAFETY: an empty property dictionary is valid for PNG.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("The page's picture could not be made a PNG.")?;
    Ok(png.to_vec())
}
