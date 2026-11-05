//! The Windows side of a teammate page's asks (`asks.rs`).
//!
//! WebView2 puts every permission a page asks for to `PermissionRequested`,
//! and with no answer shows its own prompt to whoever is at the computer;
//! the desk answers each itself, denied by name. A request to capture the
//! screen comes to `ScreenCaptureStarting` instead, and is cancelled.
//!
//! WebView2 has no event for a page's file chooser. The desk has two holds
//! on it. A file input's chooser is intercepted through the DevTools
//! protocol (`Page.setInterceptFileChooserDialog`) while the teammate
//! drives: Chromium shows nothing, tells the page it was cancelled, and the
//! desk hears of it (`Page.fileChooserOpened`) and tells the act that made
//! it. The person's Take control lifts the interception and Give it back
//! puts it on again (`win.rs`'s shield). A chooser the protocol does not
//! intercept (a page's `showOpenFilePicker`, `showSaveFilePicker` or
//! `showDirectoryPicker`) is a dialog of WebView2's owned by the teammate's
//! window: the desk watches for any dialog that starts in a window of a
//! teammate's (`EVENT_SYSTEM_DIALOGSTART`) and closes it, as Cancel would,
//! unless the person has control. That one may be on the screen for a
//! moment before it closes; nothing is chosen.
//!
//! WebView2's own WebAuthn would reach the person's Windows Hello and
//! security keys, which are not the teammate's: the desk puts Chromium's
//! virtual authenticator environment, with no authenticator in it, in front
//! of them (`WebAuthn.enable`), so a page's passkey call reaches none and no
//! prompt is shown (`edge::NO_PASSKEYS` refuses it in the page at once).

use super::{Ask, Tab, grants, opens_picker, refuse_picker};
use crate::browser::native::win::{View, devtools, root, webview};
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
    COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
    COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    COREWEBVIEW2_PERMISSION_STATE_DENY, ICoreWebView2, ICoreWebView2_27,
};
use webview2_com::{
    DevToolsProtocolEventReceivedEventHandler, PermissionRequestedEventHandler,
    ScreenCaptureStartingEventHandler,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_DIALOGSTART, GA_ROOT, GW_OWNER, GetAncestor, GetWindow, PostMessageW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE,
};
use windows::core::{HSTRING, Interface};

/// What a WebView2 permission request asks for.
fn ask_of(kind: COREWEBVIEW2_PERMISSION_KIND) -> Ask {
    match kind {
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => Ask::Microphone,
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => Ask::Camera,
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => Ask::Location,
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => Ask::Notifications,
        // Sensors, the clipboard, many downloads, files read and written
        // through the File System Access API, autoplay, local fonts, MIDI,
        // window placement and whatever a later WebView2 adds.
        _ => Ask::Other,
    }
}

/// A teammate's window, by its top-level window, while it is open: whose
/// tab, and whether the person has control of it now.
struct Watched {
    tab: Weak<Tab>,
    person_has_it: Rc<dyn Fn() -> bool>,
}

thread_local! {
    static WATCHED: RefCell<HashMap<isize, Watched>> = RefCell::default();
    /// Each page's top-level window by its controller, so it is let go of
    /// even once wry has destroyed the page's own window under it (a
    /// popup's `window.close()`).
    static ROOTS: RefCell<HashMap<usize, isize>> = RefCell::default();
    static HOOKED: RefCell<bool> = const { RefCell::new(false) };
}

/// A dialog started somewhere on the desktop: one owned by a teammate's
/// window, while the teammate drives, is closed as Cancel would close it.
unsafe extern "system" fn dialog_started(
    _hook: HWINEVENTHOOK,
    event: u32,
    dialog: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_DIALOGSTART {
        return;
    }
    // SAFETY: plain window queries of a handle Windows just named.
    let Ok(owner) = (unsafe { GetWindow(dialog, GW_OWNER) }) else {
        return;
    };
    // SAFETY: as above.
    let top = unsafe { GetAncestor(owner, GA_ROOT) };
    let refused = WATCHED.with(|watched| {
        let watched = watched.borrow();
        let entry = watched.get(&(top.0 as isize))?;
        (!opens_picker((entry.person_has_it)())).then(|| entry.tab.upgrade())
    });
    let Some(tab) = refused else {
        return;
    };
    // SAFETY: a close request posted to the dialog's own queue.
    let _ = unsafe { PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    if let Some(tab) = tab {
        refuse_picker(&tab);
    }
}

/// Watches every dialog that starts, once for the app's life.
fn hook_dialogs() {
    HOOKED.with(|hooked| {
        if *hooked.borrow() {
            return;
        }
        // SAFETY: an out-of-context hook, called on this thread's message
        // loop, the main thread's, for the app's life.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_DIALOGSTART,
                EVENT_SYSTEM_DIALOGSTART,
                None,
                Some(dialog_started),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        *hooked.borrow_mut() = !hook.is_invalid();
    });
}

/// Intercepts the page's file chooser (`on`) while the teammate drives, or
/// lets it through to the person.
pub(in super::super) fn intercept_files(page: &ICoreWebView2, on: bool) {
    devtools(
        page,
        "Page.setInterceptFileChooserDialog",
        &json!({ "enabled": on, "cancel": on }),
        |_| {},
    );
}

/// Answers the page's permission requests, screen captures, file choosers,
/// dialogs and passkey calls. `person_has_it` says whether the person has
/// control of it now.
pub(in super::super) fn attach(
    view: View,
    tab: Arc<Tab>,
    person_has_it: impl Fn() -> bool + 'static,
) {
    let Some(page) = webview(&view) else {
        return;
    };
    let person_has_it: Rc<dyn Fn() -> bool> = Rc::new(person_has_it);
    let mut token = 0;
    // SAFETY: WebView2's events, on the main thread; each handler lives as
    // long as the web view.
    unsafe {
        let _ = page.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&raw mut kind)?;
                args.SetState(if grants(ask_of(kind)) {
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW
                } else {
                    COREWEBVIEW2_PERMISSION_STATE_DENY
                })
            })),
            &raw mut token,
        );
        if let Ok(page) = page.cast::<ICoreWebView2_27>() {
            let _ = page.add_ScreenCaptureStarting(
                &ScreenCaptureStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        // `Capture`: the screen, never granted.
                        args.SetCancel(!grants(Ask::Capture))?;
                    }
                    Ok(())
                })),
                &raw mut token,
            );
        }
        if let Ok(receiver) =
            page.GetDevToolsProtocolEventReceiver(&HSTRING::from("Page.fileChooserOpened"))
        {
            let (tab, person_has_it) = (tab.clone(), person_has_it.clone());
            let _ = receiver.add_DevToolsProtocolEventReceived(
                &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, _| {
                    if !opens_picker(person_has_it()) {
                        refuse_picker(&tab);
                    }
                    Ok(())
                })),
                &raw mut token,
            );
        }
    }
    // The page domain's events reach the desk only once it is on.
    devtools(&page, "Page.enable", &json!({}), |_| {});
    intercept_files(&page, true);
    devtools(
        &page,
        "WebAuthn.enable",
        &json!({ "enableUI": false }),
        |_| {},
    );
    if let Some(top) = root(&view) {
        hook_dialogs();
        ROOTS.with(|roots| {
            roots
                .borrow_mut()
                .insert(view.as_raw() as usize, top.0 as isize)
        });
        WATCHED.with(|watched| {
            watched.borrow_mut().insert(
                top.0 as isize,
                Watched {
                    tab: Arc::downgrade(&tab),
                    person_has_it,
                },
            );
        });
    }
}

/// A page or popup is closing: its window is no longer watched.
pub(in super::super) fn forget(view: View) {
    if let Some(top) = ROOTS.with(|roots| roots.borrow_mut().remove(&(view.as_raw() as usize))) {
        WATCHED.with(|watched| watched.borrow_mut().remove(&top));
    }
}
