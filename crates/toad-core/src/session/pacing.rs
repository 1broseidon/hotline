//! How an agent's reply is shown: chat, or a note.
//!
//! A model produces one string. The window cannot: a dissertation in a lozenge
//! is unreadable, and a one-line text that should have been a report has no
//! room. So the string is classified here, by a pure function of the text,
//! never by the model's mood. Chat is one to four bubbles. Anything that does
//! not fit is a note: one event with a title, which the window draws as a
//! card the person opens.
//!
//! The rule lives in the funnel ([`super::event_of`]) so both kinds of agent
//! and a peer thread get it for free. History puts the pieces back together
//! ([`spoken`]) because the model said one thing.

/// A chat reply is a handful of bubbles, not a wall: iMessage is usually one
/// or two, and a longer train of thought still fits in four.
pub const MAX_CHAT_UNITS: usize = 4;

/// Past this, the reply is a document. Twelve hundred characters of prose in
/// bubbles cannot be scanned as chat, so it is a note the person opens instead.
pub const CHAT_CHARS: usize = 1_200;

/// A stub shorter than this is a lead-in ("Here's the fix:") or a leftover
/// ("ok."), not a bubble of its own.
pub const SHORT_UNIT_CHARS: usize = 60;

/// A note's title is a card label, not a paragraph: eighty characters is a
/// long headline and still fits on one line.
pub const TITLE_CHARS: usize = 80;

/// What Toad tells an agent about the room it is speaking in.
///
/// An agent's default register is the terminal: a headed report, bullets under
/// each heading, a summary of what it is about to do. That is the right shape
/// for a scrollback and the wrong shape for a conversation, and no agent can
/// know which one it is in unless it is told. This is a fact about Toad rather
/// than about the teammate, which is why it does not live in the teammate's
/// `AGENTS.md`: it is true of every teammate, and it has to arrive even when
/// the working directory is a real repository whose `AGENTS.md` Toad leaves
/// alone.
///
/// It asks for one acknowledgement before the work rather than banning one.
/// The typing indicator and "on it" do not say the same thing: dots mean
/// something is happening, "on it" means you were heard. And it says out loud
/// that brevity is about ceremony and not substance, because an agent told to
/// be short will otherwise shorten the explanation somebody asked for rather
/// than the packaging around it.
pub(crate) const HOUSE_STYLE: &str = "You are speaking in Toad, a desktop chat app. Your reply is shown as messages in a conversation, the way a person texts — not as a document.

There is a rhythm to that, and it matters more than anything else here. Before your first tool call, write one short line: \"on it\", \"let me check\", \"sure, one sec\". Then work in silence. Then say what came of it. The whole exchange should read like two colleagues — \"how many rust files are under crates?\" / \"let me check\" / \"41, all .rs\" — and never like one long report delivered after a minute of nothing. That opening line is not optional and it is not a summary of your plan; it is the word you would say to someone standing in your doorway.

After it, stay quiet until you have the answer. The person cannot see your tool calls, and a running commentary of what you are opening and what you found next is exactly what this app keeps off the screen.

Then say what came of it and stop. No recap of the steps, no list of the files you touched, no summary of what you just did. If it worked, saying so is enough; if it didn't, say what stopped you.

Write it the way you would text it. Lead with the answer. Plain sentences, no preamble, no restating the question, no sign-off.

Toad shows your reply as chat: each paragraph is its own message, four at most. A reply that needs more than that — more than four paragraphs, more than about 1,200 characters, a table, or anything you open with a `# Title` line — is shown whole as a note the person opens, not as chat. Chat is for talking; a note is for a report, a plan, or an explanation that needs the room. Choose on purpose: an answer that fits in two messages should not arrive as a note, and a note should not be squeezed into bubbles.

Being brief is about ceremony, not substance. A real question deserves a real answer — if someone asks how something works or why it broke, explain it properly. What gets cut is the packaging, never the thinking.

Formatting is available when the content is genuinely that shape — a fenced block for code, a list when there really are several items, a table when there are rows and columns, backticks for a filename or flag, bold for a term that carries weight. Reach for none of this to organise three sentences.";

/// How a reply will be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Paced {
    Chat(Vec<String>),
    Note { title: String, body: String },
}

/// Classify a reply as chat (one to four bubbles) or a note.
///
/// Note when any of these holds: the first non-empty line is a markdown
/// heading; after merging there are more than [`MAX_CHAT_UNITS`] units; the
/// original text is longer than [`CHAT_CHARS`]; any line outside a fence
/// starts with `|`.
pub fn paced(text: &str) -> Paced {
    let heading = heading_title(text);
    let units = merge(units(text));
    if heading.is_some()
        || units.len() > MAX_CHAT_UNITS
        || text.chars().count() > CHAT_CHARS
        || has_table(text)
    {
        let title = match heading {
            Some(title) => title.to_string(),
            None => clip_title(first_line(text)),
        };
        let body = match heading {
            Some(_) => strip_heading_line(text),
            None => text.trim().to_string(),
        };
        return Paced::Note { title, body };
    }
    Paced::Chat(units)
}

/// What the model said, given how the tape stored it.
///
/// The model said one thing; the tape shows a note as a title and a body.
/// History puts the heading back so the model sees the same words it produced.
pub(crate) fn spoken(title: Option<&str>, body: &str) -> String {
    match title {
        Some(title) if !title.is_empty() => format!("# {title}\n\n{body}"),
        _ => body.to_string(),
    }
}

fn units(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < lines.len() {
        if let Some(open) = open_fence(lines[i]) {
            let start = i;
            i += 1;
            while i < lines.len() && !is_close_fence(lines[i], open.0, open.1) {
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            push_unit(&mut out, &lines[start..i]);
            continue;
        }
        if is_list_line(lines[i]) {
            let start = i;
            i += 1;
            while i < lines.len() {
                if is_list_line(lines[i]) {
                    i += 1;
                    continue;
                }
                if lines[i].trim().is_empty() {
                    let mut j = i + 1;
                    while j < lines.len() && lines[j].trim().is_empty() {
                        j += 1;
                    }
                    if j < lines.len() && is_list_line(lines[j]) {
                        i = j;
                        continue;
                    }
                    break;
                }
                break;
            }
            push_unit(&mut out, &lines[start..i]);
            continue;
        }
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < lines.len()
            && !lines[i].trim().is_empty()
            && open_fence(lines[i]).is_none()
            && !is_list_line(lines[i])
        {
            i += 1;
        }
        push_unit(&mut out, &lines[start..i]);
    }
    out
}

fn push_unit(out: &mut Vec<String>, lines: &[&str]) {
    let unit = lines.join("\n").trim().to_string();
    if !unit.is_empty() {
        out.push(unit);
    }
}

/// Join a lead-in to what it introduces, left to right, once.
///
/// A unit that ends with `:` is introducing the next one. A stub shorter than
/// [`SHORT_UNIT_CHARS`] is not a bubble of its own unless it is the whole
/// reply: it joins the next unit, or the previous when it is last.
fn merge(units: Vec<String>) -> Vec<String> {
    if units.len() <= 1 {
        return units;
    }
    let mut out = Vec::new();
    let mut i = 0;
    let n = units.len();
    while i < n {
        let unit = &units[i];
        let short = unit.chars().count() < SHORT_UNIT_CHARS;
        let introduces = unit.ends_with(':');
        if i + 1 < n && (introduces || short) {
            out.push(format!("{unit}\n\n{}", units[i + 1]));
            i += 2;
            continue;
        }
        if i + 1 == n && short {
            if let Some(prev) = out.last_mut() {
                *prev = format!("{prev}\n\n{unit}");
            } else {
                out.push(unit.clone());
            }
            break;
        }
        out.push(unit.clone());
        i += 1;
    }
    out
}

fn open_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let marker = if trimmed.starts_with('`') {
        '`'
    } else if trimmed.starts_with('~') {
        '~'
    } else {
        return None;
    };
    let count = trimmed.chars().take_while(|&c| c == marker).count();
    (count >= 3).then_some((marker, count))
}

fn is_close_fence(line: &str, marker: char, count: usize) -> bool {
    let trimmed = line.trim();
    let n = trimmed.chars().take_while(|&c| c == marker).count();
    n >= count && trimmed.chars().skip(n).all(char::is_whitespace)
}

fn is_list_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || is_ordered_item(trimmed)
}

fn is_ordered_item(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

fn has_table(text: &str) -> bool {
    let mut fence = None;
    for line in text.lines() {
        if let Some((marker, count)) = fence {
            if is_close_fence(line, marker, count) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = open_fence(line) {
            fence = Some(open);
            continue;
        }
        if line.trim_start().starts_with('|') {
            return true;
        }
    }
    false
}

fn heading_title(text: &str) -> Option<&str> {
    first_raw_line(text).and_then(heading_of)
}

fn heading_of(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    trimmed.get(hashes..)?.strip_prefix(' ').map(str::trim)
}

fn first_raw_line(text: &str) -> Option<&str> {
    text.lines().find(|line| !line.trim().is_empty())
}

fn first_line(text: &str) -> &str {
    first_raw_line(text).map(str::trim).unwrap_or("")
}

fn strip_heading_line(text: &str) -> String {
    let mut seen = false;
    let mut rest = String::new();
    for line in text.lines() {
        if !seen {
            if line.trim().is_empty() {
                continue;
            }
            if heading_of(line).is_some() {
                seen = true;
                continue;
            }
        }
        if !rest.is_empty() {
            rest.push('\n');
        }
        rest.push_str(line);
    }
    rest.trim().to_string()
}

fn clip_title(line: &str) -> String {
    if line.chars().count() <= TITLE_CHARS {
        return line.to_string();
    }
    let mut end = 0;
    let mut at_word = 0;
    for (i, ch) in line.char_indices() {
        if line[..i].chars().count() >= TITLE_CHARS {
            break;
        }
        end = i + ch.len_utf8();
        if ch.is_whitespace() {
            at_word = i;
        }
    }
    let cut = if at_word > 0 { at_word } else { end };
    format!("{}…", line[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bubble(n: u32) -> String {
        format!("Paragraph {n} is long enough to stand as its own bubble in the chat.")
    }

    fn chat(units: &[&str]) -> Paced {
        Paced::Chat(units.iter().map(|unit| unit.to_string()).collect())
    }

    #[test]
    fn one_line_is_one_bubble() {
        assert_eq!(paced("hello"), chat(&["hello"]));
    }

    #[test]
    fn two_paragraphs_are_two_bubbles() {
        let a = bubble(1);
        let b = bubble(2);
        assert_eq!(paced(&format!("{a}\n\n{b}")), chat(&[&a, &b]));
    }

    #[test]
    fn on_it_alone_is_one_bubble() {
        assert_eq!(paced("on it"), chat(&["on it"]));
    }

    #[test]
    fn intro_colon_then_fence_stays_together() {
        let text = "Here's the fix:\n\n```\nfn x() {}\n```";
        match paced(text) {
            Paced::Chat(units) => {
                assert_eq!(units.len(), 1, "{units:?}");
                assert!(units[0].starts_with("Here's the fix:"));
                assert!(units[0].contains("fn x() {}"));
            }
            other => panic!("expected chat, got {other:?}"),
        }
    }

    #[test]
    fn a_fence_with_blank_lines_is_one_unit() {
        let text = "```\nhello\n\nworld\n```";
        assert_eq!(paced(text), chat(&[text]));
    }

    #[test]
    fn five_paragraphs_are_a_note() {
        let paras: Vec<String> = (1..=5).map(bubble).collect();
        let text = paras.join("\n\n");
        match paced(&text) {
            Paced::Note { title, body } => {
                assert_eq!(title, paras[0]);
                assert_eq!(body, text);
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn a_heading_first_is_a_note_with_that_title() {
        let text = "# Harbour plan\n\nDo the thing.\n\nThen the other thing.";
        match paced(text) {
            Paced::Note { title, body } => {
                assert_eq!(title, "Harbour plan");
                assert_eq!(body, "Do the thing.\n\nThen the other thing.");
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn thirteen_hundred_characters_of_prose_is_a_note() {
        let text = "a".repeat(1_300);
        match paced(&text) {
            Paced::Note { title, body } => {
                assert_eq!(title, "a".repeat(TITLE_CHARS) + "…");
                assert_eq!(body, text);
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn a_table_is_a_note() {
        let text = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        match paced(text) {
            Paced::Note { body, .. } => assert_eq!(body, text),
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn a_list_with_blank_lines_between_items_stays_one_unit() {
        let text = "- one\n\n- two";
        assert_eq!(paced(text), chat(&[text]));
    }

    #[test]
    fn a_short_sentence_plus_a_thirty_line_fence_is_chat() {
        let mut fence = String::from("```\n");
        for i in 1..=30 {
            fence.push_str(&format!("line {i}\n"));
        }
        fence.push_str("```");
        let text = format!("Here it is.\n\n{fence}");
        match paced(&text) {
            Paced::Chat(units) => {
                assert_eq!(units.len(), 1, "{units:?}");
                assert!(units[0].starts_with("Here it is."));
                assert!(units[0].contains("line 30"));
            }
            other => panic!("expected chat, got {other:?}"),
        }
    }
}
