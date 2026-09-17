//! How an agent's reply is shown as chat: one string becomes a few bubbles.
//!
//! A model produces one string. The window draws messages. Blank-line units
//! outside fences become bubbles, lists stay whole, a colon-intro and a stub
//! join their neighbour. The length of the reply is the prompt's job, not a
//! fold's.
//!
//! The split lives in the funnel ([`super::event_of`]) so both kinds of agent
//! and a peer thread get it for free. History rejoins consecutive agent events
//! with a blank line, because the model said one thing.

/// A stub shorter than this is a lead-in ("Here's the fix:") or a leftover
/// ("ok."), not a bubble of its own.
pub const SHORT_UNIT_CHARS: usize = 60;

/// What Hotline tells an agent about the room it is speaking in.
///
/// An agent's default register is the terminal: a headed report, bullets under
/// each heading, a summary of what it is about to do. That is the right shape
/// for a scrollback and the wrong shape for a conversation, and no agent can
/// know which one it is in unless it is told. This is a fact about Hotline rather
/// than about the teammate, which is why it does not live in the teammate's
/// `AGENTS.md`: it is true of every teammate, and it has to arrive even when
/// the working directory is a real repository whose `AGENTS.md` Hotline leaves
/// alone.
///
/// It asks for one acknowledgement before the work rather than banning one.
/// The typing indicator and "on it" do not say the same thing: dots mean
/// something is happening, "on it" means you were heard. And it says out loud
/// that brevity is about ceremony and not substance, because an agent told to
/// be short will otherwise shorten the explanation somebody asked for rather
/// than the packaging around it.
///
/// The silence during the work is asked for here and enforced in
/// [`super::narration`]: a model that narrates anyway is heard once before
/// the work and once after it. The prompt still asks, because narration
/// that reaches nobody still costs the turn its tokens and its time.
pub(crate) const HOUSE_STYLE: &str = "You are speaking in Hotline, a desktop chat app. Your reply is shown as messages in a conversation, the way a person texts — not as a document.

There is a rhythm to that, and it matters more than anything else here. Before your first tool call, write one short line: \"on it\", \"let me check\", \"sure, one sec\". Then work in silence. Then say what came of it. The whole exchange should read like two colleagues — \"how many rust files are under crates?\" / \"let me check\" / \"41, all .rs\" — and never like one long report delivered after a minute of nothing. That opening line is not optional and it is not a summary of your plan; it is the word you would say to someone standing in your doorway.

After it, say nothing until you have the answer. Not what you are opening, not what you found, not what you will do next: the person cannot see your tool calls and does not want a running commentary of them. Someone who asks a capable friend to fix something does not get a play-by-play; they get \"done\", or \"stuck, and here's why\". Work like that.

Then report and stop. Done, or what stopped you, and anything they have to decide — in a line or two. No recap of the steps, no list of the files you touched, no summary of what you just did. Two or three messages is the whole exchange, unless what they asked for is genuinely longer.

Write it the way you would text it. Lead with the answer. Plain sentences, no preamble, no restating the question, no sign-off.

Hotline shows your reply as chat: each paragraph is its own message. A report, a plan, or an explanation still has to be speech — lead with the answer, and do not open with a heading.

Being brief is about ceremony, not substance. A real question deserves a real answer — if someone asks how something works or why it broke, explain it properly. What gets cut is the packaging, never the thinking.

Formatting is available when the content is genuinely that shape — a fenced block for code, a list when there really are several items, a table when there are rows and columns, backticks for a filename or flag, bold for a term that carries weight. Reach for none of this to organise three sentences.";

/// Split a reply into chat bubbles.
///
/// Blank-line units outside fences, lists whole. A unit that ends with `:`
/// joins the next, and a stub shorter than [`SHORT_UNIT_CHARS`] joins its
/// neighbour, so "Here's the fix:" and a fence stay one bubble.
pub fn paced(text: &str) -> Vec<String> {
    merge(units(text))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bubble(n: u32) -> String {
        format!("Paragraph {n} is long enough to stand as its own bubble in the chat.")
    }

    fn chat(units: &[&str]) -> Vec<String> {
        units.iter().map(|unit| unit.to_string()).collect()
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
        let units = paced(text);
        assert_eq!(units.len(), 1, "{units:?}");
        assert!(units[0].starts_with("Here's the fix:"));
        assert!(units[0].contains("fn x() {}"));
    }

    #[test]
    fn a_fence_with_blank_lines_is_one_unit() {
        let text = "```\nhello\n\nworld\n```";
        assert_eq!(paced(text), chat(&[text]));
    }

    #[test]
    fn five_paragraphs_are_five_bubbles() {
        let paras: Vec<String> = (1..=5).map(bubble).collect();
        let text = paras.join("\n\n");
        assert_eq!(paced(&text), paras);
    }

    #[test]
    fn a_heading_first_reply_keeps_the_heading_in_the_text() {
        let a = bubble(1);
        let b = bubble(2);
        let text = format!("# Harbour plan\n\n{a}\n\n{b}");
        let units = paced(&text);
        assert_eq!(units.len(), 2, "{units:?}");
        assert!(
            units[0].starts_with("# Harbour plan"),
            "the heading stays in the first bubble: {units:?}"
        );
        assert!(units[0].contains(&a), "{units:?}");
        assert_eq!(units[1], b);
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
        let units = paced(&text);
        assert_eq!(units.len(), 1, "{units:?}");
        assert!(units[0].starts_with("Here it is."));
        assert!(units[0].contains("line 30"));
    }
}
