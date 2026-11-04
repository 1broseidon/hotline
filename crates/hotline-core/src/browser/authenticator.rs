//! The desk browser's WebAuthn authenticator, ported from the computer's
//! (hotline-computer `src/authenticator.rs`). The app hands each request a
//! page makes through `navigator.credentials` to the core with the origin
//! WebKit reports for the frame, never one the page wrote, and the answer
//! is signed here: a passkey's private key stays in the vault and this
//! process, so no page and no agent can read one back.
//!
//! It is a platform authenticator that always verifies its user: the
//! person's consent is the desk's arming and approval for a new passkey,
//! the grant for an existing one, and the card for an ask-me one.

use crate::contract::SecretUse;
use crate::vault::StoredSecret;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// What a page hears when a request is refused: a `DOMException`'s name and
/// a sentence.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub name: &'static str,
    pub message: String,
}

pub fn refuse(name: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        name,
        message: message.into(),
    }
}

/// A page's `navigator.credentials` call, as the app hands it over.
#[derive(Clone, Debug)]
pub struct Request {
    /// `create()` rather than `get()`.
    pub create: bool,
    /// The origin WebKit reports for the frame that called: never one the
    /// page wrote.
    pub origin: String,
    /// The origin of the page the frame is in, which WebKit reports too.
    pub top_origin: String,
    /// The call's `publicKey` options, as their JSON form: buffers in
    /// base64url.
    pub options: Value,
}

/// A passkey the teammate is granted, as the vault keeps it, by its name.
/// Its debug form never shows the private key.
pub struct Key {
    pub name: String,
    pub r#use: SecretUse,
    pub rp_id: String,
    /// Base64, as the vault keeps it.
    pub credential_id: String,
    /// PKCS#8, base64.
    pub private_key: String,
    pub user_handle: Option<String>,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("name", &self.name)
            .field("rp_id", &self.rp_id)
            .finish_non_exhaustive()
    }
}

impl Key {
    /// The passkey a stored record is, or `None` for a login or variable.
    pub fn of(name: String, stored: StoredSecret) -> Option<Self> {
        match stored {
            StoredSecret::Passkey {
                r#use,
                rp_id,
                credential_id,
                private_key,
                user_handle,
                ..
            } => Some(Self {
                name,
                r#use,
                rp_id,
                credential_id,
                private_key,
                user_handle,
            }),
            _ => None,
        }
    }
}

/// User present, user verified, backup eligible, backed up: a synced
/// platform passkey, which is what a teammate's vault holds.
const FLAGS: u8 = 0x01 | 0x04 | 0x08 | 0x10;
const ATTESTED: u8 = 0x40;
/// ES256, the one algorithm every relying party accepts.
const ES256: i64 = -7;
/// The DER prefix of a P-256 public key's `SubjectPublicKeyInfo`.
const SPKI_P256: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The host of an origin, from a secure context only: HTTPS, or this
/// machine itself.
fn host_of(origin: &str) -> Result<String, Refusal> {
    let url = url::Url::parse(origin)
        .map_err(|_| refuse("SecurityError", format!("{origin} is not an origin")))?;
    let host = url
        .host_str()
        .ok_or_else(|| refuse("SecurityError", format!("{origin} has no host")))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let local = host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1";
    if url.scheme() == "https" || (url.scheme() == "http" && local) {
        Ok(host)
    } else {
        Err(refuse(
            "SecurityError",
            format!("{origin} is not a secure context"),
        ))
    }
}

/// The relying party a request names, or the page's own host; it must be
/// that host or a domain the host is within.
pub fn relying_party(origin: &str, asked: Option<&str>) -> Result<String, Refusal> {
    let host = host_of(origin)?;
    let rp = asked.map_or_else(|| host.clone(), str::to_ascii_lowercase);
    if host == rp || host.ends_with(&format!(".{rp}")) {
        Ok(rp)
    } else {
        Err(refuse(
            "SecurityError",
            format!("{origin} cannot use passkeys for {rp}"),
        ))
    }
}

fn client_data(kind: &str, challenge: &str, origin: &str) -> String {
    // Chromium's own key order, which some relying parties parse by hand.
    // A request from a frame of another site is refused before this, so
    // `crossOrigin` is always false.
    format!(
        r#"{{"type":{},"challenge":{},"origin":{},"crossOrigin":false}}"#,
        json!(kind),
        json!(challenge),
        json!(origin)
    )
}

/// The signature counter: seconds since the epoch, and always past the last
/// one this desk gave. A site keeps the highest count it has seen and
/// refuses one that does not climb, as a cloned key; the same passkey may
/// also sign on a computer, whose counter is the same clock.
fn counter() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static LAST: AtomicU32 = AtomicU32::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u32::try_from(since.as_secs()).unwrap_or(u32::MAX)
        });
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last.saturating_add(1));
        match LAST.compare_exchange(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

fn authenticator_data(rp_id: &str, flags: u8, attested: &[u8]) -> Vec<u8> {
    let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
    data.push(flags);
    data.extend_from_slice(&counter().to_be_bytes());
    data.extend_from_slice(attested);
    data
}

fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn from_b64url(text: &str) -> Result<Vec<u8>, Refusal> {
    URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| refuse("DataError", "a field of the request is not base64url"))
}

fn from_std(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text).ok()
}

fn listed(options: &Value, field: &str) -> Vec<Vec<u8>> {
    options[field]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|c| c["id"].as_str())
                .filter_map(|id| from_b64url(id).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The passkey a sign-in request for `rp_id` is answered with: the first of
/// the teammate's for that relying party that the site allows.
pub fn chosen<'a>(rp_id: &str, options: &Value, keys: &'a [Key]) -> Result<&'a Key, Refusal> {
    let allowed = listed(options, "allowCredentials");
    keys.iter()
        .filter(|key| key.rp_id == rp_id)
        .find(|key| {
            allowed.is_empty()
                || from_std(&key.credential_id).is_some_and(|id| allowed.contains(&id))
        })
        .ok_or_else(|| {
            refuse(
                "NotAllowedError",
                format!("no passkey for {rp_id} is granted to this teammate"),
            )
        })
}

/// Signs a `navigator.credentials.get()` from `origin` with `key`.
pub fn assert(origin: &str, options: &Value, key: &Key) -> Result<Value, Refusal> {
    let rp_id = relying_party(origin, options["rpId"].as_str())?;
    if rp_id != key.rp_id {
        return Err(refuse(
            "SecurityError",
            format!("{origin} cannot use a passkey for {}", key.rp_id),
        ));
    }
    let challenge = options["challenge"]
        .as_str()
        .ok_or_else(|| refuse("DataError", "the request has no challenge"))?;
    let credential = from_std(&key.credential_id)
        .ok_or_else(|| refuse("UnknownError", "a granted passkey's id is not base64"))?;
    let pkcs8 = from_std(&key.private_key)
        .ok_or_else(|| refuse("UnknownError", "a granted passkey's key is not base64"))?;
    let client = client_data("webauthn.get", challenge, origin);
    let data = authenticator_data(&rp_id, FLAGS, &[]);
    let random = SystemRandom::new();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &pkcs8, &random)
        .map_err(|_| refuse("UnknownError", "a granted passkey's key is not a P-256 key"))?;
    let mut signed = data.clone();
    signed.extend_from_slice(&Sha256::digest(client.as_bytes()));
    let signature = pair
        .sign(&random, &signed)
        .map_err(|_| refuse("UnknownError", "signing failed"))?;
    let handle = key
        .user_handle
        .as_deref()
        .and_then(from_std)
        .map(|h| b64url(&h));
    Ok(json!({
        "id": b64url(&credential),
        "rawId": b64url(&credential),
        "type": "public-key",
        "authenticatorAttachment": "platform",
        "response": {
            "clientDataJSON": b64url(client.as_bytes()),
            "authenticatorData": b64url(&data),
            "signature": b64url(signature.as_ref()),
            "userHandle": handle,
        },
        "clientExtensionResults": {},
    }))
}

/// What a `navigator.credentials.create()` asks for, before anyone decides
/// whether it may.
#[derive(Debug)]
pub struct Creation {
    pub origin: String,
    pub rp_id: String,
    pub rp_name: Option<String>,
    pub user_id: Vec<u8>,
    pub user_name: Option<String>,
    pub user_display_name: Option<String>,
    pub challenge: String,
    pub excluded: Vec<Vec<u8>>,
}

pub fn creation(origin: &str, options: &Value) -> Result<Creation, Refusal> {
    let rp_id = relying_party(origin, options["rp"]["id"].as_str())?;
    let es256 = options["pubKeyCredParams"]
        .as_array()
        .is_some_and(|params| params.iter().any(|p| p["alg"].as_i64() == Some(ES256)));
    if !es256 {
        return Err(refuse(
            "NotSupportedError",
            "this browser makes ES256 passkeys only",
        ));
    }
    let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Ok(Creation {
        rp_name: text(&options["rp"]["name"]),
        user_id: from_b64url(options["user"]["id"].as_str().unwrap_or_default())?,
        user_name: text(&options["user"]["name"]),
        user_display_name: text(&options["user"]["displayName"]),
        challenge: options["challenge"]
            .as_str()
            .ok_or_else(|| refuse("DataError", "the request has no challenge"))?
            .to_owned(),
        excluded: listed(options, "excludeCredentials"),
        origin: origin.to_owned(),
        rp_id,
    })
}

/// A credential the teammate already has that the site asked not to be
/// made twice.
pub fn excluded(creation: &Creation, keys: &[Key]) -> bool {
    keys.iter().any(|key| {
        key.rp_id == creation.rp_id
            && from_std(&key.credential_id).is_some_and(|id| creation.excluded.contains(&id))
    })
}

/// Makes the passkey: a new P-256 key, its record for the vault, and the
/// response with `none` attestation, which carries the public key only.
pub fn mint(creation: &Creation) -> Result<(StoredSecret, Value), Refusal> {
    fn failed<E>(_: E) -> Refusal {
        refuse("UnknownError", "making the key failed")
    }
    let random = SystemRandom::new();
    let pkcs8 =
        EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &random).map_err(failed)?;
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &random)
        .map_err(failed)?;
    let point = pair.public_key().as_ref();
    let mut credential = [0u8; 16];
    random.fill(&mut credential).map_err(failed)?;
    let mut attested = vec![0u8; 16];
    attested.extend_from_slice(&16u16.to_be_bytes());
    attested.extend_from_slice(&credential);
    attested.extend_from_slice(&cose_key(point));
    let data = authenticator_data(&creation.rp_id, FLAGS | ATTESTED, &attested);
    let client = client_data("webauthn.create", &creation.challenge, &creation.origin);
    let mut spki = SPKI_P256.to_vec();
    spki.extend_from_slice(point);
    let record = StoredSecret::Passkey {
        r#use: SecretUse::Auto,
        rp_id: creation.rp_id.clone(),
        credential_id: STANDARD.encode(credential),
        private_key: STANDARD.encode(pkcs8.as_ref()),
        user_handle: Some(STANDARD.encode(&creation.user_id)),
        user_name: creation.user_name.clone(),
        user_display_name: creation.user_display_name.clone(),
    };
    let response = json!({
        "id": b64url(&credential),
        "rawId": b64url(&credential),
        "type": "public-key",
        "authenticatorAttachment": "platform",
        "response": {
            "clientDataJSON": b64url(client.as_bytes()),
            "attestationObject": b64url(&attestation(&data)),
            "authenticatorData": b64url(&data),
            "publicKey": b64url(&spki),
            "publicKeyAlgorithm": ES256,
            "transports": ["internal", "hybrid"],
        },
        "clientExtensionResults": {"credProps": {"rk": true}},
    });
    Ok((record, response))
}

// ---- the little CBOR WebAuthn needs ---------------------------------------

fn cbor_head(major: u8, length: usize, out: &mut Vec<u8>) {
    let major = major << 5;
    match (u8::try_from(length), u16::try_from(length)) {
        (Ok(short @ 0..=23), _) => out.push(major | short),
        (Ok(byte), _) => out.extend_from_slice(&[major | 24, byte]),
        (_, Ok(word)) => {
            out.push(major | 25);
            out.extend_from_slice(&word.to_be_bytes());
        }
        _ => {
            out.push(major | 26);
            out.extend_from_slice(&u32::try_from(length).unwrap_or(u32::MAX).to_be_bytes());
        }
    }
}

fn cbor_text(text: &str, out: &mut Vec<u8>) {
    cbor_head(3, text.len(), out);
    out.extend_from_slice(text.as_bytes());
}

fn cbor_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    cbor_head(2, bytes.len(), out);
    out.extend_from_slice(bytes);
}

/// An EC2 P-256 COSE key: kty 2, alg -7, crv 1, x, y.
fn cose_key(point: &[u8]) -> Vec<u8> {
    let mut out = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21];
    cbor_bytes(&point[1..33], &mut out);
    out.push(0x22);
    cbor_bytes(&point[33..65], &mut out);
    out
}

/// `{"fmt": "none", "attStmt": {}, "authData": ...}`.
fn attestation(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0xa3];
    cbor_text("fmt", &mut out);
    cbor_text("none", &mut out);
    cbor_text("attStmt", &mut out);
    out.push(0xa0);
    cbor_text("authData", &mut out);
    cbor_bytes(data, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

    fn made(origin: &str, rp: &str) -> (Key, Value) {
        let options = json!({
            "challenge": "Y2hhbGxlbmdl",
            "rp": {"id": rp, "name": "Example"},
            "user": {"id": "dXNlci0x", "name": "teammate", "displayName": "The teammate"},
            "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
        });
        let (record, response) = mint(&creation(origin, &options).unwrap()).unwrap();
        (Key::of("EXAMPLE".into(), record).unwrap(), response)
    }

    fn decoded(value: &Value) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(value.as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_minted_passkey_signs_for_its_site_and_the_signature_verifies() {
        let (key, created) = made("https://login.example.com", "example.com");
        assert_eq!(key.rp_id, "example.com");
        let data = decoded(&created["response"]["authenticatorData"]);
        assert_eq!(&data[..32], Sha256::digest(b"example.com").as_slice());
        assert_eq!(data[32], FLAGS | ATTESTED);
        let client: Value =
            serde_json::from_slice(&decoded(&created["response"]["clientDataJSON"])).unwrap();
        assert_eq!(client["type"], "webauthn.create");
        assert_eq!(client["origin"], "https://login.example.com");

        let options = json!({"challenge": "Y2hhbGxlbmdl", "rpId": "example.com"});
        let got = assert("https://example.com", &options, &key).unwrap();
        assert_eq!(got["id"], created["id"]);
        assert_eq!(got["response"]["userHandle"], "dXNlci0x");
        let data = decoded(&got["response"]["authenticatorData"]);
        let client = decoded(&got["response"]["clientDataJSON"]);
        let signature = decoded(&got["response"]["signature"]);
        let mut signed = data.clone();
        signed.extend_from_slice(&Sha256::digest(&client));
        let spki = decoded(&created["response"]["publicKey"]);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &spki[SPKI_P256.len()..])
            .verify(&signed, &signature)
            .expect("the site can verify what the desk signed");
        assert_eq!(data[32], FLAGS);
        let serialized = got.to_string();
        assert!(
            !serialized.contains(&key.private_key),
            "the answer carries no private key"
        );
    }

    #[test]
    fn every_signature_counts_past_what_a_site_has_seen() {
        let (key, _) = made("https://example.com", "example.com");
        let count = |got: &Value| {
            let data = decoded(&got["response"]["authenticatorData"]);
            u32::from_be_bytes(data[33..37].try_into().unwrap())
        };
        let options = json!({"challenge": "Y2hhbGxlbmdl"});
        let first = assert("https://example.com", &options, &key).unwrap();
        let second = assert("https://example.com", &options, &key).unwrap();
        assert!(count(&first) > 1_700_000_000, "{}", count(&first));
        assert!(count(&second) > count(&first));
    }

    #[test]
    fn a_passkey_is_never_offered_to_another_site_or_an_insecure_page() {
        let (key, _) = made("https://example.com", "example.com");
        let keys = [key];
        assert_eq!(
            relying_party("https://evil.test", Some("example.com"))
                .unwrap_err()
                .name,
            "SecurityError"
        );
        assert_eq!(
            relying_party("http://example.com", None).unwrap_err().name,
            "SecurityError"
        );
        assert_eq!(
            assert("https://evil.test", &json!({"challenge": "x"}), &keys[0])
                .unwrap_err()
                .name,
            "SecurityError",
            "a key signs only for its own relying party"
        );
        assert_eq!(
            chosen("other.example.org", &json!({}), &keys)
                .unwrap_err()
                .name,
            "NotAllowedError"
        );
        let listed = json!({"allowCredentials": [{"type": "public-key", "id": "AAAA"}]});
        assert_eq!(
            chosen("example.com", &listed, &keys).unwrap_err().name,
            "NotAllowedError"
        );
        assert!(relying_party("http://localhost:8123", Some("localhost")).is_ok());
    }

    #[test]
    fn a_site_that_excludes_a_known_credential_or_wants_another_algorithm_is_told() {
        let (key, created) = made("https://example.com", "example.com");
        let again = creation(
            "https://example.com",
            &json!({
                "challenge": "x", "rp": {"name": "Example"}, "user": {"id": "dXNlci0x"},
                "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
                "excludeCredentials": [{"type": "public-key", "id": created["id"]}],
            }),
        )
        .unwrap();
        assert!(excluded(&again, &[key]));
        let rsa = creation(
            "https://example.com",
            &json!({
                "challenge": "x", "rp": {}, "user": {"id": "dXNlci0x"},
                "pubKeyCredParams": [{"type": "public-key", "alg": -257}],
            }),
        );
        assert_eq!(rsa.unwrap_err().name, "NotSupportedError");
    }

    #[test]
    fn the_cose_key_and_attestation_are_the_bytes_webauthn_expects() {
        let point: Vec<u8> = std::iter::once(4).chain(1..=64).collect();
        let key = cose_key(&point);
        assert_eq!(&key[..8], &[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21]);
        assert_eq!(&key[8..10], &[0x58, 0x20]);
        assert_eq!(key.len(), 8 + 2 + 32 + 1 + 2 + 32);
        let object = attestation(&[7; 30]);
        assert_eq!(&object[..5], &[0xa3, 0x63, b'f', b'm', b't']);
        assert!(object.ends_with(&[&[0x58, 30][..], &[7; 30]].concat()));
    }
}
