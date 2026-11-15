//! What a result names when it comes back.
use super::*;

/// A colleague's answer tells its sender the request it answers, the name a
/// later handoff passes to carry that work on; a failure does not.
#[test]
fn a_result_names_the_request_it_answers() {
    let cause = |status| DeliveryCause::Peer {
        request_id: Some("req-1".into()),
        persona_id: "bob".into(),
        name: "Bob".into(),
        thread_key: "ada~bob".into(),
        status,
        about: "Fix the build.".into(),
        answer: None,
    };
    for said in ["It builds now.", ""] {
        let heard = peers::delivery_wire(&cause(PeerStatus::Done), said);
        assert!(heard.ends_with(" Its requestId is req-1."), "{heard}");
    }
    let failed = peers::delivery_wire(&cause(PeerStatus::Failed), "It broke.");
    assert!(!failed.contains("req-1"), "{failed}");
}
