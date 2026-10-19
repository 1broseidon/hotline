//! Just enough WAV for speech: wrap what a provider returns as bare PCM, join
//! the pieces of a sentence that had to be spoken in parts, and read back the
//! samples of a clip a person spoke.

const HEADER_BYTES: usize = 44;

/// Mono 16-bit PCM in a WAV container.
pub(crate) fn pcm16_wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(HEADER_BYTES + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes per sample frame
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

/// The `fmt ` chunk and the samples of a WAV. A streamed file may claim a
/// data length past its end, so the samples are whatever is left.
fn parts(wav: &[u8]) -> Option<(&[u8], &[u8])> {
    if wav.get(..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut format = None;
    while let (Some(id), Some(size)) = (wav.get(at..at + 4), wav.get(at + 4..at + 8)) {
        let size = u32::from_le_bytes(size.try_into().ok()?) as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(wav.len());
        match id {
            b"fmt " => format = Some(wav.get(body..end)?),
            b"data" => return Some((format?, wav.get(body..end)?)),
            _ => {}
        }
        // Chunks are padded to an even length.
        at = end + (size & 1);
    }
    None
}

/// One WAV from several with the same format, or `None` if any is not a WAV
/// or they disagree.
pub(crate) fn join(wavs: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut all = wavs.iter().map(|wav| parts(wav));
    let (format, first) = all.next()??;
    let mut samples = first.to_vec();
    for part in all {
        let (other, more) = part?;
        if other != format {
            return None;
        }
        samples.extend_from_slice(more);
    }
    // The pieces are all the mono 16-bit PCM this module writes, or they are
    // not something it should be rewrapping.
    let rate = mono_pcm16_rate(format)?;
    Some(pcm16_wav(&samples, rate))
}

/// The sample rate and samples of a mono 16-bit PCM WAV; `None` for any
/// other WAV, or for something that is not one.
pub(crate) fn mono_pcm16(wav: &[u8]) -> Option<(u32, &[u8])> {
    let (format, samples) = parts(wav)?;
    // A sample is two bytes; a torn last one is not a sample.
    Some((mono_pcm16_rate(format)?, &samples[..samples.len() & !1]))
}

/// The rate of a `fmt ` chunk that says mono 16-bit PCM.
fn mono_pcm16_rate(format: &[u8]) -> Option<u32> {
    let pcm = format.get(..2)? == 1u16.to_le_bytes();
    let mono = format.get(2..4)? == 1u16.to_le_bytes();
    let sixteen_bit = format.get(14..16)? == 16u16.to_le_bytes();
    let rate = u32::from_le_bytes(format.get(4..8)?.try_into().ok()?);
    (pcm && mono && sixteen_bit && rate > 0).then_some(rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_is_wrapped_with_a_header_a_player_can_read() {
        let wav = pcm16_wav(&[1, 0, 2, 0], 24_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 4);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 24_000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 48_000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 4);
        assert_eq!(&wav[44..], &[1, 0, 2, 0]);
    }

    #[test]
    fn pieces_of_one_format_join_into_one_clip() {
        let joined = join(&[pcm16_wav(&[1, 0, 2, 0], 24_000), pcm16_wav(&[3, 0], 24_000)]).unwrap();
        assert_eq!(joined, pcm16_wav(&[1, 0, 2, 0, 3, 0], 24_000));
    }

    #[test]
    fn a_streamed_header_that_claims_too_much_still_reads() {
        let mut wav = pcm16_wav(&[1, 0, 2, 0], 24_000);
        wav[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            join(&[wav, pcm16_wav(&[3, 0], 24_000)]).unwrap(),
            pcm16_wav(&[1, 0, 2, 0, 3, 0], 24_000)
        );
    }

    #[test]
    fn a_spoken_clip_reads_back_as_its_rate_and_samples() {
        assert_eq!(
            mono_pcm16(&pcm16_wav(&[1, 0, 2, 0, 3], 16_000)),
            Some((16_000, &[1u8, 0, 2, 0][..]))
        );
        let mut stereo = pcm16_wav(&[1, 0, 2, 0], 16_000);
        stereo[22..24].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(mono_pcm16(&stereo), None);
        assert_eq!(mono_pcm16(b"RIFF....WAVEnot a format"), None);
    }

    #[test]
    fn pieces_that_are_not_wavs_or_do_not_match_are_refused() {
        assert!(join(&[b"ID3 not a wav".to_vec()]).is_none());
        assert!(join(&[pcm16_wav(&[1, 0], 24_000), pcm16_wav(&[1, 0], 16_000)]).is_none());
        assert!(join(&[]).is_none());
    }
}
