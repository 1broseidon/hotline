//! The desk's one line on what a teammate working in silence is doing
//! (`voice.acknowledge`): said once, about six seconds into a quiet turn,
//! only when the teammate's session has started work a line can name, worded
//! to stand alone or to follow the acknowledgement; otherwise the waiting
//! tone, as before. Time is paused, so the seconds are the desk's own.
use super::acknowledge::{acknowledging, handed_over, says, sent_within, thread_lines};
use super::*;
use tokio::time::Instant as Clock;

const ACK: &str = "Looking at the build now.";
const WEB: &str = "I'm searching the web for that now.";
const RUNNING: &str = "Running the build checks now.";

/// Turn 1 of the call, as the teammate's session tells the call of it.
fn turn_one(id: &str) -> Origin {
    Origin {
        call_id: id.into(),
        seq: 1,
        direct: true,
        front: false,
    }
}

/// The teammate's session starts a step on turn 1 that the call pane says
/// by `group`.
fn starts(calls: &Calls, persona: &str, id: &str, group: &str) {
    calls.step(persona, &turn_one(id), "step-1", group, true);
}

/// When the line `text` is said, from `since`.
async fn said_after(
    rx: &mut broadcast::Receiver<VoiceEvent>,
    text: &str,
    since: Clock,
) -> Duration {
    event(
        rx,
        |e| matches!(e, VoiceEvent::Said { text: said, .. } if said == text),
    )
    .await;
    since.elapsed()
}

fn tone(events: &[VoiceEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, VoiceEvent::Clip { id, .. } if id == "waiting-1"))
}

/// A call to Mack with acknowledging `on` and no acknowledgement written,
/// whose call assistant says `progress`; the person has asked, and the turn
/// is with the teammate.
async fn quiet_turn(
    on: bool,
    progress: Result<&str, &str>,
) -> (
    tempfile::TempDir,
    Arc<crate::desk::Desk>,
    Arc<Calls>,
    String,
    String,
    broadcast::Receiver<VoiceEvent>,
    Arc<Fake>,
    Clock,
) {
    let (root, desk, calls, id, persona, rx, fake) = direct_call().await;
    acknowledging(&desk, on);
    *lock(&fake.ack) = Ok(String::new());
    *lock(&fake.progress) = progress.map(str::to_string).map_err(str::to_string);
    let asked = Clock::now();
    utterance(&calls, &id, 1).unwrap();
    handed_over(&calls, &id).await;
    (root, desk, calls, id, persona, rx, fake, asked)
}

/// A teammate gone straight to a web search, with nothing said: about six
/// seconds after the person's last word the line is said once, in the
/// call's voice, as the teammate's, worded by the call assistant told
/// nothing was said before it; the waiting tone does not follow it, and it
/// is kept for the teammate's later turns.
#[tokio::test(start_paused = true)]
async fn a_running_tool_is_named_once_six_seconds_into_a_quiet_turn() {
    let (_root, desk, calls, id, persona, mut rx, fake, asked) = quiet_turn(true, Ok(WEB)).await;
    starts(&calls, &persona, &id, "web");
    let after = said_after(&mut rx, WEB, asked).await;
    assert!(
        after >= Duration::from_secs(6) && after < Duration::from_millis(6_500),
        "{after:?}"
    );
    assert_eq!(
        *lock(&fake.progresses),
        [(
            "Mack".to_string(),
            "Can you check the build?".to_string(),
            "searching the web".to_string(),
            None
        )]
    );
    assert!(lock(&fake.spoken).iter().any(|spoken| spoken == WEB));
    let later = sent_within(&mut rx, Duration::from_secs(30)).await;
    assert!(!tone(&later), "no tone after it: {later:?}");
    assert!(!says(&later, WEB), "once a turn: {later:?}");
    assert_eq!(lock(&fake.progresses).len(), 1);
    assert!(
        thread_lines(&desk, &persona)
            .iter()
            .any(|event| event["kind"] == "agent" && event["text"] == WEB),
        "kept on the call's thread as the teammate's"
    );
    assert!(calls.turn_instruction(&id).contains(WEB));
    calls.end(&id).unwrap();
}

/// A teammate only thinking, or doing nothing a line can name, has no line
/// made up for it: the waiting tone plays at eight seconds, as before.
#[tokio::test(start_paused = true)]
async fn without_work_to_name_the_tone_plays_instead() {
    for group in [None, Some("think"), Some("mcp:GitHub")] {
        let (_root, _desk, calls, id, persona, mut rx, fake, _) = quiet_turn(true, Ok(WEB)).await;
        if let Some(group) = group {
            starts(&calls, &persona, &id, group);
        }
        let sent = sent_within(&mut rx, Duration::from_secs(10)).await;
        assert!(tone(&sent), "{group:?}: {sent:?}");
        assert!(!says(&sent, WEB), "{group:?}: {sent:?}");
        assert!(lock(&fake.progresses).is_empty(), "{group:?}");
        // Work that starts after the tone is not announced either.
        starts(&calls, &persona, &id, "web");
        let sent = sent_within(&mut rx, Duration::from_secs(5)).await;
        assert!(sent.is_empty(), "{group:?}: {sent:?}");
        calls.end(&id).unwrap();
    }
}

/// After an acknowledgement the call assistant is told what was said, and
/// the line comes six seconds after the acknowledgement's sound (a second
/// long) ends; a line that only says the acknowledgement again is dropped,
/// and no tone follows an acknowledgement either way.
#[tokio::test(start_paused = true)]
async fn after_an_acknowledgement_it_follows_on_and_never_repeats_it() {
    for progress in [RUNNING, ACK] {
        let (_root, desk, calls, id, persona, mut rx, fake) = direct_call().await;
        acknowledging(&desk, true);
        *lock(&fake.ack) = Ok(ACK.into());
        *lock(&fake.progress) = Ok(progress.into());
        utterance(&calls, &id, 1).unwrap();
        event(
            &mut rx,
            |e| matches!(e, VoiceEvent::Said { text, .. } if text == ACK),
        )
        .await;
        let acknowledged = Clock::now();
        starts(&calls, &persona, &id, "own:execute");
        if progress == RUNNING {
            let after = said_after(&mut rx, RUNNING, acknowledged).await;
            assert!(
                after >= Duration::from_secs(7) && after < Duration::from_millis(7_500),
                "{after:?}"
            );
        }
        let sent = sent_within(&mut rx, Duration::from_secs(20)).await;
        assert!(!tone(&sent), "{progress}: {sent:?}");
        assert!(!says(&sent, ACK), "said again: {sent:?}");
        assert_eq!(
            *lock(&fake.progresses),
            [(
                "Mack".to_string(),
                "Can you check the build?".to_string(),
                "running a command".to_string(),
                Some(ACK.to_string())
            )]
        );
        calls.end(&id).unwrap();
    }
}

/// The line comes back with "still" and nothing was said before it: it is
/// dropped, and the tone plays as if there had been no line.
#[tokio::test(start_paused = true)]
async fn a_line_saying_still_with_nothing_said_before_is_dropped() {
    let (_root, _desk, calls, id, persona, mut rx, fake, _) =
        quiet_turn(true, Ok("Still searching the schedule.")).await;
    starts(&calls, &persona, &id, "web");
    let sent = sent_within(&mut rx, Duration::from_secs(10)).await;
    assert!(tone(&sent), "{sent:?}");
    assert!(
        !sent.iter().any(|e| matches!(e, VoiceEvent::Said { .. })),
        "{sent:?}"
    );
    assert_eq!(lock(&fake.progresses).len(), 1);
    assert!(lock(&fake.spoken).is_empty(), "never synthesized");
    calls.end(&id).unwrap();
}

/// The teammate's own first words are ready while the line's sound is being
/// made: the line is dropped, and only the teammate is heard.
#[tokio::test(start_paused = true)]
async fn the_line_is_dropped_when_the_teammate_speaks_first() {
    let (_root, _desk, calls, id, persona, mut rx, fake, _) = quiet_turn(true, Ok(WEB)).await;
    *lock(&fake.delay) = Duration::from_millis(800);
    starts(&calls, &persona, &id, "web");
    until("the line is being synthesized", || {
        lock(&fake.spoken).iter().any(|spoken| spoken == WEB)
    })
    .await;
    calls.reply_delta(
        &persona,
        "reply-1",
        "<spoken>The build is green.</spoken>",
        &turn_one(&id),
    );
    let sent = sent_within(&mut rx, Duration::from_secs(3)).await;
    assert!(says(&sent, "The build is green."), "{sent:?}");
    assert!(!says(&sent, WEB), "{sent:?}");
    calls.end(&id).unwrap();
}

/// Speaking over the call, holding it or hanging up while the line is being
/// synthesized: it is never played.
#[tokio::test(start_paused = true)]
async fn interrupt_hold_and_hang_up_stop_the_line() {
    for how in ["interrupt", "hold", "hang up"] {
        let (_root, _desk, calls, id, persona, mut rx, fake, _) = quiet_turn(true, Ok(WEB)).await;
        *lock(&fake.delay) = Duration::from_millis(800);
        starts(&calls, &persona, &id, "web");
        until("the line is being synthesized", || {
            lock(&fake.spoken).iter().any(|spoken| spoken == WEB)
        })
        .await;
        match how {
            "interrupt" => calls.interrupt(&id).unwrap(),
            "hold" => calls.hold(&id, true).unwrap(),
            _ => calls.end(&id).unwrap(),
        }
        let sent = sent_within(&mut rx, Duration::from_secs(3)).await;
        assert!(!says(&sent, WEB), "{how}: {sent:?}");
        assert!(
            !sent
                .iter()
                .any(|e| matches!(e, VoiceEvent::Clip { data, .. } if !data.is_empty())),
            "{how}: {sent:?}"
        );
        let _ = calls.end(&id);
    }
}

/// With no call assistant the desk says its own line for the kinds it has
/// one for, and nothing for any other: then the tone plays.
#[tokio::test(start_paused = true)]
async fn without_a_call_assistant_the_desk_says_its_own_line_or_nothing() {
    for (group, line) in [
        ("web", Some(WEB)),
        ("execute", Some("I'm running that now.")),
        ("fetch", Some("I'm reading that now.")),
        ("browser", Some("I'm working on that in the browser now.")),
        ("edit", None),
        ("history", None),
    ] {
        let (_root, desk, calls, id, persona, mut rx, fake) = direct_call_with(false).await;
        acknowledging(&desk, true);
        let asked = Clock::now();
        utterance(&calls, &id, 1).unwrap();
        handed_over(&calls, &id).await;
        starts(&calls, &persona, &id, group);
        if let Some(line) = line {
            let after = said_after(&mut rx, line, asked).await;
            assert!(after < Duration::from_millis(6_500), "{group}: {after:?}");
            let later = sent_within(&mut rx, Duration::from_secs(5)).await;
            assert!(!tone(&later), "{group}: {later:?}");
        } else {
            let sent = sent_within(&mut rx, Duration::from_secs(10)).await;
            assert!(tone(&sent), "{group}: {sent:?}");
            assert!(
                !sent.iter().any(|e| matches!(e, VoiceEvent::Said { .. })),
                "{group}: {sent:?}"
            );
        }
        assert!(lock(&fake.acks).is_empty(), "{group}");
        calls.end(&id).unwrap();
    }
}

/// Off, a quiet turn whose teammate searches the web is today's: the call
/// assistant is never asked, nothing is said, and the tone plays at eight
/// seconds.
#[tokio::test(start_paused = true)]
async fn with_the_setting_off_a_quiet_turn_is_unchanged() {
    let (_root, _desk, calls, id, persona, mut rx, fake, asked) = quiet_turn(false, Ok(WEB)).await;
    starts(&calls, &persona, &id, "web");
    let sent = sent_within(&mut rx, Duration::from_millis(7_900)).await;
    assert!(sent.is_empty(), "{sent:?}");
    event(
        &mut rx,
        |e| matches!(e, VoiceEvent::Clip { id, .. } if id == "waiting-1"),
    )
    .await;
    assert!(asked.elapsed() >= Duration::from_secs(8));
    assert!(lock(&fake.progresses).is_empty());
    assert!(lock(&fake.acks).is_empty());
    assert!(lock(&fake.spoken).is_empty());
    calls.end(&id).unwrap();
}

/// What the call assistant is told the teammate is doing comes from the
/// kind of step alone, so no tool's name, path, address, command or id from
/// a real step reaches the line, worded or fixed.
#[tokio::test(start_paused = true)]
async fn no_raw_tool_name_path_or_id_reaches_the_line() {
    let steps = [
        ("mcp__hotline__web_search: goldfish care 2026", "other"),
        ("read docs/design.md", "read"),
        ("Fetch https://nextjs.org/docs/app", "fetch"),
        ("bash: cargo test --workspace -p hotline-core", "execute"),
        ("mcp__claude_ai_Prism__github__list_issues", "other"),
        ("computer__browser", "computer__browser"),
        ("grep -rn 9d69f0e1 crates/", "search"),
    ];
    let raw = [
        "_", "/", ".md", "http", "cargo", "--", "9d69f0e1", "goldfish", "2026", "github", "grep",
    ];
    for (title, kind) in steps {
        let group = crate::glance::step_said(title, kind, None).key;
        for assistant in [true, false] {
            let (_root, desk, calls, id, persona, mut rx, fake) = direct_call_with(assistant).await;
            acknowledging(&desk, true);
            *lock(&fake.ack) = Ok(String::new());
            *lock(&fake.progress) = Ok("Working on it now.".into());
            utterance(&calls, &id, 1).unwrap();
            handed_over(&calls, &id).await;
            starts(&calls, &persona, &id, &group);
            let sent = sent_within(&mut rx, Duration::from_secs(9)).await;
            let mut heard: Vec<String> = sent
                .iter()
                .filter_map(|e| match e {
                    VoiceEvent::Said { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            heard.extend(lock(&fake.progresses).iter().map(|asked| asked.2.clone()));
            for words in &heard {
                for raw in raw {
                    assert!(
                        !words.to_lowercase().contains(raw),
                        "{title}: {words:?} carries {raw:?}"
                    );
                }
            }
            calls.end(&id).unwrap();
        }
    }
}
