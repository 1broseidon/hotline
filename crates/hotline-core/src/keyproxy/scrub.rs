//! Writing the stand-in back over a value in a reply, as it streams.

use std::pin::Pin;
use std::task::{Context, Poll, ready};

use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};

/// `(what was sent, what the job reads instead)`, literal bytes.
pub(super) type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

/// Literal replacements over a stream of chunks, matches split across chunk
/// boundaries included. At each position the earliest match wins, and of
/// two starting there the longer.
pub(super) struct Scrubber {
    pairs: Pairs,
    longest: usize,
    pending: Vec<u8>,
}

impl Scrubber {
    pub fn new(mut pairs: Pairs) -> Self {
        pairs.retain(|(from, to)| !from.is_empty() && from != to);
        let mut seen = std::collections::HashSet::new();
        pairs.retain(|(from, _)| seen.insert(from.clone()));
        let longest = pairs.iter().map(|(from, _)| from.len()).max().unwrap_or(0);
        Self {
            pairs,
            longest,
            pending: Vec::new(),
        }
    }

    /// Whether every replacement keeps the length, so a `Content-Length`
    /// still holds.
    pub fn keeps_length(&self) -> bool {
        self.pairs.iter().all(|(from, to)| from.len() == to.len())
    }

    /// The whole of `bytes`, scrubbed.
    pub fn all(&self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        let used = self.scan(bytes, bytes.len(), &mut out);
        out.extend_from_slice(&bytes[used..]);
        out
    }

    /// The next chunk: what can be sent now. A tail that may begin a match
    /// waits for the next chunk or `finish`.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.pairs.is_empty() {
            return chunk.to_vec();
        }
        self.pending.extend_from_slice(chunk);
        let safe = self.pending.len().saturating_sub(self.longest - 1);
        let mut out = Vec::with_capacity(self.pending.len());
        let pending = std::mem::take(&mut self.pending);
        let used = self.scan(&pending, safe, &mut out);
        self.pending = pending[used..].to_vec();
        out
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let pending = std::mem::take(&mut self.pending);
        self.all(&pending)
    }

    /// Scrubs matches that start before `safe`, writing to `out`; returns
    /// how much of `bytes` it consumed (at least `safe`, unless shorter).
    fn scan(&self, bytes: &[u8], safe: usize, out: &mut Vec<u8>) -> usize {
        let mut at = 0;
        while at < safe {
            let hit = self
                .pairs
                .iter()
                .filter(|(from, _)| bytes[at..].starts_with(from))
                .max_by_key(|(from, _)| from.len());
            if let Some((from, to)) = hit {
                out.extend_from_slice(to);
                at += from.len();
            } else {
                out.push(bytes[at]);
                at += 1;
            }
        }
        at
    }
}

/// A reply body with the scrubber run over it. Trailers are dropped.
pub(super) struct ScrubBody<B> {
    inner: B,
    scrubber: Scrubber,
    done: bool,
}

impl<B> ScrubBody<B> {
    pub fn new(inner: B, scrubber: Scrubber) -> Self {
        Self {
            inner,
            scrubber,
            done: false,
        }
    }
}

impl<B> Body for ScrubBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, B::Error>>> {
        let this = &mut *self;
        loop {
            if this.done {
                return Poll::Ready(None);
            }
            match ready!(Pin::new(&mut this.inner).poll_frame(cx)) {
                Some(Ok(frame)) => {
                    let Ok(data) = frame.into_data() else {
                        continue;
                    };
                    let out = this.scrubber.push(&data);
                    if !out.is_empty() {
                        return Poll::Ready(Some(Ok(Frame::data(out.into()))));
                    }
                }
                Some(Err(error)) => return Poll::Ready(Some(Err(error))),
                None => {
                    this.done = true;
                    let tail = this.scrubber.finish();
                    if !tail.is_empty() {
                        return Poll::Ready(Some(Ok(Frame::data(tail.into()))));
                    }
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }

    fn size_hint(&self) -> SizeHint {
        if self.scrubber.keeps_length() {
            self.inner.size_hint()
        } else {
            SizeHint::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Scrubber;

    fn pairs() -> super::Pairs {
        vec![(b"SECRETVALUE".to_vec(), b"stand_in_xx".to_vec())]
    }

    #[test]
    fn a_value_split_across_chunks_is_caught() {
        let text = b"before SECRETVALUE middle SECRETVALUE after";
        for split in 1..text.len() {
            let mut s = Scrubber::new(pairs());
            let mut out = s.push(&text[..split]);
            out.extend(s.push(&text[split..]));
            out.extend(s.finish());
            assert_eq!(
                out, b"before stand_in_xx middle stand_in_xx after",
                "split {split}"
            );
        }
    }

    #[test]
    fn byte_by_byte_and_a_partial_tail() {
        let text = b"SECRETVALUESECRETVAL";
        let mut s = Scrubber::new(pairs());
        let mut out = Vec::new();
        for b in text {
            out.extend(s.push(&[*b]));
        }
        out.extend(s.finish());
        assert_eq!(out, b"stand_in_xxSECRETVAL");
    }

    #[test]
    fn the_longer_match_wins() {
        let s = Scrubber::new(vec![
            (b"VAL".to_vec(), b"x".to_vec()),
            (b"Bearer VAL".to_vec(), b"Bearer y".to_vec()),
        ]);
        assert_eq!(s.all(b"[Bearer VAL] VAL"), b"[Bearer y] x");
        assert!(!s.keeps_length());
    }
}
