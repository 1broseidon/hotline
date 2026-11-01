//! The thread index: where a teammate's other threads stand, told at the start
//! of each of its turns (`docs/design/thread-awareness.md` §1, decision 3).
//!
//! An agent is one identity with many threads, and in any one of them it
//! should know what the others are doing without having to think to ask. So
//! the line each turn of the main conversation, a work thread or a call opens
//! with carries a short index of the teammate's threads: those working or
//! open, those waiting on the person, those that ended in the last day
//! (chapters of the main conversation among them), and how many older ones
//! `search_thread` can find. The asides a turn hears are in the same block,
//! through [`super::asides::render`], so they are told once.
//!
//! A job's firing and a subagent's run are narrower work: they are told their
//! parent thread's line and the asides about them, nothing more.
//!
//! It rides on the turn's line, never in the preamble: a preamble that changed
//! every turn would be a new prefix every turn, and a harness that caches the
//! preamble (Hotline Agent marks it, `Reuse::Rounds`) would pay for it in
//! full. A turn whose index has not moved since that agent's last turn in the
//! thread is told so in one line instead of the whole block.

use super::sides::cut;
use super::{Room, lock, now_ms, timed};
use crate::contract::{Aside, End, SessionState, ThreadId, ThreadKind};
use crate::thread::{Link, Participant, Thread, ThreadState, ThreadStore};
use chrono::{Local, TimeZone};
use serde_json::Value;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// The most threads an index lists. The rest are counted with the older ones.
pub(super) const MOST: usize = 15;
/// How long an ended thread is still listed: ended today, give or take.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// A title, and a thread's last line, as an index quotes them, cut at a
/// word. The key `read_thread` takes ends each line, so these come first.
const TITLE_CHARS: usize = 56;
const SAID_CHARS: usize = 60;
/// What came of the teammate's own work, as its entry says it.
const MINE_SAID_CHARS: usize = 64;

const UNCHANGED: &str =
    "(Thread index: none of your other threads has moved since your last turn here.)";
/// How the block ends, so the line after it reads as the turn's own.
const OVER: &str = "The index is over.)";

/// Where an entry is listed. The order is the order the cap keeps them in:
/// what waits on the person first, then what is under way, then what ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Section {
    Waiting,
    Active,
    Ended,
    /// A job's or a run's one line: the thread it hangs off.
    Parent,
}

/// One thread as the index lists it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Entry {
    pub section: Section,
    /// What the agent calls it: main, work, pair, call, run, job.
    pub kind: &'static str,
    /// The key `read_thread` takes. None for a chapter, which is a stretch of
    /// the main conversation, found by `search_thread`.
    pub key: Option<String>,
    pub title: String,
    /// working, open, card, idle, parked, ended, failed, stopped.
    pub state: &'static str,
    /// When it last moved.
    pub moved: i64,
    /// Its last line, in a few words.
    pub said: Option<String>,
    /// The teammate's own work thread, told in the second person as work it
    /// did itself: `title` is what it was, `said` what came of it.
    pub mine: bool,
    /// Whom its own work was for, when a colleague handed it over or asked:
    /// the colleague's name.
    pub whom: Option<String>,
    /// How its own work went on after the ask that opened it, when another
    /// handoff, question or word of the person's followed: "then updated it".
    /// `said` is then what came of the latest.
    pub then: Option<&'static str>,
}

/// What a thread keeps of the index its agent was last told, for as long as
/// that agent lasts: the next turn is told only what moved since.
pub(super) type Seen = Mutex<Option<Index>>;

/// How much of a long key the index shows. A thread's key is a UUID, and a
/// pair's two of them; the first characters tell a teammate's threads apart,
/// and `read_thread` and `aside` take the short form.
const KEY_CHARS: usize = 8;

/// A thread's key as the index shows it: a long part cut to [`KEY_CHARS`],
/// a short one (a persona's name, a test's "crane") whole.
pub(super) fn short(thread: &ThreadId) -> String {
    let key = thread
        .key
        .split('~')
        .map(|part| {
            if part.chars().count() > KEY_CHARS + 4 {
                part.chars().take(KEY_CHARS).collect()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("~");
    match &thread.desk {
        Some(desk) => format!("{desk}/{}:{key}", thread.kind.name()),
        None => format!("{}:{key}", thread.kind.name()),
    }
}

/// Whether `short` abbreviates `full` the way [`short`] does: part for part,
/// each a prefix.
fn abbreviates(short: &str, full: &str) -> bool {
    let shorts: Vec<&str> = short.split('~').collect();
    let fulls: Vec<&str> = full.split('~').collect();
    shorts.len() == fulls.len()
        && shorts
            .iter()
            .zip(&fulls)
            .all(|(short, full)| !short.is_empty() && full.starts_with(short))
}

/// A teammate's threads, ready to be told.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct Index {
    pub entries: Vec<Entry>,
    /// Ended threads and chapters older than a day, and any the cap left out.
    pub older: usize,
}

impl Index {
    /// Keeps the [`MOST`] that matter most, newest first within a section,
    /// and counts the rest as older.
    pub(super) fn capped(mut entries: Vec<Entry>, older: usize) -> Self {
        entries.sort_by_key(|entry| (entry.section, std::cmp::Reverse(entry.moved)));
        let cut = entries.len().saturating_sub(MOST);
        entries.truncate(MOST);
        Self {
            entries,
            older: older + cut,
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.older == 0
    }

    /// What is compared turn to turn: each thread's state, not its clock or
    /// its last line. A colleague's thread that only said more is not news
    /// enough to be told again; one that started, ended or raised a card is.
    fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for entry in &self.entries {
            entry.state_key().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The fenced list, headed by whose threads they are and when.
    pub(super) fn render(&self, name: &str, person: Option<&str>, now: i64) -> String {
        let listed: Vec<&Entry> = self.entries.iter().collect();
        block(name, person, now, &listed, &[], self.older, false)
    }

    /// Only what moved since `before`: the entries whose state is new, under
    /// their sections, and those no longer listed under GONE. What was told
    /// before stands, so the turn reads a few lines, not the whole index.
    pub(super) fn render_changes(
        &self,
        before: &Index,
        name: &str,
        person: Option<&str>,
        now: i64,
    ) -> String {
        let changed: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| {
                !before
                    .entries
                    .iter()
                    .any(|old| old.state_key() == entry.state_key())
            })
            .collect();
        let gone: Vec<&Entry> = before
            .entries
            .iter()
            .filter(|old| !self.entries.iter().any(|entry| entry.is(old)))
            .collect();
        block(name, person, now, &changed, &gone, self.older, true)
    }
}

/// The fenced list: `listed` under their sections, `gone` after them, and
/// the count of older threads. `changes` heads it as what moved since the
/// agent's last turn, rather than the whole.
fn block(
    name: &str,
    person: Option<&str>,
    now: i64,
    listed: &[&Entry],
    gone: &[&Entry],
    older: usize,
    changes: bool,
) -> String {
    let at = Local
        .timestamp_millis_opt(now)
        .single()
        .unwrap_or_else(Local::now);
    let since = if changes {
        " · what moved since your last turn here"
    } else {
        ""
    };
    let mut lines = vec![format!(
        "THREADS · {name} · {}{since}",
        at.format("%a %-d %b %Y, %H:%M")
    )];
    let person = person.map_or_else(|| "THE PERSON".to_string(), str::to_uppercase);
    for (section, heading) in [
        (Section::Parent, "PARENT".to_string()),
        (Section::Active, "ACTIVE".to_string()),
        (Section::Waiting, format!("WAITING ON {person}")),
        (Section::Ended, "ENDED TODAY".to_string()),
    ] {
        let in_section: Vec<&&Entry> = listed
            .iter()
            .filter(|entry| entry.section == section)
            .collect();
        if in_section.is_empty() {
            continue;
        }
        lines.push(format!("{heading} {}", in_section.len()));
        lines.extend(in_section.into_iter().map(|entry| entry.line(now)));
    }
    if !gone.is_empty() {
        lines.push(format!("GONE {}", gone.len()));
        lines.extend(gone.iter().map(|entry| entry.gone_line()));
    }
    if older > 0 && (!changes || listed.is_empty() && gone.is_empty() || !gone.is_empty()) {
        lines.push(format!("{older} older · search_thread finds them"));
    }
    crate::fence::fenced("hotline_threads", &lines.join("\n"))
}

/// An entry's state: section, kind, key, title, state, mine, whom, then.
type StateKey<'a> = (
    Section,
    &'a str,
    Option<&'a str>,
    &'a str,
    &'a str,
    bool,
    Option<&'a str>,
    Option<&'a str>,
);

impl Entry {
    /// What the fingerprint compares and a delta lists by: everything but
    /// the clock and the last line.
    fn state_key(&self) -> StateKey<'_> {
        (
            self.section,
            self.kind,
            self.key.as_deref(),
            self.title.as_str(),
            self.state,
            self.mine,
            self.whom.as_deref(),
            self.then,
        )
    }

    /// Whether this is the same thread as `other`: by key, or for a chapter
    /// by title.
    fn is(&self, other: &Entry) -> bool {
        match (&self.key, &other.key) {
            (Some(key), Some(other)) => key == other,
            (None, None) => self.title == other.title,
            _ => false,
        }
    }

    /// The line for a thread the index no longer lists: what it was, and the
    /// state it was last told in.
    fn gone_line(&self) -> String {
        let key = self
            .key
            .as_deref()
            .map_or_else(String::new, |key| format!(" · {key}"));
        let kind = if self.key.is_some() { "" } else { " chapter" };
        format!(
            "{}{kind} · {} · was {}{key}",
            self.kind, self.title, self.state
        )
    }

    /// The entry as a line: what it is first, and the key `read_thread`
    /// takes at the end, after the words a turn reads it by.
    fn line(&self, now: i64) -> String {
        let when = match self.section {
            Section::Ended => Local
                .timestamp_millis_opt(self.moved)
                .single()
                .map_or_else(String::new, |at| at.format("%H:%M").to_string()),
            _ => since(now - self.moved),
        };
        let key = self
            .key
            .as_deref()
            .map_or_else(String::new, |key| format!(" · {key}"));
        if self.mine {
            return format!("{}{key}", self.mine_line(&when));
        }
        let said = self
            .said
            .as_deref()
            .map_or_else(String::new, |said| format!(" · \"{said}\""));
        let kind = if self.key.is_some() { "" } else { " chapter" };
        format!(
            "{}{kind} · {} · {} {when}{said}{key}",
            self.kind, self.title, self.state
        )
    }

    /// The teammate's own work, as its own: "you did "the notice" for
    /// Goldie (ended 15:48): "saved as notice.md"".
    fn mine_line(&self, when: &str) -> String {
        let verb = match (self.section, self.state) {
            (Section::Ended, _) => "you did",
            (_, "working") => "you're doing",
            _ => "you're on",
        };
        let whom = self
            .whom
            .as_deref()
            .map_or_else(String::new, |whom| format!(" for {whom}"));
        let then = self
            .then
            .map_or_else(String::new, |then| format!(", {then}"));
        let said = self
            .said
            .as_deref()
            .map_or_else(String::new, |said| format!(": \"{said}\""));
        format!(
            "{verb} \"{}\"{whom}{then} ({} {when}){said}",
            self.title, self.state
        )
    }
}

/// How long ago, as a list says it: now, 4m, 3h, 2d.
fn since(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "now".to_string(),
        1..60 => format!("{minutes}m"),
        60..2_880 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1_440),
    }
}

/// What the agent calls a kind.
fn kind_word(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Dm => "main",
        ThreadKind::Side => "work",
        ThreadKind::Pair => "pair",
        ThreadKind::Call => "call",
        ThreadKind::Run => "run",
        ThreadKind::Job => "job",
    }
}

fn ended_word(end: End) -> &'static str {
    match end {
        End::Failed => "failed",
        End::Stopped | End::Cancelled => "stopped",
        _ => "ended",
    }
}

/// What the newest card still up in a thread asks the person.
fn card_of(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        let pending = match event.get("kind").and_then(Value::as_str)? {
            "permission" => event.get("decision").is_none(),
            "human_action" | "credential_ask" => {
                event.get("status").and_then(Value::as_str) == Some("pending")
            }
            _ => false,
        };
        pending
            .then(|| {
                ["reason", "title"]
                    .iter()
                    .find_map(|key| event.get(*key)?.as_str())
            })
            .flatten()
            .map(str::to_string)
    })
}

/// What the thread's agent last said.
fn last_words(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event["kind"] == "agent")
        .and_then(|event| event.get("text").and_then(Value::as_str))
        .map(str::to_string)
}

/// The teammate's own work thread as its entry: what the work was (the
/// title its closing note gave it, else the ask that opened it, else its
/// title), whom it was for, and what came of it (the note's outcome, else
/// the last thing it said, else the outcome its link was closed with), in
/// plain words and its own voice. Work asked of it again in the same thread
/// says so ("then updated it"), and what came of it is then the latest:
/// the answer the turn the latest ask opened gave, since a closing note
/// leads with what the first ask came to and the index keeps only its head.
fn mine(
    name: Option<&str>,
    thread: &Thread,
    section: Section,
    state: &'static str,
    moved: i64,
    events: &[Value],
    link: Option<Link>,
) -> Entry {
    let asked = asked_by(events);
    let title = thread
        .title
        .clone()
        .filter(|title| !title.trim().is_empty());
    let what = match (&thread.note, &asked, title) {
        (None, Some((_, ask)), _) => ask.clone(),
        (_, _, Some(title)) => title,
        (_, _, None) => "untitled".to_string(),
    };
    let asks = asks_in(events);
    let latest = asks.last().filter(|_| asks.len() > 1).map(|&at| {
        let question = events[at]["cause"]["kind"] == "question";
        let then = match (state == "working", question) {
            (true, true) => "now answering a question about it",
            (true, false) => "now updating it",
            (false, true) => "then answered a question about it",
            (false, false) => "then updated it",
        };
        (then, answer_since(&events[at..]))
    });
    let said = match (section, &latest) {
        (Section::Waiting, _) => card_of(events),
        (_, Some((_, answer))) => answer.clone(),
        (Section::Ended, None) => thread.note.as_deref().and_then(super::sides::outcome_line),
        _ => None,
    }
    .or_else(|| last_words(events))
    .or_else(|| link.as_ref().and_then(|link| link.outcome.clone()))
    .or_else(|| super::preview_line(events));
    Entry {
        section,
        kind: kind_word(thread.kind()),
        key: Some(short(&thread.id)),
        title: cut(&plain(&what), TITLE_CHARS),
        state,
        moved,
        said: said
            .map(|said| cut(&own_words(&plain(&said), name), MINE_SAID_CHARS))
            .filter(|said| !said.is_empty()),
        mine: true,
        whom: link
            .and_then(|link| link.opener)
            .map(|opener| opener.name)
            .or_else(|| asked.map(|(name, _)| name)),
        then: latest.map(|(then, _)| then),
    }
}

/// What the turn the latest ask opened answered it with: of what it said
/// after its last tool card, the message that answers (`text::answer`, not a
/// remark it closed on), else its last words.
fn answer_since(events: &[Value]) -> Option<String> {
    let after = events
        .iter()
        .rposition(|event| event["kind"] == "tool")
        .map_or(events, |at| &events[at + 1..]);
    hotline_room::text::answer(
        after
            .iter()
            .filter(|event| event["kind"] == "agent")
            .filter_map(|event| event["text"].as_str()),
    )
    .or_else(|| last_words(events))
}

/// Where each turn of a work thread was asked for: a colleague's handoff or
/// question delivered into it, or the person's words, in order.
fn asks_in(events: &[Value]) -> Vec<usize> {
    events
        .iter()
        .enumerate()
        .filter(|(_, event)| match event["kind"].as_str() {
            Some("user") => true,
            Some("delivery") => matches!(
                event["cause"]["kind"].as_str(),
                Some("handoff" | "question")
            ),
            _ => false,
        })
        .map(|(at, _)| at)
        .collect()
}

/// What came of the teammate's own work, in its own voice: an outcome that
/// names who did it ("The teammate wrote…", "Bob wrote…") starts at the verb
/// instead ("Wrote…"), since the index tells it as work the teammate did.
fn own_words(said: &str, name: Option<&str>) -> String {
    let said = said.trim_start();
    let rest = ["The teammate ", "the teammate ", "Teammate "]
        .iter()
        .find_map(|subject| said.strip_prefix(subject))
        .or_else(|| name.and_then(|name| said.strip_prefix(name)?.strip_prefix(' ')));
    let Some(rest) = rest.filter(|rest| {
        rest.starts_with(char::is_lowercase)
            && !["and ", "or ", "with "]
                .iter()
                .any(|joined| rest.starts_with(joined))
    }) else {
        return said.to_string();
    };
    let mut chars = rest.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Who asked for the work a thread was opened with, and what they asked:
/// the first colleague's handoff or question delivered into it, by its
/// `about`, without the greeting a message opens with.
fn asked_by(events: &[Value]) -> Option<(String, String)> {
    events.iter().find_map(|event| {
        let cause = event.get("cause").filter(|_| event["kind"] == "delivery")?;
        if !matches!(cause["kind"].as_str(), Some("handoff" | "question")) {
            return None;
        }
        let name = cause["name"].as_str()?.to_string();
        let about = cause["about"].as_str()?;
        Some((name, without_greeting(about).to_string()))
    })
}

/// A message to a colleague opens like a letter ("Hi Quill, it's Goldie.");
/// what it asks starts after that sentence.
fn without_greeting(about: &str) -> &str {
    let greets = ["Hi ", "Hey ", "Hello "]
        .iter()
        .any(|greeting| about.starts_with(greeting));
    let after = about
        .find(". ")
        .map(|end| about[end + 2..].trim_start())
        .filter(|rest| !rest.is_empty());
    match after {
        Some(rest) if greets => rest,
        _ => about,
    }
}

/// Markdown said as plain words: no emphasis, code ticks, headings, quotes
/// or list marks. The line is flattened when it is cut.
fn plain(text: &str) -> String {
    text.lines()
        .map(|line| {
            let line = line
                .trim_start()
                .trim_start_matches(['#', '>'])
                .trim_start();
            line.strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace("**", "")
        .replace("__", "")
        .replace('`', "")
}

fn ts_of(events: &[Value]) -> Option<i64> {
    events
        .iter()
        .filter_map(|event| event.get("ts").and_then(Value::as_i64))
        .max()
}

impl Room {
    /// The block a turn of `persona_id`'s in `thread` opens with: the index of
    /// its other threads and the asides it hears now, or one line saying
    /// nothing moved since the turn before it, or nothing when there is
    /// nothing to tell. `seen` is what the agent in this thread was last told,
    /// kept by the thread for as long as that agent lasts; a kind that has
    /// none (a job's firing, a run) is told in full each turn.
    pub(super) fn thread_index(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        asides: &[(String, Aside)],
        seen: Option<&Seen>,
    ) -> Option<String> {
        let now = now_ms();
        let index = match thread.kind {
            ThreadKind::Job | ThreadKind::Run => Index {
                entries: vec![self.main_entry(persona_id, Section::Parent)],
                older: 0,
            },
            _ => self.index_of(persona_id, thread, now),
        };
        // What the agent was last told here, if this kind keeps it: None for
        // a kind that does not, Some(None) for an agent's first turn.
        let before = seen.map(|seen| lock(seen).replace(index.clone()));
        let moved = match &before {
            Some(Some(before)) => before.fingerprint() != index.fingerprint(),
            _ => true,
        };
        if asides.is_empty() && (index.is_empty() || !moved) {
            return (!index.is_empty()).then(|| timed(now, UNCHANGED));
        }
        let name = self
            .persona(persona_id)
            .map_or_else(|_| persona_id.to_string(), |persona| persona.name);
        let person = self.person_name();
        let listed = match &before {
            Some(Some(_)) if !moved => None,
            Some(Some(before)) => Some(index.render_changes(before, &name, person.as_deref(), now)),
            _ => (!index.is_empty()).then(|| index.render(&name, person.as_deref(), now)),
        };
        let heard: Vec<(String, &Aside)> = asides
            .iter()
            .map(|(name, aside)| (name.clone(), aside))
            .collect();
        let told = (!heard.is_empty()).then(|| super::asides::render(&heard));
        // The rules for reading it are in the preamble (`HOW_TO_USE`), told
        // once; the block itself is only the facts.
        Some(timed(
            now,
            &format!(
                "(Thread index:\n{}\n{OVER}",
                [listed, told]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ))
    }

    /// One of the teammate's own threads, by its full key or the short one
    /// the index shows ([`short`]): the main conversation, a pair it is one
    /// side of, or a work thread, call, run or job that hangs off its DM.
    /// None when no thread of theirs matches, or more than one does.
    pub(crate) fn thread_named(&self, persona_id: &str, named: &str) -> Option<ThreadId> {
        let wanted = ThreadId::parse(named.trim())?;
        if wanted.kind == ThreadKind::Dm && wanted.key == persona_id {
            return Some(wanted);
        }
        let store = ThreadStore::new(&self.log);
        if store.load(&wanted).is_some() {
            return Some(wanted);
        }
        let mut found = store
            .list(persona_id)
            .into_iter()
            .map(|thread| thread.id)
            .chain(std::iter::once(ThreadId::dm(persona_id)))
            .filter(|thread| {
                thread.kind == wanted.kind
                    && thread.desk == wanted.desk
                    && abbreviates(&wanted.key, &thread.key)
            });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Every thread of the teammate's but `this` one, sorted into what is
    /// under way, what waits on the person and what ended in the last day,
    /// with the main conversation's chapters as its ended threads.
    pub(super) fn index_of(&self, persona_id: &str, this: &ThreadId, now: i64) -> Index {
        let mut entries = Vec::new();
        let mut older = 0;
        if this.kind != ThreadKind::Dm || this.key != persona_id {
            let main = self.main_entry(persona_id, Section::Active);
            if main.state == "card" {
                entries.push(Entry {
                    section: Section::Waiting,
                    ..main
                });
            } else if main.state == "working" {
                entries.push(main);
            }
        }
        for chapter in crate::store::chapters::list(&self.log, persona_id) {
            let (Some(ended), Some(title)) = (
                chapter.get("endedAt").and_then(Value::as_i64),
                chapter.get("title").and_then(Value::as_str),
            ) else {
                continue;
            };
            if now - ended > DAY_MS {
                older += 1;
                continue;
            }
            entries.push(Entry {
                section: Section::Ended,
                kind: "main",
                key: None,
                title: cut(title, TITLE_CHARS),
                state: "ended",
                moved: ended,
                said: chapter
                    .get("note")
                    .and_then(Value::as_str)
                    .and_then(|note| note.lines().next())
                    .and_then(|line| line.split(". ").next())
                    .map(|first| cut(first, SAID_CHARS))
                    .filter(|said| !said.is_empty()),
                mine: false,
                whom: None,
                then: None,
            });
        }
        for found in ThreadStore::new(&self.log).list(persona_id) {
            if found.id == *this || found.kind() == ThreadKind::Dm {
                continue;
            }
            match self.entry_of(persona_id, &found, now) {
                Some(entry) => entries.push(entry),
                None => older += 1,
            }
        }
        Index::capped(entries, older)
    }

    /// The main conversation as one entry: working, waiting on a card, or
    /// idle, with the last thing said in it.
    fn main_entry(&self, persona_id: &str, section: Section) -> Entry {
        let root = self.log.root();
        let tail = crate::store::previews::tail(root, persona_id);
        let waiting = crate::wire::waiting_on(&tail);
        let said = waiting.then(|| card_of(&tail)).flatten().or_else(|| {
            crate::store::previews::preview_reaching(root, persona_id, &tail).and_then(|found| {
                found
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        });
        let working = matches!(
            self.info(persona_id).state,
            SessionState::Thinking | SessionState::Starting
        ) || self.session(persona_id).is_ok_and(|session| {
            let turns = lock(&session.turns);
            turns.running || !turns.waiting.is_empty()
        });
        Entry {
            section,
            kind: "main",
            key: Some(short(&ThreadId::dm(persona_id))),
            title: "main conversation".to_string(),
            state: if waiting {
                "card"
            } else if working {
                "working"
            } else {
                "idle"
            },
            moved: ts_of(&tail).unwrap_or_default(),
            said: said.map(|said| cut(&said, SAID_CHARS)),
            mine: false,
            whom: None,
            then: None,
        }
    }

    /// A thread other than the main conversation, or None when it is older
    /// than the index lists.
    fn entry_of(&self, persona_id: &str, thread: &Thread, now: i64) -> Option<Entry> {
        let link = self.log.thread_link(&thread.id);
        // A thread that closed more than a day ago is counted, not read.
        if let (ThreadState::Closed(_), Some(at)) =
            (thread.state, link.as_ref().and_then(|link| link.at))
            && now - at > DAY_MS
        {
            return None;
        }
        let events = crate::log::StreamId::of_thread(&thread.id)
            .map(|stream| self.log.load(&stream))
            .unwrap_or_default();
        let moved = ts_of(&events)
            .into_iter()
            .chain(link.as_ref().and_then(|link| link.at))
            .max()
            .unwrap_or_default();
        let live = match thread.kind() {
            ThreadKind::Side | ThreadKind::Call => self
                .sides
                .get(&thread.id.key)
                .filter(|side| side.thread == thread.id),
            _ => None,
        };
        let working = match thread.kind() {
            ThreadKind::Side | ThreadKind::Call => {
                live.as_ref().is_some_and(|side| side.turn_running())
            }
            ThreadKind::Pair => self.pair_answering(&thread.id.key).is_some(),
            ThreadKind::Job => self.working_jobs(persona_id).contains(&thread.id.key),
            ThreadKind::Run => thread.state == ThreadState::Live,
            ThreadKind::Dm => false,
        };
        let closed = matches!(thread.state, ThreadState::Closed(_));
        let (section, state) = if !closed && crate::wire::waiting_on(&events) {
            (Section::Waiting, "card")
        } else if working {
            (Section::Active, "working")
        } else if live.is_some() && thread.state == ThreadState::Live {
            (Section::Active, "open")
        } else if now - moved > DAY_MS {
            return None;
        } else {
            let state = match thread.state {
                ThreadState::Closed(end) => ended_word(end),
                ThreadState::Live if thread.kind() == ThreadKind::Job => "idle",
                _ => "parked",
            };
            (Section::Ended, state)
        };
        // Work the teammate handed a colleague is quoted in the colleague's
        // words only: what the person says in it never reaches its opener.
        let handed = thread
            .parent
            .as_ref()
            .is_some_and(|parent| parent.thread.key != persona_id);
        if thread.kind() == ThreadKind::Side && !handed {
            let name = self.persona(persona_id).ok().map(|persona| persona.name);
            return Some(mine(
                name.as_deref(),
                thread,
                section,
                state,
                moved,
                &events,
                link,
            ));
        }
        let said = match section {
            Section::Waiting => card_of(&events),
            Section::Ended => link.as_ref().and_then(|link| link.outcome.clone()),
            _ => None,
        }
        .or_else(|| {
            if handed {
                last_words(&events)
            } else {
                super::preview_line(&events)
            }
        });
        Some(Entry {
            section,
            kind: kind_word(thread.kind()),
            key: Some(short(&thread.id)),
            title: cut(&self.title_of(persona_id, thread), TITLE_CHARS),
            state,
            moved,
            said: said.map(|said| cut(&said, SAID_CHARS)),
            mine: false,
            whom: None,
            then: None,
        })
    }

    /// A thread's name in the index: its title, for a pair whom it is with,
    /// and for work the teammate handed a colleague, whose thread it is.
    fn title_of(&self, persona_id: &str, thread: &Thread) -> String {
        let name_of = |id: &str| {
            self.participant(id)
                .map_or_else(|_| id.to_string(), |persona| super::links::shown(&persona))
        };
        if thread.kind() == ThreadKind::Pair {
            let with = thread.participants.iter().find_map(|who| match who {
                Participant::Persona(id) if id != persona_id => Some(id),
                _ => None,
            });
            return with.map_or_else(
                || "with a colleague".to_string(),
                |id| format!("with {}", name_of(id)),
            );
        }
        let title = thread
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| "untitled".to_string());
        match &thread.parent {
            Some(parent) if parent.thread.key != persona_id => {
                format!("handed to {}: {title}", name_of(&parent.thread.key))
            }
            _ => title,
        }
    }
}

/// A turn's line without the index it opens with: what the turn was asked,
/// as a call's summary of it reads it.
pub(super) fn asked(line: &str) -> &str {
    line.strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .and_then(|(_, words)| split(words))
        .map_or(line, |(_, rest)| rest)
}

/// Words that open with the index, split into the index and the rest.
pub(super) fn split(words: &str) -> Option<(&str, &str)> {
    if let Some(rest) = words.strip_prefix(UNCHANGED) {
        return Some((UNCHANGED, rest.trim_start_matches('\n')));
    }
    if !words.starts_with("(Thread index:") {
        return None;
    }
    let end = words.find(OVER)? + OVER.len();
    Some((&words[..end], words[end..].trim_start_matches('\n')))
}

#[cfg(test)]
mod tests;
