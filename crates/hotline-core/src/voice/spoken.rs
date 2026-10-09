//! One brain, two outputs: a teammate's reply on a call is the same answer
//! written twice, once to be heard and once to be read.
//!
//! A call with a teammate has no voice in front of it. What the person says
//! goes into the teammate's own session as a turn of its conversation, and
//! that turn's input carries [`CONTRACT`]: answer twice, the version to say
//! between `<spoken>` tags and then the version to show between `<written>`
//! tags. Two whole versions rather than one reply cut in two, because a model
//! asked for a part to say and a part to show writes an opener and a body,
//! and the chat then reads as the opener followed by the rest of it. The
//! contract travels in the text the agent is handed, never in a model
//! setting, so Hotline Agent and an ACP agent read it alike, and never in the
//! tape, so the conversation keeps the person's words as they said them.
//!
//! [`Spoken`] takes what to say from a reply as it streams, and
//! [`speech_text`] turns each sentence of it into what a person would say for
//! it. [`Shown`] and [`versions`] take what the chat shows, which is the
//! written version alone; [`versions`] also keeps the spoken one, which the
//! chat draws as a transcript line and the model is shown again in its
//! history ([`both`]).

use regex::{Captures, Regex};
use std::sync::LazyLock;

/// What a voice turn asks of the agent, after the person's words.
pub const CONTRACT: &str = "[Voice call: answer twice, once to be heard and once to be read. Each version must stand alone; neither continues the other.

<spoken>
How you'd answer on a phone call. Lead with the answer. 1 to 3 sentences, under 40 words. Plain speech: no code, lists, links or markdown, and no mention of the written version except \"details are in the chat\" when needed. For long answers, give the gist and how many items there are.
</spoken>
<written>
The complete answer exactly as if the question were typed. Full detail, code, tables, links. Never spoken. Do not assume the reader heard the spoken version.
</written>

Example:
<spoken>Two things are wrong: the API key is missing and the port's already in use. I've put both fixes in the chat.</spoken>
<written>The build fails for two reasons:
1. **Missing API key.** ...
2. **Port 8080 in use.** ...</written>

Before using a tool, say at most one short line. If you were cut off, answer the new words without repeating yourself.]";

/// The most sentences said of a reply that has no spoken version: it was
/// written to be read, so the call says its opening and the chat has it all.
const FALLBACK_SENTENCES: usize = 3;

/// What the agent is handed for a turn said on a call: the person's words as
/// they said them, then the contract.
pub(crate) fn voice_turn(words: &str) -> String {
    format!("{words}\n\n{CONTRACT}")
}

/// One of the tags a reply on a call is written with. Case and spaces inside
/// the angle brackets are the model's to vary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    Spoken,
    EndSpoken,
    Written,
    EndWritten,
}

static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*(/?)\s*(spoken|written)\s*>").expect("fixed tag pattern")
});

/// The longest tail held back as the possible start of a tag. A `<` further
/// back than this is text, so a stray one never holds a reply up.
const LONGEST_PARTIAL_TAG: usize = 24;

fn tag_of(found: &Captures) -> Tag {
    let closing = !found[1].is_empty();
    match (found[2].eq_ignore_ascii_case("spoken"), closing) {
        (true, false) => Tag::Spoken,
        (true, true) => Tag::EndSpoken,
        (false, false) => Tag::Written,
        (false, true) => Tag::EndWritten,
    }
}

/// How many bytes at the end of `text` could be the start of a tag, which
/// the next chunk may finish.
fn partial_tag(text: &str) -> usize {
    let Some(at) = text.rfind('<') else {
        return 0;
    };
    let tail = &text[at..];
    if tail.len() > LONGEST_PARTIAL_TAG {
        return 0;
    }
    let rest = tail[1..].trim_start();
    let rest = rest.strip_prefix('/').unwrap_or(rest).trim_start();
    let name_ends = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (name, after) = rest.split_at(name_ends);
    let name = name.to_ascii_lowercase();
    let could_be = if after.is_empty() {
        "spoken".starts_with(&name) || "written".starts_with(&name)
    } else {
        after.trim().is_empty() && (name == "spoken" || name == "written")
    };
    if could_be { tail.len() } else { 0 }
}

/// A reply read as it streams: its text, and the tags between it.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Tag(Tag),
}

/// Cuts a reply into [`Piece`]s as its chunks arrive, holding back the end
/// of a chunk that could be the start of a tag until the next one says
/// whether it was.
#[derive(Default)]
struct Tags {
    held: String,
}

impl Tags {
    fn push(&mut self, chunk: &str) -> Vec<Piece> {
        self.held.push_str(chunk);
        let mut pieces = Vec::new();
        let mut read = 0;
        for found in TAG.captures_iter(&self.held) {
            let whole = found.get(0).expect("a match has its whole");
            if whole.start() > read {
                pieces.push(Piece::Text(self.held[read..whole.start()].to_string()));
            }
            pieces.push(Piece::Tag(tag_of(&found)));
            read = whole.end();
        }
        let rest = &self.held[read..];
        let ready = rest.len() - partial_tag(rest);
        if ready > 0 {
            pieces.push(Piece::Text(rest[..ready].to_string()));
        }
        self.held.drain(..read + ready);
        pieces
    }

    /// The reply is over: what was held back was text after all.
    fn finish(&mut self) -> Option<Piece> {
        let held = std::mem::take(&mut self.held);
        (!held.is_empty()).then_some(Piece::Text(held))
    }
}

/// Where in a reply the text being read is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Place {
    /// Before any version: a line before a tool, or a reply that never
    /// wrote the tags.
    #[default]
    Before,
    Spoken,
    Written,
    /// After a version, outside both.
    Outside,
}

impl Place {
    /// Where a tag leaves the reader. Only the first spoken version is the
    /// reply's, and a written one ends only at its own closing tag, so a tag
    /// anywhere else is stray: it is dropped and the text on both sides kept.
    fn after(self, tag: Tag) -> Place {
        match (self, tag) {
            (Place::Before, Tag::Spoken) => Place::Spoken,
            (Place::Spoken, Tag::Written) => Place::Written,
            (Place::Spoken, Tag::EndSpoken | Tag::EndWritten) => Place::Outside,
            (Place::Before | Place::Outside, Tag::Written) => Place::Written,
            (Place::Written, Tag::EndWritten) => Place::Outside,
            (place, _) => place,
        }
    }
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
/// Everything inside `<spoken>` is said, a sentence as soon as it is whole,
/// and nothing after it. A reply that has no spoken version was written to be
/// read, so only its opening is said: up to its first code block or table,
/// and at most [`FALLBACK_SENTENCES`] sentences. Text before the first tag is
/// read that way too, which is how the line an agent writes before a tool is
/// said as it streams. A fenced code block or a table is never read out, and
/// no tag is ever said, not even the part of one that ends a chunk.
#[derive(Default)]
pub(crate) struct Spoken {
    tags: Tags,
    place: Place,
    /// The spoken version is over: nothing more is said.
    done: bool,
    /// Text not yet read: part of a line not yet known to be prose.
    pending: String,
    /// Prose read and not yet cut into sentences.
    prose: String,
    /// The line being read began as prose, so the rest of it is prose too.
    prose_line: bool,
    /// Inside a fenced code block.
    fenced: bool,
    /// Something to be seen, code or a table, came before any spoken version.
    visual: bool,
    /// Sentences the fallback has said.
    said: usize,
}

impl Spoken {
    /// A chunk of the reply in, the sentences now ready to say out.
    pub(crate) fn push(&mut self, chunk: &str) -> Vec<String> {
        let pieces = self.tags.push(chunk);
        self.take(pieces, false)
    }

    /// The reply is over: whatever is left to say.
    pub(crate) fn finish(&mut self) -> Vec<String> {
        let pieces = self.tags.finish().into_iter().collect();
        self.take(pieces, true)
    }

    fn take(&mut self, pieces: Vec<Piece>, finished: bool) -> Vec<String> {
        let mut out = Vec::new();
        for piece in pieces {
            if self.done {
                return out;
            }
            match piece {
                Piece::Text(text) => {
                    self.pending.push_str(&text);
                    out.extend(self.read(false));
                }
                Piece::Tag(tag) => {
                    let next = self.place.after(tag);
                    if self.place == Place::Spoken && next != Place::Spoken {
                        out.extend(self.read(true));
                        self.done = true;
                    } else if next == Place::Spoken {
                        // What came before is said as far as the fallback
                        // would, and the spoken version starts afresh.
                        out.extend(self.read(true));
                        self.fenced = false;
                    }
                    self.place = next;
                }
            }
        }
        if finished && !self.done {
            out.extend(self.read(true));
        }
        out
    }

    /// Reads what is pending, all of it when the reply (or the spoken
    /// version) is over.
    fn read(&mut self, finished: bool) -> Vec<String> {
        let mut text = std::mem::take(&mut self.pending);
        let mut out = Vec::new();
        while let Some(end) = text.find('\n') {
            let line: String = text.drain(..=end).collect();
            self.line(&line, &mut out);
        }
        if finished {
            if !text.is_empty() {
                self.line(&text, &mut out);
            }
            self.prose_line = false;
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
            self.pending = text;
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
            if self.place == Place::Spoken {
                out.push(sentence);
            } else if !self.visual && self.said < FALLBACK_SENTENCES {
                self.said += 1;
                out.push(sentence);
            }
        }
    }
}

/// A reply as the chat shows it while it streams: the written version, and
/// any text outside the tags, without the spoken version or a tag. The start
/// of a tag is held back until the next chunk says whether it was one. The
/// space a version begins with is dropped until it says something, and what
/// follows something already shown is a paragraph after it.
#[derive(Default)]
pub(crate) struct Shown {
    tags: Tags,
    place: Place,
    /// Something has been shown.
    shown: bool,
    /// The text being shown has begun saying something since the last tag.
    begun: bool,
}

impl Shown {
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        let mut out = String::new();
        for piece in self.tags.push(chunk) {
            let place = self.place;
            match piece {
                Piece::Tag(tag) => {
                    let next = place.after(tag);
                    if next != place {
                        self.begun = false;
                    }
                    self.place = next;
                }
                Piece::Text(_) if place == Place::Spoken => {}
                Piece::Text(text) if place == Place::Before || self.begun => out.push_str(&text),
                Piece::Text(text) => {
                    let text = text.trim_start();
                    if text.is_empty() {
                        continue;
                    }
                    if self.shown {
                        out.push_str("\n\n");
                    }
                    self.begun = true;
                    out.push_str(text);
                }
            }
            self.shown |= !out.trim().is_empty();
        }
        out
    }
}

/// A whole reply, read for both its versions.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Versions {
    /// What the reply said, when it wrote a spoken version: that version,
    /// after any line it began with.
    pub spoken: Option<String>,
    /// What the chat shows: the written version and any text outside the
    /// tags, or, when the reply wrote no written version, the spoken one.
    /// A reply with no tags is shown as it was written.
    pub written: String,
    /// The written version looks like the rest of the spoken one rather than
    /// a version of its own ([`lazy`]): worth a line in the log.
    pub lazy: Option<&'static str>,
}

/// A whole reply as the chat shows it and as the call said it. No tag is
/// ever shown, on a call or not: an agent that keeps its own history keeps
/// the contract in it and may write the tags in a typed reply long after.
pub(crate) fn versions(text: &str) -> Versions {
    let mut tags = Tags::default();
    let mut pieces = tags.push(text);
    pieces.extend(tags.finish());
    if !pieces.iter().any(|piece| matches!(piece, Piece::Tag(_))) {
        return Versions {
            spoken: None,
            written: text.to_string(),
            lazy: None,
        };
    }
    let mut place = Place::Before;
    let (mut before, mut spoken, mut written, mut outside) =
        (String::new(), None::<String>, String::new(), String::new());
    for piece in pieces {
        match piece {
            Piece::Tag(tag) => {
                place = place.after(tag);
                if place == Place::Spoken {
                    spoken.get_or_insert_default();
                }
            }
            Piece::Text(text) => match place {
                Place::Before => before.push_str(&text),
                Place::Spoken => spoken.get_or_insert_default().push_str(&text),
                Place::Written => written.push_str(&text),
                Place::Outside => outside.push_str(&text),
            },
        }
    }
    let Some(spoken) = spoken else {
        return Versions {
            spoken: None,
            written: paragraphs(&[&before, &written, &outside]),
            lazy: None,
        };
    };
    let said = [before.trim(), spoken.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let lazy = match written.trim().is_empty() {
        true => None,
        false => lazy(spoken.trim(), written.trim()),
    };
    let shown = match paragraphs(&[&written, &outside]) {
        shown if shown.is_empty() => said.clone(),
        shown => shown,
    };
    Versions {
        spoken: (!said.is_empty()).then_some(said),
        written: shown,
        lazy,
    }
}

/// How a reply on a call was written, which decides how it was said:
/// counted by the model that wrote it ([`super::replies`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Path {
    /// A spoken version closed with `</spoken>`, and a written one.
    Both,
    /// A spoken version closed with `</spoken>`, and no written one.
    SpokenOnly,
    /// A spoken version never closed with `</spoken>`: it ended where the
    /// written one began, or with the reply.
    Unclosed,
    /// No spoken version, so the call said the reply's opening.
    Untagged,
}

impl Path {
    /// How the log and the wire name it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Path::Both => "both",
            Path::SpokenOnly => "spokenOnly",
            Path::Unclosed => "unclosed",
            Path::Untagged => "untagged",
        }
    }
}

/// How a whole reply was written, read as [`Spoken`] reads it: only the
/// first spoken version counts, and a stray tag is no version.
pub(crate) fn path(text: &str) -> Path {
    let mut tags = Tags::default();
    let mut pieces = tags.push(text);
    pieces.extend(tags.finish());
    let mut place = Place::Before;
    let (mut spoken, mut closed, mut written) = (false, false, false);
    for piece in pieces {
        let Piece::Tag(tag) = piece else { continue };
        let next = place.after(tag);
        spoken |= next == Place::Spoken;
        closed |= place == Place::Spoken && tag == Tag::EndSpoken;
        written |= next == Place::Written;
        place = next;
    }
    match (spoken, closed, written) {
        (false, _, _) => Path::Untagged,
        (true, false, _) => Path::Unclosed,
        (true, true, true) => Path::Both,
        (true, true, false) => Path::SpokenOnly,
    }
}

fn paragraphs(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A reply said on a call as the model is shown it again in a history
/// rebuilt from the tape, so a follow-up knows what was heard and what was
/// only read.
pub(crate) fn both(spoken: &str, written: &str) -> String {
    format!("<spoken>{spoken}</spoken>\n<written>{written}</written>")
}

/// Words a written version opens with when it carries on from the spoken
/// one instead of standing alone.
const CONTINUATIONS: [&str; 9] = [
    "also",
    "additionally",
    "here's the rest",
    "here is the rest",
    "as i said",
    "as i mentioned",
    "as mentioned",
    "furthermore",
    "moreover",
];

/// How much of the spoken version the written one's opening repeats, word
/// for word in order, before it reads as the same text again.
const REPEATED: f64 = 0.8;

/// Whether a written version reads as the rest of the spoken one, which the
/// contract asks the model not to write: it opens with a continuation word,
/// or with the spoken version again nearly word for word. A diagnostic only;
/// what is shown and said does not change.
fn lazy(spoken: &str, written: &str) -> Option<&'static str> {
    let opening = written
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '_' | '#' | '>'))
        .to_lowercase()
        .replace('\u{2019}', "'");
    let continues = CONTINUATIONS.iter().any(|word| {
        opening
            .strip_prefix(word)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()))
    });
    if continues {
        return Some("opens with a continuation word");
    }
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
            .filter(|word| !word.is_empty())
            .map(|word| word.to_lowercase().replace('\u{2019}', "'"))
            .collect()
    };
    let said = words(spoken);
    if said.len() < 4 {
        return None;
    }
    let shown: Vec<String> = words(written).into_iter().take(said.len()).collect();
    let common = common_in_order(&said, &shown);
    (common as f64 >= REPEATED * said.len() as f64).then_some("repeats the spoken version")
}

/// The longest run of words both lists have in the same order.
fn common_in_order(a: &[String], b: &[String]) -> usize {
    let mut row = vec![0; b.len() + 1];
    for word in a {
        let mut diagonal = 0;
        for (at, other) in b.iter().enumerate() {
            let above = row[at + 1];
            row[at + 1] = if word == other {
                diagonal + 1
            } else {
                above.max(row[at])
            };
            diagonal = above;
        }
    }
    row[b.len()]
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
        let mut shown = Shown::default();
        chunks.iter().map(|chunk| shown.push(chunk)).collect()
    }

    /// A reply one character at a time.
    fn by_char(reply: &str) -> Vec<String> {
        reply.chars().map(String::from).collect()
    }

    fn refs(chunks: &[String]) -> Vec<&str> {
        chunks.iter().map(String::as_str).collect()
    }

    #[test]
    fn the_contract_asks_for_both_versions_and_follows_the_words() {
        for tag in ["<spoken>", "</spoken>", "<written>", "</written>"] {
            assert!(CONTRACT.contains(tag), "{tag}");
        }
        let turn = voice_turn("Is the build green?");
        assert!(turn.starts_with("Is the build green?\n\n[Voice call: "));
        assert!(turn.ends_with(CONTRACT));
    }

    #[test]
    fn only_the_spoken_version_is_said_and_only_the_written_one_shown() {
        let reply = "<spoken>The build is green. All 42 tests pass.</spoken>\n<written>| job | result |\n| --- | --- |\n| unit | ok |</written>";
        assert_eq!(
            said(&[reply]),
            ["The build is green.", "All 42 tests pass."]
        );
        assert_eq!(
            shown(&[reply]),
            "| job | result |\n| --- | --- |\n| unit | ok |"
        );
        assert_eq!(
            versions(reply),
            Versions {
                spoken: Some("The build is green. All 42 tests pass.".into()),
                written: "| job | result |\n| --- | --- |\n| unit | ok |".into(),
                lazy: None,
            }
        );
    }

    #[test]
    fn a_tag_split_across_chunks_at_any_offset_is_never_said_or_shown() {
        let written = "The diff:\n```rust\nfn main() {}\n```";
        for reply in [
            format!("<spoken>Yes, it's fixed now.</spoken>\n<written>{written}</written>"),
            format!("< Spoken >Yes, it's fixed now.</SPOKEN >\n\n<  written>{written}< / Written>"),
        ] {
            for split in 0..=reply.len() {
                let (head, tail) = reply.split_at(split);
                assert_eq!(
                    said(&[head, tail]),
                    ["Yes, it's fixed now."],
                    "{reply:?} at {split}"
                );
                assert_eq!(shown(&[head, tail]), written, "{reply:?} at {split}");
            }
            let chars = by_char(&reply);
            assert_eq!(said(&refs(&chars)), ["Yes, it's fixed now."]);
            assert_eq!(shown(&refs(&chars)), written);
            assert_eq!(versions(&reply).written, written);
            assert_eq!(
                versions(&reply).spoken.as_deref(),
                Some("Yes, it's fixed now.")
            );
        }
    }

    #[test]
    fn spoken_sentences_are_said_as_they_arrive() {
        let mut spoken = Spoken::default();
        assert_eq!(spoken.push("<spoken>It passed. "), ["It passed."]);
        assert!(spoken.push("Two flaky").is_empty());
        assert_eq!(
            spoken.push(" ones retried. </spo"),
            ["Two flaky ones retried."]
        );
        assert!(
            spoken
                .push("ken>\n<written>Everything else. And more.</written>")
                .is_empty()
        );
        assert!(spoken.finish().is_empty());
    }

    #[test]
    fn an_unclosed_spoken_version_ends_where_the_written_one_begins() {
        let reply = "<spoken>Done, and it's merged. <written>Merged as **#42**.";
        assert_eq!(said(&[reply]), ["Done, and it's merged."]);
        assert_eq!(shown(&[reply]), "Merged as **#42**.");
        let both = versions(reply);
        assert_eq!(both.spoken.as_deref(), Some("Done, and it's merged."));
        assert_eq!(both.written, "Merged as **#42**.");
    }

    #[test]
    fn without_a_written_version_the_chat_shows_the_spoken_one() {
        for reply in ["<spoken>All done.</spoken>", "<spoken>All done.\n"] {
            assert_eq!(said(&[reply]), ["All done."]);
            assert_eq!(
                versions(reply),
                Versions {
                    spoken: Some("All done.".into()),
                    written: "All done.".into(),
                    lazy: None,
                }
            );
        }
        // Text after the spoken version that forgot its tag is the written one.
        let reply = "<spoken>Two fixes.</spoken>\nFirst, set the key. Then free the port.";
        assert_eq!(said(&[reply]), ["Two fixes."]);
        assert_eq!(
            versions(reply).written,
            "First, set the key. Then free the port."
        );
        assert_eq!(shown(&[reply]), "First, set the key. Then free the port.");
    }

    #[test]
    fn without_tags_only_the_opening_is_said_and_everything_is_shown() {
        for (reply, opening) in [
            (
                "Here's the fix:\n```rust\nlet x = 1;\n```\nIt compiles now.",
                vec!["Here's the fix:"],
            ),
            (
                "Results below.\n| test | result |\n|---|---|\n| unit | ok |\nAll good.",
                vec!["Results below."],
            ),
            (
                "One. Two. Three. Four. Five.",
                vec!["One.", "Two.", "Three."],
            ),
            ("Short answer.", vec!["Short answer."]),
            ("```\nonly code\n```", vec![]),
        ] {
            assert_eq!(said(&[reply]), opening, "{reply}");
            assert_eq!(said(&refs(&by_char(reply))), opening, "{reply}");
            assert_eq!(shown(&refs(&by_char(reply))), reply);
            assert_eq!(
                versions(reply),
                Versions {
                    spoken: None,
                    written: reply.into(),
                    lazy: None,
                }
            );
        }
    }

    #[test]
    fn a_line_before_the_tags_is_said_as_it_streams_and_kept_with_what_was_said() {
        let mut spoken = Spoken::default();
        assert_eq!(spoken.push("Let me check. "), ["Let me check."]);
        assert_eq!(
            spoken.push("<spoken>It's green.</spoken><written>Green: 42 of 42.</written>"),
            ["It's green."]
        );
        let both = versions(
            "Let me check. <spoken>It's green.</spoken><written>Green: 42 of 42.</written>",
        );
        assert_eq!(both.spoken.as_deref(), Some("Let me check. It's green."));
        assert_eq!(both.written, "Green: 42 of 42.");
    }

    #[test]
    fn the_spoken_version_is_said_whole_without_its_code() {
        assert_eq!(
            said(&[
                "<spoken>Run this:\n```sh\ncargo test\n```\nThen tell me. Then again. And once more.</spoken>"
            ]),
            [
                "Run this:",
                "Then tell me.",
                "Then again.",
                "And once more."
            ]
        );
        // The fallback's limit is for a reply written to be read.
        assert_eq!(
            said(&["<spoken>A. B. C. D. E.</spoken><written>F.</written>"]),
            ["A.", "B.", "C.", "D.", "E."]
        );
    }

    #[test]
    fn code_in_the_written_version_is_shown_as_written() {
        let written = "Use `Vec<String>`:\n```html\n<div><b>x</b></div>\n```\nThen check `a < b`.";
        let reply = format!("<spoken>Here's the fix.</spoken>\n<written>\n{written}\n</written>\n");
        assert_eq!(said(&[reply.as_str()]), ["Here's the fix."]);
        assert_eq!(said(&refs(&by_char(&reply))), ["Here's the fix."]);
        assert_eq!(versions(&reply).written, written);
        assert_eq!(shown(&refs(&by_char(&reply))).trim_end(), written);
    }

    #[test]
    fn text_that_only_looks_like_a_tag_is_kept() {
        let reply = "Use a <spokesperson> or <spoke> tag, a <b>bold</b> word and Vec<String>. Then a < b, <written-by> and <3. End <spo";
        assert_eq!(versions(reply).written, reply);
        assert_eq!(versions(reply).spoken, None);
        assert_eq!(
            shown(&refs(&by_char(reply))),
            reply.trim_end_matches("<spo")
        );
        assert_eq!(
            said(&refs(&by_char(reply))),
            [
                "Use a <spokesperson> or <spoke> tag, a <b>bold</b> word and Vec<String>.",
                "Then a < b, <written-by> and <3.",
                "End <spo"
            ]
        );
        // Tags inside the written version other than its end are stray.
        assert_eq!(
            versions("<spoken>Yes.</spoken><written>Wrap it in <spoken> tags.</written>").written,
            "Wrap it in  tags."
        );
    }

    #[test]
    fn a_stray_tag_in_a_typed_reply_is_never_shown() {
        assert_eq!(versions("Fixed.</written>").written, "Fixed.");
        assert_eq!(shown(&["Fixed.</wri", "tten>"]), "Fixed.");
        let typed = versions("<written>\nThe answer is 4.\n</written>");
        assert_eq!(typed.spoken, None);
        assert_eq!(typed.written, "The answer is 4.");
        assert_eq!(
            versions("On it.\n<written>Done.</written>").written,
            "On it.\n\nDone."
        );
    }

    #[test]
    fn a_written_version_that_carries_on_from_the_spoken_one_is_flagged() {
        let spoken = "The build fails because the key is missing.";
        for written in [
            "Also, the port is in use.",
            "**Additionally** the port is in use.",
            "Here's the rest: the port is in use.",
            "Here\u{2019}s the rest: the port.",
            "As I said, the key is missing.",
        ] {
            assert_eq!(
                lazy(spoken, written),
                Some("opens with a continuation word"),
                "{written}"
            );
        }
        assert_eq!(
            lazy(
                spoken,
                "The build fails because the API key is missing.\n\n1. Set `API_KEY`."
            ),
            Some("repeats the spoken version")
        );
        for written in [
            "The build fails for two reasons:\n1. **Missing API key.**",
            "Alsatian config is fine; the key is missing.",
            "Missing `API_KEY`. Set it in `.env` and the build passes.",
        ] {
            assert_eq!(lazy(spoken, written), None, "{written}");
        }
        // Too short to call a repeat.
        assert_eq!(lazy("Yes, done.", "Yes, done. The diff is below."), None);
        // Read from a reply, only when it wrote both versions.
        assert!(
            versions("<spoken>It's merged now.</spoken><written>Also: CI is green.</written>")
                .lazy
                .is_some()
        );
        assert_eq!(
            versions("<spoken>It's merged and the tests pass.</spoken>").lazy,
            None
        );
    }

    #[test]
    fn a_reply_is_counted_by_how_it_was_written() {
        for (reply, expected) in [
            (
                "<spoken>It's green.</spoken>\n<written>All 42 pass.</written>",
                Path::Both,
            ),
            (
                "Let me look. < Spoken >It's green.</SPOKEN><written>All 42 pass.",
                Path::Both,
            ),
            ("<spoken>It's green.</spoken>", Path::SpokenOnly),
            (
                "<spoken>Two fixes.</spoken>\nFirst, set the key.",
                Path::SpokenOnly,
            ),
            (
                "<spoken>Done, and merged. <written>Merged as **#42**.",
                Path::Unclosed,
            ),
            ("<spoken>All done.\n", Path::Unclosed),
            (
                "<spoken>Yes.</written><written>Yes, it's merged.</written>",
                Path::Unclosed,
            ),
            ("Here's the fix:\n```rust\nlet x = 1;\n```", Path::Untagged),
            ("<written>The answer is 4.</written>", Path::Untagged),
            (
                "<written>Four.</written><spoken>Four.</spoken>",
                Path::Untagged,
            ),
            ("Use a <spokesperson> or Vec<String>.", Path::Untagged),
            ("", Path::Untagged),
        ] {
            assert_eq!(path(reply), expected, "{reply:?}");
        }
        assert_eq!(
            [Path::Both, Path::SpokenOnly, Path::Unclosed, Path::Untagged].map(Path::name),
            ["both", "spokenOnly", "unclosed", "untagged"]
        );
    }

    #[test]
    fn the_model_is_shown_both_versions_in_order() {
        assert_eq!(
            both("It's green.", "| job | ok |"),
            "<spoken>It's green.</spoken>\n<written>| job | ok |</written>"
        );
        let again = versions(&both("It's green.", "| job | ok |"));
        assert_eq!(again.spoken.as_deref(), Some("It's green."));
        assert_eq!(again.written, "| job | ok |");
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
