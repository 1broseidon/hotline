//! Shared cookies across real desk restarts: what the room recorded, a card
//! answered Use and the old import all list as rows, In Use names their
//! holders, and a share whose end came while the desk was down is taken
//! back by the desk that opens next. No source here is a browser on the
//! host, so nothing reads the machine's own.
use super::*;

fn lend(id: &str, site: &str, ends_at: Option<i64>) -> Value {
    json!({
        "id": format!("cookie-lend:{id}"),
        "kind": "cookie_lend",
        "ts": 1,
        "lend": {
            "id": id, "teammate": "ada", "site": site,
            "browserId": "no-such-browser", "browserName": "Brave",
            "profileId": "Default", "profileName": "Default",
            "login": format!("lent-no-such-browser-Default-{id}"), "lentAt": 2,
            "endsAt": ends_at,
            "deskBrowser": false, "copied": true, "inComputer": false
        }
    })
}

async fn sites(client: &mut Client, persona: &str) -> Vec<String> {
    client
        .call("computer.cookies.lends", json!({"personaId": persona}))
        .await["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["site"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn shares_survive_real_desk_restarts_and_one_whose_end_came_meanwhile_is_taken_back() {
    use hotline_core::log::{Log, StreamId};
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    for id in ["ada", "ben"] {
        log.append(&StreamId::Room, &json!({"kind":"persona","id":id,"name":id,"cwd":root.path(),"goal":"Test","backendId":"hotline","mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
    }
    log.append(&StreamId::Room, &lend("kept", "linear.app", None))
        .unwrap();
    log.append(&StreamId::Room, &lend("ended", "github.com", Some(1)))
        .unwrap();
    log.append(&StreamId::Room, &json!({"id":"cookie-imports:ada","kind":"cookie-imports","personaId":"ada","imports":[{"browserId":"no-such-browser","browserName":"Chrome","profileId":"Default","profileName":"Default","importedAt":0,"sites":[{"domain":"gitlab.com","cookies":2}]}]})).unwrap();
    log.append(&StreamId::Tape("ben".into()), &json!({"kind":"cookie_offer","id":"card","ts":3,"teammate":"ben","site":"porkbun.com","browserName":"Firefox","profileName":"Default","login":"lent-firefox-default-card","status":"lent"})).unwrap();
    drop(log);
    for first in [true, false] {
        let desk = common::open_desk(root.path()).unwrap();
        let log = desk.log.clone();
        let parts = desk.parts();
        let door = Door::bind(log, TOKEN.into(), Arc::new(desk), parts).unwrap();
        let port = door.port();
        let serving = tokio::spawn(door.run());
        let mut client = Client::connect(port, TOKEN).await;
        // The ended share goes on the desk's own watch, without a press.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while sites(&mut client, "ada")
            .await
            .contains(&"github.com".to_string())
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the ended share stayed"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(
            sites(&mut client, "ada").await,
            ["gitlab.com", "linear.app"]
        );
        assert_eq!(sites(&mut client, "ben").await, ["porkbun.com"]);
        let holders = client.call("computer.cookies.holders", json!({})).await["result"].clone();
        assert_eq!(
            holders,
            json!([
                {"teammate":"ada","name":"ada","sites":["gitlab.com","linear.app"]},
                {"teammate":"ben","name":"ben","sites":["porkbun.com"]},
            ])
        );
        let rows = client
            .call("computer.cookies.lends", json!({"personaId":"ada"}))
            .await["result"]
            .clone();
        let kept = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["site"] == "linear.app")
            .unwrap()
            .clone();
        if first {
            let set = client
                .call(
                    "computer.cookies.lend.set",
                    json!({"personaId":"ada","id":"kept","ends":"day"}),
                )
                .await;
            assert!(set["result"]["endsAt"].as_i64().unwrap() > 2, "{set}");
        } else {
            assert!(
                kept["endsAt"].as_i64().unwrap() > 2,
                "Ends survives the restart: {kept}"
            );
        }
        drop(client);
        serving.abort();
        let _ = serving.await;
    }
}
