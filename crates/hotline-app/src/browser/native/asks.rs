//! What a teammate's page may ask of the person's computer beyond its pages
//! (docs/security.md, "A teammate's browser"): the microphone, the camera,
//! the screen, where it is, notifications, its devices, and a file to upload.
//!
//! A page is never granted the first kind. Hotline holds the microphone for
//! its calls, and a page a teammate opened, in a window nobody sees, would
//! otherwise hear the room. A file picker opens only while the person has
//! control of the teammate's browser: it is theirs to answer, and a file
//! chosen there goes to whatever site the page is on. While the teammate
//! drives, a page's picker is refused at once, nothing is chosen, and the
//! call that made it hears so.
//!
//! wry grants every capture request and runs a modal open panel whenever a
//! page asks, so the desk answers both itself, on each platform's terms: a UI
//! delegate of its own in front of wry's on macOS (`asks/apple.rs`),
//! WebKitGTK's `permission-request` and `run-file-chooser` on Linux
//! (`asks/linux.rs`), and on Windows WebView2's `PermissionRequested`, the
//! DevTools protocol's file chooser interception and a watch on the
//! dialogs a teammate's window opens (`asks/win.rs`).

use super::{Loads, Tab};

#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "macos")]
pub(super) use apple::attach;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(super) use linux::attach;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub(super) use win::{attach, forget, intercept_files};

/// What a page asks the person's computer for, as either platform's request
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    target_os = "macos",
    allow(
        dead_code,
        reason = "WebKit on macOS asks the delegate for capture only"
    )
)]
#[cfg_attr(
    windows,
    allow(
        dead_code,
        reason = "WebView2 names no request for the names of a page's devices"
    )
)]
pub(super) enum Ask {
    Microphone,
    Camera,
    /// Microphone, camera or screen, when the request does not say which.
    Capture,
    Location,
    Notifications,
    /// The names of its cameras and microphones.
    DeviceInfo,
    /// Anything else a later WebKit asks: pointer lock, storage access,
    /// protected media, the clipboard.
    Other,
}

/// Whether a page's ask is granted: never, whatever it is. Each kind is named
/// so a new one is a decision, not a default.
pub(super) fn grants(ask: Ask) -> bool {
    match ask {
        Ask::Microphone
        | Ask::Camera
        | Ask::Capture
        | Ask::Location
        | Ask::Notifications
        | Ask::DeviceInfo
        | Ask::Other => false,
    }
}

/// Whether a page may open the file picker now: only while the person has
/// control of the teammate's browser.
pub(super) fn opens_picker(person_has_it: bool) -> bool {
    person_has_it
}

/// What the call that made a page ask for a file hears, while the teammate
/// drives.
pub(super) const PICKER_REFUSED: &str = "The page asked to choose a file to upload, and your browser never opens a file picker for you: nothing was chosen or uploaded. If the task needs a file uploaded there, ask the person to open your browser's pane, take control and choose it themselves.";

/// Notes a refused picker for the call waiting on the page.
fn refuse_picker(tab: &Tab) {
    tab.loads
        .send_modify(|loads: &mut Loads| loads.picker_refused = true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_and_every_other_ask_is_always_denied() {
        for ask in [
            Ask::Microphone,
            Ask::Camera,
            Ask::Capture,
            Ask::Location,
            Ask::Notifications,
            Ask::DeviceInfo,
            Ask::Other,
        ] {
            assert!(!grants(ask), "{ask:?}");
        }
    }

    #[test]
    fn the_file_picker_opens_only_while_the_person_has_control() {
        assert!(opens_picker(true));
        assert!(!opens_picker(false));
    }
}
