//! The window's title, and on macOS the traffic lights on the title
//! strip's centre line.
//!
//! The window is Overlay, so the lights sit on the page's own title strip
//! (ui/src/index.css, `.titlebar`), and they belong on its centre line.
//! tao's inset is no use for that: it moves them only from its view's
//! `drawRect:`, which the web view covering it never triggers, and AppKit
//! puts them back at its own default whenever the window's title changes,
//! which it does with every teammate opened. So the desk seats them itself:
//! when the window is shown, when its title changes, and when it is resized,
//! refocused, rescaled or rethemed, the moments AppKit lays its title bar
//! out again.

#[cfg(target_os = "macos")]
use objc2::msg_send;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_foundation::NSRect;

/// The title strip's height, `.titlebar` in index.css.
#[cfg(target_os = "macos")]
const STRIP: f64 = 40.0;
/// The close button's left edge, from the window's.
#[cfg(target_os = "macos")]
const LEFT: f64 = 12.0;

/// Seats the lights on the main thread.
#[cfg(target_os = "macos")]
pub fn seat(window: &tauri::WebviewWindow) {
    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window = ns_window as usize;
    let _ = window.run_on_main_thread(move || {
        // SAFETY: the window outlives the task (the main window is hidden,
        // never destroyed, while the app runs), and AppKit is only touched
        // here, on the main thread.
        unsafe { seat_on_main(ns_window as *mut AnyObject) }
    });
}

/// Names the window; on macOS the lights are seated again, since AppKit
/// has just put them back.
#[tauri::command]
pub fn window_title(window: tauri::WebviewWindow, title: String) {
    let _ = window.set_title(&title);
    #[cfg(target_os = "macos")]
    seat(&window);
}

/// The title bar's container is made the strip's height, the view holding
/// the buttons fills it, and each button is centred in it, `LEFT` in.
///
/// # Safety
///
/// `window` is a live NSWindow, or null, and this runs on the main thread.
#[cfg(target_os = "macos")]
unsafe fn seat_on_main(window: *mut AnyObject) {
    // SAFETY: the caller's promise. Every message below is one NSWindow or
    // NSView answers, a missing button or view is checked for null before
    // use, and nothing is retained past the call.
    unsafe {
        let Some(window) = window.as_ref() else {
            return;
        };
        // NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton.
        let close: *mut AnyObject = msg_send![window, standardWindowButton: 0usize];
        let miniaturize: *mut AnyObject = msg_send![window, standardWindowButton: 1usize];
        let zoom: *mut AnyObject = msg_send![window, standardWindowButton: 2usize];
        let (Some(close), Some(miniaturize), Some(zoom)) =
            (close.as_ref(), miniaturize.as_ref(), zoom.as_ref())
        else {
            return;
        };
        let holder: *mut AnyObject = msg_send![close, superview];
        let Some(holder) = holder.as_ref() else {
            return;
        };
        let container: *mut AnyObject = msg_send![holder, superview];
        let Some(container) = container.as_ref() else {
            return;
        };
        let window_frame: NSRect = msg_send![window, frame];
        let mut frame: NSRect = msg_send![container, frame];
        frame.size.height = STRIP;
        frame.origin.y = window_frame.size.height - STRIP;
        let () = msg_send![container, setFrame: frame];
        let mut inside: NSRect = msg_send![holder, frame];
        inside.origin.y = 0.0;
        inside.size.height = STRIP;
        let () = msg_send![holder, setFrame: inside];
        let close_frame: NSRect = msg_send![close, frame];
        let miniaturize_frame: NSRect = msg_send![miniaturize, frame];
        let spacing = miniaturize_frame.origin.x - close_frame.origin.x;
        for (index, button) in [close, miniaturize, zoom].into_iter().enumerate() {
            let mut rect: NSRect = msg_send![button, frame];
            rect.origin.x = LEFT + index as f64 * spacing;
            rect.origin.y = (STRIP - rect.size.height) / 2.0;
            let () = msg_send![button, setFrameOrigin: rect.origin];
        }
    }
}
