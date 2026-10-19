//! What can be said about a clip from its size and length alone, before a
//! provider hears it or a sentence of it is believed.

/// A clip is at most this long, whatever its header says.
const MAX_CLIP_MS: u32 = 20_000;

/// The lowest bitrate an AAC recording of speech is trusted to have. An MP4
/// carries its own length in a header the client wrote, and a header that
/// claims less than the bytes can hold undercounts what a provider bills; the
/// bytes at this rate are the most that clip could last.
const MIN_MP4_BITS_PER_SECOND: u64 = 32_000;

/// The milliseconds of speech to reserve for: what the client says the clip
/// lasts, but never less than could fit in its bytes.
///
/// An MP4 is bounded by its size at [`MIN_MP4_BITS_PER_SECOND`], capped at the
/// longest clip there is. A WAV's length is its size, which the caller already
/// checks against the claim, so it is returned as claimed.
pub fn billable_ms(mime: &str, bytes: usize, claimed_ms: u32) -> u32 {
    let mime = super::base_mime(mime);
    if mime != "audio/mp4" {
        return claimed_ms;
    }
    let most = (bytes as u64).saturating_mul(8_000) / MIN_MP4_BITS_PER_SECOND;
    let most = u32::try_from(most).unwrap_or(u32::MAX).min(MAX_CLIP_MS);
    claimed_ms.max(most)
}

/// The shortest clip that can hold a whole farewell.
pub const MIN_GOODBYE_MS: u32 = 400;

/// Whether a clip that transcribed as a farewell could have been one.
///
/// Whisper-style engines answer noise, a click or a breath with "Bye." or
/// "Thank you.", and a call that hangs up on those hangs up on a fan. A real
/// goodbye takes at least [`MIN_GOODBYE_MS`] to say and carries at least a byte
/// for each millisecond (8 kbit/s, well under any recording of speech); a
/// shorter or emptier clip is not one, whatever the words say. This is a
/// necessary condition and not a sufficient one, so it is asked beside the
/// words, never instead of them: `goodbye(text) && plausible_goodbye(ms, bytes)`.
pub fn plausible_goodbye(duration_ms: u32, bytes: usize) -> bool {
    duration_ms >= MIN_GOODBYE_MS && bytes as u64 >= u64::from(duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_that_claims_less_than_the_bytes_can_hold_is_not_believed() {
        // 60 KB of AAC could be 15 seconds at 32 kbit/s; a header saying 1 is a lie or a bug.
        assert_eq!(billable_ms("audio/mp4", 60_000, 1_000), 15_000);
        // The bound is the longest a clip can be, however many bytes.
        assert_eq!(billable_ms("audio/mp4", 2_000_000, 1_000), 20_000);
        assert_eq!(billable_ms("audio/mp4", usize::MAX, 1_000), 20_000);
    }

    #[test]
    fn an_honest_header_is_never_billed_less_than_it_says() {
        // A clip at a lower bitrate than the bound lasts longer than the bound
        // allows for, and its own claim stands.
        assert_eq!(billable_ms("audio/mp4", 15_000, 12_000), 12_000);
        assert_eq!(billable_ms("audio/mp4", 0, 5_000), 5_000);
        assert_eq!(billable_ms("audio/mp4", 40_000, 20_000), 20_000);
    }

    #[test]
    fn a_wav_is_billed_as_claimed_and_the_type_is_read_as_a_mime_is() {
        assert_eq!(billable_ms("audio/wav", 1_000_000, 500), 500);
        assert_eq!(billable_ms("audio/webm", 1_000_000, 500), 500);
        assert_eq!(
            billable_ms("Audio/MP4; codecs=mp4a.40.2", 60_000, 1_000),
            15_000
        );
    }

    #[test]
    fn a_goodbye_needs_time_to_be_said_and_something_in_the_clip() {
        // 300 ms of a click is not "bye bye", whatever the engine wrote down.
        assert!(!plausible_goodbye(300, 9_600));
        assert!(!plausible_goodbye(399, 100_000));
        assert!(plausible_goodbye(400, 12_800));
        assert!(plausible_goodbye(1_200, 38_400));
        // A clip with almost nothing in its bytes is not speech either.
        assert!(!plausible_goodbye(1_000, 999));
        assert!(plausible_goodbye(1_000, 1_000));
        assert!(!plausible_goodbye(0, 0));
    }

    #[test]
    fn the_shortest_clips_the_clients_make_pass() {
        // 16 kHz mono PCM16 is 32 bytes a millisecond; AAC at 32 kbit/s is 4.
        assert!(plausible_goodbye(MIN_GOODBYE_MS, 400 * 32));
        assert!(plausible_goodbye(MIN_GOODBYE_MS, 400 * 4));
    }
}
