//! The Linux side of a teammate page's asks (`asks.rs`). WebKitGTK puts
//! every permission a page asks for to `permission-request` and every file
//! input to `run-file-chooser`; wry handles neither, so WebKit's defaults
//! would answer, and its default for a file input is a GTK file chooser on
//! the person's screen. The desk answers both: every permission denied by
//! name, and the chooser cancelled unless the person has control.

use super::{Ask, Tab, grants, opens_picker, refuse_picker};
use std::sync::Arc;
use webkit2gtk::glib::prelude::*;
use webkit2gtk::{
    DeviceInfoPermissionRequest, FileChooserRequestExt, GeolocationPermissionRequest,
    NotificationPermissionRequest, PermissionRequest, PermissionRequestExt,
    UserMediaPermissionRequest, UserMediaPermissionRequestExt, WebView, WebViewExt,
};

/// What a WebKitGTK permission request asks for.
fn ask_of(request: &PermissionRequest) -> Ask {
    if let Some(media) = request.downcast_ref::<UserMediaPermissionRequest>() {
        match (media.is_for_audio_device(), media.is_for_video_device()) {
            (true, false) => Ask::Microphone,
            (false, true) => Ask::Camera,
            _ => Ask::Capture,
        }
    } else if request.is::<GeolocationPermissionRequest>() {
        Ask::Location
    } else if request.is::<NotificationPermissionRequest>() {
        Ask::Notifications
    } else if request.is::<DeviceInfoPermissionRequest>() {
        Ask::DeviceInfo
    } else {
        Ask::Other
    }
}

/// Answers the page's permission requests and file inputs.
/// `person_has_it` says whether the person has control of it now.
pub(in super::super) fn attach(
    view: WebView,
    tab: Arc<Tab>,
    person_has_it: impl Fn() -> bool + 'static,
) {
    view.connect_permission_request(|_, request| {
        if grants(ask_of(request)) {
            request.allow();
        } else {
            request.deny();
        }
        // Answered: WebKit's own default never runs.
        true
    });
    view.connect_run_file_chooser(move |_, request| {
        if opens_picker(person_has_it()) {
            // WebKit's own chooser, for the person at the pane.
            return false;
        }
        request.cancel();
        refuse_picker(&tab);
        true
    });
}
