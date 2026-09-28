//! The v2 channel's cryptographic framing, independent of TLS, grants and JSON.
//!
//! Each ciphertext is one binary WebSocket message. Authentication and framing
//! errors are fatal: the caller must close the connection, never retry a frame
//! with an advanced Noise nonce. The decoder also refuses reuse after an error.

use snow::{Builder, HandshakeState, TransportState};

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"hotline/2";
const MAX_CIPHERTEXT: usize = 65_535;
const TAG_LEN: usize = 16;
const MAX_CHUNK: usize = MAX_CIPHERTEXT - TAG_LEN - 1;

fn builder() -> Result<Builder<'static>, String> {
    Builder::new(PATTERN.parse().map_err(|e: snow::Error| e.to_string())?)
        .prologue(PROLOGUE)
        .map_err(|e| e.to_string())
}

pub(crate) fn responder(private: &[u8; 32]) -> Result<HandshakeState, String> {
    builder()?
        .local_private_key(private)
        .and_then(Builder::build_responder)
        .map_err(|e| e.to_string())
}

pub(crate) fn initiator(private: &[u8; 32], desk: &[u8; 32]) -> Result<HandshakeState, String> {
    builder()?
        .local_private_key(private)
        .and_then(|builder| builder.remote_public_key(desk))
        .and_then(Builder::build_initiator)
        .map_err(|e| e.to_string())
}

/// Returns (private, public), both raw X25519 keys, not encoded strings.
pub(crate) fn keypair() -> Result<([u8; 32], [u8; 32]), String> {
    let pair = builder()?.generate_keypair().map_err(|e| e.to_string())?;
    Ok((
        pair.private
            .try_into()
            .map_err(|_| "Invalid private key length.")?,
        pair.public
            .try_into()
            .map_err(|_| "Invalid public key length.")?,
    ))
}

/// Preserves the frame's bytes, even when a chunk splits a UTF-8 code point.
pub(crate) fn encode(state: &mut TransportState, text: &str) -> Result<Vec<Vec<u8>>, String> {
    let bytes = text.as_bytes();
    let count = bytes.len().div_ceil(MAX_CHUNK).max(1);
    let mut messages = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * MAX_CHUNK;
        let end = start.saturating_add(MAX_CHUNK).min(bytes.len());
        let mut plaintext = Vec::with_capacity(1 + end - start);
        plaintext.push(u8::from(index + 1 == count));
        plaintext.extend_from_slice(&bytes[start..end]);
        let mut ciphertext = vec![0; plaintext.len() + TAG_LEN];
        let len = state
            .write_message(&plaintext, &mut ciphertext)
            .map_err(|e| e.to_string())?;
        ciphertext.truncate(len);
        messages.push(ciphertext);
    }
    Ok(messages)
}

pub(crate) struct Decoder {
    max: usize,
    pending: Vec<u8>,
    failed: bool,
}

impl Decoder {
    /// `max` bounds one whole reassembled text in bytes, excluding flags/tags.
    pub(crate) fn new(max: usize) -> Self {
        Self {
            max,
            pending: Vec::new(),
            failed: false,
        }
    }

    pub(crate) fn decode(
        &mut self,
        state: &mut TransportState,
        bytes: &[u8],
    ) -> Result<Option<String>, String> {
        if self.failed {
            return Err("The sealed channel decoder has failed.".into());
        }
        let result = self.decode_message(state, bytes);
        if result.is_err() {
            self.failed = true;
            self.pending.clear();
        }
        result
    }

    fn decode_message(
        &mut self,
        state: &mut TransportState,
        bytes: &[u8],
    ) -> Result<Option<String>, String> {
        if bytes.len() > MAX_CIPHERTEXT {
            return Err("Sealed ciphertext exceeds 65535 bytes.".into());
        }
        let mut plaintext = vec![0; bytes.len()];
        let len = state
            .read_message(bytes, &mut plaintext)
            .map_err(|e| e.to_string())?;
        let (flag, chunk) = plaintext[..len]
            .split_first()
            .ok_or("Sealed plaintext is missing its fragment flag.")?;
        if *flag > 1 {
            return Err("Invalid sealed fragment flag.".into());
        }
        if chunk.len() > self.max.saturating_sub(self.pending.len()) {
            return Err("Reassembled sealed frame exceeds its byte limit.".into());
        }
        self.pending.extend_from_slice(chunk);
        if *flag == 0 {
            return Ok(None);
        }
        String::from_utf8(std::mem::take(&mut self.pending))
            .map(Some)
            .map_err(|_| "Sealed frame is not UTF-8.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snow::{
        params::DHChoice,
        resolvers::{CryptoResolver, DefaultResolver},
    };

    fn public(private: &[u8]) -> Vec<u8> {
        let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).unwrap();
        dh.set(private);
        dh.pubkey().to_vec()
    }

    fn initiator(private: &[u8; 32], desk: &[u8]) -> HandshakeState {
        builder()
            .unwrap()
            .local_private_key(private)
            .unwrap()
            .remote_public_key(desk)
            .unwrap()
            .build_initiator()
            .unwrap()
    }

    fn handshake() -> (HandshakeState, HandshakeState) {
        let (desk, desk_public) = keypair().unwrap();
        let (device, _) = keypair().unwrap();
        (initiator(&device, &desk_public), responder(&desk).unwrap())
    }

    fn finish(mut i: HandshakeState, mut r: HandshakeState) -> (TransportState, TransportState) {
        let mut bytes = [0; MAX_CIPHERTEXT];
        let mut payload = [0; MAX_CIPHERTEXT];
        let n = i.write_message(&[], &mut bytes).unwrap();
        assert_eq!(r.read_message(&bytes[..n], &mut payload).unwrap(), 0);
        let n = r.write_message(&[], &mut bytes).unwrap();
        assert_eq!(i.read_message(&bytes[..n], &mut payload).unwrap(), 0);
        assert!(i.is_handshake_finished());
        assert!(r.is_handshake_finished());
        (
            i.into_transport_mode().unwrap(),
            r.into_transport_mode().unwrap(),
        )
    }

    fn transport() -> (TransportState, TransportState) {
        let (i, r) = handshake();
        finish(i, r)
    }

    fn raw(state: &mut TransportState, plaintext: &[u8]) -> Vec<u8> {
        let mut ciphertext = vec![0; plaintext.len() + TAG_LEN];
        let n = state.write_message(plaintext, &mut ciphertext).unwrap();
        ciphertext.truncate(n);
        ciphertext
    }

    #[test]
    fn generated_keys_are_distinct_valid_x25519_pairs() {
        let (a, a_public) = keypair().unwrap();
        let (b, b_public) = keypair().unwrap();
        assert_eq!(public(&a), a_public);
        assert_eq!(public(&b), b_public);
        assert_ne!(a, b);
        assert_ne!(a_public, b_public);
    }

    #[test]
    fn handshake_authenticates_device_and_preserves_pairing_payloads() {
        let (desk, desk_public) = keypair().unwrap();
        let (device, device_public) = keypair().unwrap();
        let mut i = initiator(&device, &desk_public);
        let mut r = responder(&desk).unwrap();
        let mut bytes = [0; MAX_CIPHERTEXT];
        let mut payload = [0; MAX_CIPHERTEXT];
        let request = br#"{"secret":"public-test-secret","name":"test phone"}"#;
        let reply = br#"{"role":"companion","deskName":"test desk"}"#;
        let n = i.write_message(request, &mut bytes).unwrap();
        let n = r.read_message(&bytes[..n], &mut payload).unwrap();
        assert_eq!(&payload[..n], request);
        assert_eq!(r.get_remote_static().unwrap(), device_public);
        let n = r.write_message(reply, &mut bytes).unwrap();
        let n = i.read_message(&bytes[..n], &mut payload).unwrap();
        assert_eq!(&payload[..n], reply);
        assert_eq!(i.get_remote_static().unwrap(), desk_public);
        assert_eq!(i.get_handshake_hash(), r.get_handshake_hash());
        assert!(i.is_handshake_finished() && r.is_handshake_finished());
    }

    #[test]
    fn wrong_pinned_desk_key_or_prologue_fails_authentication() {
        let (desk, desk_public) = keypair().unwrap();
        let (device, _) = keypair().unwrap();
        let (_, wrong_public) = keypair().unwrap();
        let wrong_key = initiator(&device, &wrong_public);
        let wrong_prologue = Builder::new(PATTERN.parse().unwrap())
            .prologue(b"hotline/1")
            .unwrap()
            .local_private_key(&device)
            .unwrap()
            .remote_public_key(&desk_public)
            .unwrap()
            .build_initiator()
            .unwrap();
        for mut i in [wrong_key, wrong_prologue] {
            let mut r = responder(&desk).unwrap();
            let mut bytes = [0; MAX_CIPHERTEXT];
            let n = i.write_message(&[], &mut bytes).unwrap();
            assert!(
                r.read_message(&bytes[..n], &mut [0; MAX_CIPHERTEXT])
                    .is_err()
            );
            assert!(!r.is_handshake_finished());
        }
    }

    #[test]
    fn handshake_tampering_is_refused_in_both_directions() {
        for which in [1, 2] {
            let (mut i, mut r) = handshake();
            let mut bytes = [0; MAX_CIPHERTEXT];
            let mut payload = [0; MAX_CIPHERTEXT];
            let n = i.write_message(&[], &mut bytes).unwrap();
            if which == 1 {
                bytes[n - 1] ^= 1;
                assert!(r.read_message(&bytes[..n], &mut payload).is_err());
            } else {
                r.read_message(&bytes[..n], &mut payload).unwrap();
                let n = r.write_message(&[], &mut bytes).unwrap();
                bytes[n - 1] ^= 1;
                assert!(i.read_message(&bytes[..n], &mut payload).is_err());
            }
        }
    }

    #[test]
    fn consecutive_frames_round_trip_exactly_in_both_directions() {
        let (mut i, mut r) = transport();
        let mut i_decoder = Decoder::new(1024);
        let mut r_decoder = Decoder::new(1024);
        // JSON interpretation belongs to the wire, not the cipher codec.
        for text in ["", "not JSON", " {\"x\": \"🦀 café\"}\n", "{}", "[]"] {
            let messages = encode(&mut i, text).unwrap();
            assert_eq!(messages.len(), 1);
            assert_ne!(&messages[0], text.as_bytes());
            assert_eq!(
                r_decoder.decode(&mut r, &messages[0]).unwrap().as_deref(),
                Some(text)
            );
            let messages = encode(&mut r, text).unwrap();
            assert_eq!(
                i_decoder.decode(&mut i, &messages[0]).unwrap().as_deref(),
                Some(text)
            );
        }
    }

    #[test]
    fn fragmentation_respects_ciphertext_cap_and_reassembles_split_utf8() {
        let (mut i, mut r) = transport();
        // The four-byte character crosses the first chunk boundary.
        let text = format!(
            "{}🦀{}",
            "a".repeat(MAX_CHUNK - 1),
            "b".repeat(MAX_CHUNK * 2)
        );
        let messages = encode(&mut i, &text).unwrap();
        assert_eq!(messages.len(), 4);
        let mut decoder = Decoder::new(text.len());
        for (index, ciphertext) in messages.iter().enumerate() {
            assert!(ciphertext.len() <= MAX_CIPHERTEXT);
            if index < 3 {
                assert_eq!(ciphertext.len(), MAX_CIPHERTEXT);
            }
            let decoded = decoder.decode(&mut r, ciphertext).unwrap();
            if index + 1 == messages.len() {
                assert_eq!(decoded.as_deref(), Some(text.as_str()));
            } else {
                assert_eq!(decoded, None);
            }
        }
        assert!(decoder.pending.is_empty());
    }

    #[test]
    fn exact_chunk_boundaries_need_no_extra_terminator_message() {
        for len in [0, 1, MAX_CHUNK - 1, MAX_CHUNK, MAX_CHUNK + 1, 2 * MAX_CHUNK] {
            let (mut i, mut r) = transport();
            let text = "x".repeat(len);
            let messages = encode(&mut i, &text).unwrap();
            assert_eq!(messages.len(), len.div_ceil(MAX_CHUNK).max(1));
            for (index, message) in messages.iter().enumerate() {
                let mut plaintext = [0; MAX_CIPHERTEXT];
                let n = r.read_message(message, &mut plaintext).unwrap();
                assert_eq!(plaintext[0], u8::from(index + 1 == messages.len()));
                assert_eq!(
                    &plaintext[1..n],
                    &text.as_bytes()[index * MAX_CHUNK..(index * MAX_CHUNK + n - 1)]
                );
            }
        }
    }

    #[test]
    fn reassembly_cap_counts_bytes_across_all_fragments_and_resets_per_frame() {
        let (mut i, mut r) = transport();
        let mut decoder = Decoder::new(4);
        assert_eq!(
            decoder.decode(&mut r, &raw(&mut i, b"\x00ab")).unwrap(),
            None
        );
        assert_eq!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01cd"))
                .unwrap()
                .as_deref(),
            Some("abcd")
        );
        assert_eq!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01wxyz"))
                .unwrap()
                .as_deref(),
            Some("wxyz")
        );
        assert_eq!(
            decoder.decode(&mut r, &raw(&mut i, b"\x00abcd")).unwrap(),
            None
        );
        assert!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01e"))
                .unwrap_err()
                .contains("byte limit")
        );
        assert!(decoder.pending.is_empty());
        assert!(decoder.decode(&mut r, &raw(&mut i, b"\x01ok")).is_err());
    }

    #[test]
    fn over_cap_continuation_and_single_frames_are_refused() {
        for plaintext in [b"\x00abcde".as_slice(), b"\x01abcde", "\u{1}🦀a".as_bytes()] {
            let (mut i, mut r) = transport();
            assert!(
                Decoder::new(4)
                    .decode(&mut r, &raw(&mut i, plaintext))
                    .unwrap_err()
                    .contains("byte limit")
            );
        }
        let (mut i, mut r) = transport();
        let mut decoder = Decoder::new(0);
        assert_eq!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01"))
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert!(decoder.decode(&mut r, &raw(&mut i, b"\x01x")).is_err());
    }

    #[test]
    fn missing_and_unknown_flags_are_refused_even_when_authenticated() {
        for plaintext in [b"".as_slice(), b"\x02{}", b"\xff{}"] {
            let (mut i, mut r) = transport();
            let mut decoder = Decoder::new(1024);
            assert!(
                decoder
                    .decode(&mut r, &raw(&mut i, plaintext))
                    .unwrap_err()
                    .contains("flag")
            );
            assert!(decoder.failed);
        }
    }

    #[test]
    fn utf8_is_checked_only_after_the_final_fragment() {
        let (mut i, mut r) = transport();
        let mut decoder = Decoder::new(10);
        assert_eq!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x00\xf0\x9f"))
                .unwrap(),
            None
        );
        assert_eq!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01\xa6\x80"))
                .unwrap()
                .as_deref(),
            Some("🦀")
        );
        assert_eq!(
            decoder.decode(&mut r, &raw(&mut i, b"\x00\xff")).unwrap(),
            None
        );
        assert!(
            decoder
                .decode(&mut r, &raw(&mut i, b"\x01"))
                .unwrap_err()
                .contains("UTF-8")
        );
    }

    #[test]
    fn tampering_truncation_and_oversized_ciphertexts_fail_closed() {
        for which in 0..5 {
            let (mut i, mut r) = transport();
            let mut ciphertext = encode(&mut i, "{}").unwrap().remove(0);
            match which {
                0 => ciphertext[0] ^= 1,
                1 => {
                    let last = ciphertext.len() - 1;
                    ciphertext[last] ^= 1;
                }
                2 => {
                    ciphertext.pop();
                }
                3 => ciphertext.clear(),
                4 => ciphertext.resize(MAX_CIPHERTEXT + 1, 0),
                _ => unreachable!(),
            }
            let mut decoder = Decoder::new(usize::MAX);
            assert!(decoder.decode(&mut r, &ciphertext).is_err());
            assert!(decoder.failed);
            assert!(decoder.pending.is_empty());
        }
    }

    #[test]
    fn replay_and_out_of_order_frames_are_refused() {
        let (mut i, mut r) = transport();
        let first = encode(&mut i, "one").unwrap().remove(0);
        let mut decoder = Decoder::new(100);
        assert_eq!(
            decoder.decode(&mut r, &first).unwrap().as_deref(),
            Some("one")
        );
        assert!(decoder.decode(&mut r, &first).is_err());
        let (mut i, mut r) = transport();
        let _first = encode(&mut i, "one").unwrap();
        let second = encode(&mut i, "two").unwrap().remove(0);
        assert!(Decoder::new(100).decode(&mut r, &second).is_err());
    }

    #[test]
    fn fixed_key_interoperability_vector_matches_every_byte() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/noise_v2.json")).unwrap();
        assert_interoperability_vector(&fixture);
    }

    #[test]
    fn fixed_key_viewer_binding_vector_matches_every_byte() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/noise_v2.json")).unwrap();
        // Keep the original room vector at the root for existing clients.
        let viewer = &fixture["viewer"];
        let binding = r#"{"purpose":"computer","personaId":"ada"}"#;
        assert_eq!(viewer["message_1_payload_utf8"], binding);
        assert_eq!(viewer["message_1_payload_hex"], hex::encode(binding));
        assert_eq!(viewer["message_2_payload_hex"], "");
        // The binding is authenticated even though message 2 has no payload.
        for field in ["message_1_hex", "message_2_hex", "handshake_hash_hex"] {
            assert_ne!(viewer[field], fixture[field], "{field}");
        }
        assert_interoperability_vector(viewer);
    }

    fn assert_interoperability_vector(fixture: &serde_json::Value) {
        let text = |name: &str| fixture[name].as_str().unwrap();
        let bytes = |name: &str| hex::decode(text(name)).unwrap();
        assert_eq!(text("protocol_name"), PATTERN);
        assert_eq!(text("prologue_utf8").as_bytes(), PROLOGUE);
        assert_eq!(bytes("prologue_hex"), PROLOGUE);
        for prefix in [
            "desk_static",
            "device_static",
            "desk_ephemeral",
            "device_ephemeral",
        ] {
            assert_eq!(
                public(&bytes(&format!("{prefix}_private_key_hex"))),
                bytes(&format!("{prefix}_public_key_hex"))
            );
        }
        let device = bytes("device_static_private_key_hex");
        let desk = bytes("desk_static_private_key_hex");
        let desk_public = bytes("desk_static_public_key_hex");
        let device_e = bytes("device_ephemeral_private_key_hex");
        let desk_e = bytes("desk_ephemeral_private_key_hex");
        // The fixed-ephemeral hook is used only here, never in production.
        let mut i = builder()
            .unwrap()
            .local_private_key(&device)
            .unwrap()
            .remote_public_key(&desk_public)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(&device_e)
            .build_initiator()
            .unwrap();
        let mut r = builder()
            .unwrap()
            .local_private_key(&desk)
            .unwrap()
            .fixed_ephemeral_key_for_testing_only(&desk_e)
            .build_responder()
            .unwrap();
        let mut message = [0; MAX_CIPHERTEXT];
        let mut payload = [0; MAX_CIPHERTEXT];
        let n = i
            .write_message(&bytes("message_1_payload_hex"), &mut message)
            .unwrap();
        assert_eq!(&message[..n], bytes("message_1_hex"));
        assert_eq!(&message[..32], bytes("device_ephemeral_public_key_hex"));
        let n = r.read_message(&message[..n], &mut payload).unwrap();
        assert_eq!(&payload[..n], bytes("message_1_payload_hex"));
        assert_eq!(
            r.get_remote_static().unwrap(),
            bytes("device_static_public_key_hex")
        );
        let n = r
            .write_message(&bytes("message_2_payload_hex"), &mut message)
            .unwrap();
        assert_eq!(&message[..n], bytes("message_2_hex"));
        assert_eq!(&message[..32], bytes("desk_ephemeral_public_key_hex"));
        let n = i.read_message(&message[..n], &mut payload).unwrap();
        assert_eq!(&payload[..n], bytes("message_2_payload_hex"));
        assert_eq!(i.get_handshake_hash(), bytes("handshake_hash_hex"));
        assert_eq!(r.get_handshake_hash(), bytes("handshake_hash_hex"));
        let mut i = i.into_transport_mode().unwrap();
        let mut r = r.into_transport_mode().unwrap();
        for prefix in ["initiator", "responder"] {
            let (sender, receiver) = if prefix == "initiator" {
                (&mut i, &mut r)
            } else {
                (&mut r, &mut i)
            };
            let frame = text(&format!("{prefix}_first_transport_text_utf8"));
            let expected_plaintext = [b"\x01".as_slice(), frame.as_bytes()].concat();
            assert_eq!(
                bytes(&format!("{prefix}_first_transport_plaintext_hex")),
                expected_plaintext
            );
            let messages = encode(sender, frame).unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(
                messages[0],
                bytes(&format!("{prefix}_first_transport_ciphertext_hex"))
            );
            assert_eq!(
                Decoder::new(1024)
                    .decode(receiver, &messages[0])
                    .unwrap()
                    .as_deref(),
                Some(frame)
            );
        }
    }
}
