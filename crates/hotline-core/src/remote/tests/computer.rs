use super::*;

#[test]
fn the_computer_target_is_read_off_the_desk_s_own_viewer_and_only_while_running() {
    use crate::contract::{ComputerState, ComputerStatus};
    let status = |state: ComputerState, viewer: Option<&str>| ComputerStatus {
        state,
        url: None,
        viewer: viewer.map(str::to_owned),
        release: None,
        available: None,
        update_failed: None,
        limits_changed: false,
    };
    assert_eq!(
        server::computer_target(&status(
            ComputerState::Running,
            Some("http://127.0.0.1:41234/#abc")
        )),
        Some((41234, "abc".to_string()))
    );
    assert_eq!(
        server::computer_target(&status(
            ComputerState::Stopped,
            Some("http://127.0.0.1:41234/#abc")
        )),
        None
    );
    assert_eq!(
        server::computer_target(&status(ComputerState::Running, None)),
        None
    );
    assert_eq!(
        server::computer_target(&status(
            ComputerState::Running,
            Some("http://127.0.0.1:41234/#")
        )),
        None
    );
    assert_eq!(
        server::computer_target(&status(
            ComputerState::Running,
            Some("http://example.com:41234/#abc")
        )),
        None
    );
}
