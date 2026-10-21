use super::*;

const UUID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

#[tokio::test(flavor = "multi_thread")]
async fn a_checkpoint_opens_only_under_the_account_that_wrote_it() {
    for (runs_on, saved_on, opened) in [
        (Some("b"), Some("a"), "session/new"),
        (Some("b"), None, "session/new"),
        (None, Some("a"), "session/new"),
        (Some("b"), Some("b"), "session/load old-session"),
    ] {
        let held = room("account-checkpoint-room");
        let heard = Heard::default();
        let agent = scripted_agent(heard.clone(), true);
        let driver = ChildAgent::new(
            scratch("account-checkpoint"),
            "cursor".to_string(),
            String::new(),
            TeammateTools::new(&held, "ada"),
        )
        .with_account(runs_on.map(|id| resolved(id, scratch(id))));
        tokio::spawn(agent);
        let ada = persona(
            &scratch_cwd(),
            vec![SessionCheckpoint {
                backend_id: "cursor".to_string(),
                session_id: "old-session".to_string(),
                account_id: saved_on.map(str::to_string),
            }],
        );
        let info = driver.handshake(&ada, client_transport()).await.unwrap();
        assert_eq!(
            heard.opened.lock().unwrap().as_slice(),
            [opened],
            "{runs_on:?} {saved_on:?}"
        );
        assert_eq!(info.context_restored, opened != "session/new");
    }
}

#[tokio::test]
async fn a_child_whose_folder_is_not_its_accounts_comes_up_refused_in_public_words() {
    let held = room("account-refused-room");
    let driver = ChildAgent::new(
        scratch("account-refused"),
        "claude-acp".to_string(),
        String::new(),
        TeammateTools::new(&held, "ada"),
    )
    .with_account(Some(resolved(UUID, scratch("elsewhere"))));
    let info = driver
        .start(&persona(&scratch_cwd(), Vec::new()))
        .await
        .unwrap();
    let notice = driver.startup_failure().unwrap();
    for part in [
        r#""kind":"account_unavailable""#,
        r#""code":"folder_unavailable""#,
        UUID,
        "Ada can't start: its Claude Code account needs attention at the desk.",
    ] {
        assert!(notice.contains(part), "{notice}");
    }
    assert!(!notice.contains("Account 2") && !notice.contains("elsewhere"));
    assert!(info.session_id.is_none() && lock(&driver.child).is_none());
    let mut updates = driver
        .prompt("go".to_string(), Vec::new(), Reach::Workspace)
        .await;
    assert!(matches!(next(&mut updates).await, Update::Notice { text, .. } if text == notice));
}
