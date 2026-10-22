use super::*;

#[tokio::test]
async fn cookie_offers_expire_on_real_desk_restart_while_lends_and_per_teammate_never_survive() {
    use hotline_core::log::{Log, StreamId};
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    for id in ["ada", "ben"] {
        log.append(&StreamId::Room, &json!({"kind":"persona","id":id,"name":id,"cwd":root.path(),"goal":"Test","backendId":"hotline","mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
    }
    log.append(&StreamId::Room, &json!({"id":"cookie-offer-never:ada","kind":"cookie_offer_never","ts":1,"teammate":"ada","sites":["porkbun.com"]})).unwrap();
    for (id, status) in [("waiting", "pending"), ("accepted", "lent")] {
        log.append(&StreamId::Tape("ada".into()), &json!({"kind":"cookie_offer","id":id,"ts":1,"teammate":"ada","site":"example.com","browserName":"Brave","profileName":"Default","login":"lent-brave-default","status":status})).unwrap();
    }
    drop(log);
    for reset in [true, false] {
        let desk = common::open_desk(root.path()).unwrap();
        let log = desk.log.clone();
        let parts = desk.parts();
        let door = Door::bind(log.clone(), TOKEN.into(), Arc::new(desk), parts).unwrap();
        let port = door.port();
        let serving = tokio::spawn(door.run());
        let mut client = Client::connect(port, TOKEN).await;
        let blocked = client.call("computer.cookies.blocked", json!({})).await;
        assert_eq!(
            blocked["result"].as_array().unwrap().len(),
            usize::from(reset)
        );
        if reset {
            assert_eq!(blocked["result"][0]["teammate"], "ada");
            assert_eq!(
                client
                    .call(
                        "computer.cookies.reset",
                        json!({"personaId":"ada","site":"porkbun.com"})
                    )
                    .await["ok"],
                true
            );
        }
        assert_eq!(
            client
                .call(
                    "computer.cookies.answer",
                    json!({"personaId":"ada","id":"waiting","answer":"use"})
                )
                .await["ok"],
            false
        );
        let offers = client
            .call("computer.cookies.offers", json!({"personaId":"ada"}))
            .await;
        assert_eq!(offers["result"][0]["status"], "expired");
        assert_eq!(offers["result"][1]["status"], "lent");
        assert_eq!(
            client
                .call("computer.cookies.offers", json!({"personaId":"ben"}))
                .await["result"],
            json!([])
        );
        drop(client);
        serving.abort();
        let _ = serving.await;
    }
}

#[tokio::test]
async fn cookie_offer_settings_survive_real_desk_restarts_and_clearing_restores_all_sites() {
    let root = tempfile::tempdir().unwrap();
    let mut expected = json!({"enabled":true});
    for next in [
        Some(json!({"enabled":false,"sites":["porkbun.com"]})),
        Some(json!({"enabled":true,"sites":[]})),
        Some(Value::Null),
        None,
    ] {
        let desk = common::open_desk(root.path()).unwrap();
        let log = desk.log.clone();
        let parts = desk.parts();
        let door = Door::bind(log, TOKEN.into(), Arc::new(desk), parts).unwrap();
        let port = door.port();
        let serving = tokio::spawn(door.run());
        let mut client = Client::connect(port, TOKEN).await;
        assert_eq!(
            client.call("computer.cookies.settings", json!({})).await["result"],
            expected
        );
        if let Some(next) = next {
            let response = client
                .call("settings.update", json!({"patch":{"cookieOffers":next}}))
                .await;
            assert_eq!(response["ok"], true);
            expected = if next.is_null() {
                json!({"enabled":true})
            } else {
                next
            };
        }
        drop(client);
        serving.abort();
        let _ = serving.await;
    }
}
