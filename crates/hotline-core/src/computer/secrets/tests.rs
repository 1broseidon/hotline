use super::*;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post, put};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// What the stand-in computer says it is at `/capabilities`.
#[derive(Clone, Copy, PartialEq)]
enum Is {
    /// A release before 0.13: no such door.
    Old,
    /// 0.13 booted as one user.
    Legacy,
    /// 0.13 split, attesting with the bearer (v1, rc.2 and before).
    Split,
    /// Says split, attests with another key (v1).
    Forged,
    /// 0.13.0-rc.3 split: v2, attesting with its boot's key, which
    /// `POST /capabilities/key` hands once.
    SplitV2,
}

/// How a v2 stand-in spoils its proof.
#[derive(Clone, Copy, PartialEq)]
enum Spoil {
    None,
    /// Made with a key that is not the one it handed.
    WrongKey,
    /// Made for `legacy` while the answer says `split`.
    Mode,
    /// Made for another release than the answer names.
    Version,
    /// Made for an earlier nonce, and sent again.
    Replay,
}

/// The key a v2 stand-in's boot makes: fixed, so a test can look for it.
const BOOT_KEY: [u8; 32] = [0x5a; 32];

fn mac(key: &[u8], input: &str) -> String {
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    mac.update(input.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// A v1 `/capabilities` answer for `nonce`, attested under `key`.
fn answer(key: &str, boot: &str, nonce: &str, mode: &str) -> Value {
    let attest = mac(key.as_bytes(), &format!("hotline-broker-v1|{boot}|{nonce}"));
    json!({"version": "0.13.0", "boot": boot, "protected": {"v": 1, "mode": mode}, "attest": attest})
}

/// A v2 `/capabilities` answer for `nonce`, attested under `key`, as
/// 0.13.0-rc.3 makes one.
fn answer_v2(key: &[u8], boot: &str, nonce: &str, mode: &str) -> Value {
    let version = "0.13.0-rc.3";
    let attest = mac(
        key,
        &format!("hotline-broker-v2|{boot}|{mode}|{version}|{nonce}"),
    );
    json!({"version": version, "boot": boot, "attest": attest,
           "protected": {"v": 2, "mode": mode, "loopback": "readable", "other": "readable", "readable": []}})
}

#[test]
fn an_attest_is_the_bearers_mac_of_the_broker_context_the_boot_and_this_nonce() {
    assert_eq!(
        attest_input_v1("boot-1", "n1"),
        "hotline-broker-v1|boot-1|n1"
    );
    let key = b"the-token";
    let good = answer("the-token", "boot-1", "n1", "split");
    assert_eq!(verify_attest_v1(key, "n1", &good), Door::Protected);
}

#[test]
fn a_split_claim_without_its_proof_is_unproven() {
    let key = b"the-token";
    let replayed = answer("the-token", "boot-1", "an-older-nonce", "split");
    let forged = answer("not-the-token", "boot-1", "n1", "split");
    let mut altered = answer("the-token", "boot-1", "n1", "split");
    altered["boot"] = json!("boot-2");
    let mut missing = answer("the-token", "boot-1", "n1", "split");
    missing.as_object_mut().unwrap().remove("attest");
    let mut garbled = answer("the-token", "boot-1", "n1", "split");
    garbled["attest"] = json!("not base64!");
    let mut no_boot = answer("the-token", "", "n1", "split");
    no_boot["boot"] = json!("");
    for (what, answer) in [
        ("replayed", replayed),
        ("forged", forged),
        ("altered boot", altered),
        ("missing", missing),
        ("garbled", garbled),
        ("no boot", no_boot),
    ] {
        assert_eq!(
            verify_attest_v1(key, "n1", &answer),
            Door::Unproven,
            "{what}"
        );
    }
}

#[test]
fn a_computer_that_does_not_claim_split_is_unprotected_whatever_it_attests() {
    let key = b"the-token";
    let legacy = answer("the-token", "boot-1", "n1", "legacy");
    let mut later = answer("the-token", "boot-1", "n1", "split");
    later["protected"]["v"] = json!(3);
    let mut none = answer("the-token", "boot-1", "n1", "split");
    none.as_object_mut().unwrap().remove("protected");
    for answer in [legacy, later, none] {
        assert_eq!(verify_attest_v1(key, "n1", &answer), Door::Unprotected);
    }
}

#[test]
fn a_v2_attest_is_the_boots_keys_mac_of_the_boot_mode_release_and_nonce() {
    assert_eq!(
        attest_input_v2("boot-1", "split", "0.13.0-rc.3", "n1"),
        "hotline-broker-v2|boot-1|split|0.13.0-rc.3|n1"
    );
    let good = answer_v2(&BOOT_KEY, "boot-1", "n1", "split");
    assert_eq!(verify_attest_v2(&BOOT_KEY, "n1", &good), Door::Protected);

    let wrong_key = answer_v2(&[0x11; 32], "boot-1", "n1", "split");
    let bearer = answer_v2(b"the-token", "boot-1", "n1", "split");
    let replayed = answer_v2(&BOOT_KEY, "boot-1", "an-older-nonce", "split");
    let mut mode = answer_v2(&BOOT_KEY, "boot-1", "n1", "legacy");
    mode["protected"]["mode"] = json!("split");
    let mut version = answer_v2(&BOOT_KEY, "boot-1", "n1", "split");
    version["version"] = json!("0.13.0");
    let mut boot = answer_v2(&BOOT_KEY, "boot-1", "n1", "split");
    boot["boot"] = json!("boot-2");
    let mut missing = answer_v2(&BOOT_KEY, "boot-1", "n1", "split");
    missing.as_object_mut().unwrap().remove("attest");
    let mut v1_made = answer_v2(&BOOT_KEY, "boot-1", "n1", "split");
    v1_made["attest"] = answer("the-token", "boot-1", "n1", "split")["attest"].clone();
    for (what, answer) in [
        ("wrong key", wrong_key),
        ("the bearer", bearer),
        ("replayed nonce", replayed),
        ("altered mode", mode),
        ("altered version", version),
        ("altered boot", boot),
        ("missing", missing),
        ("a v1 proof", v1_made),
    ] {
        assert_eq!(
            verify_attest_v2(&BOOT_KEY, "n1", &answer),
            Door::Unproven,
            "{what}"
        );
    }
}

#[test]
fn a_site_on_the_computer_itself_is_loopback_in_any_spelling() {
    for site in [
        "http://localhost:8600",
        "localhost",
        "https://app.localhost",
        "http://127.0.0.1:3000",
        "http://127.1:3000",
        "http://0x7f.1",
        "http://2130706433",
        "http://[::1]:8080",
        "http://[::ffff:127.0.0.1]",
        "LOCALHOST",
    ] {
        assert!(loopback_site(site), "{site}");
    }
    for site in [
        "https://github.com",
        "github.com",
        "http://128.0.0.1",
        "https://localhost.example",
        "http://[2001:db8::1]",
        "not a site at all",
        "",
    ] {
        assert!(!loopback_site(site), "{site}");
    }
}

/// A body as it arrived, with its content type.
type Taken = (Option<String>, Value);

#[derive(Clone)]
struct Fake {
    is: Is,
    /// Every body that arrived with the right bearer, and its type.
    taken: Arc<Mutex<Vec<Taken>>>,
    /// The boot a v2 stand-in is on, and the key it made for it.
    boot: Arc<Mutex<(String, [u8; 32])>>,
    /// Whether that boot's key was handed out.
    claimed: Arc<AtomicBool>,
    spoil: Arc<Mutex<Spoil>>,
    /// How many times its key was asked for.
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

impl Fake {
    /// The stand-in restarts: a new boot, with a new key nobody has taken.
    fn reboot(&self, boot: &str, key: [u8; 32]) {
        *self.boot.lock().unwrap() = (boot.to_owned(), key);
        self.claimed.store(false, Ordering::SeqCst);
    }

    /// Something else asked for this boot's key before the desk did.
    fn taken_by_another(&self) {
        self.claimed.store(true, Ordering::SeqCst);
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

async fn secrets(State(fake): State<Fake>, headers: HeaderMap, body: String) -> StatusCode {
    if bearer(&headers) != Some("the-token") {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(set) = serde_json::from_str::<Value>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    let kind = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    fake.taken.lock().unwrap().push((kind, set));
    StatusCode::NO_CONTENT
}

/// What a v2 stand-in says it cannot keep from its programs, of the set it
/// holds now: as 0.13.0-rc.3, a login or passkey for a site on itself and a
/// key used anywhere but one named service.
fn readable_of(set: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for (name, entry) in set["secrets"].as_object().into_iter().flatten() {
        let loopback = entry["sites"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(std::iter::once(&entry["rpId"]))
            .filter_map(Value::as_str)
            .any(|site| site.contains("localhost"));
        let other = entry["use"]["origins"]
            .as_array()
            .is_some_and(|origins| !origins.contains(&json!("https://api.github.com")));
        if loopback || other {
            names.push(name.clone());
        }
    }
    names
}

async fn capabilities(
    State(fake): State<Fake>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> (StatusCode, String) {
    let Some(token) = bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, String::new());
    };
    let nonce = uri
        .query()
        .and_then(|query| query.strip_prefix("nonce="))
        .unwrap_or_default();
    if fake.is == Is::SplitV2 {
        let (boot, key) = fake.boot.lock().unwrap().clone();
        let mut body = match *fake.spoil.lock().unwrap() {
            Spoil::None => answer_v2(&key, &boot, nonce, "split"),
            Spoil::WrongKey => answer_v2(&[0x11; 32], &boot, nonce, "split"),
            Spoil::Mode => {
                let mut made = answer_v2(&key, &boot, nonce, "legacy");
                made["protected"]["mode"] = json!("split");
                made
            }
            Spoil::Version => {
                let mut made = answer_v2(&key, &boot, nonce, "split");
                made["version"] = json!("0.13.1");
                made
            }
            Spoil::Replay => answer_v2(&key, &boot, "a-nonce-asked-before", "split"),
        };
        let held = fake
            .taken
            .lock()
            .unwrap()
            .last()
            .map(|(_, set)| set.clone());
        body["protected"]["readable"] = json!(held.as_ref().map(readable_of).unwrap_or_default());
        return (StatusCode::OK, body.to_string());
    }
    let key = if fake.is == Is::Forged {
        "not-the-token"
    } else {
        token
    };
    let mode = if fake.is == Is::Legacy {
        "legacy"
    } else {
        "split"
    };
    let body = answer(key, "boot-1", nonce, mode);
    (StatusCode::OK, body.to_string())
}

/// `POST /capabilities/key`: this boot's key, once, to the bearer's holder.
async fn attest_key(State(fake): State<Fake>, headers: HeaderMap) -> (StatusCode, String) {
    if bearer(&headers) != Some("the-token") {
        return (StatusCode::UNAUTHORIZED, String::new());
    }
    fake.asked.fetch_add(1, Ordering::SeqCst);
    if fake.claimed.swap(true, Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            json!({"error": "this boot's attest key was already taken"}).to_string(),
        );
    }
    let (boot, key) = fake.boot.lock().unwrap().clone();
    let key = base64::engine::general_purpose::STANDARD.encode(key);
    (
        StatusCode::OK,
        json!({"v": 2, "boot": boot, "key": key}).to_string(),
    )
}

/// A stand-in computer; `with_route` false is a release from before
/// secrets, which knows no such path.
async fn computer(with_route: bool, is: Is) -> (Ready, Fake, tokio::task::JoinHandle<()>) {
    let fake = Fake {
        is,
        taken: Arc::default(),
        boot: Arc::new(Mutex::new(("boot-1".to_owned(), BOOT_KEY))),
        claimed: Arc::default(),
        spoil: Arc::new(Mutex::new(Spoil::None)),
        asked: Arc::default(),
    };
    let mut app = axum::Router::new();
    if with_route {
        app = app.route("/secrets", put(secrets));
    }
    if is != Is::Old {
        app = app.route("/capabilities", get(capabilities));
    }
    if is == Is::SplitV2 {
        app = app.route("/capabilities/key", post(attest_key));
    }
    let app = app.with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let ready = Ready {
        made_to_split: true,
        ..Ready::new(format!("http://127.0.0.1:{port}/mcp"), "the-token")
    };
    (ready, fake, serving)
}

fn granted(secret: StoredSecret, readable: bool) -> Granted {
    Granted { secret, readable }
}

fn login() -> StoredSecret {
    StoredSecret::Login {
        r#use: crate::contract::SecretUse::Auto,
        sites: vec!["https://github.com".to_owned()],
        username: "george".to_owned(),
        password: "correct horse battery".to_owned(),
        totp: None,
    }
}

fn variable(value: &str, bound: bool) -> StoredSecret {
    StoredSecret::Variable {
        value: value.to_owned(),
        r#use: bound.then(|| {
            crate::vault::resolve_used_with(&crate::contract::UsedWith {
                service: "github".into(),
                host: None,
            })
            .unwrap()
        }),
    }
}

/// A readable variable, a key Used with GitHub, and a login.
fn the_set() -> BTreeMap<String, Granted> {
    BTreeMap::from([
        (
            "NPM_TOKEN".to_owned(),
            granted(variable("npm_notarealtoken01", false), true),
        ),
        (
            "GITHUB_TOKEN".to_owned(),
            granted(
                variable("ghp_notarealtoken00000000000000000000001", true),
                false,
            ),
        ),
        ("GITHUB".to_owned(), granted(login(), false)),
    ])
}

#[tokio::test]
async fn an_old_or_legacy_computer_is_handed_what_it_was_and_no_protected_key() {
    for is in [Is::Old, Is::Legacy] {
        let (ready, fake, serving) = computer(true, is).await;
        let handed = deliver(&ready, &the_set()).await.expect("taken");
        assert_eq!(
            handed,
            Handed {
                door: Door::Unprotected,
                waiting: vec!["GITHUB_TOKEN".into()],
                readable: Vec::new(),
            }
        );
        let taken = fake.taken.lock().unwrap().clone();
        // The v1 form, which every release takes: the variable as its
        // bare value, the login as its record, as 0.49 sent them.
        assert_eq!(taken.len(), 1);
        assert_ne!(taken[0].0.as_deref(), Some(V2));
        assert_eq!(taken[0].1["NPM_TOKEN"], "npm_notarealtoken01");
        assert_eq!(taken[0].1["GITHUB"]["kind"], "login");
        assert!(taken[0].1.get("GITHUB_TOKEN").is_none());
        serving.abort();
    }
}

#[tokio::test]
async fn a_split_computer_that_attests_is_handed_every_grant_in_the_v2_form() {
    let (ready, fake, serving) = computer(true, Is::Split).await;
    let handed = deliver(&ready, &the_set()).await.expect("taken");
    assert_eq!(
        handed,
        Handed {
            door: Door::Protected,
            waiting: Vec::new(),
            readable: Vec::new(),
        }
    );
    deliver(&ready, &BTreeMap::new()).await.expect("cleared");
    let taken = fake.taken.lock().unwrap().clone();
    assert_eq!(taken[0].0.as_deref(), Some(V2));
    let body = &taken[0].1;
    assert_eq!(body["v"], 2);
    assert!(taken[1].1["generation"].as_i64() > body["generation"].as_i64());
    assert_eq!(
        body["secrets"]["NPM_TOKEN"],
        json!({"kind": "variable", "value": "npm_notarealtoken01", "readable": true})
    );
    let key = &body["secrets"]["GITHUB_TOKEN"];
    assert_eq!(key["use"]["header"], "Authorization");
    assert!(
        key["use"]["origins"]
            .as_array()
            .unwrap()
            .contains(&json!("https://api.github.com"))
    );
    assert_eq!(
        key["use"]["standIn"],
        json!({"prefix": "ghp_", "length": 40})
    );
    assert!(key.get("readable").is_none());
    assert_eq!(body["secrets"]["GITHUB"]["kind"], "login");
    assert_eq!(taken[1].1["secrets"], json!({}), "an empty set clears it");
    serving.abort();
}

#[tokio::test]
async fn a_forged_attest_gets_nothing() {
    let (ready, fake, serving) = computer(true, Is::Forged).await;
    let handed = deliver(&ready, &the_set()).await.unwrap();
    assert_eq!(handed.door, Door::Unproven);
    assert_eq!(handed.waiting, ["GITHUB", "GITHUB_TOKEN", "NPM_TOKEN"]);
    assert!(fake.taken.lock().unwrap().is_empty(), "not even a variable");
    serving.abort();
}

#[tokio::test]
async fn a_release_from_before_secrets_is_told_apart_from_a_failure() {
    let (ready, _, serving) = computer(false, Is::Old).await;
    assert_eq!(
        deliver(&ready, &BTreeMap::new()).await,
        Err(Refusal::TooOld)
    );
    serving.abort();

    let (ready, fake, serving) = computer(true, Is::Old).await;
    let wrong = Ready {
        token: "not-the-token".to_owned(),
        ..ready
    };
    let refused = deliver(&wrong, &BTreeMap::new()).await.unwrap_err();
    assert!(
        matches!(&refused, Refusal::Failed(reason) if reason.contains("401")),
        "{refused:?}"
    );
    assert!(fake.taken.lock().unwrap().is_empty());
    serving.abort();
}

#[tokio::test]
async fn capture_delivery_failure_cannot_echo_values_into_the_desk_log() {
    let app = axum::Router::new().route(
        "/secrets",
        put(|body: String| async move { (StatusCode::BAD_REQUEST, body) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ready = Ready::new(
        format!("http://{}/mcp", listener.local_addr().unwrap()),
        "test-token",
    );
    let serving = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let secret = StoredSecret::Variable {
        value: "capture-private-value".into(),
        r#use: None,
    };
    let debug = format!("{secret:?}");
    let error = deliver(
        &ready,
        &BTreeMap::from([("GITHUB_TOKEN".into(), granted(secret, true))]),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, Refusal::Failed(reason) if reason.contains("400")));
    for sink in [debug, format!("{error:?}")] {
        assert!(!sink.contains("capture-private-value"));
    }
    serving.abort();
}

/// The set, with a login and a passkey for a site on the computer itself.
fn the_v2_set() -> BTreeMap<String, Granted> {
    let mut set = the_set();
    set.insert(
        "LOCAL".to_owned(),
        granted(
            StoredSecret::Login {
                r#use: crate::contract::SecretUse::Auto,
                sites: vec!["http://localhost:8600".to_owned()],
                username: "rc-user".to_owned(),
                password: "not a real password".to_owned(),
                totp: None,
            },
            false,
        ),
    );
    set.insert(
        "LOCAL_KEY".to_owned(),
        granted(
            StoredSecret::Passkey {
                r#use: crate::contract::SecretUse::Auto,
                rp_id: "localhost".to_owned(),
                credential_id: "AQID".to_owned(),
                private_key: "bm90IGEgcmVhbCBrZXk=".to_owned(),
                user_handle: None,
                user_name: None,
                user_display_name: None,
            },
            false,
        ),
    );
    set
}

#[tokio::test]
async fn a_v2_computer_proves_itself_with_the_key_its_boot_handed_once() {
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    take_attest_key(&ready).await;
    assert!(fake.claimed.load(Ordering::SeqCst), "taken as it starts");
    let handed = deliver(&ready, &the_v2_set()).await.expect("taken");
    assert_eq!(handed.door, Door::Protected);
    assert!(handed.waiting.is_empty(), "{handed:?}");
    assert_eq!(
        handed.readable,
        ["LOCAL", "LOCAL_KEY"],
        "what it said of the set it took"
    );
    assert_eq!(
        fake.asked.load(Ordering::SeqCst),
        1,
        "the key held is used, never asked for again"
    );
    let taken = fake.taken.lock().unwrap().clone();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].0.as_deref(), Some(V2));
    let secrets = &taken[0].1["secrets"];
    assert_eq!(secrets["GITHUB_TOKEN"]["use"]["header"], "Authorization");
    // A login or passkey for the computer itself goes readable, so it is
    // still typed and signed there; one for any other site does not.
    assert_eq!(secrets["LOCAL"]["readable"], true);
    assert_eq!(secrets["LOCAL_KEY"]["readable"], true);
    assert!(secrets["GITHUB"].get("readable").is_none());
    serving.abort();

    // A desk that did not take it as the computer started takes it at its
    // first asking, if the boot still hands it.
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    assert_eq!(door(&ready).await, Ok(Door::Protected));
    assert_eq!(door(&ready).await, Ok(Door::Protected));
    assert_eq!(fake.asked.load(Ordering::SeqCst), 1);
    serving.abort();
}

#[tokio::test]
async fn a_v2_computer_this_desk_holds_no_key_for_is_not_protected() {
    let no_key = |taken: &[Taken]| {
        // What a desk that knew only v1 hands it: the v1 form, and nothing
        // with a "Used with" service.
        assert_eq!(taken.len(), 1);
        assert_ne!(taken[0].0.as_deref(), Some(V2));
        assert!(taken[0].1.get("GITHUB_TOKEN").is_none(), "{:?}", taken[0].1);
        assert_eq!(taken[0].1["NPM_TOKEN"], "npm_notarealtoken01");
    };

    // Its boot's key was taken before this desk asked: a 409, no key.
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    fake.taken_by_another();
    take_attest_key(&ready).await;
    let handed = deliver(&ready, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Unprotected);
    assert_eq!(handed.waiting, ["GITHUB_TOKEN"]);
    assert_eq!(handed.readable, Vec::<String>::new());
    no_key(&fake.taken.lock().unwrap());
    serving.abort();

    // It booted again and something else took the new boot's key: the key
    // held is for a boot that is gone, and proves nothing of this one.
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    take_attest_key(&ready).await;
    assert_eq!(door(&ready).await, Ok(Door::Protected));
    fake.reboot("boot-2", [0x77; 32]);
    fake.taken_by_another();
    let handed = deliver(&ready, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Unprotected);
    assert_eq!(handed.waiting, ["GITHUB_TOKEN"]);
    no_key(&fake.taken.lock().unwrap());

    // A boot whose key this desk takes proves itself again.
    fake.reboot("boot-3", [0x78; 32]);
    assert_eq!(door(&ready).await, Ok(Door::Protected));
    serving.abort();
}

#[tokio::test]
async fn a_v2_proof_that_fails_under_the_key_held_gets_nothing() {
    for spoil in [Spoil::WrongKey, Spoil::Mode, Spoil::Version, Spoil::Replay] {
        let (ready, fake, serving) = computer(true, Is::SplitV2).await;
        take_attest_key(&ready).await;
        *fake.spoil.lock().unwrap() = spoil;
        let handed = deliver(&ready, &the_v2_set()).await.unwrap();
        assert_eq!(handed.door, Door::Unproven);
        assert_eq!(
            handed.waiting,
            ["GITHUB", "GITHUB_TOKEN", "LOCAL", "LOCAL_KEY", "NPM_TOKEN"]
        );
        assert!(fake.taken.lock().unwrap().is_empty(), "not even a variable");
        serving.abort();
    }
}

#[tokio::test]
async fn the_attest_key_is_never_said_and_goes_with_the_computer() {
    let (ready, _fake, serving) = computer(true, Is::SplitV2).await;
    take_attest_key(&ready).await;
    let handed = deliver(&ready, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Protected);
    let wrong = Ready {
        token: "not-the-token".to_owned(),
        ..ready.clone()
    };
    let refused = deliver(&wrong, &the_v2_set()).await.unwrap_err();
    let encoded = base64::engine::general_purpose::STANDARD.encode(BOOT_KEY);
    for said in [
        format!("{ready:?}"),
        format!("{:?}", ready.attest),
        format!("{handed:?}"),
        format!("{refused:?}"),
    ] {
        assert!(!said.contains(&encoded), "{said}");
        assert!(!said.contains("90, 90"), "{said}");
        assert!(!said.to_ascii_lowercase().contains("5a5a"), "{said}");
    }
    // Forgotten with the computer: its boot will not hand it again, so
    // nothing this desk holds proves it protected.
    ready.attest.forget().await;
    assert_eq!(door(&ready).await, Ok(Door::Unprotected));
    serving.abort();

    // Kept in a store, it is said no more than in memory: not by the key,
    // its computer, or a line the desk prints about keeping it.
    let store = Arc::new(Refusing::default());
    let (ready, _fake, serving) = computer(true, Is::SplitV2).await;
    let ready = Ready {
        attest: AttestKey::kept(store, "said-ada", &ready.token),
        ..ready
    };
    take_attest_key(&ready).await;
    let handed = deliver(&ready, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Protected);
    let lines = SAID.lock().unwrap().clone();
    assert!(lines.iter().any(|line| line.contains("said-ada")));
    for said in [format!("{ready:?}"), format!("{:?}", ready.attest)]
        .into_iter()
        .chain(lines)
    {
        assert!(!said.contains(&encoded), "{said}");
        assert!(!said.contains("90, 90"), "{said}");
        assert!(!said.to_ascii_lowercase().contains("5a5a"), "{said}");
    }
    serving.abort();
}

/// The entry a kept key is under, as the store holds it.
fn kept(store: &crate::credentials::MemoryStore, persona_id: &str) -> Option<Value> {
    use crate::credentials::SecretStore;
    store
        .get(&entry_name(persona_id))
        .unwrap()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap())
}

/// `ready` again, as a desk started since sees it: the same computer and
/// bearer, and a key read from `store`, nothing in memory.
fn after_restart(
    ready: &Ready,
    store: &Arc<crate::credentials::MemoryStore>,
    persona_id: &str,
) -> Ready {
    Ready {
        attest: AttestKey::kept(store.clone(), persona_id, &ready.token),
        ..ready.clone()
    }
}

/// A v2 stand-in whose `Ready` keeps its key in `store` for `persona_id`.
async fn kept_computer(
    store: &Arc<crate::credentials::MemoryStore>,
    persona_id: &str,
) -> (Ready, Fake, tokio::task::JoinHandle<()>) {
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    let ready = Ready {
        attest: AttestKey::kept(store.clone(), persona_id, &ready.token),
        ..ready
    };
    (ready, fake, serving)
}

#[tokio::test]
async fn the_attest_key_is_kept_in_the_os_store_and_a_new_desk_reads_it_back() {
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let (ready, fake, serving) = kept_computer(&store, "kept-ada").await;
    take_attest_key(&ready).await;
    let entry = kept(&store, "kept-ada").expect("written as it was taken");
    assert_eq!(entry["v"], 1);
    assert_eq!(entry["boot"], "boot-1");
    assert_eq!(entry["bearer"], bearer_hash("the-token"));
    assert_ne!(
        entry["bearer"], "the-token",
        "the bearer's hash, not the bearer"
    );
    assert_eq!(
        entry["key"],
        base64::engine::general_purpose::STANDARD.encode(BOOT_KEY)
    );

    // A desk started since holds nothing in memory: it reads the entry,
    // and the computer proves itself without its door being asked again.
    let restarted = after_restart(&ready, &store, "kept-ada");
    let handed = deliver(&restarted, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Protected);
    assert!(handed.waiting.is_empty(), "{handed:?}");
    assert_eq!(fake.taken.lock().unwrap()[0].0.as_deref(), Some(V2));
    let restarted = after_restart(&ready, &store, "kept-ada");
    assert!(restarted.attest.holds().await, "read from the store");
    assert_eq!(door(&restarted).await, Ok(Door::Protected));
    assert_eq!(fake.asked.load(Ordering::SeqCst), 1, "asked once in all");
    serving.abort();
}

#[tokio::test]
async fn a_kept_key_is_used_only_under_the_bearer_it_was_taken_with() {
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let (ready, fake, serving) = kept_computer(&store, "bearer-ada").await;
    take_attest_key(&ready).await;
    assert!(kept(&store, "bearer-ada").is_some());

    // The container was made again: another bearer, and the entry of the
    // one before. It is not this computer's key.
    let another = Ready {
        attest: AttestKey::kept(store.clone(), "bearer-ada", "another-bearer"),
        ..ready.clone()
    };
    assert!(!another.attest.holds().await, "not used");
    assert_eq!(kept(&store, "bearer-ada"), None, "and deleted when met");
    assert_eq!(
        door(&another).await,
        Ok(Door::Unprotected),
        "no key for this boot"
    );
    assert_eq!(fake.asked.load(Ordering::SeqCst), 2);

    // Nor is a record this desk did not write, or one with a key of
    // another length: each is deleted.
    use crate::credentials::SecretStore;
    for record in [
        json!({"v": 2, "bearer": bearer_hash("the-token"), "boot": "boot-1",
               "key": base64::engine::general_purpose::STANDARD.encode(BOOT_KEY)}),
        json!({"v": 1, "bearer": bearer_hash("the-token"), "boot": "boot-1",
               "key": base64::engine::general_purpose::STANDARD.encode([0x5a; 16])}),
        json!("not a record"),
    ] {
        store
            .set(&entry_name("bearer-ada"), record.to_string().as_bytes())
            .unwrap();
        let reread = after_restart(&ready, &store, "bearer-ada");
        assert!(!reread.attest.holds().await, "{record}");
        assert_eq!(kept(&store, "bearer-ada"), None, "{record}");
    }
    serving.abort();
}

#[tokio::test]
async fn a_new_boots_key_replaces_the_kept_one_and_a_boot_whose_key_is_gone_leaves_it() {
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let (ready, fake, serving) = kept_computer(&store, "reboot-ada").await;
    take_attest_key(&ready).await;
    assert_eq!(kept(&store, "reboot-ada").unwrap()["boot"], "boot-1");

    // It booted again: the desk asks once for the new boot, and keeps it.
    fake.reboot("boot-2", [0x77; 32]);
    assert_eq!(door(&ready).await, Ok(Door::Protected));
    let entry = kept(&store, "reboot-ada").unwrap();
    assert_eq!(entry["boot"], "boot-2");
    assert_eq!(
        entry["key"],
        base64::engine::general_purpose::STANDARD.encode([0x77; 32])
    );

    // It booted again and something else took that boot's key: this
    // asking is unprotected, and the entry stays as it was, so nothing that
    // merely answers the port erases it.
    fake.reboot("boot-3", [0x78; 32]);
    fake.taken_by_another();
    assert_eq!(door(&ready).await, Ok(Door::Unprotected));
    assert_eq!(kept(&store, "reboot-ada"), Some(entry.clone()));
    let restarted = after_restart(&ready, &store, "reboot-ada");
    assert_eq!(door(&restarted).await, Ok(Door::Unprotected));
    assert_eq!(kept(&store, "reboot-ada"), Some(entry));
    serving.abort();
}

#[tokio::test]
async fn a_proof_that_fails_under_a_kept_key_gets_nothing_and_keeps_the_key() {
    for spoil in [Spoil::WrongKey, Spoil::Mode, Spoil::Version, Spoil::Replay] {
        let store = Arc::new(crate::credentials::MemoryStore::default());
        let (ready, fake, serving) = kept_computer(&store, "spoil-ada").await;
        take_attest_key(&ready).await;
        let entry = kept(&store, "spoil-ada").unwrap();

        let restarted = after_restart(&ready, &store, "spoil-ada");
        *fake.spoil.lock().unwrap() = spoil;
        let handed = deliver(&restarted, &the_v2_set()).await.unwrap();
        assert_eq!(handed.door, Door::Unproven);
        assert!(fake.taken.lock().unwrap().is_empty(), "not even a variable");
        assert_eq!(kept(&store, "spoil-ada"), Some(entry), "the key is kept");

        // The real computer proves itself at the next asking.
        *fake.spoil.lock().unwrap() = Spoil::None;
        let handed = deliver(&restarted, &the_v2_set()).await.unwrap();
        assert_eq!(handed.door, Door::Protected);
        assert_eq!(fake.asked.load(Ordering::SeqCst), 1, "never asked again");
        serving.abort();
    }
}

#[tokio::test]
async fn forgetting_the_key_removes_it_from_the_store() {
    let store = Arc::new(crate::credentials::MemoryStore::default());
    let (ready, fake, serving) = kept_computer(&store, "forget-ada").await;
    take_attest_key(&ready).await;
    assert!(kept(&store, "forget-ada").is_some());
    ready.attest.forget().await;
    assert_eq!(kept(&store, "forget-ada"), None);
    let restarted = after_restart(&ready, &store, "forget-ada");
    assert!(!restarted.attest.holds().await);

    // A forgotten key never writes its teammate's entry again, which may
    // be the next computer's by then.
    fake.reboot("boot-2", [0x77; 32]);
    assert_eq!(door(&ready).await, Ok(Door::Protected), "held in memory");
    assert_eq!(kept(&store, "forget-ada"), None);
    serving.abort();
}

/// A store whose writes fail, as a locked keychain's do.
#[derive(Default)]
struct Refusing(crate::credentials::MemoryStore);

impl crate::credentials::SecretStore for Refusing {
    fn get(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }
    fn set(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("the keychain is locked"))
    }
    fn delete(&self, key: &str) -> std::io::Result<()> {
        self.0.delete(key)
    }
}

#[tokio::test]
async fn a_store_that_fails_costs_only_the_next_restart() {
    use crate::credentials::SecretStore;
    let store = Arc::new(Refusing::default());
    let (ready, fake, serving) = computer(true, Is::SplitV2).await;
    let ready = Ready {
        attest: AttestKey::kept(store.clone(), "locked-ada", &ready.token),
        ..ready
    };
    take_attest_key(&ready).await;
    let handed = deliver(&ready, &the_v2_set()).await.unwrap();
    assert_eq!(handed.door, Door::Protected, "protected now");
    assert_eq!(fake.asked.load(Ordering::SeqCst), 1);
    assert_eq!(store.get(&entry_name("locked-ada")).unwrap(), None);
    let said: Vec<String> = SAID
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("locked-ada"))
        .cloned()
        .collect();
    assert_eq!(said.len(), 1, "one line, naming the teammate: {said:?}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(BOOT_KEY);
    assert!(!said[0].contains(&encoded), "{}", said[0]);
    assert!(
        !said[0].to_ascii_lowercase().contains("5a5a"),
        "{}",
        said[0]
    );
    assert!(said[0].contains("the keychain is locked"), "{}", said[0]);

    // After a restart there is nothing to read back: today's behaviour.
    let restarted = Ready {
        attest: AttestKey::kept(store.clone(), "locked-ada", &ready.token),
        ..ready.clone()
    };
    assert_eq!(door(&restarted).await, Ok(Door::Unprotected));
    serving.abort();
}

#[tokio::test]
async fn a_split_claim_from_a_computer_not_made_to_split_gets_nothing() {
    for is in [Is::Split, Is::SplitV2] {
        let (ready, fake, serving) = computer(true, is).await;
        let ready = Ready {
            made_to_split: false,
            ..ready
        };
        assert_eq!(door(&ready).await, Ok(Door::Unproven));
        let handed = deliver(&ready, &the_v2_set()).await.unwrap();
        assert_eq!(handed.door, Door::Unproven);
        assert_eq!(
            handed.waiting,
            ["GITHUB", "GITHUB_TOKEN", "LOCAL", "LOCAL_KEY", "NPM_TOKEN"]
        );
        assert!(fake.taken.lock().unwrap().is_empty(), "not even a variable");
        assert_eq!(
            fake.asked.load(Ordering::SeqCst),
            0,
            "the key door never asked"
        );
        assert!(
            !fake.claimed.load(Ordering::SeqCst),
            "and its key not taken"
        );
        serving.abort();
    }

    // An honest one says legacy, or has no such door: unaffected.
    for is in [Is::Old, Is::Legacy] {
        let (ready, _fake, serving) = computer(true, is).await;
        let ready = Ready {
            made_to_split: false,
            ..ready
        };
        assert_eq!(door(&ready).await, Ok(Door::Unprotected));
        serving.abort();
    }
}
