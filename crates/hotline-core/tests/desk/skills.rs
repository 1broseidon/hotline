use super::*;

/// The skills catalog over the wire: the built-ins come first, the gateway
/// folder lists what is wrong with an entry rather than hiding it, and a
/// teammate's own workspace skills are listed without the ones Hotline copied
/// there, which are the grant and not the teammate's.
#[tokio::test]
async fn the_skills_catalog_lists_builtins_the_gateway_and_a_teammates_own() {
    let (root, port) = open("skills").await;
    let skill = |name: &str| {
        format!("---\nname: {name}\ndescription: Use when testing {name}.\n---\n\nDo the thing.\n")
    };
    let gateway = root.join("skills");
    std::fs::create_dir_all(gateway.join("cut-release")).unwrap();
    std::fs::write(gateway.join("cut-release/SKILL.md"), skill("cut-release")).unwrap();
    std::fs::create_dir_all(gateway.join("Broken")).unwrap();
    std::fs::write(gateway.join("Broken/SKILL.md"), skill("Broken")).unwrap();
    let workspace = root.join("ws");
    std::fs::create_dir_all(workspace.join(".agents/skills/mine")).unwrap();
    std::fs::write(
        workspace.join(".agents/skills/mine/SKILL.md"),
        skill("mine"),
    )
    .unwrap();
    std::fs::create_dir_all(workspace.join(".agents/skills/copied")).unwrap();
    std::fs::write(
        workspace.join(".agents/skills/copied/SKILL.md"),
        skill("copied"),
    )
    .unwrap();
    std::fs::write(
        workspace.join(".agents/skills/copied/.managed-by-hotline"),
        "",
    )
    .unwrap();

    let mut client = Client::connect(port, TOKEN).await;
    // The person's own folder is the machine's; this room reads an empty one.
    let nowhere = root.join("no-home-skills");
    std::fs::create_dir_all(&nowhere).unwrap();
    let pointed = client
        .call(
            "settings.update",
            json!({ "patch": { "skillsHome": nowhere.to_string_lossy() } }),
        )
        .await;
    assert_eq!(pointed["ok"], true, "{pointed}");
    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "cwd": workspace.to_string_lossy() } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();

    let listed = client.call("skills.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    let entries = listed["result"].as_array().unwrap();
    let summary: Vec<(String, String, Option<String>)> = entries
        .iter()
        .map(|one| {
            (
                one["source"].as_str().unwrap().to_string(),
                one["name"].as_str().unwrap().to_string(),
                one["invalid"].as_str().map(str::to_string),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("builtin".to_string(), "hotline-room".to_string(), None),
            (
                "gateway".to_string(),
                "Broken".to_string(),
                Some(
                    "The name Broken may only have lowercase letters, digits and hyphens."
                        .to_string()
                )
            ),
            ("gateway".to_string(), "cut-release".to_string(), None),
        ]
    );
    assert_eq!(entries[0]["path"], ".agents/skills/hotline-room/SKILL.md");
    assert_eq!(entries[2]["description"], "Use when testing cut-release.");

    let with_own = client
        .call("skills.list", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(with_own["ok"], true, "{with_own}");
    let own: Vec<&str> = with_own["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|one| one["source"] == "workspace")
        .map(|one| one["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        own,
        ["mine"],
        "a copied entry is the grant, not the teammate's own"
    );

    let nobody = client
        .call("skills.list", json!({ "personaId": "nobody" }))
        .await;
    assert_eq!(nobody["ok"], false, "{nobody}");

    // A grant is files in the workspace from the moment the teammate is
    // started, whichever driver runs it and even when the start is refused
    // for want of a key: the folder is part of the workspace, like AGENTS.md.
    let granted = client
        .call(
            "persona.update",
            json!({ "id": persona_id, "patch": { "skillPolicy": { "mode": "some", "names": ["cut-release"] } } }),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    assert_eq!(
        granted["result"]["skillPolicy"]["names"],
        json!(["cut-release"])
    );
    let refused = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    let folder = workspace.join(".agents/skills");
    assert!(folder.join("hotline-room/SKILL.md").exists());
    assert!(folder.join("hotline-room/.managed-by-hotline").exists());
    assert!(folder.join("cut-release/SKILL.md").exists());
    assert!(folder.join("cut-release/.managed-by-hotline").exists());
    assert!(
        !folder.join("copied").exists(),
        "a stale copy Hotline made is removed"
    );
    assert!(
        folder.join("mine/SKILL.md").exists(),
        "the teammate's own is untouched"
    );
    assert!(
        !folder.join("Broken").exists(),
        "an invalid gateway skill is never copied"
    );

    let revoked = client
        .call(
            "persona.update",
            json!({ "id": persona_id, "patch": { "skillPolicy": { "mode": "none", "names": [] } } }),
        )
        .await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    let refused = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(
        !folder.join("cut-release").exists(),
        "a revoked grant leaves nothing of Hotline's"
    );
    assert!(folder.join("hotline-room/SKILL.md").exists());
    assert!(folder.join("mine/SKILL.md").exists());

    // The gateway is filled from the window by picking a folder: a skill is
    // copied in under its own name, a folder that is not one is refused with
    // the reason, a name already there is refused, and removal is by name.
    let picked = root.join("picked").join("triage");
    std::fs::create_dir_all(picked.join("scripts")).unwrap();
    std::fs::write(picked.join("SKILL.md"), skill("triage")).unwrap();
    std::fs::write(picked.join("scripts/run.sh"), "#!/bin/sh\n").unwrap();
    let added = client
        .call("skills.add", json!({ "path": picked.to_string_lossy() }))
        .await;
    assert_eq!(added["ok"], true, "{added}");
    assert_eq!(added["result"]["source"], "gateway");
    assert_eq!(added["result"]["name"], "triage");
    assert!(gateway.join("triage/scripts/run.sh").exists());
    let again = client
        .call("skills.add", json!({ "path": picked.to_string_lossy() }))
        .await;
    assert_eq!(again["ok"], false, "{again}");
    assert!(
        again["error"]
            .as_str()
            .unwrap()
            .contains("already has a skill named triage")
    );
    let not_one = root.join("picked").join("notes");
    std::fs::create_dir_all(&not_one).unwrap();
    let refused = client
        .call("skills.add", json!({ "path": not_one.to_string_lossy() }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], "notes has no SKILL.md.");
    let gone = client
        .call("skills.remove", json!({ "name": "triage" }))
        .await;
    assert_eq!(gone["ok"], true, "{gone}");
    assert!(!gateway.join("triage").exists());
    let listed = client.call("skills.list", json!({})).await;
    assert!(
        !listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|one| one["name"] == "triage"),
        "{listed}"
    );
}

/// The person's own skills, in the standard folder or one the room names,
/// are offered by switch and granted like the gateway's: listed with what
/// is offered and what is wrong, read from where they are at every start
/// with nothing written into the folder, and gone from a workspace at the
/// next start once withdrawn.
#[tokio::test]
async fn the_persons_own_skills_are_offered_by_switch_and_granted_like_the_gateways() {
    let (root, port) = open("skills-home").await;
    let skill = |name: &str, when: &str| {
        format!("---\nname: {name}\ndescription: Use when {when}.\n---\n\nDo the thing.\n")
    };
    let home = root.join("home-skills");
    std::fs::create_dir_all(home.join("cut-release")).unwrap();
    std::fs::write(
        home.join("cut-release/SKILL.md"),
        skill("cut-release", "cutting a release"),
    )
    .unwrap();
    std::fs::create_dir_all(home.join("Broken")).unwrap();
    std::fs::write(home.join("Broken/SKILL.md"), skill("Broken", "nothing")).unwrap();
    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut client = Client::connect(port, TOKEN).await;
    // The room names the folder; unset, it is the machine's own standard one.
    let pointed = client
        .call(
            "settings.update",
            json!({ "patch": { "skillsHome": home.to_string_lossy() } }),
        )
        .await;
    assert_eq!(pointed["ok"], true, "{pointed}");
    let own = |listed: &Value| -> Vec<(String, Option<bool>, Option<String>)> {
        listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|one| one["source"] == "home")
            .map(|one| {
                (
                    one["name"].as_str().unwrap().to_string(),
                    one["offered"].as_bool(),
                    one["invalid"].as_str().map(str::to_string),
                )
            })
            .collect()
    };
    let listed = client.call("skills.list", json!({})).await;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(
        own(&listed),
        [
            (
                "Broken".to_string(),
                Some(false),
                Some(
                    "The name Broken may only have lowercase letters, digits and hyphens."
                        .to_string()
                )
            ),
            ("cut-release".to_string(), Some(false), None),
        ]
    );

    let offered = client
        .call(
            "skills.offer",
            json!({ "name": "cut-release", "offered": true }),
        )
        .await;
    assert_eq!(offered["ok"], true, "{offered}");
    assert_eq!(offered["result"]["source"], "home");
    assert_eq!(offered["result"]["offered"], true);
    assert_eq!(
        offered["result"]["path"].as_str(),
        Some(home.join("cut-release").to_string_lossy().as_ref())
    );
    let refused = client
        .call("skills.offer", json!({ "name": "Broken", "offered": true }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused["error"].as_str().unwrap().contains("lowercase"));
    let missing = client
        .call("skills.offer", json!({ "name": "nope", "offered": true }))
        .await;
    assert_eq!(missing["ok"], false, "{missing}");
    assert_eq!(
        missing["error"],
        "Your skills folder has no skill named nope."
    );
    let listed = client.call("skills.list", json!({})).await;
    assert_eq!(
        own(&listed)[1],
        ("cut-release".to_string(), Some(true), None)
    );

    // Granted like a gateway skill, it is copied from the person's folder at
    // the start, and the folder itself is never written.
    let created = client
        .call(
            "persona.create",
            json!({ "draft": { "name": "Ada", "cwd": workspace.to_string_lossy() } }),
        )
        .await;
    assert_eq!(created["ok"], true, "{created}");
    let persona_id = created["result"]["id"].as_str().unwrap().to_string();
    let granted = client
        .call(
            "persona.update",
            json!({ "id": persona_id, "patch": { "skillPolicy": { "mode": "some", "names": ["cut-release"] } } }),
        )
        .await;
    assert_eq!(granted["ok"], true, "{granted}");
    let refused = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    let folder = workspace.join(".agents/skills");
    assert!(folder.join("cut-release/SKILL.md").exists());
    assert!(folder.join("cut-release/.managed-by-hotline").exists());
    assert!(
        !home.join("cut-release/.managed-by-hotline").exists(),
        "the person's folder is read, never written"
    );

    // Withdrawn, it is gone from the workspace at the next start, and the
    // grant by name stays for the day it is offered again.
    let withdrawn = client
        .call(
            "skills.offer",
            json!({ "name": "cut-release", "offered": false }),
        )
        .await;
    assert_eq!(withdrawn["ok"], true, "{withdrawn}");
    assert_eq!(withdrawn["result"]["offered"], false);
    let refused = client
        .call("session.start", json!({ "personaId": persona_id }))
        .await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(!folder.join("cut-release").exists());
    assert!(home.join("cut-release/SKILL.md").exists());
}
