//! What a turn writes as it goes: a tool's output cut to what the model is
//! shown, with the whole of it kept on disk and pictures only where the
//! provider takes them, and the model's words chunked onto the update channel
//! as they stream.

use super::Budget;
use crate::driver::{MessageKind, Update};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Whether a provider takes an image block inside a tool result. Anthropic
/// and OpenAI do; OpenRouter refuses the message outright ("does not support
/// images in tool results") and the turn dies, so everyone else is handed a
/// placeholder line and the picture goes to the tape only.
pub(super) fn images_to_model(provider: &str) -> bool {
    matches!(provider, "anthropic" | "openai" | "openai-codex")
}

/// The text the model is given for a tool result: the whole thing when it
/// fits, otherwise the head and tail with the rest written to
/// `{output_dir}/{call_id}.txt` and that path on the last line.
pub(super) fn hand_to_model(
    output_dir: &Path,
    call_id: &str,
    output: &str,
    budget: Budget,
) -> String {
    if output.len() <= budget.bytes() {
        return output.to_string();
    }
    let elided = elide(output, budget);
    match keep(output_dir, call_id, output) {
        Some(path) => format!("{elided}\nFull output: {}", path.display()),
        None => elided,
    }
}

/// Writes a whole result to `{output_dir}/{name}.txt`, where the workspace
/// tools can read it even when the teammate is confined to its folder.
pub(super) fn keep(output_dir: &Path, name: &str, text: &str) -> Option<PathBuf> {
    let path = output_dir.join(format!("{name}.txt"));
    std::fs::create_dir_all(output_dir)
        .and_then(|()| std::fs::write(&path, text))
        .ok()?;
    Some(path.canonicalize().unwrap_or(path))
}

/// The head and the tail of the output, with one line where the middle was.
///
/// A cut in the middle is the honest one: the head holds what the command
/// said it was doing and the tail holds how it ended, and a build log that
/// only kept its first quarter would hide the error the agent ran it for.
pub(super) fn elide(text: &str, budget: Budget) -> String {
    if text.len() <= budget.bytes() {
        return text.to_string();
    }
    let mut head = floor_boundary(text, budget.head);
    let mut tail = ceil_boundary(text, text.len() - budget.tail);
    // Whole lines read better, when a line ends near where the cut falls.
    if let Some(end) = text[..head].rfind('\n').filter(|end| *end >= head / 2) {
        head = end;
    }
    if let Some(start) = text[tail..]
        .find('\n')
        .filter(|start| *start < budget.tail / 2)
    {
        tail += start + 1;
    }
    let cut = tail - head;
    format!(
        "{}\n[… {cut} bytes elided …]\n{}",
        &text[..head],
        &text[tail..]
    )
}

fn floor_boundary(text: &str, at: usize) -> usize {
    (0..=at)
        .rev()
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(0)
}

fn ceil_boundary(text: &str, at: usize) -> usize {
    (at..=text.len())
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(text.len())
}

/// A command's output as a terminal would have left it. Colour, cursor and
/// title codes read as noise outside a terminal, and a progress bar redraws
/// its line with carriage returns: keep the last draw. The computer answers
/// its own commands the same way.
pub(super) fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            skip_escape(&mut chars);
        } else {
            out.push(c);
        }
    }
    out.split('\n')
        .map(|line| {
            line.rsplit('\r')
                .find(|part| !part.trim().is_empty())
                .unwrap_or("")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Consumes the rest of the escape sequence an ESC opened, and nothing after.
fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek() {
        // A control sequence: parameters, then a final byte from @ to ~.
        Some('[') => {
            chars.next();
            while let Some(&c) = chars.peek() {
                if !(' '..='~').contains(&c) {
                    break;
                }
                chars.next();
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        // A string, such as a window title, up to BEL or ESC \.
        Some(']' | 'P' | 'X' | '^' | '_') => {
            chars.next();
            while let Some(&c) = chars.peek() {
                if c == '\n' {
                    break;
                }
                chars.next();
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' {
                    chars.next_if_eq(&'\\');
                    break;
                }
            }
        }
        // Anything shorter, such as a character set: intermediates from
        // space to /, then one final byte.
        _ => {
            while let Some(&c) = chars.peek() {
                if !(' '..='~').contains(&c) {
                    break;
                }
                chars.next();
                if !(' '..='/').contains(&c) {
                    break;
                }
            }
        }
    }
}

/// A message being streamed: its id, whether it is speech or thought, and
/// what has arrived so far.
pub(super) struct OpenMessage {
    id: String,
    kind: MessageKind,
    text: String,
}

pub(super) async fn send(sender: &mpsc::Sender<Update>, update: Update) {
    let _ = sender.send(update).await;
}

/// Adds text to the message being streamed, opening one — and closing any
/// message of the other kind — when the agent changes voice.
pub(super) async fn chunk_into(
    sender: &mpsc::Sender<Update>,
    open: &mut Option<OpenMessage>,
    kind: MessageKind,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    if open.as_ref().is_none_or(|message| message.kind != kind) {
        flush(sender, open).await;
        *open = Some(OpenMessage {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            text: String::new(),
        });
    }
    let Some(message) = open.as_mut() else { return };
    message.text.push_str(text);
    send(
        sender,
        Update::Delta {
            kind,
            message_id: message.id.clone(),
            text: text.to_string(),
        },
    )
    .await;
}

/// Closes the streamed message: one durable [`Update::Message`] holding
/// everything its deltas carried.
pub(super) async fn flush(sender: &mpsc::Sender<Update>, open: &mut Option<OpenMessage>) {
    let Some(message) = open.take() else { return };
    if message.text.is_empty() {
        return;
    }
    send(
        sender,
        Update::Message {
            kind: message.kind,
            id: message.id,
            text: message.text,
        },
    )
    .await;
}
