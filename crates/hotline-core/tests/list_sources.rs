//! Source counts and results through the real list handlers, with no model calls.
use crate::common::Client;
use hotline_core::{
    credentials::SecretStore,
    desk::Desk,
    log::{Log, StreamId},
    wire::Door,
};
use serde_json::json;
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct CountedStore {
    values: Mutex<HashMap<String, Vec<u8>>>,
    gets: AtomicUsize,
}
impl SecretStore for CountedStore {
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.values.lock().unwrap().insert(key.into(), bytes.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn list_sources_are_read_once_and_results_stay_current() {
    let root = tempfile::tempdir().unwrap();
    // Import fixtures before the core starts; its Log is the only live writer.
    let seed = Log::open(root.path());
    for id in ["ada", "bob", "cal"] {
        seed.append(
            &StreamId::Room,
            &json!({"kind":"persona", "id":id, "name":id,
            "goal":"Help", "backendId":"hotline", "cwd":root.path(), "mcpPolicy":{"mode":"none","serverIds":[]},
            "sessionCheckpoints":[], "createdAt":1,"updatedAt":1}),
        )
        .unwrap();
        let link = json!({"kind":"link", "id":format!("link:side:{id}"), "ts":2,
            "thread":id, "threadKind":"side", "personaId":id, "title":id, "state":"parked"});
        seed.append(&StreamId::Side(id.into()), &link).unwrap();
        seed.append(&StreamId::Tape(id.into()), &link).unwrap();
        seed.append(
            &StreamId::Side(id.into()),
            &json!({"kind":"agent", "id":"answer", "ts":3, "text":"Done"}),
        )
        .unwrap();
    }
    hotline_core::log::thread::ensure(root.path(), "ada~bob").unwrap();
    drop(seed);
    let secrets = Arc::new(CountedStore::default());
    let desk = Desk::open_with_store(root.path(), secrets.clone()).unwrap();
    let log = desk.log.clone();
    let parts = desk.parts();
    let door = Door::bind(log.clone(), "lists-test".into(), Arc::new(desk), parts).unwrap();
    let port = door.port();
    let server = tokio::spawn(door.run());
    let mut client = Client::connect(port, "lists-test").await;
    let credential = client
        .call(
            "credential.create",
            json!({"providerId":"anthropic", "label":"fixture", "secret":"fixture-key"}),
        )
        .await;
    assert_eq!(credential["ok"], true, "{credential}");
    secrets.gets.store(0, Ordering::SeqCst);
    log.take_load_counts();
    let models = client.call("models.list", json!({})).await;
    let gets = secrets.gets.load(Ordering::SeqCst);
    let reads = log.take_load_counts();
    let models = models["result"].as_array().unwrap();
    assert_ne!(models.as_slice(), [] as [serde_json::Value; 0]);
    eprintln!("model results: {}", serde_json::to_string(models).unwrap());
    eprintln!(
        "models: results={} secret_gets={gets} loads={reads:?}",
        models.len()
    );
    assert_eq!(gets, 1);
    assert_eq!(reads.get(&StreamId::Room), Some(&1));
    for (params, expected) in [
        (json!({}), 7),
        (json!({"personaId":"ada"}), 3),
        (json!({"personaId":"bob"}), 3),
        (json!({"personaId":"cal"}), 2),
    ] {
        log.take_load_counts();
        let result = client.call("thread.list", params.clone()).await;
        let reads = log.take_load_counts();
        let threads = result["result"].as_array().unwrap();
        eprintln!(
            "threads {params}: results={} loads={reads:?}",
            threads.len()
        );
        assert_eq!(threads.len(), expected, "{result}");
        eprintln!(
            "thread results: {}",
            serde_json::to_string(threads).unwrap()
        );
        assert!(reads.values().all(|count| *count == 1), "{reads:?}");
        if let Some(selected) = params["personaId"].as_str() {
            for other in ["ada", "bob", "cal"]
                .into_iter()
                .filter(|id| *id != selected)
            {
                assert!(!reads.contains_key(&StreamId::Side(other.into())));
            }
            let side = threads
                .iter()
                .find(|t| t["thread"]["kind"] == "side")
                .unwrap();
            assert_eq!(side["thread"]["key"], selected);
            assert_eq!(side["preview"], "Done");
            assert_eq!(side["state"], "parked");
        }
    }
    let revoked = client
        .call(
            "credential.revoke",
            json!({"id":credential["result"]["id"]}),
        )
        .await;
    assert_eq!(revoked["ok"], true, "{revoked}");
    assert_eq!(
        client.call("models.list", json!({})).await["result"],
        json!([])
    );
    let replacement = client
        .call(
            "credential.create",
            json!({"providerId":"anthropic", "label":"replacement", "secret":"replacement-key"}),
        )
        .await;
    assert_eq!(replacement["ok"], true, "{replacement}");
    assert_eq!(
        client.call("models.list", json!({})).await["result"],
        json!(models)
    );
    server.abort();
}
