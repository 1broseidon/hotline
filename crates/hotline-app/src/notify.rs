//! Toasts on macOS, through the notification center Apple keeps rather than
//! the one it retired: this is the API that threads toasts by teammate and
//! hands a click back to the process. The judgement — when a toast is
//! earned — stays in the page (`ui/src/notify.ts`); this only posts what it
//! is told and reports a click as an event the window handles.
//!
//! The center will only speak for a bundled app. A `make dev` run is a bare
//! binary, so it posts nothing and says so, rather than raising the ObjC
//! exception the center throws at a process with no bundle.

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tauri::{AppHandle, Emitter};

/// The key under which a toast remembers whose it is, so a click can say.
const PERSONA_KEY: &str = "personaId";

/// The event the window hears when a toast is clicked; its payload is the
/// teammate's id.
const CLICK_EVENT: &str = "hotline://notification";

/// Whether this process is an app the center will post for.
fn bundled() -> bool {
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".app")
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = AppHandle]
    struct ToastDelegate;

    unsafe impl NSObjectProtocol for ToastDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for ToastDelegate {
        /// The page already decided the window was not being looked at;
        /// the toast shows either way, in the banner and in the list.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }

        /// A click raises the window and names the teammate; the page opens
        /// them.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let persona = response
                .notification()
                .request()
                .content()
                .userInfo()
                .objectForKey(&NSString::from_str(PERSONA_KEY))
                .and_then(|value| value.downcast::<NSString>().ok())
                .map(|value| value.to_string());
            let app = self.ivars();
            crate::show_main_window(app);
            if let Some(persona) = persona {
                let _ = app.emit(CLICK_EVENT, persona);
            }
            handler.call(());
        }
    }
);

/// Register for clicks. The center holds its delegate weakly, and this one
/// lives as long as the process, so it is handed over and never freed.
pub fn install(app: &AppHandle) {
    if !bundled() {
        return;
    }
    let delegate = ToastDelegate::alloc().set_ivars(app.clone());
    let delegate: Retained<ToastDelegate> = unsafe { msg_send![super(delegate), init] };
    UNUserNotificationCenter::currentNotificationCenter()
        .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
}

/// Post one toast for a teammate. The first ever asks the person's leave,
/// the way any Mac app does, and a refusal is a quiet no. Returns whether
/// there was a center to post to at all.
#[tauri::command]
pub fn notify(persona_id: String, title: String, body: String) -> bool {
    if !bundled() {
        return false;
    }
    let request = unsafe {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));
        content.setThreadIdentifier(&NSString::from_str(&persona_id));
        let key = NSString::from_str(PERSONA_KEY);
        let value = NSString::from_str(&persona_id);
        let info = NSDictionary::<NSString, NSString>::from_slices(&[&*key], &[&*value]);
        content.setUserInfo(&Retained::cast_unchecked(info));
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or(0);
        UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!("{persona_id}:{millis}")),
            &content,
            None,
        )
    };
    let post = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        if granted.as_bool() {
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, None);
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Badge
                | UNAuthorizationOptions::Sound,
            &post,
        );
    true
}
