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
//! The first cut is deliberately plain: every registered phone hears every
//! reply and every card. Knowing that the desk is in front of the person,
//! or that the phone is already looking, is the next cut.

use crate::remote;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const EXPO_PUSH: &str = "https://exp.host/--/api/v2/push/send";
/// A notification body is a glance, not the message.
const BODY_CHARS: usize = 140;

pub struct Push {
    root: PathBuf,
    http: reqwest::Client,
}

impl Push {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            http: reqwest::Client::new(),
        }
    }

    /// One notification to every phone with a token, carrying which desk and
    /// which teammate it is about so a tap opens the right conversation.
    pub fn notify(&self, title: &str, body: &str, persona_id: &str) {
        let targets = remote::push_targets(&self.root);
        if targets.tokens.is_empty() {
            return;
        }
        let body = glance(body);
        let messages: Vec<Value> = targets
            .tokens
            .iter()
            .map(|token| {
                json!({
                    "to": token,
                    "title": title,
                    "body": body,
                    "sound": "default",
                    // Lets the phone's notification service turn the glance into a
                    // message from the teammate, with their picture.
                    "mutableContent": true,
                    "data": { "desktopId": targets.desktop_id, "personaId": persona_id },
                })
            })
            .collect();
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
    use super::glance;

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
}
