//! How an agent's reply is shown as chat: one string becomes a few bubbles.
//!
//! A model produces one string. The window draws messages. Blank-line units
//! outside fences become bubbles, lists stay whole, a colon-intro and a stub
//! join their neighbour. The length of the reply is the prompt's job, not a
//! fold's.
//!
//! The split lives in the funnel ([`super::prompt::event_of`]) so both kinds of agent
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
/// It asks for an opening line that fits the request, not a fixed one. A
/// stock list of three phrases, one of them compulsory, made nearly every
/// reply open "On it", and nobody says that on being asked for a story. The
/// line is the one sign the teammate heard the right thing, so it says what
/// was understood, and it is skipped when there is no wait. The kinds are
/// described rather than quoted, because a model copies an example it is
/// given. And it says out loud that brevity is about ceremony and not
/// substance, because an agent told to be short will otherwise shorten the
/// explanation somebody asked for rather than the packaging around it.
///
/// What the teammate is on and what a turn left behind are its own words,
/// through `activity` and `outcome`: the desk draws them beside the mark and
/// under the reply and never makes them up, so the prompt asks for them in
/// the same voice. Anything that cannot be undone is drafted and asked about
/// first, because the line under a reply is a receipt, and a receipt comes
/// too late for a message that went out in the person's name.
///
/// A turn that hands work to helpers and stops to wait says what it waits
/// for beside the mark, where it stays for the whole wait, and not in the
/// chat, where each helper's report used to become a progress message of
/// its own. Outcomes and replies are written for a person: a process id, a
/// raw path or a command to run is the terminal's register, not theirs.
///
/// The silence during the work is asked for here and enforced in
/// [`super::narration`]: a model that narrates anyway is heard once before
/// the work and once after it. The prompt still asks, because narration
/// that reaches nobody still costs the turn its tokens and its time.
pub(crate) const HOUSE_STYLE: &str = "You are speaking in Hotline, a desktop chat app. Your reply is shown as messages in a conversation, the way a person texts — not as a document.

There is a rhythm to that, and it matters more than anything else here. When a request will take any work, first say the one short line a person would say on hearing it. Then work in silence. Then say what came of it.

The line fits the request. A lookup: say you are checking, and what. A job: take it, and if it will run more than a minute or two, say so. Something to write or make: begin, or take a beat to think. Something broken: say you are looking. A correction: own it. A go-ahead: go, and a reaction can stand in for words. A hard question: say you need a moment to think. Something for a colleague: say who you will ask. Match the person: short and loose for a short, loose ask.

If you can answer with no work, answer and skip the line. If you are about to ask a question or decline, do that and skip the line. Do not open with the words you opened with last time. The line is what you would say to someone standing in your doorway, about the thing they just asked, and it is not a summary of your plan.

After it, say nothing in the conversation until you have the answer: no running commentary of what you open, what you find or what you will do next. Someone who asks a capable friend to fix something does not get a play-by-play; they get \"done\", or \"stuck, and here's why\". Work like that.

What you are on is shown beside your mark instead, in your own words, with `activity`. Say it as the work moves from one broad step to the next: a handful for a job, none for a quick answer. A few plain words about the task, never the tool, starting with what you are doing. Its type is what someone watching would see: thinking when you plan or work something out, reading when you go through something that exists, searching when you find, compare or check, making when you write, fill in, edit, build or add up, talking when you message or call someone who is not the person. When you are held up by someone else, say what you are waiting for. When you hand work to helpers and stop to wait for them, that is where you say what you are waiting for, not in the conversation, and it stays beside your mark while you wait. A helper reporting back is not a reason to speak: update the activity instead, and speak again when you have the outcome or need the person.

When the work is over, say what it left behind with `outcome`, once for each thing: sent for what went out to someone else (a message, money, the person's information), made for something new they keep, changed for something of theirs that is now different, open for something still going after your reply, yours for what is left for them to do. Each is written for the person, not a terminal: a few plain words in their terms, with no process ids, no raw links or paths unless that is the thing they asked for, and nothing they would have to be technical to read. Reading, searching, running commands and writing code on the way are the path, not outcomes, and a plain answer has none. If something failed or only partly happened, say so with its state and say what is missing: never leave a failure out.

Before anything that cannot be undone — sending something in the person's name, paying, cancelling, sharing their information — draft it and ask, and act on their yes, unless they have told you to go ahead without asking.

Then report and stop. Done, or what stopped you, and anything they have to decide — in a line or two. No recap of the steps, no list of the files you touched, no summary of what you just did. Do not hand them commands to run unless they asked for one; if something is left that you could do for them, such as stopping what you started, offer to do it. Two or three messages is the whole exchange, unless what they asked for is genuinely longer.

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
    if !says_nothing(&unit) {
        out.push(unit);
    }
}

/// Whether words say nothing a bubble could show: empty, or only whitespace
/// and the zero-width characters a model sometimes answers with in place of
/// silence. Such a reply is no message, never an empty bubble.
pub(crate) fn says_nothing(text: &str) -> bool {
    text.chars().all(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
            )
    })
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
    use std::fmt::Write;

    fn bubble(n: u32) -> String {
        format!("Paragraph {n} is long enough to stand as its own bubble in the chat.")
    }

    fn chat(units: &[&str]) -> Vec<String> {
        units.iter().map(ToString::to_string).collect()
    }

    /// The opening line fits the request and is skipped when there is no
    /// wait: no stock phrase to copy and nothing compulsory about it.
    #[test]
    fn the_opening_line_fits_the_request_and_gives_no_stock_phrase() {
        let style = HOUSE_STYLE.to_lowercase();
        for stock in ["on it", "let me check", "one sec", "not optional"] {
            assert!(!style.contains(stock), "{stock:?} is still offered");
        }
        for rule in [
            "the one short line a person would say on hearing it",
            "a lookup: say you are checking, and what",
            "if it will run more than a minute or two, say so",
            "if you can answer with no work, answer and skip the line",
            "if you are about to ask a question or decline, do that and skip the line",
            "do not open with the words you opened with last time",
        ] {
            assert!(style.contains(rule), "{rule:?} is missing");
        }
    }

    /// What the teammate is on and what came of it are its own words, by
    /// the tools that carry them, with every kind said once.
    #[test]
    fn it_asks_for_activity_and_outcomes_by_their_kinds() {
        assert!(HOUSE_STYLE.contains("`activity`"));
        assert!(HOUSE_STYLE.contains("`outcome`"));
        assert!(HOUSE_STYLE.contains("about the task, never the tool"));
        for kind in super::super::declared::ACTIVITY_TYPES
            .iter()
            .chain(&super::super::declared::OUTCOME_TYPES)
        {
            assert!(
                HOUSE_STYLE.contains(&format!("{kind} when"))
                    || HOUSE_STYLE.contains(&format!("{kind} for")),
                "{kind} is not described"
            );
        }
        assert!(HOUSE_STYLE.contains("are the path, not outcomes"));
        assert!(HOUSE_STYLE.contains("never leave a failure out"));
    }

    /// Waiting on helpers is said beside the mark and not in the chat, and a
    /// helper reporting back is no reason to speak.
    #[test]
    fn it_says_what_it_waits_on_in_its_activity_and_not_in_the_chat() {
        for rule in [
            "When you hand work to helpers and stop to wait for them, that is where you say what you are waiting for, not in the conversation, and it stays beside your mark while you wait.",
            "A helper reporting back is not a reason to speak: update the activity instead, and speak again when you have the outcome or need the person.",
        ] {
            assert!(HOUSE_STYLE.contains(rule), "{rule:?} is missing");
        }
    }

    /// What came of the work and the reply are written for a person, and no
    /// command is quoted for the model to copy.
    #[test]
    fn outcomes_and_replies_are_written_for_a_person_not_a_terminal() {
        for rule in [
            "Each is written for the person, not a terminal: a few plain words in their terms, with no process ids, no raw links or paths unless that is the thing they asked for, and nothing they would have to be technical to read.",
            "Do not hand them commands to run unless they asked for one; if something is left that you could do for them, such as stopping what you started, offer to do it.",
        ] {
            assert!(HOUSE_STYLE.contains(rule), "{rule:?} is missing");
        }
        assert!(!HOUSE_STYLE.contains("kill "));
    }

    /// What cannot be undone is drafted and asked about first.
    #[test]
    fn it_drafts_and_asks_before_what_cannot_be_undone() {
        assert!(HOUSE_STYLE.contains(
            "Before anything that cannot be undone — sending something in the person's name, paying, cancelling, sharing their information — draft it and ask, and act on their yes, unless they have told you to go ahead without asking."
        ));
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
    fn a_reply_of_only_zero_width_characters_is_no_bubble() {
        for nothing in [
            "",
            " \n\t",
            "\u{200B}",
            "\u{200C}\u{200D}",
            "\u{FEFF} \u{2060}",
        ] {
            assert!(paced(nothing).is_empty(), "{nothing:?}");
        }
        let said = bubble(1);
        assert_eq!(paced(&format!("{said}\n\n\u{200B}")), chat(&[&said]));
        assert_eq!(paced("ok\u{200B}"), chat(&["ok\u{200B}"]));
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
            let _ = writeln!(fence, "line {i}");
        }
        fence.push_str("```");
        let text = format!("Here it is.\n\n{fence}");
        let units = paced(&text);
        assert_eq!(units.len(), 1, "{units:?}");
        assert!(units[0].starts_with("Here it is."));
        assert!(units[0].contains("line 30"));
    }
}
