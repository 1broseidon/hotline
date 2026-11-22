//! What the `send` tool takes: whom an item is for, its title and words,
//! what it wants back, and what it answers or follows up on.

use crate::contract::InboxWants;
use serde_json::Value;

/// A `to` alias for every teammate on this desk but the sender.
pub(crate) fn whole_team(named: &str) -> bool {
    ["team", "everyone", "all"].contains(&named.trim().to_lowercase().as_str())
}

/// An item a teammate sends, as the `send` tool takes it.
pub(crate) struct Sending {
    /// Teammates by name or id, or a whole-team alias.
    pub to: Vec<String>,
    /// Absent, the message's first plain line is the title.
    pub title: Option<String>,
    pub message: String,
    /// Absent, an `answer`, or `nothing` for an answer to a question.
    pub wants: Option<InboxWants>,
    pub waiting: bool,
    /// A thread of the receiver's, as `kind:key`.
    pub about: Option<String>,
    /// An item in the sender's own inbox this one answers, or one it sent
    /// that this follows up on.
    pub reply_to: Option<String>,
    /// Paths of files to send with it, as the sender's read tools name them.
    pub files: Vec<String>,
}

/// Every argument `send` takes, as its schema lists them.
const FIELDS: [&str; 8] = [
    "to", "title", "message", "wants", "waiting", "about", "reply_to", "files",
];

impl Sending {
    /// The `send` tool's arguments. One it does not take is refused rather
    /// than dropped: a misnamed `wants` (`"work": "work"`) would otherwise
    /// send a hand-off as a question, which its receiver answers instead of
    /// doing.
    pub(crate) fn from_arguments(arguments: &Value) -> Result<Self, String> {
        if let Some(unknown) = arguments
            .as_object()
            .and_then(|fields| fields.keys().find(|key| !FIELDS.contains(&key.as_str())))
        {
            return Err(format!(
                "Nothing was sent: send has no `{unknown}`. It takes {}.",
                FIELDS.map(|field| format!("`{field}`")).join(", ")
            ));
        }
        let text = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let to = match arguments.get("to") {
            Some(Value::String(one)) => vec![one.clone()],
            Some(Value::Array(many)) => many
                .iter()
                .map(|named| named.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .ok_or("`to` is a list of teammates' names.")?,
            _ => {
                return Err(
                    "send needs `to`: a teammate's name, several, or \"team\" for everyone.".into(),
                );
            }
        };
        let wants = match text("wants") {
            None => None,
            Some(wants) => {
                Some(InboxWants::parse(&wants).ok_or("wants is answer, work or nothing.")?)
            }
        };
        let files = match arguments.get("files").filter(|files| !files.is_null()) {
            None => Vec::new(),
            Some(files) => files
                .as_array()
                .and_then(|files| {
                    files
                        .iter()
                        .map(|path| path.as_str().map(str::to_string))
                        .collect()
                })
                .ok_or("`files` is a list of paths your read tools can open.")?,
        };
        Ok(Self {
            to,
            title: text("title"),
            message: text("message").unwrap_or_default(),
            wants,
            waiting: arguments
                .get("waiting")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            about: text("about"),
            reply_to: text("reply_to"),
            files,
        })
    }
}
