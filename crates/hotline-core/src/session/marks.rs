//! Hotline's one-line marks on the person's own line, for the moments a
//! reaction is the right answer.
//!
//! A reaction can stand in for words, and the standing text says when
//! ([`super::pacing::HOUSE_STYLE`]). But a teammate told in general that it
//! may react starts reacting by habit, to a fresh request as readily as to a
//! thumbs up. So the permission comes with the situation, in a line of
//! Hotline's said with the person's words, and only where Hotline can tell
//! the situation for certain:
//!
//! - **A reaction of theirs.** A line that is only emoji is a reaction to
//!   what the teammate last said, not a question. Emoji-only is the one rule
//!   with no guessing; a bare "ok" or "thanks" is words, which may answer or
//!   ask anything, so it is left to the teammate. The turn it begins waits
//!   on no reply, so an empty answer ends it with nothing said
//!   ([`crate::driver::Driver::prompt_unasked`]).
//! - **A line said while the teammate works.** It goes into the running turn
//!   (steering, on either agent), and the teammate is told so: a question, a
//!   change of direction or something to decide is answered at once, in
//!   words, which are shown; something that changes nothing, or a small
//!   addition it will fold in, may have a reaction instead. That answer is
//!   the turn's first word since the person spoke, and is shown as one
//!   ([`super::narration::Voice::heard_person`]).
//!
//! The marks describe the moment and quote nothing for the model to copy.

use regex::Regex;
use std::sync::LazyLock;

/// What a line of the person's is, as Hotline can tell for certain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Person {
    /// Words, or anything that is not emoji alone.
    Words,
    /// Emoji and nothing else, with nothing attached.
    Reaction,
}

/// Said after a line of the person's that is only emoji.
pub(crate) const REACTION: &str = "Hotline: the person's message is only an emoji, a reaction to what you last said, not a question. No reply is needed. If it answers something you asked, such as whether to go ahead, act on it. Write something only if there is something important they do not yet know.";

/// Said after a line of the person's that arrives while the teammate works.
pub(crate) const MID_TURN: &str = "Hotline: the person wrote this while you were working. A question, a change of direction or something to decide gets its answer now, in words, before you go back to the work: those words are shown to them. If it changes nothing you are doing, or is a small addition you will fold in, a reaction is enough to show you saw it.";

/// Pictographs, their skin tones, the joiner and the presentation selector,
/// as the window's own test for a line drawn large (`transcriptFolds.ts`),
/// and spaces between them.
static EMOJI_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\p{Extended_Pictographic}|\p{Emoji_Modifier}|\x{200d}|\x{fe0f}|\s)+$")
        .expect("the emoji pattern compiles")
});

/// Whether `text` is emoji and nothing else.
pub(crate) fn emoji_only(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && EMOJI_ONLY.is_match(text)
        && text
            .chars()
            .any(|c| !c.is_whitespace() && c != '\u{200d}' && c != '\u{fe0f}')
}

/// What the person's line `text` is, with `attached` files or not.
pub(super) fn person(text: &str, attached: bool) -> Person {
    if !attached && emoji_only(text) {
        Person::Reaction
    } else {
        Person::Words
    }
}

/// The line as a driver hears it when it begins a turn of its own: an emoji
/// alone carries the reaction's mark.
pub(super) fn at_the_door(text: String, person: Option<Person>) -> String {
    match person {
        Some(Person::Reaction) => format!("{text}\n\n{REACTION}"),
        _ => text,
    }
}

/// The line as a driver hears it when it joins a turn already running: the
/// reaction's mark for an emoji alone, the working mark for anything else of
/// the person's, and a line of anyone else's as it is.
pub(super) fn steered(text: &str, person: Option<Person>) -> String {
    match person {
        Some(Person::Reaction) => format!("{text}\n\n{REACTION}"),
        Some(Person::Words) => format!("{text}\n\n{MID_TURN}"),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Emoji alone, however many and however composed, and nothing else.
    #[test]
    fn only_emoji_is_a_reaction() {
        for reaction in ["👍", " 👍 ", "👍🏽", "❤️", "🎉🎉", "👨‍💻", "🙏 👍"]
        {
            assert!(emoji_only(reaction), "{reaction:?}");
        }
        for words in [
            "ok",
            "thanks",
            "👍 thanks",
            "good, makes sense",
            "",
            "  ",
            "1",
            ":)",
            "#",
            "\u{fe0f}",
        ] {
            assert!(!emoji_only(words), "{words:?}");
        }
        assert_eq!(person("👍", false), Person::Reaction);
        assert_eq!(
            person("👍", true),
            Person::Words,
            "a picture with it is more"
        );
    }

    /// A mark is said once, after the person's words, and a line not the
    /// person's carries none.
    #[test]
    fn the_marks_follow_the_persons_words() {
        assert_eq!(
            at_the_door("👍".into(), Some(Person::Reaction)),
            format!("👍\n\n{REACTION}")
        );
        assert_eq!(at_the_door("do it".into(), Some(Person::Words)), "do it");
        assert_eq!(
            steered("how far?", Some(Person::Words)),
            format!("how far?\n\n{MID_TURN}")
        );
        assert_eq!(
            steered("👍", Some(Person::Reaction)),
            format!("👍\n\n{REACTION}")
        );
        assert_eq!(steered("notice", None), "notice");
    }

    /// Short, Hotline's own, and each says when words are owed.
    #[test]
    fn the_marks_are_short_and_say_when_words_are_owed() {
        for mark in [REACTION, MID_TURN] {
            assert!(mark.starts_with("Hotline: "), "{mark}");
            assert!(mark.split_whitespace().count() < 70, "{mark}");
        }
        assert!(REACTION.contains("No reply is needed."));
        assert!(MID_TURN.contains("gets its answer now, in words"));
        assert!(MID_TURN.contains("those words are shown to them"));
    }
}
