//! `send`'s description: the two ways a teammate writes to a colleague.

use super::*;

/// A quick line reads like a chat and a hand-off like a letter, and the
/// description says both, with the title left out of the quick one and an
/// answer sent back on `reply_to`. The schema asks for `to` alone.
#[test]
fn the_send_description_says_a_quick_line_and_a_hand_off() {
    let tools = listing(descriptors()).tools;
    let send = tools.iter().find(|tool| tool.name == SEND).unwrap();
    let said = send.description.as_deref().unwrap();
    for words in [
        "A quick question or answer is one short line in `message` and no `title`; the person reads it like a chat.",
        "A hand-off is a `title` and a full brief in `message`; the person reads it like a letter.",
        "Answer a quick question with a quick line, `reply_to` its id: that closes it as answered.",
        "in their inbox, or awaiting the person's approval",
    ] {
        assert!(said.contains(words), "{words}: {said}");
    }
    assert_eq!(send.input_schema["required"], serde_json::json!(["to"]));
}
