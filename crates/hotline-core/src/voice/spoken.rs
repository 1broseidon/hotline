//! One brain, two outputs: a teammate's reply on a call is both what the call
//! says and what the chat shows.
//!
//! A call with a teammate has no voice in front of it. What the person says
//! goes into the teammate's own session as a turn of its conversation, and
//! that turn's input carries [`CONTRACT`]: answer in two parts with
//! [`MARKER`] between them, the first to be spoken and the second only shown.
//! The contract travels in the text the agent is handed, never in a model
//! setting, so Hotline Agent and an ACP agent read it alike, and never in the
//! tape, so the conversation keeps the person's words as they said them.
//!
//! [`Spoken`] takes the part to say from a reply as it streams, and
//! [`speech_text`] turns each sentence of it into what a person would say for
//! it. [`Unmarked`] and [`unmarked`] take the marker out of what the chat
//! shows, which is the whole reply.

use regex::{Captures, Regex};
use std::sync::LazyLock;

/// Between what a reply on a call says and what it only shows.
pub const MARKER: &str = "<<<ENDSPEAK>>>";

/// What a voice turn asks of the agent, after the person's words.
pub const CONTRACT: &str = "[You were told this on a live voice call, and your reply is heard as well as shown. Write one response in two parts, in this order, separated by <<<ENDSPEAK>>>. Part 1 is spoken: short and conversational, self-contained, and it leads with the answer. Describe anything visual in words and never reproduce it (no code, tables, lists, links or markdown), and never refer to what follows. Part 2 is shown in the chat and never spoken: the full detail, code, tables and links. It may be empty. Always write the marker, even when part 2 is empty. Anything you write before using a tool is spoken as it comes, so keep it to a brief line.]";

/// The most sentences said of a reply that never wrote the marker: it was
/// written to be read, so the call says its opening and the chat has the rest.
const FALLBACK_SENTENCES: usize = 3;

/// What the agent is handed for a turn said on a call: the person's words as
/// they said them, then the contract.
pub(crate) fn voice_turn(words: &str) -> String {
    format!("{words}\n\n{CONTRACT}")
}

/// How many bytes at the end of `text` could be the start of the marker,
/// which the next chunk may finish.
fn partial_marker(text: &str) -> usize {
    (1..MARKER.len())
        .rev()
        .find(|&length| text.ends_with(&MARKER[..length]))
        .unwrap_or(0)
}

fn fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

fn table(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// The sentences of a reply that are said, taken as the reply streams.
///
/// Before the marker is prose to say; after it, nothing is. A fenced code
/// block or a table is never read out, and the marker is never said, not even
/// the part of it that ends one chunk. A reply that never writes the marker
/// was written to be read, so only its opening is said: up to its first code
/// block or table, and at most [`FALLBACK_SENTENCES`] sentences. Until the
/// marker comes, a sentence the fallback would not say waits for it.
#[derive(Default)]
pub(crate) struct Spoken {
    /// Text not yet read: a line that is not whole, or a tail that could be
    /// the start of the marker.
    pending: String,
    /// Prose read and not yet cut into sentences.
    prose: String,
    /// The line being read began as prose, so the rest of it is prose too.
    prose_line: bool,
    /// Inside a fenced code block.
    fenced: bool,
    /// Something to be seen, code or a table, came before the marker.
    visual: bool,
    /// The marker has come.
    marked: bool,
    /// Sentences said so far.
    said: usize,
    /// Sentences that are said only if the marker comes.
    waiting: Vec<String>,
}

impl Spoken {
    /// A chunk of the reply in, the sentences now ready to say out.
    pub(crate) fn push(&mut self, chunk: &str) -> Vec<String> {
        if self.marked {
            return Vec::new();
        }
        self.pending.push_str(chunk);
        if let Some(at) = self.pending.find(MARKER) {
            self.pending.truncate(at);
            self.marked = true;
            let mut out = std::mem::take(&mut self.waiting);
            out.extend(self.read(true));
            return out;
        }
        self.read(false)
    }

    /// The reply is over: whatever is left to say.
    pub(crate) fn finish(&mut self) -> Vec<String> {
        if self.marked {
            return Vec::new();
        }
        let out = self.read(true);
        self.waiting.clear();
        out
    }

    /// Reads what is pending up to where the marker could begin, or all of
    /// it when the reply (or its spoken part) is over.
    fn read(&mut self, finished: bool) -> Vec<String> {
        let ready = if finished {
            self.pending.len()
        } else {
            self.pending.len() - partial_marker(&self.pending)
        };
        let mut text: String = self.pending.drain(..ready).collect();
        let mut out = Vec::new();
        while let Some(end) = text.find('\n') {
            let line: String = text.drain(..=end).collect();
            self.line(&line, &mut out);
        }
        if finished {
            if !text.is_empty() {
                self.line(&text, &mut out);
            }
            let rest = super::take_sentences(&mut self.prose, true);
            self.gate(rest, &mut out);
            return out;
        }
        // Part of a line: prose as soon as it cannot be a fence or a table.
        let start = text.trim_start();
        if self.prose_line
            || (!self.fenced && !start.is_empty() && !start.starts_with(['`', '~', '|']))
        {
            self.prose_line = true;
            self.prose.push_str(&text);
            let sentences = super::take_sentences(&mut self.prose, false);
            self.gate(sentences, &mut out);
        } else {
            // Not yet known: it waits for the rest of its line.
            self.pending.insert_str(0, &text);
        }
        out
    }

    /// One whole line.
    fn line(&mut self, line: &str, out: &mut Vec<String>) {
        let prose_line = std::mem::take(&mut self.prose_line);
        if !prose_line {
            if fence(line) {
                self.fenced = !self.fenced;
                self.visual = true;
                return;
            }
            if self.fenced {
                return;
            }
            if table(line) {
                self.visual = true;
                return;
            }
        }
        self.prose.push_str(line);
        let sentences = super::take_sentences(&mut self.prose, false);
        self.gate(sentences, out);
    }

    fn gate(&mut self, sentences: Vec<String>, out: &mut Vec<String>) {
        for sentence in sentences {
            if self.marked {
                out.push(sentence);
            } else if self.visual || self.said >= FALLBACK_SENTENCES || !self.waiting.is_empty() {
                self.waiting.push(sentence);
            } else {
                self.said += 1;
                out.push(sentence);
            }
        }
    }
}

/// A reply as the chat shows it while it streams: the marker taken out, and
/// the start of one held back until the next chunk says whether it was.
/// The paragraph break that stands in for the marker goes out with it, and
/// the space the second part begins with is dropped until it says something.
#[derive(Default)]
pub(crate) struct Unmarked {
    held: String,
    marked: bool,
    /// The second part has begun saying something.
    begun: bool,
}

impl Unmarked {
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        if self.marked {
            if self.begun {
                return chunk.to_string();
            }
            let chunk = chunk.trim_start();
            self.begun = !chunk.is_empty();
            return chunk.to_string();
        }
        self.held.push_str(chunk);
        if let Some(at) = self.held.find(MARKER) {
            self.marked = true;
            let after = self.held.split_off(at + MARKER.len());
            self.held.truncate(at);
            let mut shown = std::mem::take(&mut self.held);
            shown.push_str("\n\n");
            shown.push_str(&self.push(&after));
            return shown;
        }
        let ready = self.held.len() - partial_marker(&self.held);
        self.held.drain(..ready).collect()
    }
}

/// A whole reply as the chat shows it: both parts, a paragraph apart, without
/// the marker. Only the first marker is the reply's; any later one is text.
pub(crate) fn unmarked(text: &str) -> String {
    let Some((said, shown)) = text.split_once(MARKER) else {
        return text.to_string();
    };
    let (said, shown) = (said.trim_end(), shown.trim_start());
    match (said.is_empty(), shown.is_empty()) {
        (_, true) => said.to_string(),
        (true, false) => shown.to_string(),
        (false, false) => format!("{said}\n\n{shown}"),
    }
}

/// What is sent to speech for a sentence: the words a person would say for
/// what is written. Markdown and code marks go, a link is said as "a link",
/// and money, percentages and arrows are said as words. The line shown on the
/// call keeps the text as it was written.
pub(crate) fn speech_text(text: &str) -> String {
    fn rule(pattern: &str) -> Regex {
        Regex::new(pattern).expect("fixed speech rule")
    }
    static LABELLED_LINK: LazyLock<Regex> = LazyLock::new(|| rule(r"\[([^\]\n]+)\]\([^)\s]+\)"));
    static LINK: LazyLock<Regex> = LazyLock::new(|| rule(r"(?i)\b(?:https?://|www\.)[^\s<>()]+"));
    static LINE_MARK: LazyLock<Regex> =
        LazyLock::new(|| rule(r"(?m)^\s*(?:#{1,6}\s+|>\s*|[-*+•]\s+|\d{1,3}[.)]\s+)"));
    static CODE_MARK: LazyLock<Regex> = LazyLock::new(|| rule(r"```[A-Za-z0-9_+-]*|`|\*+|__|~~"));
    static MONEY: LazyLock<Regex> =
        LazyLock::new(|| rule(r"([$€£])\s?(\d+(?:,\d{3})*(?:\.\d+)?)(?:\s?(bn|[KkMBT])\b)?"));
    static SCALED: LazyLock<Regex> =
        LazyLock::new(|| rule(r"\b(\d+(?:,\d{3})*(?:\.\d+)?)(bn|[KkMBT])\b"));
    static PERCENT: LazyLock<Regex> = LazyLock::new(|| rule(r"\s*%"));
    static TOWARDS: LazyLock<Regex> = LazyLock::new(|| rule(r"\s*(?:->|=>|→|⇒)\s*"));
    static NUMBER_SIGN: LazyLock<Regex> = LazyLock::new(|| rule(r"#(\d)"));
    static ABOUT: LazyLock<Regex> = LazyLock::new(|| rule(r"(?:~|≈)\s*(\d)"));
    static SPACES: LazyLock<Regex> = LazyLock::new(|| rule(r"\s+"));

    fn scale(word: &str) -> &'static str {
        match word {
            "K" | "k" => "thousand",
            "M" => "million",
            "B" | "bn" => "billion",
            _ => "trillion",
        }
    }

    let text = LABELLED_LINK.replace_all(text, "$1");
    let text = LINK.replace_all(&text, |link: &Captures| {
        // Punctuation that ends the sentence is not part of the link.
        let whole = &link[0];
        let bare = whole.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        format!("a link{}", &whole[bare.len()..])
    });
    let text = LINE_MARK.replace_all(&text, "");
    let text = CODE_MARK.replace_all(&text, "");
    let text = MONEY.replace_all(&text, |money: &Captures| {
        let amount = &money[2];
        let (one, many) = match &money[1] {
            "$" => ("dollar", "dollars"),
            "€" => ("euro", "euros"),
            _ => ("pound", "pounds"),
        };
        match money.get(3) {
            Some(word) => format!("{amount} {} {many}", scale(word.as_str())),
            None if amount == "1" => format!("1 {one}"),
            None => format!("{amount} {many}"),
        }
    });
    let text = SCALED.replace_all(&text, |number: &Captures| {
        format!("{} {}", &number[1], scale(&number[2]))
    });
    let text = PERCENT.replace_all(&text, " percent");
    let text = TOWARDS.replace_all(&text, " to ");
    let text = NUMBER_SIGN.replace_all(&text, "number $1");
    let text = ABOUT.replace_all(&text, "about $1");
    let text = text.replace(" & ", " and ");
    SPACES.replace_all(&text, " ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything said of a reply that arrives in these chunks.
    fn said(chunks: &[&str]) -> Vec<String> {
        let mut spoken = Spoken::default();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(spoken.push(chunk));
        }
        out.extend(spoken.finish());
        out
    }

    /// What the chat is shown of a reply streamed in these chunks.
    fn shown(chunks: &[&str]) -> String {
        let mut unmarked = Unmarked::default();
        chunks.iter().map(|chunk| unmarked.push(chunk)).collect()
    }

    #[test]
    fn the_contract_names_the_marker_and_follows_the_words() {
        assert!(CONTRACT.contains(MARKER));
        let turn = voice_turn("Is the build green?");
        assert!(turn.starts_with("Is the build green?\n\n["));
        assert!(turn.ends_with(CONTRACT));
    }

    #[test]
    fn only_the_part_before_the_marker_is_said() {
        assert_eq!(
            said(&["The build is green. All 42 tests pass. <<<ENDSPEAK>>>\n| a | b |\n"]),
            ["The build is green.", "All 42 tests pass."]
        );
    }

    #[test]
    fn a_marker_split_across_chunks_is_never_said() {
        for split in 1..MARKER.len() {
            let (head, tail) = MARKER.split_at(split);
            let chunks = [
                "Yes, it's fixed",
                &format!(" now {head}"),
                &format!("{tail}\nThe diff:\n```rust\nfn main() {{}}\n```"),
            ];
            assert_eq!(said(&chunks), ["Yes, it's fixed now"], "split at {split}");
            assert_eq!(
                shown(&chunks),
                "Yes, it's fixed now \n\nThe diff:\n```rust\nfn main() {}\n```",
                "split at {split}"
            );
        }
        // One character at a time.
        let reply = "Done. <<<ENDSPEAK>>> Details.";
        let chars: Vec<String> = reply.chars().map(String::from).collect();
        let chunks: Vec<&str> = chars.iter().map(String::as_str).collect();
        assert_eq!(said(&chunks), ["Done."]);
        assert_eq!(shown(&chunks), "Done. \n\nDetails.");
    }

    #[test]
    fn sentences_before_the_marker_are_said_as_they_arrive() {
        let mut spoken = Spoken::default();
        assert_eq!(spoken.push("It passed. "), ["It passed."]);
        assert!(spoken.push("Two flaky").is_empty());
        assert_eq!(
            spoken.push(" ones retried. <<<"),
            ["Two flaky ones retried."]
        );
        assert!(spoken.push("ENDSPEAK>>> Then: everything else.").is_empty());
        assert!(spoken.finish().is_empty());
    }

    #[test]
    fn without_a_marker_only_the_opening_is_said() {
        assert_eq!(
            said(&["Here's the fix:\n```rust\nlet x = 1;\n```\nIt compiles now."]),
            ["Here's the fix:"]
        );
        assert_eq!(
            said(&["Results below.\n| test | result |\n|---|---|\n| unit | ok |\nAll good."]),
            ["Results below."]
        );
        assert_eq!(
            said(&["One. Two. Three. Four. Five."]),
            ["One.", "Two.", "Three."]
        );
        assert_eq!(said(&["Short answer."]), ["Short answer."]);
        assert!(said(&["```\nonly code\n```"]).is_empty());
    }

    #[test]
    fn an_empty_second_part_is_fine() {
        assert_eq!(said(&["All done. <<<ENDSPEAK>>>"]), ["All done."]);
        assert_eq!(unmarked("All done. <<<ENDSPEAK>>>\n"), "All done.");
        assert_eq!(shown(&["All done. <<<ENDSPEAK>>>", "\n"]), "All done. \n\n");
    }

    #[test]
    fn code_in_the_spoken_part_is_skipped_and_the_rest_is_said() {
        assert_eq!(
            said(&[
                "Run this:\n```sh\ncargo test\n```\nThen tell me. Then again. And once more. <<<ENDSPEAK>>>"
            ]),
            [
                "Run this:",
                "Then tell me.",
                "Then again.",
                "And once more."
            ]
        );
        // More than the fallback would say waits for the marker, and is said
        // once it comes.
        let mut spoken = Spoken::default();
        assert_eq!(spoken.push("A. B. C. D. "), ["A.", "B.", "C."]);
        assert_eq!(spoken.push("E. <<<ENDSPEAK>>>"), ["D.", "E."]);
    }

    #[test]
    fn text_that_only_looks_like_the_marker_is_kept() {
        assert_eq!(
            said(&["Use a <<<EOF here-string", ". <<<ENDSPEAK>>>"]),
            ["Use a <<<EOF here-string."]
        );
        assert_eq!(
            shown(&["Use a <<<END", "SPEAKER tag."]),
            "Use a <<<ENDSPEAKER tag."
        );
        assert_eq!(
            said(&["Odd <<<ENDSPEAK>> text."]),
            ["Odd <<<ENDSPEAK>> text."]
        );
        // Only the first marker is the reply's.
        assert_eq!(
            unmarked("Said. <<<ENDSPEAK>>> The marker is `<<<ENDSPEAK>>>`."),
            "Said.\n\nThe marker is `<<<ENDSPEAK>>>`."
        );
        assert_eq!(unmarked("No marker here."), "No marker here.");
        assert_eq!(unmarked("<<<ENDSPEAK>>>\nOnly shown."), "Only shown.");
    }

    #[test]
    fn speech_is_what_a_person_would_say_for_the_text() {
        for (written, spoken) in [
            (
                "Revenue hit $3.4B, up 12% -> a record.",
                "Revenue hit 3.4 billion dollars, up 12 percent to a record.",
            ),
            (
                "It costs $1 or €20, and £5M later.",
                "It costs 1 dollar or 20 euros, and 5 million pounds later.",
            ),
            ("It came to $1,250,000.", "It came to 1,250,000 dollars."),
            (
                "We have 10k users and $2.5 bn in the bank.",
                "We have 10 thousand users and 2.5 billion dollars in the bank.",
            ),
            ("See https://ketch.run/docs?x=1.", "See a link."),
            (
                "Read [the guide](https://example.com/guide) first.",
                "Read the guide first.",
            ),
            (
                "Run `cargo test` and **then** ship.",
                "Run cargo test and then ship.",
            ),
            ("## Summary", "Summary"),
            ("- The first item", "The first item"),
            ("2. The second step", "The second step"),
            (
                "Fixed in PR #42 => merged",
                "Fixed in PR number 42 to merged",
            ),
            (
                "It takes ~5 minutes & a coffee.",
                "It takes about 5 minutes and a coffee.",
            ),
            ("Check main.rs in v1.2.", "Check main.rs in v1.2."),
            ("```rust", ""),
        ] {
            assert_eq!(speech_text(written), spoken, "{written}");
        }
    }
}
