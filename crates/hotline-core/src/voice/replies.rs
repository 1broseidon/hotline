//! How teammates' replies on calls to them were said, counted by the model
//! that wrote each one, so a model that keeps to the call's contract can be
//! told from one that does not ("Claude Code followed the format 47 of 50
//! times").
//!
//! Each reply a call said is counted once, under the way it was written
//! and so said ([`Path`]), in `<data dir>/voice-replies.json`, written whole and
//! atomically after every count as the ledgers are. The counts are a
//! diagnostic: a file that cannot be read starts them again rather than
//! stopping a call.

use super::spoken::Path;
use crate::contract::VoiceReplies;
use std::io::Write;
use std::path::{Path as FilePath, PathBuf};
use std::sync::Mutex;

const FILE: &str = "voice-replies.json";

pub struct Replies {
    path: PathBuf,
    /// The counts, sorted by model; `None` until the file has been read.
    counts: Mutex<Option<Vec<VoiceReplies>>>,
}

impl Replies {
    /// The counts under a desk's data directory. Nothing is read until asked.
    pub fn open(data_root: &FilePath) -> Self {
        Self {
            path: data_root.join(FILE),
            counts: Mutex::new(None),
        }
    }

    /// Counts one reply `model` wrote, said by way of `path`.
    pub fn count(&self, model: &str, path: Path) {
        let mut counts = super::lock(&self.counts);
        let counts = counts.get_or_insert_with(|| self.read());
        let at = match counts.binary_search_by(|entry| entry.model.as_str().cmp(model)) {
            Ok(at) => at,
            Err(at) => {
                counts.insert(
                    at,
                    VoiceReplies {
                        model: model.to_string(),
                        ..VoiceReplies::default()
                    },
                );
                at
            }
        };
        let entry = &mut counts[at];
        let count = match path {
            Path::Both => &mut entry.both,
            Path::SpokenOnly => &mut entry.spoken_only,
            Path::Unclosed => &mut entry.unclosed,
            Path::Untagged => &mut entry.untagged,
            Path::Rewritten => &mut entry.rewritten,
        };
        *count = count.saturating_add(1);
        if let Err(error) = self.write(counts) {
            eprintln!("[voice] the reply counts could not be written: {error}");
        }
    }

    /// Every model's counts, sorted by model.
    pub fn counts(&self) -> Vec<VoiceReplies> {
        super::lock(&self.counts)
            .get_or_insert_with(|| self.read())
            .clone()
    }

    fn read(&self) -> Vec<VoiceReplies> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                eprintln!(
                    "[voice] the reply counts could not be read, so they start again: {error}"
                );
                return Vec::new();
            }
        };
        match serde_json::from_slice::<Vec<VoiceReplies>>(&bytes) {
            Ok(mut counts) => {
                counts.sort_by(|a, b| a.model.cmp(&b.model));
                counts.dedup_by(|a, b| a.model == b.model);
                counts
            }
            Err(error) => {
                eprintln!(
                    "[voice] the reply counts could not be read, so they start again: {error}"
                );
                Vec::new()
            }
        }
    }

    fn write(&self, counts: &[VoiceReplies]) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(counts)?;
        super::ledger::off_the_runtime(|| {
            let parent = self.path.parent().unwrap_or(FilePath::new("."));
            let mut staged = tempfile::NamedTempFile::new_in(parent)?;
            staged.write_all(&bytes)?;
            staged.as_file().sync_all()?;
            staged.persist(&self.path).map_err(|error| error.error)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_kept_by_model_and_survive_a_reopen() {
        let root = tempfile::tempdir().unwrap();
        let replies = Replies::open(root.path());
        assert!(replies.counts().is_empty());
        replies.count("acp/claude-code", Path::Both);
        replies.count("acp/claude-code", Path::Both);
        replies.count("acp/claude-code", Path::Untagged);
        replies.count("hotline/anthropic/claude-haiku-5-5", Path::SpokenOnly);
        replies.count("acp/codex", Path::Unclosed);
        replies.count("acp/codex", Path::Rewritten);

        let again = Replies::open(root.path());
        assert_eq!(
            again.counts(),
            [
                VoiceReplies {
                    model: "acp/claude-code".into(),
                    both: 2,
                    untagged: 1,
                    ..VoiceReplies::default()
                },
                VoiceReplies {
                    model: "acp/codex".into(),
                    unclosed: 1,
                    rewritten: 1,
                    ..VoiceReplies::default()
                },
                VoiceReplies {
                    model: "hotline/anthropic/claude-haiku-5-5".into(),
                    spoken_only: 1,
                    ..VoiceReplies::default()
                },
            ]
        );
        again.count("acp/codex", Path::Both);
        assert_eq!(Replies::open(root.path()).counts()[1].both, 1);
        // On disk as the wire has it.
        let file: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.path().join(FILE)).unwrap()).unwrap();
        assert_eq!(file[2]["spokenOnly"], 1);
    }

    #[test]
    fn a_file_that_cannot_be_read_starts_the_counts_again() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(FILE), "not json").unwrap();
        let replies = Replies::open(root.path());
        assert!(replies.counts().is_empty());
        replies.count("acp/codex", Path::Untagged);
        assert_eq!(Replies::open(root.path()).counts()[0].untagged, 1);
    }
}
