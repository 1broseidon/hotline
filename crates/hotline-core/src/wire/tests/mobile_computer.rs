use super::*;

#[test]
fn mobile_persona_computer_contract_distinguishes_absent_null_and_number() {
    for (params, expected) in [
        (json!({"id":"ada"}), None),
        (json!({"id":"ada","cpus":null}), Some(None)),
        (json!({"id":"ada","cpus":1.5}), Some(Some(1.5))),
    ] {
        let command: Command = serde_json::from_value(json!({
            "cmd":"mobile.persona_computer", "params":params,
        }))
        .unwrap();
        let Command::MobilePersonaComputer { cpus, .. } = &command else {
            panic!("wrong command")
        };
        assert_eq!(*cpus, expected);
        assert_eq!(serde_json::to_value(&command).unwrap()["params"], params);
        assert!(Seat::Owner.permits(&command));
        assert!(Seat::Desk.permits(&command));
        assert!(!Seat::Phone.permits(&command));
    }
    assert!(Seat::Owner.capabilities().contains(&"personaComputer"));
    assert!(!Seat::Phone.capabilities().contains(&"personaComputer"));
    for seat in [Seat::Desk, Seat::Owner, Seat::Phone] {
        assert!(seat.permits(&Command::ComputerCapacity {}));
    }
    use ts_rs::TS;
    let command_ts = Command::decl(&ts_rs::Config::default());
    let computer_ts = command_ts
        .split("mobile.persona_computer")
        .nth(1)
        .unwrap()
        .split(" | { \"cmd\"")
        .next()
        .unwrap();
    assert!(
        computer_ts.contains("cpus?: number | null,"),
        "{computer_ts}"
    );
    let capacity_ts = crate::contract::ComputerCapacity::decl(&ts_rs::Config::default());
    assert!(capacity_ts.contains("memoryBytes: number"), "{capacity_ts}");
    assert!(
        capacity_ts.contains("runtime: ComputerRuntime | null"),
        "{capacity_ts}"
    );
    let capacity = crate::contract::ComputerCapacity {
        runtime: None,
        cpus: 2,
        memory_bytes: 4 * 1024 * 1024 * 1024,
        source: crate::contract::ComputerCapacitySource::Default,
    };
    assert_eq!(
        json!(capacity),
        json!({"runtime":null,"cpus":2,"memoryBytes":4_294_967_296_u64,"source":"default"})
    );
}

#[tokio::test]
async fn mobile_persona_computer_owner_controls_preserve_other_settings() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("mobile-computer", quiet.clone());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let mut socket = desk(port).await;
    let id = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let request = |params| json!({"id":2,"cmd":"mobile.persona_computer","params":params});
    let denied = remote_control_answer(
        Seat::Phone,
        &handle,
        &log,
        &Parts::default(),
        request(json!({"id":id,"enabled":true})),
    )
    .await;
    assert_eq!(denied["code"], FORBIDDEN, "{denied}");
    assert!(room::roster(&log)[0].computer.is_none());
    let enabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        request(json!({"id":id,"enabled":true})),
    )
    .await;
    assert_eq!(enabled["ok"], true, "{enabled}");
    assert_eq!(enabled["result"]["id"], id);
    assert_eq!(enabled["result"]["computer"], json!({"enabled":true}));
    assert_eq!(quiet.reattached(), vec![id.clone()]);
    let full = json!({"enabled":true,"image":"pinned:test","memory":"4g","cpus":2.0,"pids":2048,
        "mounts":[{"host":"/projects","path":"/work","readonly":true}],"secrets":["TOKEN"]});
    let seed = remote_control_answer(
        Seat::Desk,
        &handle,
        &log,
        &Parts::default(),
        json!({"id":3,"cmd":"persona.update","params":{"id":id,"patch":{"computer":full}}}),
    )
    .await;
    assert_eq!(seed["ok"], true, "{seed}");
    let updated = remote_control_answer(Seat::Owner, &handle, &log, &Parts::default(), request(json!({
        "id":id,"memory":"4608m","cpus":1.5,"image":"untrusted","pids":1,"secrets":[],"mounts":[],"reach":"machine"
    }))).await;
    assert_eq!(updated["ok"], true, "{updated}");
    let mut wanted = full.clone();
    wanted["memory"] = json!("4608m");
    wanted["cpus"] = json!(1.5);
    assert_eq!(updated["result"]["computer"], wanted);
    assert!(updated["result"].get("reach").is_none());
    let absent = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        request(json!({"id":id,"memory":"5g"})),
    )
    .await;
    assert_eq!(absent["result"]["computer"]["cpus"], 1.5);
    let cleared = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        request(json!({"id":id,"cpus":null})),
    )
    .await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    assert!(cleared["result"]["computer"].get("cpus").is_none());
    assert_eq!(cleared["result"]["computer"]["image"], "pinned:test");
    assert_eq!(
        quiet.reattached(),
        vec![id.clone()],
        "resource edits must not restart"
    );
    assert_eq!(quiet.computer_changes.lock().unwrap().len(), 4);
    let disabled = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        request(json!({"id":id,"enabled":false})),
    )
    .await;
    assert_eq!(disabled["result"]["computer"]["enabled"], false);
    assert_eq!(quiet.reattached(), vec![id.clone(), id]);
}

#[tokio::test]
async fn mobile_persona_computer_validates_limits_atomically_against_fake_capacity() {
    let quiet = Arc::new(Quiet::new());
    let (_root, log, port) = door_with("mobile-computer-limits", quiet.clone());
    let handle: Arc<dyn RoomHandle> = quiet.clone();
    let mut socket = desk(port).await;
    let id = create(&mut socket, 1, "Ada").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    for seat in [Seat::Owner, Seat::Phone, Seat::Desk] {
        let answer = remote_control_answer(
            seat,
            &handle,
            &log,
            &Parts::default(),
            json!({"id":2,"cmd":"computer.capacity","params":{}}),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["result"],
            json!({"runtime":"podman","cpus":8,"memoryBytes":17_179_869_184_u64,"source":"runtime"})
        );
    }
    let empty = remote_control_answer(
        Seat::Owner,
        &handle,
        &log,
        &Parts::default(),
        json!({"id":3,"cmd":"mobile.persona_computer","params":{"id":id}}),
    )
    .await;
    assert_eq!(empty["ok"], false);
    assert!(
        empty["error"]
            .as_str()
            .unwrap()
            .contains("Nothing to change")
    );
    for patch in [
        json!({"cpus":0}),
        json!({"cpus":-0.5}),
        json!({"cpus":0.25}),
        json!({"cpus":8.5}),
        json!({"memory":"256m"}),
        json!({"memory":"513m"}),
        json!({"memory":"17g"}),
        json!({"memory":"1.5g"}),
        json!({"memory":"512.0m"}),
        json!({"memory":"+512m"}),
        json!({"memory":"512"}),
        json!({"memory":"512mb"}),
        json!({"memory":" 512m"}),
        json!({"memory":"18446744073709551615g"}),
        json!({"memory":""}),
        json!({"memory":"512m","cpus":9}),
        json!({"memory":"17g","cpus":1}),
    ] {
        let mut params = patch.clone();
        params["id"] = json!(id);
        params["enabled"] = json!(true);
        let refused = remote_control_answer(
            Seat::Owner,
            &handle,
            &log,
            &Parts::default(),
            json!({"id":4,"cmd":"mobile.persona_computer","params":params}),
        )
        .await;
        assert_eq!(refused["ok"], false, "{patch}: {refused}");
        assert!(
            room::roster(&log)[0].computer.is_none(),
            "invalid edits must not enable or partly save"
        );
    }
    for (cpus, memory) in [(0.5, "512m"), (8.0, "16g"), (1.0, "1024M"), (2.5, "2G")] {
        let answer = remote_control_answer(Seat::Owner, &handle, &log, &Parts::default(), json!({"id":5,"cmd":"mobile.persona_computer","params":{"id":id,"cpus":cpus,"memory":memory}})).await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["result"]["computer"]["cpus"], cpus);
        assert_eq!(answer["result"]["computer"]["memory"], memory);
        assert_eq!(
            answer["result"]["computer"]["enabled"], false,
            "limits alone are not an enable grant"
        );
    }
    // JSON cannot encode these, but the typed boundary must also reject them.
    for cpus in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let command = Command::MobilePersonaComputer {
            id: id.clone(),
            enabled: None,
            memory: None,
            cpus: Some(Some(cpus)),
        };
        assert!(
            commands::run(command, &log, &handle, &Parts::default())
                .await
                .unwrap_err()
                .contains("CPUs")
        );
    }
    assert_eq!(quiet.reattached(), [] as [String; 0]);
}
