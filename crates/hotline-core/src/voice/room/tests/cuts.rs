use super::*;
use hotline_contract::VoicePlayed;

const REPLY: &str =
    "I looked at the logs. The cache went stale after the deploy. I can clear it now.";

/// A teammate's three-sentence reply said whole on a direct call: its line,
/// whose clips 0 to 2 are a second each, then the empty closing clip.
async fn said_reply(
    calls: &Arc<Calls>,
    id: &str,
    persona: &str,
    rx: &mut broadcast::Receiver<VoiceEvent>,
) -> String {
    let origin = on_turn(calls, id, 1);
    let reply = format!("<spoken>{REPLY}</spoken>");
    calls.reply_delta(persona, "reply-1", &reply, &origin);
    assert!(calls.delivery(persona, "reply-1", "Mack", &reply, true, Some(&origin)));
    let (said, clips) = one_reply(rx).await;
    assert_eq!(clips.len(), 4, "{clips:?}");
    said[0].0.clone()
}

/// The cut notice written on the call's thread.
async fn cut_on_thread(desk: &crate::desk::Desk, persona: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let thread = call_link(desk, persona).unwrap()["thread"]
                .as_str()
                .unwrap()
                .to_string();
            let lines = desk.log.load(&StreamId::Call(thread));
            if let Some(cut) = lines.iter().find(|line| line["interrupted"] == true) {
                return cut["text"].as_str().unwrap().to_string();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

/// A tap with the window's playback position: the first sentence played to
/// its end, the second was half played, the third never began. The thread
/// and the next turn are told exactly that, the cut estimated and said to be.
#[tokio::test]
async fn a_cut_in_with_the_playback_position_says_what_was_heard() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let line = said_reply(&calls, &id, &persona, &mut rx).await;
    let played = VoicePlayed {
        line: Some(line),
        index: 1,
        ms: 500,
        clip_ms: 1_000,
    };
    calls.interrupt(&id, Some(played)).unwrap();
    let text = cut_on_thread(&desk, &persona).await;
    assert!(text.starts_with("Speech interrupted: the caller cut in.\nHeard in full: \"I looked at the logs.\"\nCut off during: \"The cache went stale [cut about here] after the deploy.\" (0.5 s of its 1.0 s played)\nNot heard: \"I can clear it now.\""), "{text}");
    assert!(!text.to_lowercase().contains("exact"), "{text}");
    assert_eq!(calls.take_interruption(&id).as_deref(), Some(text.as_str()));
    calls.end(&id).unwrap();
}

/// Hanging up mid-reply notes what was never played, so the closing note
/// does not take it for said; the call ends as before.
#[tokio::test]
async fn hanging_up_mid_reply_notes_what_was_never_played() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    let line = said_reply(&calls, &id, &persona, &mut rx).await;
    let played = VoicePlayed {
        line: Some(line),
        index: 0,
        ms: 1_000,
        clip_ms: 1_000,
    };
    calls.hang_up(&id, Some(played)).unwrap();
    let text = cut_on_thread(&desk, &persona).await;
    assert!(text.starts_with("Speech interrupted: the caller hung up.\nHeard in full: \"I looked at the logs.\"\nNot heard: \"The cache went stale after the deploy. I can clear it now.\"\nIt stopped between sentences, so this split is certain."), "{text}");
    assert!(calls.take_interruption(&id).is_none(), "the call has ended");
}

/// A hold with the position is a cut as a tap's is; without it, as from an
/// older client, nothing is noted, as before.
#[tokio::test]
async fn a_hold_with_the_position_notes_the_cut_and_one_without_notes_nothing() {
    let (_root, desk, calls, id, persona, mut rx, _fake) = direct_call().await;
    said_reply(&calls, &id, &persona, &mut rx).await;
    calls.hold(&id, true, None).unwrap();
    assert!(calls.take_interruption(&id).is_none());
    calls.hold(&id, false, None).unwrap();
    let (_root, desk2, calls2, id2, persona2, mut rx2, _fake2) = direct_call().await;
    let line = said_reply(&calls2, &id2, &persona2, &mut rx2).await;
    let played = VoicePlayed {
        line: Some(line),
        index: 2,
        ms: 10,
        clip_ms: 1_000,
    };
    calls2.hold(&id2, true, Some(played)).unwrap();
    let text = cut_on_thread(&desk2, &persona2).await;
    assert!(text.starts_with("Speech interrupted: the caller put the call on hold.\nHeard in full: \"I looked at the logs. The cache went stale after the deploy.\"\nCut off during: \"[cut about here] I can clear it now.\""), "{text}");
    calls.end(&id).unwrap();
    calls2.end(&id2).unwrap();
    drop(desk);
}
