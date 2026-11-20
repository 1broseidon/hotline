//! Handing a running computer the secrets its teammate is granted, out of
//! band.
//!
//! The computer keeps the set in memory and puts each one where its kind
//! says — a variable in the environment of every job the agent starts, a
//! login behind `browser fill` on its own sites, a passkey in the browser's
//! authenticator; it never answers a value back, over any route, and
//! redacts them from what its tools return. The desk replaces the whole set
//! at every grant — session start, reattach, a stored value changing — so a
//! rotation or a revocation reaches a running machine without a restart.
//! The values ride desk → container over the container's authenticated
//! loopback port, bearer in a header, and touch no tape, model or log on
//! the way.
//!
//! What a computer is handed depends on what it proves it is. Each
//! handing over first asks `GET /capabilities` with a fresh nonce. A
//! computer that runs split — its jobs a user apart from the broker that
//! holds values — answers an `attest` only this desk can check, and takes
//! every grant through the v2 form: readable variables, keys with their
//! "Used with", logins and passkeys. One that has no such door, or runs as
//! one user (legacy), is handed what 0.49 handed it — readable variables,
//! logins and passkeys, in the v1 form every release takes — and a key
//! granted protected waits on the desk until it is updated, since its
//! programs could read it there. One whose `attest` is wrong is handed
//! nothing at all: what answers the port is not this computer.
//!
//! The proof comes in two versions, told apart by `protected.v`. A v1
//! computer (0.13.0-rc.2 and before) attests with the bearer. A v2 one
//! attests with a key its door makes at each boot and hands once, to the
//! first bearer holder who asks at `POST /capabilities/key`: the desk asks
//! right after it starts the computer and keeps the key beside the bearer
//! ([`AttestKey`]). A split computer whose boot this desk holds no key for
//! (the key was taken before the desk asked, or by an earlier run of the
//! desk) cannot be checked, and is handed what a desk that knew only v1
//! would hand it: what a legacy computer gets.

use super::Ready;
use crate::vault::StoredSecret;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// The v2 form's media type; a body without it is read as v1.
const V2: &str = "application/vnd.hotline.secrets.v2+json";

/// What a v1 `attest` is the HMAC of, under the bearer, after the boot and
/// the nonce are joined on: the broker's own words, so it is never another
/// door's MAC.
fn attest_input_v1(boot: &str, nonce: &str) -> String {
    format!("hotline-broker-v1|{boot}|{nonce}")
}

/// What a v2 `attest` is the HMAC of, under the boot's key: every field
/// the desk relies on, so a proof of one mode, boot, release or nonce is
/// none of another.
fn attest_input_v2(boot: &str, mode: &str, version: &str, nonce: &str) -> String {
    format!("hotline-broker-v2|{boot}|{mode}|{version}|{nonce}")
}

/// The key a v2 computer attests with, for the boot it was handed on. It
/// lives beside the bearer, in this process alone: never in the
/// container's settings, an argument, a file or a log, and sent nowhere —
/// the desk only computes with it. Every [`Ready`] of one running computer
/// shares one, and it is forgotten when the computer is removed or made
/// again; a key for another boot is never used. The lock is held across
/// taking it, so two askers at once do not both ask and one get the 409.
#[derive(Clone, Default)]
pub struct AttestKey(Arc<tokio::sync::Mutex<Option<BootKey>>>);

struct BootKey {
    boot: String,
    key: Vec<u8>,
}

impl std::fmt::Debug for AttestKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AttestKey(..)")
    }
}

impl AttestKey {
    /// Drops the key, so nothing holding this computer's [`Ready`] keeps it.
    pub async fn forget(&self) {
        *self.0.lock().await = None;
    }

    /// The key for `boot`: the one held, or this boot's, taken now.
    async fn for_boot(
        &self,
        client: &reqwest::Client,
        ready: &Ready,
        boot: &str,
    ) -> Option<Vec<u8>> {
        let mut held = self.0.lock().await;
        if held.as_ref().is_none_or(|held| held.boot != boot)
            && let Some(taken) = take(client, ready).await
        {
            *held = Some(taken);
        }
        held.as_ref()
            .filter(|held| held.boot == boot)
            .map(|held| held.key.clone())
    }
}

/// Takes this boot's attest key, once, as soon as the computer answers:
/// the desk starting it is the first to ask. A release without the door
/// (before 0.13.0-rc.3), or a boot whose key was taken already, leaves
/// what is held as it was.
pub async fn take_attest_key(ready: &Ready) {
    let Ok(client) = client() else {
        return;
    };
    let mut held = ready.attest.0.lock().await;
    if let Some(taken) = take(&client, ready).await {
        *held = Some(taken);
    }
}

/// `POST /capabilities/key`, bearer in the header: `{v: 2, boot, key}` the
/// first time a boot is asked, 409 after. Nothing about the answer is kept
/// or said but the key and its boot.
async fn take(client: &reqwest::Client, ready: &Ready) -> Option<BootKey> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let response = client
        .post(format!("{base}/capabilities/key"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let answer: Value = response.json().await.ok()?;
    let boot = answer["boot"].as_str().filter(|boot| !boot.is_empty())?;
    let key = answer["key"]
        .as_str()
        .and_then(|key| base64::engine::general_purpose::STANDARD.decode(key).ok())
        .filter(|key| key.len() == 32)?;
    (answer["v"] == 2).then(|| BootKey {
        boot: boot.to_owned(),
        key,
    })
}

/// What is said of a computer that claimed to run split and did not prove
/// it, after its name.
pub const UNPROVEN: &str =
    "did not prove it is this teammate's, so it was handed nothing. Update or restart it.";

/// Why a computer did not take the set.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The release has no `/secrets` route: it predates them. The pane's
    /// Update is the fix.
    TooOld,
    /// It could not be reached, or refused for a reason of its own.
    Failed(String),
}

/// What a computer proved about keeping secrets from its jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    /// Split, and it answered for the boot it is on: under this computer's
    /// bearer (v1), or under the key that boot handed this desk (v2).
    Protected,
    /// No `/capabilities`, a release before 0.13, or one that says it runs
    /// legacy: today's one user, which a job shares with every value. Also a
    /// v2 computer whose boot's key this desk does not hold: it cannot be
    /// checked, so it is handed what a legacy one is.
    Unprotected,
    /// It said it runs split but did not prove it: no attest, one for
    /// another nonce or boot, or one another key made. What answered may
    /// not be this computer, so it is handed nothing, and it is a computer
    /// that needs an update until it proves itself.
    Unproven,
}

/// Asks the computer what it is, with a nonce of this asking's own. An
/// answer that says protected but whose `attest` fails is refused, so
/// nothing is sent to whatever made it.
pub async fn door(ready: &Ready) -> Result<Door, Refusal> {
    Ok(door_on(&client()?, ready).await?.0)
}

fn client() -> Result<reqwest::Client, Refusal> {
    super::door_client()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Refusal::Failed(error.to_string()))
}

/// [`door`] on `client`, with the answer it was told: the request carries
/// the bearer and a nonce, and nothing else, so no value leaves before the
/// answer is verified. The version of the proof is the computer's
/// `protected.v`: 1 is checked under the bearer as it always was, 2 under
/// the boot's key ([`AttestKey`]), taken now if this desk has none for that
/// boot and the computer still hands it; any other is a computer this desk
/// cannot read, and so unprotected.
async fn door_on(client: &reqwest::Client, ready: &Ready) -> Result<(Door, Value), Refusal> {
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let response = client
        .get(format!("{base}/capabilities?nonce={nonce}"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    match response.status() {
        reqwest::StatusCode::NOT_FOUND => return Ok((Door::Unprotected, Value::Null)),
        status if !status.is_success() => {
            return Err(Refusal::Failed(format!(
                "The computer would not say what it is ({status})."
            )));
        }
        _ => {}
    }
    let answer: Value = response
        .json()
        .await
        .map_err(|_| Refusal::Failed("The computer's capabilities could not be read.".into()))?;
    let door = if answer["protected"]["v"] == 2 && answer["protected"]["mode"] == "split" {
        let boot = answer["boot"].as_str().unwrap_or_default();
        match ready.attest.for_boot(client, ready, boot).await {
            Some(key) => verify_attest_v2(&key, &nonce, &answer),
            None => Door::Unprotected,
        }
    } else {
        verify_attest_v1(ready.token.as_bytes(), &nonce, &answer)
    };
    Ok((door, answer))
}

/// Whether `attest` is base64 of HMAC-SHA256 under `key` of `input`,
/// compared in constant time (`verify_slice`).
fn attests(key: &[u8], input: &str, attest: &Value) -> bool {
    attest
        .as_str()
        .and_then(|attest| {
            base64::engine::general_purpose::STANDARD
                .decode(attest)
                .ok()
        })
        .is_some_and(|attest: Vec<u8>| {
            let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(key) else {
                return false;
            };
            mac.update(input.as_bytes());
            mac.verify_slice(&attest).is_ok()
        })
}

/// What a v1 `/capabilities` answer to `nonce` proves, checked with `key`
/// (the bearer), as every desk before v2 checked it:
///
/// 1. the claim: `protected.v` is the number 1 and `protected.mode` is the
///    string `"split"`; anything else (absent, `legacy`, a version this
///    desk does not know) is an unprotected computer, handed what 0.49
///    handed it;
/// 2. `boot` is a non-empty string;
/// 3. `attest` is the MAC of [`attest_input_v1`] for that boot and this
///    asking's own nonce.
///
/// A claim of split that fails 2 or 3 is [`Door::Unproven`]: a replayed
/// attest (another nonce), an altered boot, or one another key made. v1
/// does not cover the mode or the release; v2 does.
fn verify_attest_v1(key: &[u8], nonce: &str, answer: &Value) -> Door {
    let split = answer["protected"]["v"] == 1 && answer["protected"]["mode"] == "split";
    if !split {
        return Door::Unprotected;
    }
    let boot = answer["boot"].as_str().unwrap_or_default();
    if boot.is_empty() || !attests(key, &attest_input_v1(boot, nonce), &answer["attest"]) {
        return Door::Unproven;
    }
    Door::Protected
}

/// What a v2 claim of split proves, checked with the key this desk took
/// for the answer's boot: `attest` must be the MAC of [`attest_input_v2`]
/// over the answer's own boot, mode and version and this asking's nonce.
/// Anything else — another key, a replayed nonce, an altered mode, boot or
/// release, or no attest — is [`Door::Unproven`].
fn verify_attest_v2(key: &[u8], nonce: &str, answer: &Value) -> Door {
    let boot = answer["boot"].as_str().unwrap_or_default();
    let mode = answer["protected"]["mode"].as_str().unwrap_or_default();
    let version = answer["version"].as_str().unwrap_or_default();
    let input = attest_input_v2(boot, mode, version, nonce);
    if boot.is_empty() || mode != "split" || !attests(key, &input, &answer["attest"]) {
        return Door::Unproven;
    }
    Door::Protected
}

/// The names a protected computer says it holds but cannot keep from its
/// programs (`protected.readable`: logins and passkeys for a site on the
/// computer itself, keys used with an "Other" service), for the pane to
/// label as readable beside what the desk knows to be.
fn readable_names(answer: &Value) -> Vec<String> {
    answer["protected"]["readable"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Whether a site is the computer itself (`localhost`, `*.localhost`,
/// 127.0.0.0/8, `::1`, or IPv4 loopback mapped into IPv6), in any spelling
/// a URL parser reads as one, as the window's `loopbackSite` reads it. A
/// program there may serve the page, so a login or passkey for it is
/// granted only after the person was told it is readable.
fn loopback_site(site: &str) -> bool {
    let address = if site.contains("://") {
        site.to_owned()
    } else {
        format!("https://{site}")
    };
    match url::Url::parse(&address)
        .ok()
        .as_ref()
        .and_then(url::Url::host)
    {
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => ip.octets()[0] == 127,
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|ip| ip.octets()[0] == 127)
        }
        None => false,
    }
}

/// One granted secret as the desk hands it: the record, and whether the
/// teammate's programs may read it.
#[derive(Clone, Debug)]
pub struct Granted {
    pub secret: StoredSecret,
    pub readable: bool,
}

/// What a handing over came to: what the computer proved, and the names it
/// was not handed: keys granted protected, which wait on an unprotected
/// computer for its update, and on a protected one for a "Used with" to
/// protect them by; every name, on one that did not prove what it said.
/// `readable` is what a protected computer said, once it took the set, it
/// holds but cannot keep from its programs (empty on any other).
#[derive(Debug, PartialEq, Eq)]
pub struct Handed {
    pub door: Door,
    pub waiting: Vec<String>,
    pub readable: Vec<String>,
}

/// The v1 set: a variable as its bare value, which every computer release
/// with a `/secrets` route takes; a login or a passkey as the record, kind
/// and all, which a release from 0.8 on takes.
pub fn wire_form(secrets: &BTreeMap<String, StoredSecret>) -> BTreeMap<String, Value> {
    secrets
        .iter()
        .map(|(name, secret)| {
            let value = match secret {
                StoredSecret::Variable { value, .. } => Value::String(value.clone()),
                typed => serde_json::to_value(typed).unwrap_or(Value::Null),
            };
            (name.clone(), value)
        })
        .collect()
}

/// One entry of the v2 set, or none for a key granted protected that has
/// no "Used with". A key's stand-in keeps its known prefix and its length,
/// so a program that checks the token's shape still runs. A login or
/// passkey for a site on the computer itself says `readable`: the person
/// was told it is readable when they granted it, and a v2 computer types
/// or signs with one there only if it says so.
fn entry_v2(granted: &Granted) -> Option<Value> {
    match &granted.secret {
        StoredSecret::Variable { value, .. } if granted.readable => {
            Some(json!({"kind": "variable", "value": value, "readable": true}))
        }
        StoredSecret::Variable {
            value,
            r#use: Some(usage),
        } => {
            // The refuse list is always sent, empty too: a computer applies
            // its own shorter built-in list only where it is absent.
            let usage = json!({
                "origins": usage.origins,
                "header": usage.header,
                "schemes": usage.schemes,
                "refuse": usage.refuse,
                "standIn": {
                    "prefix": crate::vault::stand_in_prefix(value),
                    "length": value.chars().count(),
                },
            });
            Some(json!({"kind": "variable", "value": value, "use": usage}))
        }
        StoredSecret::Variable { r#use: None, .. } => None,
        typed => {
            let on_loopback = match typed {
                StoredSecret::Login { sites, .. } => sites.iter().any(|site| loopback_site(site)),
                StoredSecret::Passkey { rp_id, .. } => loopback_site(rp_id),
                StoredSecret::Variable { .. } => false,
            };
            let mut entry = serde_json::to_value(typed).ok()?;
            if granted.readable || on_loopback {
                entry["readable"] = Value::Bool(true);
            }
            Some(entry)
        }
    }
}

/// Each v2 set's generation is past every one this desk sent before, across
/// a restart too, so a computer can tell the newest set and drop what an
/// older one gave its jobs.
fn next_generation() -> i64 {
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = chrono::Utc::now().timestamp_millis();
    let previous = LAST.fetch_max(now, Ordering::SeqCst);
    if previous < now {
        return now;
    }
    LAST.fetch_add(1, Ordering::SeqCst) + 1
}

/// Replaces the computer's whole set with what it can be handed of `set`,
/// through `PUT /secrets`. An empty set clears it, which is how a
/// revocation lands.
pub async fn deliver(ready: &Ready, set: &BTreeMap<String, Granted>) -> Result<Handed, Refusal> {
    // The set goes only after the proof verified, on the client (and so the
    // kept-alive connection) that asked for it. SEAM: the request does not
    // yet name the boot or instance it was proved for, since today's
    // computer takes no such field; when it does, it is added here and the
    // computer refuses a set for another.
    let client = client()?;
    let (door, _) = door_on(&client, ready).await?;
    if door == Door::Unproven {
        return Ok(Handed {
            door,
            waiting: set.keys().cloned().collect(),
            readable: Vec::new(),
        });
    }
    let protected = door == Door::Protected;
    let mut waiting = Vec::new();
    let base = ready.url.strip_suffix("/mcp").unwrap_or(ready.url.as_str());
    let request = if protected {
        let mut secrets = serde_json::Map::new();
        for (name, granted) in set {
            match entry_v2(granted) {
                Some(entry) => {
                    secrets.insert(name.clone(), entry);
                }
                None => waiting.push(name.clone()),
            }
        }
        let body = json!({"v": 2, "generation": next_generation(), "secrets": secrets});
        client
            .put(format!("{base}/secrets"))
            .header(reqwest::header::CONTENT_TYPE, V2)
            .body(body.to_string())
    } else {
        let mut today = BTreeMap::new();
        for (name, granted) in set {
            match &granted.secret {
                StoredSecret::Variable { .. } if !granted.readable => waiting.push(name.clone()),
                secret => {
                    today.insert(name.clone(), secret.clone());
                }
            }
        }
        client
            .put(format!("{base}/secrets"))
            .json(&wire_form(&today))
    };
    let response = request
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|error| Refusal::Failed(format!("Could not reach the computer: {error}")))?;
    let status = response.status();
    if status.is_success() {
        // What it cannot keep from its programs is of the set it holds now,
        // so it is asked again, and believed only from a proven answer.
        let readable = if protected {
            match door_on(&client, ready).await {
                Ok((Door::Protected, answer)) => readable_names(&answer),
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        return Ok(Handed {
            door,
            waiting,
            readable,
        });
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Refusal::TooOld);
    }
    // An upstream error may echo the submitted body. Its status is enough
    // to report failure without copying a password into the desk's logs.
    Err(Refusal::Failed(format!(
        "The computer refused them ({status})."
    )))
}

#[cfg(test)]
mod tests;
