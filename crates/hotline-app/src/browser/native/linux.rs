//! WebKitGTK itself, on the GTK main thread `with_webview` runs on: the
//! Linux side of the desk browser's platform seam (`browser.rs`).
//!
//! GTK packs a Tauri window's web views in one box, so the page is taken
//! out of it into an overlay of the desk's: the page, the shield over it
//! and the teammate's hands (`linux/hands.rs`). While nobody watches, that
//! overlay lives in an offscreen window: mapped as far as WebKit can tell,
//! so the page lays out at its size, draws, takes input and is never
//! throttled as a hidden one, but on no screen and in no window list. The
//! pane moves it into the browser's own window, and closing the pane moves
//! it back.

use super::Step;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::oneshot;
use webkit2gtk::gio::Cancellable;
use webkit2gtk::glib::{self, prelude::*, translate::*};
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebView, WebViewExt};

use gtk::gdk;
use gtk::prelude::*;
use javascriptcore::ValueExt;

/// The script world the desk's scripts run in: the page's own scripts
/// never see its variables, and it never sees theirs. A named world lives
/// only while something holds it, and its variables (a read's refs) with
/// it; the GTK API holds none for a script run, so [`keep_world`] does.
const WORLD: &str = "hotline-desk-browser";

/// Holds the desk's world for the page's life: a user script in it, which
/// does nothing, keeps WebKit's world of that name, so every script the desk
/// runs meets the variables the last one left. Without it each run gets a
/// new world, and a ref a read gave is gone by the act that names it.
fn keep_world(view: &WebView) {
    use webkit2gtk::{
        UserContentInjectedFrames, UserContentManagerExt, UserScript, UserScriptInjectionTime,
    };
    if let Some(manager) = view.user_content_manager() {
        manager.add_script(&UserScript::for_world(
            "",
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::End,
            WORLD,
            &[],
            &[],
        ));
    }
}

type Reply<T> = Arc<Mutex<Option<oneshot::Sender<Result<T, String>>>>>;

fn send<T>(reply: &Reply<T>, answer: Result<T, String>) {
    if let Some(sender) = reply.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = sender.send(answer);
    }
}

/// The teammate's web data directory: its cookies, storage and cache, apart
/// from Hotline's window and every other teammate's, under Hotline's own
/// data directory and named by the same hash macOS names its store by.
pub(super) fn store_dir(persona_id: &str) -> PathBuf {
    let id: String = hotline_core::browser::store_id(persona_id)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    hotline_core::paths::data_root()
        .join("desk-browser")
        .join(id)
}

/// Where a page sits: the overlay it is in, the window's box it goes in
/// while the pane shows, the offscreen window it waits in otherwise, and
/// the shield.
struct Seat {
    overlay: gtk::Overlay,
    home: gtk::Box,
    parked: gtk::OffscreenWindow,
    shield: gtk::EventBox,
}

thread_local! {
    static SEATS: RefCell<HashMap<usize, Seat>> = RefCell::default();
}

fn key(view: &WebView) -> usize {
    glib::ObjectType::as_ptr(view) as usize
}

/// The overlay a page is in, for the hands to go over it.
pub(super) fn overlay_of(view: &WebView) -> Option<gtk::Overlay> {
    SEATS.with(|seats| {
        seats
            .borrow()
            .get(&key(view))
            .map(|seat| seat.overlay.clone())
    })
}

/// Takes the page out of the window's box into the desk's overlay, and
/// parks it offscreen unless the window shows from the start (`show`).
pub(super) fn seat(view: WebView, show: bool) {
    let Some(home) = view
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    keep_world(&view);
    home.remove(&view);
    let overlay = gtk::Overlay::new();
    overlay.add(&view);
    // A watcher's clicks and scrolls land here and go no further. It has
    // no window of its own to draw: it only takes input.
    let shield = gtk::EventBox::new();
    shield.set_visible_window(false);
    shield.set_above_child(true);
    shield.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK
            | gdk::EventMask::TOUCH_MASK,
    );
    shield.connect_button_press_event(|_, _| glib::Propagation::Stop);
    shield.connect_button_release_event(|_, _| glib::Propagation::Stop);
    shield.connect_scroll_event(|_, _| glib::Propagation::Stop);
    shield.connect_touch_event(|_, _| glib::Propagation::Stop);
    shield.set_no_show_all(true);
    overlay.add_overlay(&shield);
    let parked = gtk::OffscreenWindow::new();
    parked.set_default_size(super::WIDTH as i32, super::HEIGHT as i32);
    if show {
        home.pack_start(&overlay, true, true, 0);
        overlay.show_all();
    } else {
        parked.add(&overlay);
        parked.show_all();
    }
    SEATS.with(|seats| {
        seats.borrow_mut().insert(
            key(&view),
            Seat {
                overlay,
                home,
                parked,
                shield,
            },
        );
    });
}

/// Hotline's bar above the page: its own height, first in the box, where
/// GTK would otherwise share the window's height out between the two.
pub(super) fn seat_bar(view: WebView, height: f64) {
    let Some(home) = view
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    home.set_child_packing(&view, false, true, 0, gtk::PackType::Start);
    view.set_size_request(-1, height as i32);
    home.reorder_child(&view, 0);
}

fn toplevel(seat: &Seat) -> Option<gtk::Window> {
    seat.home
        .toplevel()
        .and_then(|top| top.downcast::<gtk::Window>().ok())
}

/// Shows the pane: the page moved into its window, the window in front,
/// and the page shielded until the person takes it over.
pub(super) fn reveal(view: WebView) {
    SEATS.with(|seats| {
        let seats = seats.borrow();
        let Some(seat) = seats.get(&key(&view)) else {
            return;
        };
        if seat.overlay.parent().as_ref() == Some(seat.parked.upcast_ref()) {
            seat.parked.remove(&seat.overlay);
            seat.home.pack_start(&seat.overlay, true, true, 0);
        }
        if let Some(window) = toplevel(seat) {
            window.show_all();
            window.present();
        }
    });
    shield(view, true);
}

/// Hides the pane again: the page goes back offscreen at the size it had,
/// and the teammate's browser goes on there.
pub(super) fn conceal(view: WebView) {
    SEATS.with(|seats| {
        let seats = seats.borrow();
        let Some(seat) = seats.get(&key(&view)) else {
            return;
        };
        if seat.overlay.parent().as_ref() == Some(seat.home.upcast_ref()) {
            let size = seat.overlay.allocation();
            seat.parked
                .set_default_size(size.width().max(1), size.height().max(1));
            seat.home.remove(&seat.overlay);
            seat.parked.add(&seat.overlay);
            seat.parked.show_all();
        }
        if let Some(window) = toplevel(seat) {
            window.hide();
        }
    });
}

/// The shield over the page while a watcher sees it, so watching never
/// drives the page; with it on, the page loses the keyboard. The desk's own
/// input goes to the page directly and is not stopped by it.
pub(super) fn shield(view: WebView, on: bool) {
    SEATS.with(|seats| {
        let seats = seats.borrow();
        let Some(seat) = seats.get(&key(&view)) else {
            return;
        };
        if on {
            seat.shield.show();
            if let Some(window) = toplevel(seat) {
                window.set_focus(None::<&gtk::Widget>);
            }
        } else {
            seat.shield.hide();
        }
    });
}

/// Lets go of a page that is closing: it goes back into its window's box,
/// so it goes with the window, and the offscreen window it waited in is
/// destroyed. Left parked, the page would outlive its window there.
pub(super) fn forget(view: WebView) {
    let Some(seat) = SEATS.with(|seats| seats.borrow_mut().remove(&key(&view))) else {
        return;
    };
    if seat.overlay.parent().as_ref() == Some(seat.parked.upcast_ref()) {
        seat.parked.remove(&seat.overlay);
        seat.home.pack_start(&seat.overlay, true, true, 0);
    }
    // SAFETY: the desk's own offscreen window, holding nothing now and
    // referenced by nothing but this seat, which is gone.
    unsafe { seat.parked.destroy() };
}

/// The overlay keeps the shield over the page whatever the window's size.
pub(super) fn shield_follow(_view: WebView) {}

/// Sizes the page the teammate asked for: the page's overlay keeps that
/// size, centred at the top where the pane shows it, and the offscreen
/// window it is parked in while hidden takes it too, so a picture is that
/// size. `None` lets it fill again.
pub(super) fn fit(view: WebView, size: Option<(u32, u32)>) {
    SEATS.with(|seats| {
        let seats = seats.borrow();
        let Some(seat) = seats.get(&key(&view)) else {
            return;
        };
        let (width, height) = size.map_or(
            (super::WIDTH as i32, super::HEIGHT as i32),
            |(width, height)| {
                (
                    i32::try_from(width).unwrap_or(i32::MAX),
                    i32::try_from(height).unwrap_or(i32::MAX),
                )
            },
        );
        if size.is_some() {
            seat.overlay.set_size_request(width, height);
            seat.overlay.set_halign(gtk::Align::Center);
            seat.overlay.set_valign(gtk::Align::Start);
        } else {
            seat.overlay.set_size_request(-1, -1);
            seat.overlay.set_halign(gtk::Align::Fill);
            seat.overlay.set_valign(gtk::Align::Fill);
        }
        seat.parked.resize(width, height);
    });
}

/// Runs `body` as an async function in the desk's script world and
/// answers its string.
pub(super) fn call(view: WebView, body: &str, reply: Reply<String>) {
    view.call_async_javascript_function(
        body,
        None,
        Some(WORLD),
        None,
        None::<&Cancellable>,
        move |result| {
            let answer = match result {
                Ok(value) if value.is_string() => Ok(value.to_str().to_string()),
                Ok(_) => Err("The page answered nothing.".to_string()),
                Err(error) => Err(error.message().to_string()),
            };
            send(&reply, answer);
        },
    );
}

pub(super) fn step(view: WebView, step: Step) {
    match step {
        Step::Back => view.go_back(),
        Step::Forward => view.go_forward(),
        Step::Reload => view.reload(),
    }
}

/// What the desk sends the web view as the person's own input: a click or
/// a hover at a point in the page's CSS pixels, with the page's viewport
/// height, or one key.
pub(super) enum Input {
    Click(f64, f64, f64),
    Hover(f64, f64, f64),
    Key(String),
}

/// A named key as GDK spells it: its key value and whether Shift is held.
fn key_of(key: &str) -> Option<(u32, bool)> {
    use gdk::ffi::*;
    let named = match key {
        "Enter" => GDK_KEY_Return,
        "Tab" => GDK_KEY_Tab,
        "Shift+Tab" => return Some((GDK_KEY_ISO_Left_Tab as u32, true)),
        "Escape" => GDK_KEY_Escape,
        "Backspace" => GDK_KEY_BackSpace,
        "Delete" => GDK_KEY_Delete,
        "Space" => GDK_KEY_space,
        "ArrowUp" => GDK_KEY_Up,
        "ArrowDown" => GDK_KEY_Down,
        "ArrowLeft" => GDK_KEY_Left,
        "ArrowRight" => GDK_KEY_Right,
        "Home" => GDK_KEY_Home,
        "End" => GDK_KEY_End,
        "PageUp" => GDK_KEY_Page_Up,
        "PageDown" => GDK_KEY_Page_Down,
        other => {
            let mut chars = other.chars();
            let (Some(only), None) = (chars.next(), chars.next()) else {
                return None;
            };
            // SAFETY: a pure lookup.
            return Some((unsafe { gdk_unicode_to_keyval(only as u32) }, false));
        }
    };
    Some((named as u32, false))
}

/// A millisecond clock for the events' times, as GDK's own are.
fn now() -> u32 {
    (glib::monotonic_time() / 1000) as u32
}

/// Sends `input` to the page's web view as GDK events of the person's own
/// pointer and keyboard. A click WebKit did not take is made by the page's
/// own script instead (`CLICK`), as on macOS.
pub(super) fn input(view: WebView, input: &Input) {
    let Some(window) = view.window() else {
        return;
    };
    let Some(seat) = window.display().default_seat() else {
        return;
    };
    // A web view without a window of its own counts from its parent's.
    let (dx, dy) = if view.has_window() {
        (0.0, 0.0)
    } else {
        let at = view.allocation();
        (f64::from(at.x()), f64::from(at.y()))
    };
    let (ox, oy) = {
        let (_, x, y) = window.origin();
        (f64::from(x), f64::from(y))
    };
    // The page's viewport is the view's whole height here, as WebKitGTK
    // draws no bar of its own inside it; anything taller is above it.
    let below =
        |y: f64, viewport: f64| y + (f64::from(view.allocated_height()) - viewport).max(0.0);
    let pointer = seat.pointer();
    let mouse = |kind: gdk::EventType, x: f64, y: f64, state: gdk::ffi::GdkModifierType| {
        let Some(pointer) = &pointer else {
            return;
        };
        // SAFETY: a fresh event of this type, its fields set as GDK reads
        // them; the window is referenced for the event, which frees it.
        unsafe {
            let event = gdk::ffi::gdk_event_new(kind.into_glib());
            let (x, y) = (x + dx, y + dy);
            if kind == gdk::EventType::MotionNotify {
                let motion = &mut *(event as *mut gdk::ffi::GdkEventMotion);
                motion.window = window.to_glib_full();
                motion.send_event = 1;
                motion.time = now();
                motion.x = x;
                motion.y = y;
                motion.x_root = x + ox;
                motion.y_root = y + oy;
                motion.state = state;
            } else {
                let button = &mut *(event as *mut gdk::ffi::GdkEventButton);
                button.window = window.to_glib_full();
                button.send_event = 1;
                button.time = now();
                button.x = x;
                button.y = y;
                button.x_root = x + ox;
                button.y_root = y + oy;
                button.state = state;
                button.button = 1;
            }
            gdk::ffi::gdk_event_set_device(event, pointer.to_glib_none().0);
            gdk::ffi::gdk_event_set_source_device(event, pointer.to_glib_none().0);
            let event: gdk::Event = from_glib_full(event);
            view.event(&event);
        }
    };
    match input {
        Input::Click(x, y, viewport) => {
            let y = below(*y, *viewport);
            mouse(gdk::EventType::MotionNotify, *x, y, 0);
            mouse(gdk::EventType::ButtonPress, *x, y, 0);
            mouse(
                gdk::EventType::ButtonRelease,
                *x,
                y,
                gdk::ffi::GDK_BUTTON1_MASK,
            );
        }
        Input::Hover(x, y, viewport) => {
            mouse(gdk::EventType::MotionNotify, *x, below(*y, *viewport), 0);
        }
        Input::Key(key) => {
            let Some((keyval, shift)) = key_of(key) else {
                return;
            };
            let keyboard = seat.keyboard();
            let code = gdk::Keymap::for_display(&window.display())
                .and_then(|keymap| keymap.entries_for_keyval(keyval).first().copied())
                .map_or(0, |entry| entry.keycode() as u16);
            view.grab_focus();
            for kind in [gdk::EventType::KeyPress, gdk::EventType::KeyRelease] {
                // SAFETY: as for the mouse above; the string is left null.
                unsafe {
                    let event = gdk::ffi::gdk_event_new(kind.into_glib());
                    let key = &mut *(event as *mut gdk::ffi::GdkEventKey);
                    key.window = window.to_glib_full();
                    key.send_event = 1;
                    key.time = now();
                    key.keyval = keyval;
                    key.hardware_keycode = code;
                    key.state = if shift { gdk::ffi::GDK_SHIFT_MASK } else { 0 };
                    if let Some(keyboard) = &keyboard {
                        gdk::ffi::gdk_event_set_device(event, keyboard.to_glib_none().0);
                        gdk::ffi::gdk_event_set_source_device(event, keyboard.to_glib_none().0);
                    }
                    let event: gdk::Event = from_glib_full(event);
                    view.event(&event);
                }
            }
        }
    }
}

/// The visible page as a PNG.
pub(super) fn snapshot(view: WebView, reply: Reply<Vec<u8>>) {
    view.snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&Cancellable>,
        move |drawn| {
            let answer = drawn
                .map_err(|error| error.message().to_string())
                .and_then(png);
            send(&reply, answer);
        },
    );
}

fn png(surface: cairo::Surface) -> Result<Vec<u8>, String> {
    let image = cairo::ImageSurface::try_from(surface)
        .map_err(|_| "The page's picture could not be read.".to_string())?;
    let mut png = Vec::new();
    image
        .write_to_png(&mut png)
        .map_err(|_| "The page's picture could not be made a PNG.".to_string())?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_the_tool_names_has_a_gdk_key() {
        for key in hotline_core::browser::KEYS {
            assert!(key_of(key).is_some(), "{key}");
        }
        assert_eq!(
            key_of("Shift+Tab"),
            Some((gdk::ffi::GDK_KEY_ISO_Left_Tab as u32, true))
        );
        assert_eq!(key_of("a"), Some((u32::from(b'a'), false)));
        assert_eq!(key_of("Hyper+Q"), None);
    }

    #[test]
    fn a_teammates_store_is_its_own_directory_under_hotlines() {
        let quill = store_dir("quill");
        assert!(quill.starts_with(hotline_core::paths::data_root().join("desk-browser")));
        assert_ne!(quill, store_dir("kit"));
        assert_eq!(quill, store_dir("quill"));
    }
}
