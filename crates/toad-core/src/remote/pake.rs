//! The manual pairing exchange: a six-digit code as the password of a
//! balanced PAKE, and a key confirmation that names the certificate the phone
//! actually connected to.
//!
//! Why not a bearer code: the QR carries the certificate fingerprint, so a
//! phone that scans pins before it sends anything. Six digits cannot carry a
//! fingerprint, so a phone that types them connects trust-on-first-use, and
//! anyone on the LAN who can answer that connection could relay the code to
//! the real desktop. With a PAKE the code never leaves either device, a wrong
//! guess is only an online guess, and a relay's own certificate breaks both
//! confirmations.
//!
//! The construction is CPace-shaped over ristretto255. Every byte of it is
//! spelled here because the phone implements the same thing in another
//! language and proves it against the vectors in the tests below.
//!
//! ```text
//! DSI   = "toad-manual-pair-v1"
//! lv(x) = one byte len(x) || x                  (every field is < 256 bytes)
//! G     = ristretto255 one-way map of SHA-512(DSI || lv(code) || lv(claimId))
//! phone: a random, A = a·G      desktop: b random, B = b·G
//! K     = a·B = b·A               (A, B must decode and must not be the identity)
//! isk   = SHA-256(DSI || "/isk" || lv(desktopId) || lv(claimId) || K || A || B || cert)
//! phoneConfirm   = HMAC-SHA256(isk, DSI || "/phone")
//! desktopConfirm = HMAC-SHA256(isk, DSI || "/desktop")
//! ```
//!
//! `cert` is the 32 raw bytes of the SHA-256 of the listener's DER
//! certificate: the desktop uses its own, the phone uses the one it observed
//! on the TLS connection. Points and MACs travel as lowercase hex.
use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

const DSI: &[u8] = b"toad-manual-pair-v1";

fn lv(out: &mut Vec<u8>, field: &[u8]) {
    debug_assert!(field.len() < 256);
    out.push(field.len() as u8);
    out.extend_from_slice(field);
}

/// The password-derived generator both sides scale their secret by. The
/// desktop id is absent on purpose: the phone sends its share before it
/// learns who it is talking to, and the id binds the key later instead.
fn generator(code: &str, claim_id: &str) -> RistrettoPoint {
    let mut seed = Vec::with_capacity(128);
    seed.extend_from_slice(DSI);
    lv(&mut seed, code.as_bytes());
    lv(&mut seed, claim_id.as_bytes());
    RistrettoPoint::from_uniform_bytes(&Sha512::digest(&seed).into())
}

/// A wire-encoded ristretto255 point that is not the identity.
pub(super) fn decode_public(hex_point: &str) -> Option<RistrettoPoint> {
    let bytes = hex::decode(hex_point).ok()?;
    let point = CompressedRistretto::from_slice(&bytes).ok()?.decompress()?;
    (point != RistrettoPoint::default()).then_some(point)
}

pub(super) fn random_scalar() -> Scalar {
    let mut wide = [0u8; 64];
    getrandom::fill(&mut wide).expect("the OS random source is available");
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// One side's public share of the exchange, as it travels on the wire.
pub(super) fn public(code: &str, claim_id: &str, secret: &Scalar) -> String {
    hex::encode((secret * generator(code, claim_id)).compress().as_bytes())
}

/// What both sides can compute once they hold the other's share.
pub(super) struct Confirmations {
    pub phone: String,
    pub desktop: String,
}

/// `cert` is the hex SHA-256 the caller trusts: the desktop's own, or the one
/// the phone saw. `secret` is this side's scalar; `peer` the other's share.
pub(super) fn confirmations(
    desktop_id: &str,
    claim_id: &str,
    secret: &Scalar,
    peer: &RistrettoPoint,
    phone_public: &str,
    desktop_public: &str,
    cert_sha256: &str,
) -> Option<Confirmations> {
    let shared = (secret * peer).compress();
    let mut transcript = Vec::with_capacity(256);
    transcript.extend_from_slice(DSI);
    transcript.extend_from_slice(b"/isk");
    lv(&mut transcript, desktop_id.as_bytes());
    lv(&mut transcript, claim_id.as_bytes());
    transcript.extend_from_slice(shared.as_bytes());
    transcript.extend_from_slice(&hex::decode(phone_public).ok()?);
    transcript.extend_from_slice(&hex::decode(desktop_public).ok()?);
    let cert = hex::decode(cert_sha256).ok()?;
    if cert.len() != 32 {
        return None;
    }
    transcript.extend_from_slice(&cert);
    let isk = Sha256::digest(&transcript);
    let tag = |side: &[u8]| {
        let mut mac = Hmac::<Sha256>::new_from_slice(&isk).expect("any key length");
        mac.update(DSI);
        mac.update(side);
        hex::encode(mac.finalize().into_bytes())
    };
    Some(Confirmations {
        phone: tag(b"/phone"),
        desktop: tag(b"/desktop"),
    })
}

pub(super) fn same_tag(presented: &str, expected: &str) -> bool {
    crate::wire::same_secret(presented, expected)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// (code, desktopId, claimId, cert, a, b, A, B, phoneConfirm, desktopConfirm).
    /// The phone's implementation reproduces these; changing one is a wire
    /// break for every paired phone's onboarding.
    pub(super) const VECTORS: &[[&str; 10]] = &[
        [
            "000000",
            "8f0c1d2e-3a4b-4c5d-8e6f-70a1b2c3d4e5",
            "11111111-2222-4333-8444-555555555555",
            "f4d0d9aef8ec9a269f0dc46c9eb199bc3c65f1fc0b55af9463b29d8745d08baa",
            "0303030303030303030303030303030303030303030303030303030303030300",
            "0101010101010101010101010101010101010101010101010101010101010100",
            "049c361a9a0329f3abdc4a4e7826847bf01ea81364cfbf668302fb97bffe9567",
            "36e06822c8cb76a1a7e14a8287a68690bf64e92eb055cf300d6cfc1f084b0a0d",
            "5373e1ce84534dac60629c4ce55350e3c284f0c6ad794e5d5a3862afbbd83331",
            "6258d2ee8f0fb936eae91f62c84bd887d762ebdfd51f91af540656318c92def4",
        ],
        [
            "482913",
            "d4b7e2c1-9a3f-4e8b-9c6d-2f1a0b3c4d5e",
            "5c6d7e8f-9a0b-4c1d-8e2f-3a4b5c6d7e8f",
            "cba1c16035eca5706eb4e0af22c99b94c1f2b76278497aa8e93f2d0ba8a96223",
            "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a00",
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b00",
            "0828a821a791aba797abc795b27008a0b43a9f26c841ca9c217b2dbce1c27e30",
            "54d720e9c0feff060c194b7c9731fdf8110ef0c5edd32144c17f9780a7c2cb03",
            "f8d631bf16dc1270dff7e2e8c1ded109e86b6faad8fe19607efcfbcc60a558b1",
            "46d53c88fe9bb5aee1a3eac2de9e88cfe2c7d09a2fd7f691576b7ec5da07d788",
        ],
        [
            "999999",
            "00000000-0000-4000-8000-000000000000",
            "ffffffff-ffff-4fff-bfff-ffffffffffff",
            "3e5ef2fb2a4ade8b62dd840bd52f29352a151d11f39dfb63c848189dba5ac2ff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00",
            "fefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefe00",
            "ca65c88efd60ce3a1920042b012c7db20f6923f1002419f4617b0f13b14b2f7e",
            "b434e9e739ed3df829f42d3ceb3aaf9a0eeec7865b2ab41a2c12b33deef41451",
            "972c333cb39de5ae5c595cf00c60072141aed5ab5b61f96b27ea07d7889ae040",
            "576a38f6c2e98f2b7bf1a4b82e5dba72d9101d09a081cb25bc4331a73d0670fa",
        ],
        [
            "123456",
            "b1f6a9d3-2e7c-4a5b-9d8e-1c2b3a4f5e6d",
            "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
            "7a90b429f83e3f8c605b686dfb1d7ee994245308e6c8ca8415300eb58465145b",
            "0707070707070707070707070707070707070707070707070707070707070700",
            "1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f00",
            "02fba08230551840ce3abe6dbb72ff8de8018277db0e1feced2dd93e76434a52",
            "92446142481d71c35a4591f829a30123ec0eae984486d959101f9be632beed50",
            "727109e19eb3fff087c337faa1c92fb11ee462692b24454be58fedcdcd24cfae",
            "a1ebb5c7d4a28466905e295104dbfcfe208ae9f30166b900db24479a9b25848f",
        ],
    ];

    pub(super) fn scalar(hex_scalar: &str) -> Scalar {
        Scalar::from_bytes_mod_order(hex::decode(hex_scalar).unwrap().try_into().unwrap())
    }

    #[test]
    fn the_published_vectors_hold_on_both_sides() {
        for [
            code,
            desktop,
            claim,
            cert,
            a,
            b,
            pa,
            pb,
            phone_tag,
            desktop_tag,
        ] in VECTORS
        {
            let (a, b) = (scalar(a), scalar(b));
            assert_eq!(&public(code, claim, &a), pa);
            assert_eq!(&public(code, claim, &b), pb);
            let phone = confirmations(
                desktop,
                claim,
                &a,
                &decode_public(pb).unwrap(),
                pa,
                pb,
                cert,
            )
            .unwrap();
            let desk = confirmations(
                desktop,
                claim,
                &b,
                &decode_public(pa).unwrap(),
                pa,
                pb,
                cert,
            )
            .unwrap();
            for side in [&phone, &desk] {
                assert_eq!(&side.phone, phone_tag);
                assert_eq!(&side.desktop, desktop_tag);
            }
        }
    }

    #[test]
    fn a_wrong_code_or_a_relayed_certificate_breaks_both_confirmations() {
        let [code, desktop, claim, cert, a, b, pa, pb, phone_tag, _] = VECTORS[1];
        let (a, b) = (scalar(a), scalar(b));
        // The phone typed one digit wrong: its share lies on another generator.
        let wrong = public("482914", claim, &a);
        let desk = confirmations(
            desktop,
            claim,
            &b,
            &decode_public(&wrong).unwrap(),
            &wrong,
            pb,
            cert,
        )
        .unwrap();
        assert_ne!(desk.phone, phone_tag);
        // The phone saw a relay's certificate: the right code still fails.
        let relayed = hex::encode([7u8; 32]);
        let phone = confirmations(
            desktop,
            claim,
            &a,
            &decode_public(pb).unwrap(),
            pa,
            pb,
            &relayed,
        )
        .unwrap();
        assert_ne!(phone.phone, phone_tag);
        let _ = code;
        // The identity point and junk never decode into an exchange.
        assert!(decode_public(&hex::encode([0u8; 32])).is_none());
        assert!(decode_public("zz").is_none());
        assert!(decode_public(&hex::encode([1u8; 31])).is_none());
    }
}
