use super::*;

#[tokio::test]
async fn desktop_legacy_routes_are_absent_and_old_grants_only_remain_for_revocation() {
    use tokio_tungstenite::tungstenite::Error;

    let mut h = Harness::new().await;
    let sealed_phone = h.pair().await;
    h.remote
        .authenticate_v2(&sealed_phone.public)
        .unwrap()
        .register_push("ExponentPushToken[sealed]".into(), "ios".into())
        .unwrap();
    h.remote.configure(false, network::ALL).await.unwrap();
    let path = h.root.path().join("remote.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let bearer = "old-phone-bearer";
    saved["grants"].as_array_mut().unwrap().push(json!({
        "device": {"id":"old-phone", "name":"Before 0.33", "pairedAt":1},
        "tokenHash":hash(bearer),
        "push":{"token":"ExponentPushToken[old]", "platform":"ios"}
    }));
    atomic_write(&path, &serde_json::to_vec(&saved).unwrap()).unwrap();
    h.remote = h.reopen(h.store.clone());
    h.remote.configure(true, network::ALL).await.unwrap();

    let listed = h.remote.devices();
    assert_eq!(listed.len(), 2);
    let old = listed
        .iter()
        .find(|device| device.id == "old-phone")
        .unwrap();
    assert_eq!(old.name, "Before 0.33");
    assert_eq!(old.role, DeviceRole::Owner);
    assert!(old.public_key.is_none());
    assert_eq!(
        serde_json::to_value(h.remote.status().devices).unwrap(),
        serde_json::to_value(listed).unwrap()
    );
    assert!(h.remote.authenticate_v2(&sealed_phone.public).is_some());
    assert!(
        h.remote
            .authenticate_v2(&hex::decode(hash(bearer)).unwrap())
            .is_none()
    );
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        ["ExponentPushToken[sealed]"]
    );

    for token in ["", bearer, "desk-token"] {
        for path in ["/pair", "/pair/manual/start", "/pair/manual/finish"] {
            assert_eq!(
                h.http
                    .post(format!("{}{path}", h.endpoint()))
                    .bearer_auth(token)
                    .json(&json!({"claimId":Uuid::new_v4(), "secret":"old"}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                404,
                "legacy pairing route {path} still exists"
            );
        }
        for path in ["/ws", "/computer/ada/ws"] {
            assert_eq!(
                h.http
                    .get(format!("{}{path}", h.endpoint()))
                    .bearer_auth(token)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                404
            );
            let Error::Http(refused) = h.socket_at(path, token).await.err().unwrap() else {
                panic!("legacy wire route {path} did not return an HTTP refusal");
            };
            assert_eq!(refused.status(), 404);
        }
    }
    let (private, _) = sealed::keypair().unwrap();
    let (_, public) = h.remote.noise_keys().unwrap();
    let (mut socket, _, answer) = sealed_network::exchange(&h, "/v2", &private, &public, &[])
        .await
        .unwrap();
    assert_eq!(answer, sealed::DEVICE_REJECTED);
    sealed_network::closed(&mut socket).await;

    h.remote.revoke("old-phone").unwrap();
    assert_eq!(h.remote.devices().len(), 1);
    assert_eq!(h.remote.devices()[0].id, sealed_phone.id);
    assert_eq!(
        crate::remote::push_targets(h.root.path()).tokens,
        ["ExponentPushToken[sealed]"]
    );
    h.remote.configure(false, network::ALL).await.unwrap();
}

#[tokio::test]
async fn old_grants_do_not_consume_the_sealed_device_limit() {
    let h = Harness::new().await;
    {
        let mut state = h.remote.state.lock().unwrap();
        for id in 0..16 {
            state.saved.grants.push(Grant {
                device: RemoteDevice {
                    id: format!("old-phone-{id}"),
                    name: "Before 0.33".into(),
                    paired_at: 1,
                    role: DeviceRole::Owner,
                    public_key: None,
                    current: None,
                },
                push: None,
            });
        }
        h.remote.save(&state.saved).unwrap();
    }
    for _ in 0..16 {
        h.pair().await;
    }
    assert_eq!(h.remote.devices().len(), 32);
    let pairing = h.remote.pairing_v2(DeviceRole::Owner).unwrap();
    let (private, _) = sealed::keypair().unwrap();
    assert!(
        sealed_network::claim(&h, &pairing, &private)
            .await
            .is_none()
    );
    assert!(h.remote.pairing_result(&pairing.id).unwrap().is_none());
    assert_eq!(h.remote.devices().len(), 32);
    h.remote.configure(false, network::ALL).await.unwrap();
}
