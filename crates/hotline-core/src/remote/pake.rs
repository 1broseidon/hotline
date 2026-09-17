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
//! DSI   = "hotline-manual-pair-v1"
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

const DSI: &[u8] = b"hotline-manual-pair-v1";

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
    /// break for every paired phone's onboarding. They were repinned when the
    /// DSI became `hotline-manual-pair-v1`, so a phone still speaking the Toad
    /// string cannot pair and has to be updated.
    pub(super) const VECTORS: &[[&str; 10]] = &[
        [
            "000000",
            "8f0c1d2e-3a4b-4c5d-8e6f-70a1b2c3d4e5",
            "11111111-2222-4333-8444-555555555555",
            "f4d0d9aef8ec9a269f0dc46c9eb199bc3c65f1fc0b55af9463b29d8745d08baa",
            "0303030303030303030303030303030303030303030303030303030303030300",
            "0101010101010101010101010101010101010101010101010101010101010100",
            "747572575f30a082c2be437cfa0c4f93697fe9036b6b1e39177fdd2e24dd3f1d",
            "4e985ad10bcbd1641f6aad2fdeac760394915ba033fe719a5c8378e8bf0ebf3e",
            "56eefdf95653568044e2fc0fc5191737dbac7024a5516d7bfae3fe1de0f7e920",
            "7256da3b5bf47c2bc3cc40b54695f448be8e395760111b6db06c372b62a3a996",
        ],
        [
            "482913",
            "d4b7e2c1-9a3f-4e8b-9c6d-2f1a0b3c4d5e",
            "5c6d7e8f-9a0b-4c1d-8e2f-3a4b5c6d7e8f",
            "cba1c16035eca5706eb4e0af22c99b94c1f2b76278497aa8e93f2d0ba8a96223",
            "0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a00",
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b00",
            "3ed5d150d7e549c704baaa19d7509bc57aaca4b3b3aa7559c221c7dc53020f3c",
            "a832d52d5ce784661db78b7dc656966bc8a6365472fac3f6943b6658014a2427",
            "743b7c8ed414aad717e6fc9f7fa147b54958c0210f996a1d0397823aa07acf51",
            "411d16242166c23776c58715ea6b5711c273f76b22e5eca41f967b244ad37512",
        ],
        [
            "999999",
            "00000000-0000-4000-8000-000000000000",
            "ffffffff-ffff-4fff-bfff-ffffffffffff",
            "3e5ef2fb2a4ade8b62dd840bd52f29352a151d11f39dfb63c848189dba5ac2ff",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff00",
            "fefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefe00",
            "7eedc430a885c905fd00d18e7528b0cd7dbb4b5069b5114db1862c860e7a9370",
            "b23d4950193c68eb8f492fbb9585dcab8e88811181917e789e95755d677c6f23",
            "dd3d25c2c73581402609b1126355b3cf5573b87edcce063aea4a71d6df4ced61",
            "26c1d7ae42880221530310960e3f87d250d4776ed66edd42df1a1f4decadc94a",
        ],
        [
            "123456",
            "b1f6a9d3-2e7c-4a5b-9d8e-1c2b3a4f5e6d",
            "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d",
            "7a90b429f83e3f8c605b686dfb1d7ee994245308e6c8ca8415300eb58465145b",
            "0707070707070707070707070707070707070707070707070707070707070700",
            "1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f00",
            "f03d6b94e07bab9543eb2aff726c4c6e2a0c4cad9704e337b97c3449b000961f",
            "38663746fe84c808fec1add490d44a6baa9be95d5d6a05f3a6ecc551b0698f78",
            "a26abfa344fa447efaa962899ea696bd63fe86d1f2715739a1a7614bdd895bdf",
            "c3182c7a9f705e9589d8352e97649aed9e9cf46bfaf7bb13846090954678e88f",
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
