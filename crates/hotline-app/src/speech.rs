//! On-device speech for a call or dictation: what the person says becomes
//! text on this machine and nowhere else. On macOS the Swift in `macos/speech` does the
//! hearing (SpeechAnalyzer, or SFSpeechRecognizer held to on-device
//! recognition; see docs/voice.md), compiled in by `build.rs`. Elsewhere
//! every command answers that speech is not built here.
//!
//! One utterance is current at a time, named by the session id the window
//! picks. Every text event carries the whole utterance so far, never a delta.
//! `speech_stop` closes the microphone and waits for the complete final text;
//! it fails rather than hand back a partial. Events for the current session
//! reach the main window as `speech-event`.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Whether this machine can hear the person without the network. Reading
/// it prompts for nothing, downloads nothing and opens no microphone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    available: bool,
    on_device: bool,
    /// `apple-analyzer` or `apple-recognizer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engine: Option<String>,
    locale: String,
    /// Only the analyzer has a model to install; `speech_permit` installs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_installed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl Capability {
    fn unavailable(locale: String, reason: &str) -> Self {
        Self {
            available: false,
            on_device: true,
            engine: None,
            locale,
            model_installed: None,
            reason: Some(reason.to_string()),
        }
    }
}

#[tauri::command]
pub async fn speech_capability() -> Capability {
    #[cfg(target_os = "macos")]
    {
        apple::capability().await
    }
    #[cfg(not(target_os = "macos"))]
    {
        Capability::unavailable(
            String::new(),
            "On-device speech is not built for this platform yet.",
        )
    }
}

/// Asks for speech recognition and the microphone, then installs the
/// analyzer's language model if it is missing, for at most a minute.
#[tauri::command]
pub async fn speech_permit() -> bool {
    #[cfg(target_os = "macos")]
    {
        apple::permit().await
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Starts a fresh utterance, cancelling any other. False when speech is
/// unavailable or not permitted; an error when the microphone or the
/// recognizer would not start.
#[tauri::command]
pub async fn speech_start(app: AppHandle, session_id: String) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        apple::start(app, session_id).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, session_id);
        Ok(false)
    }
}

/// The utterance's final text, or empty when `session_id` is not current.
/// An error when recognition failed or did not finish in five seconds.
#[tauri::command]
pub async fn speech_stop(session_id: String) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        apple::stop(session_id).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = session_id;
        Ok(String::new())
    }
}

/// Cancels `session_id` if it is current, silencing its later events. The
/// empty id cancels whatever is current, a model download included.
#[tauri::command]
pub async fn speech_cancel(session_id: String) {
    #[cfg(target_os = "macos")]
    apple::cancel(session_id).await;
    #[cfg(not(target_os = "macos"))]
    let _ = session_id;
}

#[cfg(target_os = "macos")]
mod apple {
    use super::Capability;
    use serde::{Deserialize, Serialize};
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::OnceLock;
    use tauri::{AppHandle, Emitter};
    use tokio::sync::oneshot;

    /// The name the main window hears events under.
    const EVENT: &str = "speech-event";

    /// What the Swift side emits, checked here so the window only ever
    /// hears these shapes.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(
        tag = "type",
        rename_all = "camelCase",
        rename_all_fields = "camelCase"
    )]
    pub(super) enum Event {
        Partial {
            session_id: String,
            text: String,
        },
        Final {
            session_id: String,
            text: String,
        },
        /// `level_db` is dBFS of one microphone buffer; `at` is epoch
        /// milliseconds.
        Level {
            session_id: String,
            level_db: f64,
            at: f64,
            unit: String,
        },
        Error {
            session_id: String,
            message: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            code: Option<String>,
        },
        /// `final`, `no-speech`, `cancelled` or `error`.
        Ended {
            session_id: String,
            reason: String,
        },
    }

    type Reply = extern "C" fn(*mut c_void, bool, *const c_char);
    type Events = extern "C" fn(*const c_char);

    // macos/speech/Bridge.swift. Each calls `done` exactly once, with the
    // context it was given; every string argument is copied before return.
    unsafe extern "C" {
        fn hotline_speech_capability(context: *mut c_void, done: Reply);
        fn hotline_speech_permit(context: *mut c_void, done: Reply);
        fn hotline_speech_start(
            session_id: *const c_char,
            events: Events,
            context: *mut c_void,
            done: Reply,
        );
        fn hotline_speech_stop(session_id: *const c_char, context: *mut c_void, done: Reply);
        fn hotline_speech_cancel(session_id: *const c_char, context: *mut c_void, done: Reply);
    }

    struct Answer {
        ok: bool,
        text: Option<String>,
    }

    extern "C" fn answered(context: *mut c_void, ok: bool, text: *const c_char) {
        // SAFETY: `context` is the sender `ask` boxed, and Swift replies to
        // it exactly once; `text` is null or a C string live for this call.
        let sender = unsafe { Box::from_raw(context.cast::<oneshot::Sender<Answer>>()) };
        let text = (!text.is_null()).then(|| {
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        });
        let _ = sender.send(Answer { ok, text });
    }

    async fn ask(call: impl FnOnce(*mut c_void, Reply)) -> Answer {
        let (sender, receiver) = oneshot::channel::<Answer>();
        call(Box::into_raw(Box::new(sender)).cast(), answered);
        receiver.await.unwrap_or(Answer {
            ok: false,
            text: None,
        })
    }

    /// Events are emitted on the main thread with no context of their own,
    /// so the app they go to is kept here, from the first start.
    static APP: OnceLock<AppHandle> = OnceLock::new();

    extern "C" fn heard(json: *const c_char) {
        // SAFETY: Swift passes a C string live for this call.
        let json = unsafe { CStr::from_ptr(json) }.to_bytes();
        match serde_json::from_slice::<Event>(json) {
            Ok(event) => {
                if let Some(app) = APP.get() {
                    let _ = app.emit_to("main", EVENT, event);
                }
            }
            Err(error) => eprintln!("[speech] unreadable event: {error}"),
        }
    }

    pub(super) async fn capability() -> Capability {
        let answer = ask(|context, done| unsafe { hotline_speech_capability(context, done) }).await;
        answer
            .text
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_else(|| {
                Capability::unavailable(String::new(), "On-device speech did not answer.")
            })
    }

    pub(super) async fn permit() -> bool {
        ask(|context, done| unsafe { hotline_speech_permit(context, done) })
            .await
            .ok
    }

    pub(super) async fn start(app: AppHandle, session_id: String) -> Result<bool, String> {
        let Ok(id) = CString::new(session_id) else {
            return Ok(false);
        };
        APP.get_or_init(|| app);
        let answer =
            ask(|context, done| unsafe { hotline_speech_start(id.as_ptr(), heard, context, done) })
                .await;
        match answer {
            Answer { ok: true, .. } => Ok(true),
            Answer {
                text: Some(message),
                ..
            } => Err(message),
            Answer { text: None, .. } => Ok(false),
        }
    }

    pub(super) async fn stop(session_id: String) -> Result<String, String> {
        let Ok(id) = CString::new(session_id) else {
            return Ok(String::new());
        };
        let answer =
            ask(|context, done| unsafe { hotline_speech_stop(id.as_ptr(), context, done) }).await;
        let text = answer.text.unwrap_or_default();
        if answer.ok { Ok(text) } else { Err(text) }
    }

    pub(super) async fn cancel(session_id: String) {
        let Ok(id) = CString::new(session_id) else {
            return;
        };
        ask(|context, done| unsafe { hotline_speech_cancel(id.as_ptr(), context, done) }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capability_is_camel_case_and_omits_what_it_does_not_know() {
        let unbuilt = Capability::unavailable(String::new(), "Not here.");
        assert_eq!(
            serde_json::to_value(&unbuilt).unwrap(),
            json!({ "available": false, "onDevice": true, "locale": "", "reason": "Not here." })
        );
        let swift = json!({
            "available": true, "onDevice": true, "engine": "apple-analyzer",
            "locale": "en_US", "modelInstalled": false,
        });
        let capability: Capability = serde_json::from_value(swift.clone()).unwrap();
        assert_eq!(serde_json::to_value(&capability).unwrap(), swift);
    }

    /// The shapes the Swift bridge emits pass through unchanged, and
    /// anything else is refused rather than forwarded.
    #[cfg(target_os = "macos")]
    #[test]
    fn events_keep_the_shapes_the_window_expects() {
        use super::apple::Event;
        for event in [
            json!({ "sessionId": "s1", "type": "partial", "text": "hello" }),
            json!({ "sessionId": "s1", "type": "final", "text": "hello there" }),
            json!({ "sessionId": "s1", "type": "level", "levelDb": -42.5, "at": 1_760_000_000_000.0, "unit": "dbfs" }),
            json!({ "sessionId": "s1", "type": "error", "message": "The microphone changed. Call again." }),
            json!({ "sessionId": "s1", "type": "error", "message": "No speech.", "code": "1110" }),
            json!({ "sessionId": "s1", "type": "ended", "reason": "no-speech" }),
        ] {
            let parsed: Event = serde_json::from_value(event.clone()).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(), event);
        }
        assert!(
            serde_json::from_value::<Event>(json!({ "type": "partial", "text": "x" })).is_err()
        );
        assert!(
            serde_json::from_value::<Event>(json!({ "sessionId": "s1", "type": "loud" })).is_err()
        );
    }

    /// Prints what this Mac reports without asking for anything:
    /// `cargo test -p hotline-app speech -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this machine's speech support"]
    fn capability_on_this_machine() {
        let capability = tauri::async_runtime::block_on(speech_capability());
        println!("{}", serde_json::to_string_pretty(&capability).unwrap());
    }
}
