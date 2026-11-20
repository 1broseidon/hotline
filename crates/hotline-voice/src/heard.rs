//! What the caller heard of what a call said, once they stop it.
//!
//! A line's sound goes out as clips: each sentence one clip, or a run of
//! them when it streams. When the person cuts in, holds or hangs up, the
//! window reports the last clip that began to play and how far into it
//! playback got ([`VoicePlayed`]). From that alone, every sentence whose
//! sound played to its end was heard, every one whose sound never began was
//! not, and at most one sentence was cut. Where in that sentence the cut
//! fell is exact only from a voice's own word timings; otherwise it is an
//! estimate from playback time, snapped to a word, and the notice says so.
//! Generated words, or a share of a clip's length, are never presented as
//! exactly what was heard (decision-9).

use crate::speech::WordMark;
use crate::spoken::speech_text;
use hotline_contract::VoicePlayed;
use serde_json::{Value, json};
use std::collections::VecDeque;

/// Lines remembered. A notice needs the line the cut fell in and those
/// generated after it, which are never more than an acknowledgement, a line
/// on the work and the reply.
const LINES: usize = 8;
/// A heard or unheard part longer than this many words has its middle left
/// out, with the number of words left out.
const LONG: usize = 40;
/// Characters a second assumed for a streamed sentence whose length is not
/// known yet, before the call has a sentence of its own to measure.
const TYPICAL_RATE: f64 = 14.0;

/// What words a clip carries.
#[derive(Clone, Copy)]
pub(super) enum Words<'a> {
    /// None: a line's empty closing clip.
    None,
    /// The first sound of a sentence, as written; `whole` when the clip is
    /// all of it.
    Sentence {
        written: &'a str,
        whole: bool,
        marks: &'a [WordMark],
    },
    /// More of the sentence before it; the last of it when `ends`.
    More { ends: bool, marks: &'a [WordMark] },
}

impl<'a> Words<'a> {
    /// A sentence said in one clip, by a voice that does not time its words.
    pub(super) fn sentence(written: &'a str) -> Self {
        Words::Sentence {
            written,
            whole: true,
            marks: &[],
        }
    }
}

/// Why playback stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stop {
    CutIn,
    Hold,
    HungUp,
}

/// What the client said of where playback stood.
pub(super) enum Playback {
    /// Nothing: an older client, or the desk stopping the speech itself.
    Unreported,
    /// A live front's reply, which plays as one stream: how much of it played.
    Front(u32),
    /// A Turn by turn call's clips.
    Clip(VoicePlayed),
}

/// One sentence, as its sound went out.
struct Piece {
    written: String,
    /// Its clips' indices and lengths in ms, when the desk can read them.
    clips: Vec<(u32, Option<u32>)>,
    /// All of its sound has gone out.
    whole: bool,
    marks: Vec<WordMark>,
}

impl Piece {
    fn spoken(&self) -> String {
        speech_text(&self.written)
    }

    /// Its length in ms, with the clip the window measured counted as that,
    /// once all of its sound has gone out and every clip's length is known.
    fn length(&self, measured: Option<(u32, u32)>) -> Option<u32> {
        if !self.whole {
            return None;
        }
        self.clips.iter().try_fold(0u32, |sum, &(index, ms)| {
            let ms = match measured {
                Some((at, length)) if at == index && length > 0 => Some(length),
                _ => ms,
            };
            Some(sum + ms?)
        })
    }
}

/// One line said on the call: the words generated for it, as shown, and
/// the sentences given sound.
pub(super) struct Line {
    pub(super) id: String,
    pub(super) text: String,
    /// Kept on the call's thread as it now reads (a front's line).
    pub(super) recorded: bool,
    /// Settled by a cut: nothing more of it is reported.
    cut: bool,
    pieces: Vec<Piece>,
    /// Its last clip has gone out.
    closed: bool,
}

impl Line {
    /// What of its words never had sound: those after its last sentence
    /// given sound, when that sentence is where the line says it is.
    fn never_voiced(&self) -> String {
        let voiced = self
            .pieces
            .iter()
            .map(|piece| piece.written.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        self.text
            .strip_prefix(&voiced)
            .map_or_else(String::new, |rest| speech_text(rest.trim()))
    }

    fn spoken(&self) -> String {
        speech_text(&self.text)
    }
}

/// Where in a sentence the cut fell, and how well that is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    /// From the voice's word timings.
    Exact,
    /// From playback time over the sentence's length.
    Estimated,
    /// From playback time and a speaking rate: the sentence's length was not
    /// known yet.
    Rated,
    /// Between sentences: every one either played to its end or never began.
    Between,
    /// Nothing had begun to play.
    NotStarted,
    /// In a sentence whose sound has no length the desk can read.
    InSentence,
}

/// The cut sentence, split where the cut fell; `after` alone when where is
/// unknown.
struct CutSentence {
    before: Option<String>,
    after: String,
    played_ms: Option<u32>,
    length_ms: Option<u32>,
}

/// What the caller heard.
struct Heard {
    line: String,
    full: Vec<String>,
    cut: Option<CutSentence>,
    unheard: Vec<String>,
    /// The last line was still being said: whatever it went on to say was
    /// not said aloud.
    more: bool,
    position: Position,
}

enum Placed {
    /// The report does not match the lines the desk sent.
    Unknown,
    /// Everything said had played to its end, and nothing more was coming.
    AllHeard,
    Heard(Heard),
}

/// The lines a call has said, and the cut notice its next turn is told.
#[derive(Default)]
pub(super) struct SpeechLine {
    lines: VecDeque<Line>,
    pub(super) pending: Option<String>,
}

impl SpeechLine {
    /// The line said last.
    pub(super) fn current(&self) -> Option<&Line> {
        self.lines.back()
    }

    pub(super) fn current_mut(&mut self) -> Option<&mut Line> {
        self.lines.back_mut()
    }

    /// A line was said, or grew, under `id`.
    pub(super) fn update(&mut self, id: &str, text: &str) {
        if let Some(line) = self.lines.iter_mut().find(|line| line.id == id) {
            if line.text != text {
                line.recorded = false;
            }
            line.text = text.into();
            return;
        }
        if self.lines.len() == LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(Line {
            id: id.into(),
            text: text.into(),
            recorded: false,
            cut: false,
            pieces: Vec::new(),
            closed: false,
        });
    }

    /// Clip `index` of line `id` went out, `ms` long when known, carrying
    /// `words`; `last` closes the line.
    pub(super) fn sent(
        &mut self,
        id: &str,
        index: u32,
        last: bool,
        words: Words<'_>,
        ms: Option<u32>,
    ) {
        let Some(line) = self.lines.iter_mut().find(|line| line.id == id) else {
            return;
        };
        line.closed |= last;
        match words {
            Words::None => {}
            Words::Sentence {
                written,
                whole,
                marks,
            } => line.pieces.push(Piece {
                written: written.into(),
                clips: vec![(index, ms)],
                whole,
                marks: marks.to_vec(),
            }),
            Words::More { ends, marks } => {
                if let Some(piece) = line.pieces.last_mut() {
                    piece.clips.push((index, ms));
                    piece.whole |= ends;
                    piece.marks.extend_from_slice(marks);
                }
            }
        }
    }

    /// The speech was stopped. Writes the notice the next turn is told and
    /// answers the line to keep on the call's thread, or None when nothing
    /// was cut: the line said last was already settled, or everything said
    /// had played to its end.
    pub(super) fn interrupt(&mut self, stop: Stop, playback: Playback) -> Option<Value> {
        let current = self.current()?;
        if current.cut || current.text.is_empty() {
            return None;
        }
        let (text, speech_id, played_ms) = match &playback {
            Playback::Unreported => (unknown(stop, &current.text), current.id.clone(), None),
            Playback::Front(ms) => (
                front(stop, *ms, &current.text),
                current.id.clone(),
                Some(*ms),
            ),
            Playback::Clip(played) => match self.place(played) {
                Placed::Unknown => (unknown(stop, &current.text), current.id.clone(), None),
                Placed::AllHeard => {
                    self.settle();
                    return None;
                }
                Placed::Heard(heard) => (notice(stop, &heard), heard.line, None),
            },
        };
        self.settle();
        self.pending = Some(text.clone());
        Some(json!({
            "kind": "notice",
            "id": uuid::Uuid::new_v4().to_string(),
            "ts": chrono::Local::now().timestamp_millis(),
            "level": "info",
            "text": text,
            "interrupted": true,
            "speechId": speech_id,
            "playedMs": played_ms,
        }))
    }

    /// Every line said so far is settled by the cut.
    fn settle(&mut self) {
        for line in &mut self.lines {
            line.cut = true;
        }
    }

    /// Where `played` puts the cut among the lines not yet settled.
    fn place(&self, played: &VoicePlayed) -> Placed {
        let open: Vec<&Line> = self.lines.iter().filter(|line| !line.cut).collect();
        let Some(last) = open.last() else {
            return Placed::Unknown;
        };
        let more = !last.closed;
        let Some(id) = &played.line else {
            return Placed::Heard(Heard {
                line: last.id.clone(),
                full: Vec::new(),
                cut: None,
                unheard: open.iter().map(|line| line.spoken()).collect(),
                more,
                position: Position::NotStarted,
            });
        };
        let Some(at) = open.iter().position(|line| &line.id == id) else {
            return Placed::Unknown;
        };
        let line = open[at];
        let Some(sentence) = line
            .pieces
            .iter()
            .position(|piece| piece.clips.iter().any(|&(index, _)| index == played.index))
        else {
            return Placed::Unknown;
        };
        let piece = &line.pieces[sentence];
        let mut full: Vec<String> = line.pieces[..sentence].iter().map(Piece::spoken).collect();
        let clip = piece
            .clips
            .iter()
            .position(|&(index, _)| index == played.index)
            .unwrap_or_default();
        let ended = played.clip_ms > 0 && played.ms >= played.clip_ms;
        let (cut, position) = if ended && clip + 1 == piece.clips.len() && piece.whole {
            full.push(piece.spoken());
            (None, Position::Between)
        } else {
            let (cut, position) = self.split(piece, clip, played);
            (Some(cut), position)
        };
        let mut unheard: Vec<String> = line.pieces[sentence + 1..]
            .iter()
            .map(Piece::spoken)
            .collect();
        unheard.push(line.never_voiced());
        unheard.extend(open[at + 1..].iter().map(|line| line.spoken()));
        unheard.retain(|part| !part.is_empty());
        if cut.is_none() && unheard.is_empty() && !more {
            return Placed::AllHeard;
        }
        Placed::Heard(Heard {
            line: line.id.clone(),
            full,
            cut,
            unheard,
            more,
            position,
        })
    }

    /// The sentence `piece` cut in its `clip`th clip, at `played.ms` into it.
    fn split(&self, piece: &Piece, clip: usize, played: &VoicePlayed) -> (CutSentence, Position) {
        let spoken = piece.spoken();
        let earlier = piece.clips[..clip]
            .iter()
            .try_fold(0u32, |sum, &(_, ms)| Some(sum + ms?));
        let Some(played_ms) = earlier.map(|ms| ms + played.ms) else {
            let cut = CutSentence {
                before: None,
                after: spoken,
                played_ms: None,
                length_ms: None,
            };
            return (cut, Position::InSentence);
        };
        let length_ms = piece.length(Some((played.index, played.clip_ms)));
        let (at, position) = if piece.marks.is_empty() {
            let chars = spoken.chars().count() as f64;
            let (share, position) = match length_ms {
                Some(length) if length > 0 => (
                    f64::from(played_ms) / f64::from(length),
                    Position::Estimated,
                ),
                _ => (
                    f64::from(played_ms) / 1000.0 * self.rate() / chars.max(1.0),
                    Position::Rated,
                ),
            };
            (nearest_word_end(&spoken, share.clamp(0.0, 1.0)), position)
        } else {
            let at = piece
                .marks
                .iter()
                .filter(|mark| mark.ms <= played_ms)
                .map(|mark| mark.end)
                .max()
                .unwrap_or(0);
            (floor_char(&spoken, at), Position::Exact)
        };
        let cut = CutSentence {
            before: Some(spoken[..at].trim().to_string()),
            after: spoken[at..].trim().to_string(),
            played_ms: Some(played_ms),
            length_ms,
        };
        (cut, position)
    }

    /// The call's speaking rate in characters a second, from the sentences
    /// it has sent whole with lengths it can read.
    fn rate(&self) -> f64 {
        let (chars, ms) = self
            .lines
            .iter()
            .flat_map(|line| &line.pieces)
            .filter_map(|piece| Some((piece.spoken().chars().count(), piece.length(None)?)))
            .fold((0usize, 0u32), |(chars, ms), (c, m)| (chars + c, ms + m));
        if chars == 0 || ms == 0 {
            TYPICAL_RATE
        } else {
            chars as f64 * 1000.0 / f64::from(ms)
        }
    }
}

/// The end of the word nearest `share` of the way through `text`, in bytes;
/// 0 is before its first word.
fn nearest_word_end(text: &str, share: f64) -> usize {
    let target = share * text.chars().count() as f64;
    let mut ends = vec![(0usize, 0usize)];
    let mut chars = text.char_indices().enumerate().peekable();
    while let Some((count, (at, c))) = chars.next() {
        let next_is_space = chars
            .peek()
            .is_none_or(|(_, (_, next))| next.is_whitespace());
        if !c.is_whitespace() && next_is_space {
            ends.push((at + c.len_utf8(), count + 1));
        }
    }
    ends.into_iter()
        .min_by(|(_, a), (_, b)| {
            (*a as f64 - target)
                .abs()
                .total_cmp(&(*b as f64 - target).abs())
        })
        .map_or(0, |(at, _)| at)
}

/// `at`, or the char boundary before it, within `text`.
fn floor_char(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// `text` with its middle left out once it is long, keeping `head` words
/// before and `tail` after.
fn elide(text: &str, head: usize, tail: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= LONG {
        return words.join(" ");
    }
    let left_out = words.len() - head - tail;
    let start = words[..head].join(" ");
    let end = words[words.len() - tail..].join(" ");
    match (start.is_empty(), end.is_empty()) {
        (true, _) => format!("[{left_out} words] … {end}"),
        (_, true) => format!("{start} … [{left_out} words]"),
        _ => format!("{start} … [{left_out} words] … {end}"),
    }
}

fn seconds(ms: u32) -> String {
    format!("{:.1} s", f64::from(ms) / 1000.0)
}

fn what_happened(stop: Stop) -> &'static str {
    match stop {
        Stop::CutIn => "Speech interrupted: the caller cut in.",
        Stop::Hold => "Speech interrupted: the caller put the call on hold.",
        Stop::HungUp => "Speech interrupted: the caller hung up.",
    }
}

/// What the next turn is to do with it; a call that has ended has none.
fn what_now(stop: Stop) -> &'static str {
    match stop {
        Stop::CutIn | Stop::Hold => {
            " Answer what the caller said, and carry on from what was not heard only if it still matters after that."
        }
        Stop::HungUp => "",
    }
}

const NO_CLAIM: &str = " Do not claim a more precise stopping point than this.";

/// The notice when nothing places the cut: the whole line, as before.
fn unknown(stop: Stop, text: &str) -> String {
    format!(
        "{} The caller did not hear this line to the end: {text}\nWhere playback stopped is unknown: the caller's app did not report it.{NO_CLAIM}{}",
        what_happened(stop),
        what_now(stop)
    )
}

fn front(stop: Stop, ms: u32, text: &str) -> String {
    format!(
        "{} Playback stopped {} into this line, which the caller did not hear to the end: {text}\nWhich words that covers is unknown: the live voice gives no word timings.{NO_CLAIM}{}",
        what_happened(stop),
        seconds(ms),
        what_now(stop)
    )
}

fn notice(stop: Stop, heard: &Heard) -> String {
    let mut parts = vec![what_happened(stop).to_string()];
    if !heard.full.is_empty() {
        parts.push(format!(
            "Heard in full: \"{}\"",
            elide(&heard.full.join(" "), 8, 16)
        ));
    }
    if let Some(cut) = &heard.cut {
        let marker = if heard.position == Position::Exact {
            "[cut here]"
        } else {
            "[cut about here]"
        };
        let sentence = match &cut.before {
            Some(before) => [
                elide(before, 4, 16),
                marker.into(),
                elide(&cut.after, 16, 4),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
            None => elide(&cut.after, 16, 8),
        };
        let played = match (cut.played_ms, cut.length_ms, heard.position) {
            (Some(played), Some(length), _) => {
                format!(" ({} of its {} played)", seconds(played), seconds(length))
            }
            (Some(played), None, Position::Rated) => format!(
                " ({} played; its full length was not known yet)",
                seconds(played)
            ),
            (Some(played), None, _) => format!(" ({} played)", seconds(played)),
            _ => String::new(),
        };
        parts.push(format!("Cut off during: \"{sentence}\"{played}"));
    }
    let unheard = elide(&heard.unheard.join(" "), 16, 8);
    match (unheard.is_empty(), heard.more) {
        (false, false) => parts.push(format!("Not heard: \"{unheard}\"")),
        (false, true) => parts.push(format!(
            "Not heard: \"{unheard}\", and anything it went on to say after that."
        )),
        (true, true) => {
            parts.push("Not heard: anything it went on to say after that.".into());
        }
        (true, false) => {}
    }
    let how = match heard.position {
        Position::Exact => "Where it stopped is exact, from the voice's own word timings.",
        Position::Estimated => {
            "Where it stopped in that sentence is estimated from how long it played."
        }
        Position::Rated => {
            "Where it stopped in that sentence is estimated from how long it played and the speaking rate."
        }
        Position::Between => "It stopped between sentences, so this split is certain.",
        Position::NotStarted => "None of it had begun to play, so this is certain.",
        Position::InSentence => "Which words of that sentence were heard is unknown.",
    };
    let claim = match heard.position {
        Position::Estimated | Position::Rated | Position::InSentence => NO_CLAIM,
        _ => "",
    };
    parts.push(format!("{how}{claim}{}", what_now(stop)));
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "The build failed on the linker.";
    const TWO: &str = "It ran out of memory while linking the desk.";
    const THREE: &str = "I can raise the limit and run it again.";

    /// A line of three sentences, each sent as one clip a second long.
    fn three_sentences() -> SpeechLine {
        let mut line = SpeechLine::default();
        line.update("reply", &format!("{ONE} {TWO} {THREE}"));
        for (index, sentence) in [ONE, TWO, THREE].into_iter().enumerate() {
            line.sent(
                "reply",
                index as u32,
                false,
                Words::sentence(sentence),
                Some(1_000),
            );
        }
        line.sent("reply", 3, true, Words::None, None);
        line
    }

    fn played(line: &str, index: u32, ms: u32, clip_ms: u32) -> Playback {
        Playback::Clip(VoicePlayed {
            line: Some(line.into()),
            index,
            ms,
            clip_ms,
        })
    }

    fn cut(line: &mut SpeechLine, playback: Playback) -> String {
        let notice = line.interrupt(Stop::CutIn, playback).expect("a cut");
        let _: hotline_contract::TranscriptEvent =
            serde_json::from_value(notice.clone()).expect("a notice line");
        assert_eq!(notice["interrupted"], true);
        let text = notice["text"].as_str().unwrap().to_string();
        assert_eq!(line.pending.as_deref(), Some(text.as_str()));
        text
    }

    #[test]
    fn sentences_that_played_to_their_end_were_heard_and_those_never_begun_were_not() {
        let mut line = three_sentences();
        let text = cut(&mut line, played("reply", 1, 1_000, 1_000));
        assert_eq!(
            text,
            format!(
                "Speech interrupted: the caller cut in.\nHeard in full: \"{ONE} {TWO}\"\nNot heard: \"{THREE}\"\nIt stopped between sentences, so this split is certain. Answer what the caller said, and carry on from what was not heard only if it still matters after that."
            )
        );
        // Settled: the same line is not reported twice.
        assert!(
            line.interrupt(Stop::CutIn, played("reply", 2, 10, 1_000))
                .is_none()
        );
    }

    #[test]
    fn a_cut_inside_a_sentence_without_timings_is_an_estimate_at_a_word_and_says_so() {
        let mut line = three_sentences();
        // Half of "It ran out of memory while linking the desk." is 22 of 44 characters.
        let text = cut(&mut line, played("reply", 1, 500, 1_000));
        assert!(
            text.contains(&format!("Heard in full: \"{ONE}\"")),
            "{text}"
        );
        assert!(
            text.contains("Cut off during: \"It ran out of memory [cut about here] while linking the desk.\" (0.5 s of its 1.0 s played)"),
            "{text}"
        );
        assert!(text.contains(&format!("Not heard: \"{THREE}\"")), "{text}");
        assert!(text.contains("estimated from how long it played"), "{text}");
        assert!(
            text.contains("Do not claim a more precise stopping point"),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("exact"), "{text}");
    }

    #[test]
    fn a_voice_that_times_its_words_places_the_cut_exactly() {
        let mut line = SpeechLine::default();
        line.update("reply", TWO);
        let spoken = speech_text(TWO);
        // Each word ends 100 ms after the one before it.
        let marks: Vec<WordMark> = spoken
            .match_indices(' ')
            .map(|(at, _)| at)
            .chain([spoken.len()])
            .zip(1..)
            .map(|(end, n)| WordMark { end, ms: n * 100 })
            .collect();
        line.sent(
            "reply",
            0,
            false,
            Words::Sentence {
                written: TWO,
                whole: false,
                marks: &marks[..4],
            },
            Some(400),
        );
        line.sent(
            "reply",
            1,
            true,
            Words::More {
                ends: true,
                marks: &marks[4..],
            },
            Some(500),
        );
        let text = cut(&mut line, played("reply", 1, 260, 500));
        assert!(
            text.contains("Cut off during: \"It ran out of memory while [cut here] linking the desk.\" (0.7 s of its 0.9 s played)"),
            "{text}"
        );
        assert!(
            text.contains("exact, from the voice's own word timings"),
            "{text}"
        );
        assert!(!text.contains("about"), "{text}");
    }

    #[test]
    fn a_streamed_sentence_not_yet_whole_is_estimated_from_the_speaking_rate() {
        let mut line = SpeechLine::default();
        line.update("reply", &format!("{ONE} {TWO}"));
        // 31 characters in 2.2 s: about 14 a second.
        line.sent("reply", 0, false, Words::sentence(ONE), Some(2_200));
        line.sent(
            "reply",
            1,
            false,
            Words::Sentence {
                written: TWO,
                whole: false,
                marks: &[],
            },
            Some(200),
        );
        line.sent(
            "reply",
            2,
            false,
            Words::More {
                ends: false,
                marks: &[],
            },
            Some(500),
        );
        let text = cut(&mut line, played("reply", 2, 300, 500));
        assert!(text.contains("[cut about here]"), "{text}");
        assert!(
            text.contains("(0.5 s played; its full length was not known yet)"),
            "{text}"
        );
        assert!(text.contains("speaking rate"), "{text}");
        assert!(
            text.contains("Not heard: anything it went on to say after that."),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("exact"), "{text}");
    }

    #[test]
    fn nothing_begun_is_all_not_heard_and_certain() {
        let mut line = three_sentences();
        let text = cut(&mut line, Playback::Clip(VoicePlayed::default()));
        assert!(
            text.contains(&format!("Not heard: \"{ONE} {TWO} {THREE}\"")),
            "{text}"
        );
        assert!(!text.contains("Heard in full"), "{text}");
        assert!(text.contains("None of it had begun to play"), "{text}");
    }

    #[test]
    fn a_client_that_reports_nothing_leaves_the_whole_line_and_says_where_is_unknown() {
        let mut line = three_sentences();
        let text = cut(&mut line, Playback::Unreported);
        assert_eq!(
            text,
            format!(
                "Speech interrupted: the caller cut in. The caller did not hear this line to the end: {ONE} {TWO} {THREE}\nWhere playback stopped is unknown: the caller's app did not report it. Do not claim a more precise stopping point than this. Answer what the caller said, and carry on from what was not heard only if it still matters after that."
            )
        );
        // A report naming a clip the desk never sent is no better.
        let mut line = three_sentences();
        let text = cut(&mut line, played("waiting-1", 0, 10, 1_000));
        assert!(text.contains("is unknown"), "{text}");
    }

    #[test]
    fn a_live_front_gives_its_played_time_and_no_words() {
        let mut line = SpeechLine::default();
        line.update("front-1", "First sentence. Unheard ending.");
        let notice = line.interrupt(Stop::CutIn, Playback::Front(420)).unwrap();
        assert_eq!(notice["playedMs"], 420);
        assert_eq!(notice["speechId"], "front-1");
        let text = notice["text"].as_str().unwrap();
        assert!(
            text.contains("Playback stopped 0.4 s into this line"),
            "{text}"
        );
        assert!(text.contains("Unheard ending."), "{text}");
        // A late tail of the same line is not cut again; a new line is.
        line.update("front-1", "A late tail.");
        assert!(line.interrupt(Stop::CutIn, Playback::Front(0)).is_none());
        line.update("front-2", "The new answer.");
        assert!(line.interrupt(Stop::CutIn, Playback::Unreported).is_some());
    }

    #[test]
    fn a_cut_acknowledgement_leaves_the_reply_after_it_unheard() {
        let mut line = SpeechLine::default();
        line.update("ack", "Checking the deploy logs now.");
        line.sent(
            "ack",
            0,
            true,
            Words::sentence("Checking the deploy logs now."),
            Some(1_000),
        );
        line.update("reply", ONE);
        let text = cut(&mut line, played("ack", 0, 300, 1_000));
        assert!(
            text.contains("Cut off during: \"Checking [cut about here] the deploy logs now.\""),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "Not heard: \"{ONE}\", and anything it went on to say"
            )),
            "{text}"
        );
    }

    #[test]
    fn a_hang_up_mid_reply_keeps_what_was_not_said_and_asks_nothing_of_a_next_turn() {
        let mut line = three_sentences();
        let notice = line
            .interrupt(Stop::HungUp, played("reply", 0, 1_000, 1_000))
            .unwrap();
        let text = notice["text"].as_str().unwrap();
        assert!(
            text.starts_with("Speech interrupted: the caller hung up."),
            "{text}"
        );
        assert!(
            text.contains(&format!("Not heard: \"{TWO} {THREE}\"")),
            "{text}"
        );
        assert!(!text.contains("Answer"), "{text}");
    }

    #[test]
    fn a_line_played_to_its_end_and_closed_is_no_cut() {
        let mut line = three_sentences();
        assert!(
            line.interrupt(Stop::CutIn, played("reply", 2, 1_000, 1_000))
                .is_none()
        );
        assert!(line.pending.is_none());
    }

    #[test]
    fn a_long_reply_keeps_the_words_near_the_cut_and_counts_the_rest() {
        let long: Vec<String> = (1..=60).map(|n| format!("word{n}")).collect();
        let heard = format!("{}.", long.join(" "));
        let unheard = format!("{}.", long.join(" ").replace("word", "later"));
        let mut line = SpeechLine::default();
        line.update("reply", &format!("{heard} {unheard}"));
        line.sent("reply", 0, false, Words::sentence(&heard), Some(20_000));
        line.sent("reply", 1, true, Words::sentence(&unheard), Some(20_000));
        let text = cut(&mut line, played("reply", 0, 20_000, 20_000));
        assert!(text.contains("Heard in full: \"word1 word2"), "{text}");
        assert!(text.contains("word8 … [36 words] … word45"), "{text}");
        assert!(text.contains("word60.\""), "{text}");
        assert!(text.contains("Not heard: \"later1"), "{text}");
        assert!(text.contains("later16 … [36 words] … later53"), "{text}");
        assert!(!text.contains("word30"), "{text}");
    }
}
