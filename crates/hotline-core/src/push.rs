//! A notification to the phones paired with this desk.
//!
//! The phone registers the token its push service issued when it connects;
//! the desk keeps it on the phone's grant. When something on the desk is
//! worth a glance at a phone — a reply landed, a card needs the person —
//! one message goes to Expo's push service, which carries it to APNs or
//! FCM. Fire and forget: a notification that did not arrive is a phone that
//! will see the tape when it next connects, and the tape is never behind
//! the notification.
//!
//! Every push may be rewritten on the phone (`mutableContent`), so its
//! notification service can show a reply as the teammate's own message. A
//! card that waits on the person also names its kind as the category, whose
//! buttons the phone registered, and the request the answer names, so it
//! can be answered from the notification (docs/wire.md, "Pushes").
//!
//! Nothing goes while the person is at the desk: its window says so while
//! it has the focus and is being used ([`Push::looking`]). A phone that is
//! open already has every tape live, and its own notification handler
//! hides the banner for the conversation on its screen.

use crate::contract::PermissionOption;
use crate::remote;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const EXPO_PUSH: &str = "https://exp.host/--/api/v2/push/send";
/// A notification body is a glance, not the message.
const BODY_CHARS: usize = 140;
/// How long one sign of the person at the desk keeps the phone quiet. The
/// window repeats it while they keep using it, well inside this.
const AT_THE_DESK: Duration = Duration::from_secs(120);

pub struct Push {
    root: PathBuf,
    http: reqwest::Client,
    /// When the desk window last said the person was using it, in
    /// milliseconds; zero once it said they left.
    looked_at: AtomicU64,
    /// What would have gone to Expo, in a test.
    #[cfg(test)]
    pub(crate) sent: std::sync::Mutex<Vec<Value>>,
}

impl Push {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            http: reqwest::Client::new(),
            looked_at: AtomicU64::new(0),
            #[cfg(test)]
            sent: Default::default(),
        }
    }

    /// The desk window says the person is using it, focused and touched
    /// lately (`true`, repeated while they are), or has lost them: blurred,
    /// hidden or closed (`false`).
    pub fn looking(&self, looking: bool) {
        let at = if looking { now_ms() } else { 0 };
        self.looked_at.store(at, Ordering::Relaxed);
    }

    /// Whether the person was at the desk a moment before `now`.
    fn at_the_desk(&self, now: u64) -> bool {
        let at = self.looked_at.load(Ordering::Relaxed);
        at != 0 && now.saturating_sub(at) < AT_THE_DESK.as_millis() as u64
    }

    /// One notification to every phone with a token, carrying which desk and
    /// which teammate it is about so a tap opens the right conversation, and
    /// the card it is about when one waits on the person. None while the
    /// person is at the desk, where the same thing is already in front of
    /// them.
    pub fn notify(&self, title: &str, body: &str, persona_id: &str, waiting: Option<Waiting>) {
        if self.at_the_desk(now_ms()) {
            return;
        }
        let targets = remote::push_targets(&self.root);
        if targets.tokens.is_empty() {
            return;
        }
        let body = glance(body);
        let messages: Vec<Value> = targets
            .tokens
            .iter()
            .map(|token| {
                message(
                    token,
                    title,
                    &body,
                    &targets.desktop_id,
                    persona_id,
                    waiting.as_ref(),
                )
            })
            .collect();
        #[cfg(test)]
        {
            self.sent.lock().unwrap().extend(messages);
        }
        #[cfg(not(test))]
        self.send(messages);
    }

    #[cfg_attr(test, allow(dead_code))]
    fn send(&self, messages: Vec<Value>) {
        let http = self.http.clone();
        tokio::spawn(async move {
            match http.post(EXPO_PUSH).json(&messages).send().await {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => eprintln!("[push] Expo answered {}", response.status()),
                Err(error) => eprintln!("[push] could not reach Expo: {error}"),
            }
        });
    }
}

/// A card on the tape that waits on the person. Its kind is the push's
/// category, one set of buttons per kind on the phone, and its id is what
/// the phone's answer names: `session.answer_permission`'s `requestId`,
/// `human.answer`'s `actionId`, `secrets.passkey.answer`'s `askId`.
pub enum Waiting {
    /// The options go along, id and kind only, because an answer names an
    /// option and a button can only mean "allow" or "reject".
    Permission {
        request_id: String,
        options: Vec<PermissionOption>,
    },
    Human {
        action_id: String,
    },
    Passkey {
        ask_id: String,
    },
}

impl Waiting {
    /// The transcript event kind of the card, which the phone already knows.
    fn category(&self) -> &'static str {
        match self {
            Self::Permission { .. } => "permission",
            Self::Human { .. } => "human_action",
            Self::Passkey { .. } => "passkey_ask",
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Permission { request_id, .. } => request_id,
            Self::Human { action_id } => action_id,
            Self::Passkey { ask_id } => ask_id,
        }
    }
}

/// One Expo push message.
fn message(
    token: &str,
    title: &str,
    body: &str,
    desktop_id: &str,
    persona_id: &str,
    waiting: Option<&Waiting>,
) -> Value {
    let mut data = json!({ "desktopId": desktop_id, "personaId": persona_id });
    let mut message = json!({
        "to": token,
        "title": title,
        "body": body,
        "sound": "default",
        // Lets the phone's notification service turn the glance into a
        // message from the teammate, with their picture.
        "mutableContent": true,
    });
    if let Some(waiting) = waiting {
        message["categoryId"] = json!(waiting.category());
        data["requestId"] = json!(waiting.id());
        if let Waiting::Permission { options, .. } = waiting {
            data["options"] = options
                .iter()
                .map(|option| json!({ "optionId": option.option_id, "kind": option.kind }))
                .collect();
        }
    }
    message["data"] = data;
    message
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// The first line of a message, cut to a glance, markdown marks dropped.
pub fn glance(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let plain: String = line
        .trim_start_matches(['#', ' ', '>', '-', '*'])
        .replace(['*', '`', '_'], "");
    let mut out: String = plain.chars().take(BODY_CHARS).collect();
    if plain.chars().count() > BODY_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{AT_THE_DESK, Push, Waiting, glance, message, now_ms};
    use crate::contract::PermissionOption;
    use serde_json::json;

    #[test]
    fn the_phone_stays_quiet_while_the_person_is_at_the_desk() {
        let scratch = tempfile::tempdir().unwrap();
        let push = Push::new(scratch.path());
        let now = now_ms();
        assert!(!push.at_the_desk(now), "no window has said anything yet");
        push.looking(true);
        assert!(push.at_the_desk(now_ms()));
        let limit = AT_THE_DESK.as_millis() as u64;
        assert!(push.at_the_desk(now + limit - 1_000));
        assert!(
            !push.at_the_desk(now + limit + 1_000),
            "a window left focused and untouched lets the phone hear again"
        );
        push.looking(false);
        assert!(!push.at_the_desk(now_ms()), "a blur ends it at once");
    }

    #[test]
    fn a_phone_hears_nothing_the_desk_is_showing_the_person() {
        let scratch = tempfile::tempdir().unwrap();
        std::fs::write(
            scratch.path().join("remote.json"),
            json!({
                "desktopId": "desk-1",
                "host": "desk.local",
                "enabled": true,
                "grants": [{
                    "device": {"id": "phone-1", "name": "Phone", "pairedAt": 0},
                    "tokenHash": "hash",
                    "push": {"token": "ExponentPushToken[phone]", "platform": "ios"}
                }]
            })
            .to_string(),
        )
        .unwrap();
        let push = Push::new(scratch.path());
        push.looking(true);
        push.notify("Frankie", "done", "frankie", None);
        assert!(push.sent.lock().unwrap().is_empty());
        push.looking(false);
        push.notify("Frankie", "done", "frankie", None);
        let sent = push.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["to"], "ExponentPushToken[phone]");
    }

    #[test]
    fn a_glance_is_the_first_line_plain_and_short() {
        assert_eq!(
            glance("**done** — see `main.rs`\n\nmore"),
            "done — see main.rs"
        );
        assert_eq!(glance("\n\n# Harbour plan\nrest"), "Harbour plan");
        let long = "x".repeat(200);
        let cut = glance(&long);
        assert_eq!(cut.chars().count(), 141);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_reply_is_mutable_and_names_no_card() {
        let reply = message("tok", "Frankie", "done", "desk-1", "frankie", None);
        assert_eq!(reply["mutableContent"], true);
        assert!(reply.get("categoryId").is_none(), "{reply}");
        assert_eq!(
            reply["data"],
            json!({ "desktopId": "desk-1", "personaId": "frankie" })
        );
    }

    #[test]
    fn a_waiting_card_names_its_kind_and_the_request_the_answer_names() {
        let option = |id: &str, kind: &str| PermissionOption {
            option_id: id.to_string(),
            name: format!("Say {id}"),
            kind: Some(kind.to_string()),
        };
        let permission = Waiting::Permission {
            request_id: "req-7".to_string(),
            options: vec![option("yes", "allow_once"), option("no", "reject_once")],
        };
        let push = message(
            "tok",
            "Frankie needs you",
            "Run it?",
            "desk-1",
            "frankie",
            Some(&permission),
        );
        assert_eq!(push["mutableContent"], true);
        assert_eq!(push["categoryId"], "permission");
        assert_eq!(push["data"]["requestId"], "req-7");
        assert_eq!(push["data"]["personaId"], "frankie");
        assert_eq!(
            push["data"]["options"],
            json!([
                { "optionId": "yes", "kind": "allow_once" },
                { "optionId": "no", "kind": "reject_once" },
            ])
        );

        let human = Waiting::Human {
            action_id: "act-1".to_string(),
        };
        let push = message("tok", "t", "b", "desk-1", "frankie", Some(&human));
        assert_eq!(push["categoryId"], "human_action");
        assert_eq!(push["data"]["requestId"], "act-1");
        assert!(push["data"].get("options").is_none());

        let passkey = Waiting::Passkey {
            ask_id: "ask-2".to_string(),
        };
        let push = message("tok", "t", "b", "desk-1", "frankie", Some(&passkey));
        assert_eq!(push["categoryId"], "passkey_ask");
        assert_eq!(push["data"]["requestId"], "ask-2");
    }
}
