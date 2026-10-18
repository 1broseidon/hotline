use super::*;

/// The reach selected over the real wire governs the real tools. No model is
/// needed to choose an attack command, and a provider cannot skip the probe.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_reach_keeps_another_projects_env_out_of_the_tools() {
    use hotline_core::contract::Persona;
    use hotline_core::tools::{ReadFile, RunCommand, Workspace};
    use rig::tool::{Tool, ToolContext};

    #[cfg(target_os = "linux")]
    {
        let probe = std::process::Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--unshare-pid", "/bin/true"])
            .output();
        if !probe.is_ok_and(|output| output.status.success()) {
            eprintln!("skipped: this machine cannot run bubblewrap");
            return;
        }
    }
    hotline_core::tools::shell_available(hotline_core::contract::Reach::Workspace).unwrap();
    let (root, port) = open("workspace-reach").await;
    let workspace = root.join("project");
    let other = root.join("other-project");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let secret = other.join(".env");
    std::fs::write(&secret, "OTHER_PROJECT_TOKEN=outside-canary").unwrap();
    std::os::unix::fs::symlink("../other-project/.env", workspace.join("escape")).unwrap();
    let mut client = Client::connect(port).await;
    let credential = client
        .call(
            "credential.custom_save",
            json!({"draft": {
                "name": "Isolation test", "baseUrl": "http://127.0.0.1:9/v1",
                "api": "chat_completions", "secret": "", "models": ["test"]
            }}),
        )
        .await;
    assert_eq!(credential["ok"], true, "{credential}");
    let created = client
        .call(
            "persona.create",
            json!({"draft": {
                "name": "Confined", "goal": "Prove workspace reach.", "cwd": workspace,
            }}),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let mut persona: Persona = serde_json::from_value(created["result"].clone()).unwrap();

    for (reach, allowed) in [
        ("workspace", false),
        ("machine", true),
        ("workspace", false),
    ] {
        let updated = client
            .call(
                "persona.update",
                json!({
                    "id": persona.id, "patch": {"reach": reach},
                }),
            )
            .await;
        assert_eq!(updated["ok"], true, "{updated}");
        persona = serde_json::from_value(updated["result"].clone()).unwrap();
        let started = client
            .call("session.start", json!({"personaId": persona.id}))
            .await;
        assert_eq!(started["ok"], true, "{started}");
        let ledger = client
            .call("teammate.tools", json!({"personaId": persona.id}))
            .await;
        let shell = ledger["result"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["source"] == "builtin" && row["name"] == "shell")
            .unwrap();
        assert_eq!(shell["state"], "verified", "{ledger}");
        let tools = Workspace::open(
            PathBuf::from(&persona.cwd),
            persona.reach.unwrap_or_default(),
            root.join("tool-output").join(&persona.id),
        )
        .unwrap();
        for path in ["../other-project/.env", "escape"] {
            let read = ReadFile::new(tools.clone())
                .call(
                    &mut ToolContext::new(),
                    serde_json::from_value(json!({"path": path})).unwrap(),
                )
                .await;
            assert_eq!(read.is_ok(), allowed, "{reach}: {path}: {read:?}");
            let output = RunCommand::new(tools.clone())
                .call(
                    &mut ToolContext::new(),
                    serde_json::from_value(json!({"command": format!("cat {path}")})).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                output.contains("outside-canary"),
                allowed,
                "{reach}: {output}"
            );
            assert_eq!(
                output.contains("[exit status"),
                !allowed,
                "{reach}: {output}"
            );
        }
    }
}
