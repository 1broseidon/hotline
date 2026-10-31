//! The thread index on scripted agents: what a seeded room's turns are told,
//! the cap, who is told what, and the line a quiet room gets instead.

use super::*;
use crate::contract::{AsideLevel, AsideSource, Persona, Reach, ScheduleKind, ScheduledJob};
use crate::driver::{MessageKind, Update};
use crate::log::{Log, StreamId};
use crate::session::tests::{DeskKeys, Fake, Scripted, enrol, persona, scratch};
use crate::test_support::until;
use crate::thread::Link;
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const MINUTE: i64 = 60 * 1000;
const HOUR: i64 = 60 * MINUTE;

/// Each turn says `said-<n>` and ends.
fn replies() -> Scripted {
    Scripted::turns(
        (0..20)
            .map(|i| {
                vec![
                    Update::Message {
                        kind: MessageKind::Agent,
                        id: format!("said-{i}"),
                        text: format!("said-{i}"),
                    },
                    Update::Turn {
                        stop_reason: "end_turn".into(),
                        usage: None,
                    },
                ]
            })
            .collect(),
    )
}

fn named(id: &str, name: &str) -> Persona {
    let mut teammate = persona(id);
    teammate.name = name.to_string();
    teammate.reach = Some(Reach::Machine);
    teammate.background_work = true;
    teammate
}

/// Bob, and Ada, who leaves him asides, on a log `ready` has written to.
fn room_with(name: &str, ready: impl FnOnce(&Log)) -> (Arc<Room>, Arc<Fake>) {
    let log = scratch(name);
    enrol(&log, &named("ada", "Ada"));
    enrol(&log, &named("bob", "Bob"));
    ready(&log);
    let agents = Fake::new(replies());
    (
        Room::with_agents(log, Arc::new(DeskKeys), agents.clone()),
        agents,
    )
}

fn room(name: &str) -> (Arc<Room>, Arc<Fake>) {
    room_with(name, |_| {})
}

/// One of Bob's threads, as its link says it stands, on his tape and its
/// own stream.
fn thread(log: &Log, thread: ThreadId, title: &str, state: ThreadState, ts: i64) -> Link {
    let link = Link {
        id: Link::fresh_id(&thread),
        ts,
        thread,
        persona_id: Some("bob".into()),
        title: title.into(),
        state,
        outcome: None,
        at: None,
        note: None,
        binding: None,
        elapsed_ms: None,
        opener: None,
        handoff_ended: false,
        asked_in: None,
    };
    write_link(log, &link);
    link
}

fn write_link(log: &Log, link: &Link) {
    for stream in [
        StreamId::Tape("bob".into()),
        StreamId::of_thread(&link.thread).unwrap(),
    ] {
        log.append(&stream, &link.event()).unwrap();
    }
}

/// A thread of Bob's that closed `ago` ms ago, saying `outcome`.
fn ended(log: &Log, id: ThreadId, title: &str, ago: i64, outcome: &str) {
    let now = now_ms();
    let mut link = thread(log, id, title, ThreadState::Live, now - ago - HOUR);
    link.state = ThreadState::Closed(End::Agent);
    link.at = Some(now - ago);
    link.outcome = Some(outcome.into());
    write_link(log, &link);
}

fn chapter(log: &Log, id: &str, title: &str, ended_ago: i64, note: &str) {
    let now = now_ms();
    log.append(
        &StreamId::Tape("bob".into()),
        &json!({
            "kind": "chapter", "id": id, "ts": now - ended_ago - HOUR,
            "backendId": "hotline", "endedAt": now - ended_ago,
            "title": title, "note": note, "status": "done", "closedBy": "idle",
        }),
    )
    .unwrap();
}

/// An aside from Ada, left for Bob and not yet heard.
fn aside(log: &Log, text: &str, about: Option<ThreadId>) {
    let aside = Aside {
        id: new_id(),
        ts: now_ms(),
        to: "bob".into(),
        from: AsideSource::Thread(ThreadId::dm("ada")),
        by: Some("ada".into()),
        text: text.into(),
        about,
        level: AsideLevel::Note,
        heard: None,
        seen: None,
        looped_in: false,
        source_name: None,
    };
    log.append(
        &StreamId::Asides("bob".into()),
        &serde_json::to_value(crate::contract::TranscriptEvent::Aside(aside)).unwrap(),
    )
    .unwrap();
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A run of Bob's at work. Written once the room is up, since a room that
/// starts settles a run the last process left live.
fn at_work(log: &Log) {
    thread(
        log,
        ThreadId::run("crane"),
        "Check the crane",
        ThreadState::Live,
        now_ms() - 2 * MINUTE,
    );
}

/// The room frame 05 draws, for Bob, but for the run at work: a work thread
/// waiting on a card, a work thread and a chapter that ended today, a call and
/// a chapter from days ago, and an aside from Ada.
fn seed(log: &Log) {
    let now = now_ms();
    thread(
        log,
        ThreadId::side("tag"),
        "Release 0.11.1",
        ThreadState::Parked,
        now - 20 * MINUTE,
    );
    log.append(
        &StreamId::Side("tag".into()),
        &json!({
            "kind": "human_action", "id": "card-1", "ts": now - 10 * MINUTE,
            "actionId": "card-1", "reason": "Push the tag v0.11.1?", "status": "pending",
        }),
    )
    .unwrap();
    ended(
        log,
        ThreadId::side("contract"),
        "Contract check",
        3 * HOUR,
        "Huck's notes are in the doc",
    );
    ended(log, ThreadId::call("old"), "Call", 72 * HOUR, "Hung up");
    chapter(
        log,
        "ch-today",
        "The 0.11.1 notes",
        5 * HOUR,
        "Drafted the 0.11.1 notes. Signing is next.",
    );
    chapter(log, "ch-old", "Last week's triage", 80 * HOUR, "Triaged.");
    aside(log, "The Q3 numbers are in /shared/q3.csv", None);
}

/// The main conversation's turn opens with the index of a seeded room: what
/// is at work, what waits on the person, what ended today (a chapter among
/// them, as an ended main-conversation thread), how many are older, and the
/// asides it hears now, in one block before the person's words.
#[tokio::test]
async fn the_main_conversation_opens_each_turn_with_the_index_of_its_threads() {
    let (room, agents) = room_with("index-main", seed);
    at_work(&room.log);
    room.start("bob").await.unwrap();
    room.prompt("bob", "What is going on?", None, None)
        .await
        .unwrap();
    until("bob's turn was told", || agents.told().len() == 1).await;
    let told = &agents.told()[0];

    assert!(told.starts_with("(Thread index:"), "{told}");
    assert!(told.contains("not the person"), "{told}");
    assert!(
        told.contains("These are your own threads, and the same you worked in them"),
        "{told}"
    );
    assert!(
        told.contains("<hotline_threads>\nTHREADS · Bob · "),
        "{told}"
    );
    assert!(
        told.contains("ACTIVE 1\nrun · Check the crane · working 2m · run:crane\n"),
        "{told}"
    );
    assert!(
        told.contains(
            "WAITING ON THE PERSON 1\nyou're on \"Release 0.11.1\" (card 10m): \"Push the tag v0.11.1?\" · side:tag\n"
        ),
        "{told}"
    );
    assert!(told.contains("ENDED TODAY 2\n"), "{told}");
    assert!(
        told.contains("you did \"Contract check\" (ended "),
        "{told}"
    );
    assert!(
        told.contains("): \"Huck's notes are in the doc\" · side:contract\n"),
        "{told}"
    );
    assert!(
        told.contains("main chapter · The 0.11.1 notes · ended "),
        "a chapter is an ended main-conversation thread: {told}"
    );
    assert!(told.contains("· \"Drafted the 0.11.1 notes\""), "{told}");
    assert!(
        told.contains("2 older · search_thread finds them"),
        "the old call and the old chapter are counted, not listed: {told}"
    );
    assert!(!told.contains("call:old"), "{told}");
    assert!(!told.contains("Last week's triage"), "{told}");
    assert!(
        !told.contains("dm:bob"),
        "the thread the turn is in is left out: {told}"
    );
    assert!(
        told.contains("Ada, in dm:ada: The Q3 numbers are in /shared/q3.csv"),
        "{told}"
    );
    assert_eq!(told.matches("Q3 numbers").count(), 1, "told once: {told}");
    let threads = told.find("</hotline_threads>").unwrap();
    let asides = told.find("<hotline_asides>").unwrap();
    assert!(threads < asides, "the asides come last: {told}");
    assert!(told.contains("The index is over.)\n\n["), "{told}");
    assert!(told.ends_with("What is going on?"), "{told}");
    assert_eq!(agents.prompts()[0], "What is going on?");
}

/// However many threads a teammate has, the index lists fifteen, keeps what
/// waits on the person before what ended, counts the rest as older, and stays
/// within some 750 tokens with long titles and long last lines, its own work
/// told as its own, for whom and how it was updated included.
#[tokio::test]
async fn the_index_holds_its_cap() {
    let (room, _) = room("index-cap");
    let long = "a very long account of what happened in this thread ".repeat(8);
    for n in 0..20 {
        let id = ThreadId::side(format!("{n:08}-0000-0000-0000-000000000000"));
        ended(
            &room.log,
            id.clone(),
            &format!("{n} {long}"),
            (n + 1) * MINUTE,
            &long,
        );
        // Half were handed over, and say for whom.
        if n % 2 == 0 {
            let mut link = room.log.thread_link(&id).unwrap();
            link.opener = Some(crate::thread::Opener {
                persona_id: "ada".into(),
                name: "Ada".into(),
            });
            write_link(&room.log, &link);
        }
        // A third were asked again, and say so.
        if n % 3 == 0 {
            let stream = StreamId::of_thread(&id).unwrap();
            for (at, kind) in ["user", "agent", "user", "agent"].into_iter().enumerate() {
                room.log
                    .append(
                        &stream,
                        &json!({"kind": kind, "id": format!("{n}-{at}"), "ts": now_ms(), "text": long}),
                    )
                    .unwrap();
            }
        }
    }
    let now = now_ms();
    thread(
        &room.log,
        ThreadId::side("waits"),
        "Waits",
        ThreadState::Parked,
        now - 3 * HOUR,
    );
    room.log
        .append(
            &StreamId::Side("waits".into()),
            &json!({
                "kind": "human_action", "id": "card-2", "ts": now - 3 * HOUR,
                "actionId": "card-2", "reason": "May I?", "status": "pending",
            }),
        )
        .unwrap();

    let index = room.index_of("bob", &ThreadId::dm("bob"), now);
    assert_eq!(index.entries.len(), MOST, "{index:?}");
    assert_eq!(index.older, 6, "{index:?}");
    assert_eq!(index.entries[0].section, Section::Waiting, "{index:?}");
    assert!(
        index.entries[1..]
            .windows(2)
            .all(|pair| pair[0].moved >= pair[1].moved),
        "newest first: {index:?}"
    );
    let rendered = index.render("Bob", Some("George"), now);
    assert!(rendered.contains("WAITING ON GEORGE 1"), "{rendered}");
    assert!(rendered.contains("ENDED TODAY 14"), "{rendered}");
    assert!(
        rendered.contains("6 older · search_thread finds them"),
        "{rendered}"
    );
    assert!(rendered.contains(", then updated it ("), "{rendered}");
    assert!(
        rendered.chars().count() <= 3_100,
        "{} chars: {rendered}",
        rendered.chars().count()
    );
}

/// No text the index quotes is cut in half a word: a title, a last line, an
/// outcome or a chapter's note cut short ends at a whole word, in the
/// teammate's own work, a colleague's, a call and a chapter alike, so a cut
/// never reads as another word ("moving Hotl…").
#[tokio::test]
async fn no_index_text_ends_mid_word() {
    const WORDS: [&str; 26] = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliett", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo",
        "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];
    let words = |from: usize| {
        (0..30)
            .map(|at| WORDS[(from + at * 7) % WORDS.len()])
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (room, _) = room("index-words");
    for n in 0..7 {
        let ago = (n as i64 + 1) * MINUTE;
        let own = ThreadId::side(format!("own-{n}"));
        ended(&room.log, own.clone(), &words(n), ago, &words(n + 1));
        let mut link = room.log.thread_link(&own).unwrap();
        link.opener = Some(crate::thread::Opener {
            persona_id: "ada".into(),
            name: "Ada".into(),
        });
        write_link(&room.log, &link);
        room.log
            .append(
                &StreamId::of_thread(&own).unwrap(),
                &json!({"kind": "agent", "id": format!("a-{n}"), "ts": now_ms() - ago, "text": words(n + 2)}),
            )
            .unwrap();
        ended(
            &room.log,
            ThreadId::call(format!("call-{n}")),
            &words(n + 3),
            ago,
            &words(n + 4),
        );
        chapter(
            &room.log,
            &format!("ch-{n}"),
            &words(n + 5),
            ago,
            &words(n + 6),
        );
    }
    let now = now_ms();
    for (who, name) in [("bob", "Bob"), ("ada", "Ada")] {
        let told = room
            .index_of(who, &ThreadId::dm(who), now)
            .render(name, None, now);
        let cuts: Vec<&str> = told
            .match_indices('…')
            .map(|(at, _)| {
                told[..at]
                    .rsplit(|c: char| c.is_whitespace() || c == '"')
                    .next()
                    .unwrap_or_default()
            })
            .collect();
        assert!(cuts.len() >= 7, "{told}");
        for cut in cuts {
            assert!(WORDS.contains(&cut), "{cut:?} is half a word in {told}");
        }
    }
}

#[test]
fn a_cut_ends_at_a_word() {
    assert_eq!(cut("moving Hotline desks", 13), "moving…");
    assert_eq!(cut("moving Hotline desks", 15), "moving Hotline…");
    assert_eq!(cut("moving  Hotline", 40), "moving Hotline");
    assert_eq!(cut("one, two three", 8), "one…");
    assert_eq!(cut("supercalifragilistic", 6), "super…");
}

/// Work a teammate handed a colleague is in the sender's index as the
/// colleague's thread, quoted in the colleague's words and never the
/// person's.
#[tokio::test]
async fn handed_over_work_is_listed_for_its_sender_in_the_colleagues_words_only() {
    let (room, _) = room("index-handed");
    let now = now_ms();
    let mut link = thread(
        &room.log,
        ThreadId::side("hull"),
        "Paint the hull",
        ThreadState::Parked,
        now - HOUR,
    );
    link.opener = Some(crate::thread::Opener {
        persona_id: "ada".into(),
        name: "Ada".into(),
    });
    write_link(&room.log, &link);
    let said = |kind: &str, id: &str, text: &str| {
        room.log
            .append(
                &StreamId::Side("hull".into()),
                &json!({"kind": kind, "id": id, "ts": now_ms(), "text": text}),
            )
            .unwrap();
    };
    said("user", "u1", "The vault code is 4411");
    let listed = |room: &Room| {
        let index = room.index_of("ada", &ThreadId::dm("ada"), now_ms());
        index.render("Ada", None, now_ms())
    };
    let index = listed(&room);
    assert!(
        index.contains("work · handed to Bob: Paint the hull · parked "),
        "{index}"
    );
    assert!(
        !index.contains("4411"),
        "the person's words stay in the thread: {index}"
    );
    said("agent", "a1", "The hull is painted.");
    let index = listed(&room);
    assert!(index.contains("\"The hull is painted.\""), "{index}");
    assert!(!index.contains("4411"), "{index}");
    let own = room
        .index_of("bob", &ThreadId::dm("bob"), now_ms())
        .render("Bob", None, now_ms());
    assert!(
        own.contains("you did \"Paint the hull\" for Ada (parked "),
        "{own}"
    );
    assert!(
        own.contains("): \"The hull is painted.\" · side:hull\n"),
        "{own}"
    );
}

/// A thread a colleague handed the teammate is, in the teammate's own index,
/// work it did itself, told in the second person: what it was, for whom, and
/// what came of it in plain words. Before a closing note names it, what it
/// was is the ask without its greeting, and what came of it its last words;
/// once the thread has ended, the title its note gave it and the note's
/// outcome. So asked what it is working on, it can say without reading it.
#[tokio::test]
async fn a_handed_over_thread_that_ended_is_the_teammates_own_work_in_the_second_person() {
    let (room, _) = room("index-mine");
    let now = now_ms();
    let mut link = thread(
        &room.log,
        ThreadId::side("gull"),
        "Hi Bob, it's Ada with a small writing task. Please wri…",
        ThreadState::Parked,
        now - 5 * MINUTE,
    );
    link.opener = Some(crate::thread::Opener {
        persona_id: "ada".into(),
        name: "Ada".into(),
    });
    write_link(&room.log, &link);
    let stream = StreamId::Side("gull".into());
    for line in [
        json!({
            "kind": "delivery", "id": "handoff:r1", "ts": now - 5 * MINUTE,
            "cause": {
                "kind": "handoff", "requestId": "r1", "personaId": "ada", "name": "Ada",
                "threadKey": "ada~bob",
                "about": "Hi Bob, it's Ada with a small writing task. Please write a winter-hours notice for Gull Point.",
            },
            "text": "Hi Bob, it's Ada with a small writing task. Please write a winter-hours notice for Gull Point.",
        }),
        json!({
            "kind": "agent", "id": "a1", "ts": now - 4 * MINUTE,
            "text": "The notice is saved as `gull-point-winter-hours.md`, **120 words**.",
        }),
    ] {
        room.log.append(&stream, &line).unwrap();
    }
    let told = |room: &Room| {
        room.index_of("bob", &ThreadId::dm("bob"), now_ms())
            .render("Bob", None, now_ms())
    };

    let open = told(&room);
    assert!(
        open.contains(
            "you did \"Please write a winter-hours notice for Gull Point.\" for Ada (parked "
        ),
        "{open}"
    );
    assert!(
        open.contains(
            "): \"The notice is saved as gull-point-winter-hours.md, 120 words.\" · side:gull\n"
        ),
        "in plain words: {open}"
    );

    link.state = ThreadState::Closed(End::Agent);
    link.title = "Gull Point winter hours notice".into();
    link.at = Some(now - 3 * MINUTE);
    link.outcome = Some("The notice is saved as gull-point-winter-hours.md, 120 words.".into());
    link.note = Some(
        "Goal: Write Gull Point's winter-hours notice.\nOutcome: Saved as gull-point-winter-hours.md, from 15 Dec. Gift-shop line added.\nNext: nothing."
            .into(),
    );
    write_link(&room.log, &link);
    let ended = told(&room);
    let at = Local
        .timestamp_millis_opt(now - 3 * MINUTE)
        .single()
        .unwrap()
        .format("%H:%M");
    assert!(
        ended.contains(&format!(
            "ENDED TODAY 1\nyou did \"Gull Point winter hours notice\" for Ada (ended {at}): \"Saved as gull-point-winter-hours.md, from 15 Dec. Gift-shop…\" · side:gull\n"
        )),
        "{ended}"
    );
    assert!(
        !ended.contains("handed"),
        "it is not a colleague's: {ended}"
    );
}

/// What came of the teammate's own work is quoted in its own voice: a
/// closing note's outcome that names who did it, as "the teammate" or by the
/// teammate's name, starts at the verb instead.
#[tokio::test]
async fn the_outcome_of_the_teammates_own_work_is_in_its_own_voice() {
    let (room, _) = room("index-own-voice");
    for (key, outcome) in [
        (
            "plan",
            "The teammate wrote release-plan.md with three steps.",
        ),
        ("notes", "Bob saved notes.md for the release."),
        ("draft", "Teammate drafted the FAQ."),
    ] {
        let mut link = thread(
            &room.log,
            ThreadId::side(key),
            key,
            ThreadState::Live,
            now_ms() - HOUR,
        );
        link.state = ThreadState::Closed(End::Agent);
        link.at = Some(now_ms() - MINUTE);
        link.note = Some(format!("Goal: {key}.\nOutcome: {outcome}"));
        write_link(&room.log, &link);
    }
    let told = room
        .index_of("bob", &ThreadId::dm("bob"), now_ms())
        .render("Bob", None, now_ms());
    for said in [
        "\"Wrote release-plan.md with three steps.\"",
        "\"Saved notes.md for the release.\"",
        "\"Drafted the FAQ.\"",
    ] {
        assert!(told.contains(said), "{said} in {told}");
    }
    assert!(!told.contains("eammate"), "{told}");
}

/// A work thread a colleague handed over twice, the second a follow-up into
/// the same thread, is told as work the teammate did and then updated, and
/// what came of it is the second handoff's answer, not the first's, which
/// the closing note leads with, nor a remark that turn closed on.
#[tokio::test]
async fn own_work_handed_over_again_is_told_by_its_latest_result() {
    let (room, _) = room("index-mine-again");
    let now = now_ms();
    let mut link = thread(
        &room.log,
        ThreadId::side("plan"),
        "Release plan",
        ThreadState::Live,
        now - 10 * MINUTE,
    );
    link.opener = Some(crate::thread::Opener {
        persona_id: "ada".into(),
        name: "Ada".into(),
    });
    link.state = ThreadState::Closed(End::Agent);
    link.at = Some(now - MINUTE);
    link.note = Some(
        "Goal: Write the release plan.\nOutcome: Wrote release-plan.md with three steps, then added a fourth.".into(),
    );
    write_link(&room.log, &link);
    let handoff = |id: &str, ts: i64, about: &str| {
        json!({
            "kind": "delivery", "id": format!("handoff:{id}"), "ts": ts,
            "cause": {
                "kind": "handoff", "requestId": id, "personaId": "ada", "name": "Ada",
                "threadKey": "ada~bob", "about": about,
            },
            "text": about,
        })
    };
    let said =
        |id: &str, ts: i64, text: &str| json!({"kind": "agent", "id": id, "ts": ts, "text": text});
    for line in [
        handoff(
            "r1",
            now - 10 * MINUTE,
            "Write release-plan.md: freeze, test, publish.",
        ),
        said(
            "a1",
            now - 9 * MINUTE,
            "Done. `release-plan.md` has three steps.",
        ),
        handoff("r2", now - 3 * MINUTE, "Add a fourth step, Tag v0.46.0."),
        said("a2", now - 3 * MINUTE, "on it, one sec."),
        json!({"kind": "tool", "id": "tool:c1", "ts": now - 3 * MINUTE, "toolCallId": "c1",
            "title": "edit release-plan.md", "status": "completed"}),
        said(
            "a3",
            now - 2 * MINUTE,
            "Done. Step 4, **Tag v0.46.0**, is appended after Publish.",
        ),
        said(
            "a4",
            now - 2 * MINUTE,
            "Note: step 3 still says \"Tag the release\"; I left it.",
        ),
    ] {
        room.log
            .append(&StreamId::Side("plan".into()), &line)
            .unwrap();
    }

    let told = room
        .index_of("bob", &ThreadId::dm("bob"), now_ms())
        .render("Bob", None, now_ms());
    let line = told
        .lines()
        .find(|line| line.contains("side:plan"))
        .unwrap_or_default();
    assert!(
        line.starts_with("you did \"Release plan\" for Ada, then updated it ("),
        "{told}"
    );
    assert!(
        line.ends_with(": \"Done. Step 4, Tag v0.46.0, is appended after Publish.\" · side:plan"),
        "{told}"
    );
    assert!(!line.contains("three steps"), "{told}");
    assert!(!line.contains("left it"), "{told}");
}

#[test]
fn only_a_subject_that_names_the_teammate_is_dropped() {
    for (said, own) in [
        ("The teammate wrote it.", "Wrote it."),
        ("the teammate fixed CI.", "Fixed CI."),
        ("Bob added a step.", "Added a step."),
        ("I wrote it.", "I wrote it."),
        ("Wrote it.", "Wrote it."),
        ("The teammate's draft is in.", "The teammate's draft is in."),
        ("Bob's draft is in.", "Bob's draft is in."),
        ("Bob and Ada agreed.", "Bob and Ada agreed."),
        ("Bobby wrote it.", "Bobby wrote it."),
    ] {
        assert_eq!(own_words(said, Some("Bob")), own, "{said}");
    }
}

/// While the turn of a thread a colleague handed it is running, the
/// teammate's main conversation is told it is at that work now, for whom.
#[tokio::test]
async fn a_handed_over_thread_at_work_is_the_teammates_own_work_under_way() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let log = scratch("index-mine-working");
    enrol(&log, &named("ada", "Ada"));
    enrol(&log, &named("bob", "Bob"));
    let agents = Fake::new(replies().gated(gate.clone()));
    let room = Room::with_agents(log, Arc::new(DeskKeys), agents.clone());
    room.allow_sender("bob", "ada").unwrap();
    crate::mcp::server::TeammateTools::new(&room, "ada")
        .call(
            "message_teammate",
            &json!({
                "to": "bob", "intent": "handoff",
                "message": "Hi Bob, it's Ada. Please build the Gull Point visitor page.",
            }),
        )
        .await
        .unwrap();
    until("bob is at work on it", || agents.prompts().len() == 1).await;

    let index = room
        .index_of("bob", &ThreadId::dm("bob"), now_ms())
        .render("Bob", None, now_ms());
    assert!(
        index.contains(
            "\nyou're doing \"Please build the Gull Point visitor page.\" for Ada (working now"
        ),
        "{index}"
    );
    assert!(index.contains(") · side:"), "{index}");
    gate.add_permits(10);
}

/// A work thread's turn has the index too, with the main conversation's
/// chapters and its other threads, and leaves itself out.
#[tokio::test]
async fn a_work_thread_opens_each_turn_with_the_index_and_leaves_itself_out() {
    let (room, agents) = room_with("index-work", seed);
    let started = room.start_side("bob", "Look into the crane").await.unwrap();
    until("the work thread was told", || agents.told().len() == 1).await;
    let told = &agents.told()[0];
    assert!(told.contains("THREADS · Bob · "), "{told}");
    assert!(
        told.contains("you're on \"Release 0.11.1\" (card "),
        "{told}"
    );
    assert!(told.contains("main chapter · The 0.11.1 notes"), "{told}");
    assert!(
        !told.contains("Q3 numbers"),
        "an aside about nothing waits for the main conversation: {told}"
    );
    assert!(
        !told.contains(&format!("side:{}", started.side_id)),
        "{told}"
    );
    assert!(told.ends_with("Look into the crane"), "{told}");
}

/// A call's turn runs on the shared work-thread occupant and is told the
/// index the same way, without itself.
#[tokio::test]
async fn a_call_turn_opens_with_the_index_and_leaves_itself_out() {
    let (room, agents) = room_with("index-call", seed);
    at_work(&room.log);
    thread(
        &room.log,
        ThreadId::call("live"),
        "Call",
        ThreadState::Parked,
        now_ms() - MINUTE,
    );
    room.prompt_call_work("bob", &ThreadId::call("live"), "Look up the tide")
        .await
        .unwrap();
    until("the call's turn was told", || agents.told().len() == 1).await;
    let told = &agents.told()[0];
    assert!(told.contains("THREADS · Bob · "), "{told}");
    assert!(told.contains("ACTIVE 1\nrun · Check the crane"), "{told}");
    assert!(!told.contains("call:live"), "{told}");
    assert!(told.ends_with("Look up the tide"), "{told}");
}

/// A run is told its parent thread's line and nothing else of the
/// teammate's threads; asides wait for a thread that takes turns.
#[tokio::test]
async fn a_run_hears_only_its_parent_thread() {
    let (room, agents) = room_with("index-run", seed);
    let outcome = room
        .run(
            crate::session::runner::RunSpec {
                persona_id: "bob".into(),
                run_id: "r2".into(),
                title: "Measure the quay".into(),
                brief: crate::session::Brief::subagent("Measure the quay.".into(), Vec::new()),
                capability: room.capability_lease("bob"),
            },
            CancellationToken::new(),
        )
        .await;
    assert_eq!(outcome.report, "said-0");
    let told = &agents.told()[0];
    assert!(
        told.contains("PARENT 1\nmain · main conversation · idle "),
        "{told}"
    );
    for elsewhere in [
        "side:tag",
        "side:contract",
        "run:crane",
        "chapter",
        "older ·",
        "Q3",
    ] {
        assert!(!told.contains(elsewhere), "{elsewhere}: {told}");
    }
    assert!(told.ends_with("Measure the quay."), "{told}");
    assert_eq!(
        room.asides_unheard("bob"),
        1,
        "the aside for the main conversation is still to be heard there"
    );
}

/// A job's firing is told its parent thread's line and the asides about the
/// job, and none of the teammate's other threads or asides.
#[tokio::test]
async fn a_jobs_firing_hears_only_its_parent_thread_and_the_asides_about_it() {
    let (room, agents) = room_with("index-job", |log| {
        seed(log);
        let now = now_ms();
        crate::room::append_schedule(
            log,
            &ScheduledJob {
                id: "nightly".into(),
                persona_id: "bob".into(),
                kind: ScheduleKind::Loop,
                when: None,
                every: Some(15_000),
                prompt: "run the nightly tests".into(),
                quiet: None,
                reminder: None,
                goal: None,
                until: None,
                times: None,
                without: None,
                operator_created: false,
                next_at: now - 1,
                created_at: now - MINUTE,
                last_post: None,
                ended_at: None,
            },
        )
        .unwrap();
        aside(
            log,
            "The nightly box is back up",
            Some(ThreadId::job("nightly")),
        );
    });
    until("the job fired", || !agents.told().is_empty()).await;
    let told = &agents.told()[0];
    assert!(
        told.contains("PARENT 1\nmain · main conversation · idle "),
        "{told}"
    );
    assert!(
        told.contains("Ada, in dm:ada: The nightly box is back up"),
        "{told}"
    );
    for elsewhere in [
        "side:tag",
        "side:contract",
        "run:crane",
        "chapter",
        "older ·",
        "Q3",
    ] {
        assert!(!told.contains(elsewhere), "{elsewhere}: {told}");
    }
    assert!(told.contains("run the nightly tests"), "{told}");
    assert_eq!(
        room.asides_unheard("bob"),
        1,
        "the main conversation's aside waits for it"
    );
}

/// A turn in a room where nothing moved since the agent's last turn there
/// is told so in one line, and the whole index again once something moves
/// or an aside arrives. A room with no other threads is told nothing.
#[tokio::test]
async fn an_unchanged_index_is_one_line_and_an_empty_one_is_nothing() {
    let (room, agents) = room("index-unchanged");
    room.start("bob").await.unwrap();
    room.prompt("bob", "Hello", None, None).await.unwrap();
    until("the first turn", || agents.told().len() == 1).await;
    assert_eq!(agents.told()[0], "Hello", "no threads, no index");

    ended(
        &room.log,
        ThreadId::side("contract"),
        "Contract check",
        HOUR,
        "Done",
    );
    let session = room.session("bob").unwrap();
    let next = async |text: &str, count: usize| {
        until("bob is at rest", || !lock(&session.turns).running).await;
        room.prompt("bob", text, None, None).await.unwrap();
        until("bob's turn", || agents.told().len() == count).await;
        agents.told()[count - 1].clone()
    };
    let told = next("Anything?", 2).await;
    assert!(told.contains("ENDED TODAY 1"), "{told}");

    let told = next("And now?", 3).await;
    assert!(
        told.starts_with(UNCHANGED),
        "nothing moved, so one line: {told}"
    );
    assert!(told.ends_with("And now?"), "{told}");
    assert!(!told.contains("<hotline_threads>"), "{told}");

    aside(&room.log, "Heads up", None);
    let told = next("And now?", 4).await;
    assert!(
        told.contains("<hotline_threads>"),
        "an aside is news: {told}"
    );
    assert!(told.contains("Ada, in dm:ada: Heads up"), "{told}");

    let told = next("Still?", 5).await;
    assert!(told.starts_with(UNCHANGED), "{told}");

    ended(
        &room.log,
        ThreadId::side("tide"),
        "Tide tables",
        MINUTE,
        "Done",
    );
    let told = next("Now?", 6).await;
    assert!(told.contains("ENDED TODAY 2"), "a thread moved: {told}");
}

/// A call's summary of what it was asked reads the turn's line without the
/// index in front of it.
#[test]
fn what_a_turn_was_asked_is_its_line_without_the_index() {
    let now = now_ms();
    let index = timed(
        now,
        &format!("(Thread index: x\n<hotline_threads>\nTHREADS\n</hotline_threads>\n{OVER}"),
    );
    let line = timed(now, "Look up the tide");
    assert_eq!(asked(&format!("{index}\n\n{line}")), line);
    assert_eq!(asked(&format!("{}\n\n{line}", timed(now, UNCHANGED))), line);
    assert_eq!(asked(&line), line);
}
