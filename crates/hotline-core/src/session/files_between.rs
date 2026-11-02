//! Files between teammates (`docs/design/thread-awareness.md` §3).
//!
//! A message or an aside may carry files. The sender's are read where its
//! read tools find them, by the check `send_file` makes
//! ([`super::files::from_workspace`]): its working directory and the folders
//! granted it, or anywhere under whole-machine reach. They are read when the
//! teammate sends them, and a message's are held on the desk
//! (`files-between/<request id>/`) until the message is let in, once the
//! grant an ask needs allows it; an aside's go as it is left, past the same
//! grant. Then each is copied into the recipient's working directory under
//! `inbox/<sender>/`, never over a file there, and the message or the aside
//! says where. To a teammate on a linked desk they cross the link inside the
//! message's or aside's one frame, so together they are at most
//! [`LINKED_FILES_MAX`]; the desk there checks, holds and copies them the
//! same way. Nothing new is approved: a file goes only where the message or
//! aside carrying it may.

use super::Room;
use crate::contract::{LinkedFile, Persona, Reach};
use crate::driver::CapabilityLease;
use crate::tools::Workspace;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::{Path, PathBuf};

/// The most a message's or an aside's files come to together on one desk:
/// what `send_file` sends the person.
pub(crate) const FILES_MAX: u64 = crate::sent::MAX_BYTES;

/// The most they come to across a link. They cross in the frame of the
/// message or aside they ride on, and a link's frame is 1 MiB: this much,
/// grown by a third as base64, leaves room for the longest message.
pub(crate) const LINKED_FILES_MAX: u64 = 512 * 1024;

/// The most files one message or aside carries.
pub(crate) const FILES_COUNT_MAX: usize = 10;

/// Where the desk holds a message's files until it is let in.
const HELD: &str = "files-between";

/// The folder of the recipient's working directory they are copied into.
const INBOX: &str = "inbox";

/// Files read for a message, held on the desk under `key` until the message
/// is queued, and then under its request id ([`Held::file_under`]).
pub(crate) struct Held {
    key: String,
    pub(crate) names: Vec<String>,
}

impl Held {
    /// Moves the held files under the request that carries them.
    pub(crate) fn file_under(&self, root: &Path, id: &str) -> Result<(), String> {
        std::fs::rename(held_dir(root, &self.key), held_dir(root, id))
            .map_err(|error| format!("The files could not be kept for the message: {error}"))
    }

    /// Lets go of files a message that was never queued would have carried.
    pub(crate) fn discard(&self, root: &Path) {
        let _ = std::fs::remove_dir_all(held_dir(root, &self.key));
    }
}

fn held_dir(root: &Path, id: &str) -> PathBuf {
    root.join(HELD).join(id)
}

/// Where a transport stages what an inbound source delivered, on the desk:
/// the one place a source's files are taken from.
pub(super) fn inbound_dir(root: &Path, source_id: &str) -> PathBuf {
    root.join("inbound").join(source_id)
}

/// The folder under `inbox/` a sender's files land in: its id, or for a
/// teammate on a linked desk `personaId@deskId`.
fn sender_folder(from: &str) -> PathBuf {
    let folder = match crate::linked::split(from) {
        Some((desk, persona)) => format!("{persona}@{desk}"),
        None => from.to_string(),
    };
    Path::new(INBOX).join(folder)
}

/// A name a file may be written under: one plain component, which a linked
/// desk must keep to as well.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// What the message or the aside says of the files: where each now is in
/// the recipient's working directory, written with `/` on every platform, as
/// the recipient may be on another desk's.
pub(super) fn files_line(sender: &str, paths: &[PathBuf]) -> String {
    let paths: Vec<String> = paths
        .iter()
        .map(|path| {
            path.components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    format!(
        "Files from {sender}, now in your working directory: {}",
        paths.join(", ")
    )
}

/// Files a linked desk sent, decoded and checked: as many, as large and as
/// plainly named as this desk would send.
pub(super) fn from_link(files: &[LinkedFile]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if files.len() > FILES_COUNT_MAX {
        return Err(format!(
            "At most {FILES_COUNT_MAX} files go with one message."
        ));
    }
    let mut taken: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for file in files {
        if !plain_name(&file.name) || taken.iter().any(|(name, _)| *name == file.name) {
            return Err("Each file needs a plain name of its own.".into());
        }
        let bytes = STANDARD
            .decode(&file.data)
            .map_err(|_| format!("{} is not base64.", file.name))?;
        total += bytes.len() as u64;
        if total > LINKED_FILES_MAX {
            return Err(over_the_link());
        }
        taken.push((file.name.clone(), bytes));
    }
    Ok(taken)
}

/// Files as they cross a link.
pub(super) fn to_link(files: Vec<(String, Vec<u8>)>) -> Vec<LinkedFile> {
    files
        .into_iter()
        .map(|(name, bytes)| LinkedFile {
            name,
            data: STANDARD.encode(bytes),
        })
        .collect()
}

fn over_the_link() -> String {
    format!(
        "Files to a teammate on a linked desk come to at most {} together, and these come to more. Send fewer or smaller files, or put them where both desks can reach.",
        crate::sent::size_text(LINKED_FILES_MAX)
    )
}

impl Room {
    /// Reads the files `from` asks to send `to`, at `paths`, as `send_file`
    /// reads one, and answers each by name with its bytes. Refused, with
    /// why: a path its read tools cannot reach, more than
    /// [`FILES_COUNT_MAX`] files, two of one name, and files that come to
    /// more than [`FILES_MAX`], or [`LINKED_FILES_MAX`] to a teammate on a
    /// linked desk.
    pub(crate) async fn take_files(
        &self,
        from: &str,
        to: &Persona,
        paths: &[String],
        capability: Option<CapabilityLease>,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        if paths.len() > FILES_COUNT_MAX {
            return Err(format!(
                "At most {FILES_COUNT_MAX} files go with one message or aside."
            ));
        }
        let persona = self.persona(from)?;
        let linked = to.node.is_some();
        let mut taken: Vec<(String, Vec<u8>)> = Vec::new();
        let mut total = 0u64;
        for path in paths {
            let file = super::files::from_workspace(
                &persona,
                self.log.root(),
                path,
                capability.clone(),
                false,
            )
            .await?;
            if taken.iter().any(|(name, _)| *name == file.name) {
                return Err(format!(
                    "Two of the files are called {}: send one of them on its own.",
                    file.name
                ));
            }
            total += file.bytes.len() as u64;
            if linked && total > LINKED_FILES_MAX {
                return Err(over_the_link());
            }
            if total > FILES_MAX {
                return Err(format!(
                    "Files to a teammate come to at most {} together, and these come to more.",
                    crate::sent::size_text(FILES_MAX)
                ));
            }
            taken.push((file.name, file.bytes));
        }
        Ok(taken)
    }

    /// Reads the files a message carries and holds them on the desk until it
    /// is queued. None when it carries none.
    pub(crate) async fn hold_files(
        &self,
        from: &str,
        to: &str,
        paths: &[String],
        capability: Option<CapabilityLease>,
    ) -> Result<Option<Held>, String> {
        if paths.is_empty() {
            return Ok(None);
        }
        let to = self.teammate_named(to)?;
        let files = self.take_files(from, &to, paths, capability).await?;
        let key = format!("held-{}", super::new_id());
        let names = self.hold(&key, &files)?;
        Ok(Some(Held { key, names }))
    }

    /// Holds files under `id`, and answers their names.
    pub(super) fn hold(
        &self,
        id: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<String>, String> {
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let dir = held_dir(self.log.root(), id);
        let held = (|| {
            std::fs::create_dir_all(&dir)?;
            for (name, bytes) in files {
                std::fs::write(dir.join(name), bytes)?;
            }
            Ok::<_, std::io::Error>(())
        })();
        if let Err(error) = held {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("The desk could not hold the files: {error}"));
        }
        Ok(files.iter().map(|(name, _)| name.clone()).collect())
    }

    /// The files held under `id`, by name.
    pub(super) fn held(
        &self,
        id: &str,
        names: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        let dir = held_dir(self.log.root(), id);
        names
            .iter()
            .map(|name| {
                std::fs::read(dir.join(name))
                    .map(|bytes| (name.clone(), bytes))
                    .map_err(|error| format!("{name} is no longer held on the desk: {error}"))
            })
            .collect()
    }

    /// Lets go of the files held under `id`.
    pub(super) fn drop_held(&self, id: &str) {
        let _ = std::fs::remove_dir_all(held_dir(self.log.root(), id));
    }

    /// Lets go of every held file no queued or running request still
    /// carries: a message stopped before it was let in, or a send cut off
    /// between reading its files and queueing it. Run as the desk starts.
    pub(super) fn sweep_held(&self, carried: &[String]) {
        let Ok(entries) = std::fs::read_dir(self.log.root().join(HELD)) else {
            return;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if !carried.contains(&id) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Reads the files a transport staged for an inbound source, at
    /// `paths`, checked as a colleague's are: each a plain file of a plain
    /// name directly in the source's own staging folder on the desk
    /// ([`inbound_dir`]), never a link, at most [`FILES_COUNT_MAX`] of them
    /// coming to at most [`FILES_MAX`] together.
    pub(super) fn take_staged(
        &self,
        source_id: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        if paths.len() > FILES_COUNT_MAX {
            return Err(format!(
                "At most {FILES_COUNT_MAX} files go with one aside."
            ));
        }
        let staged = inbound_dir(self.log.root(), source_id)
            .canonicalize()
            .map_err(|_| "The source has no files staged on this desk.".to_string())?;
        let mut taken: Vec<(String, Vec<u8>)> = Vec::new();
        let mut total = 0u64;
        for path in paths {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let outside = || {
                format!(
                    "{} is not a file the source staged on this desk.",
                    path.display()
                )
            };
            let metadata = std::fs::symlink_metadata(path).map_err(|_| outside())?;
            let inside = path
                .canonicalize()
                .is_ok_and(|real| real.parent() == Some(staged.as_path()));
            if !metadata.is_file() || !inside || !plain_name(&name) {
                return Err(outside());
            }
            if taken.iter().any(|(kept, _)| *kept == name) {
                return Err(format!("Two of the files are called {name}."));
            }
            total += metadata.len();
            if total > FILES_MAX {
                return Err(format!(
                    "Files with one aside come to at most {} together, and these come to more.",
                    crate::sent::size_text(FILES_MAX)
                ));
            }
            let bytes = std::fs::read(path).map_err(|_| outside())?;
            taken.push((name, bytes));
        }
        Ok(taken)
    }

    /// Copies files from `from` into `to`'s working directory, under
    /// `inbox/<sender>/`, under `to`'s own lease, and answers where each
    /// landed, relative to that directory. A teammate of any reach gets them
    /// in its working directory, and nowhere else.
    pub(super) fn deliver_files(
        &self,
        from: &str,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        self.deliver_into(&sender_folder(from), to, files)
    }

    /// Copies an inbound source's files into `to`'s working directory, under
    /// `inbox/inbound-<source id>/`, as a colleague's are.
    pub(super) fn deliver_inbound(
        &self,
        source_id: &str,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        let folder = Path::new(INBOX).join(format!("inbound-{source_id}"));
        self.deliver_into(&folder, to, files)
    }

    fn deliver_into(
        &self,
        folder: &Path,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        let target = self.persona(to)?;
        let workspace = Workspace::open_with_capability(
            PathBuf::from(&target.cwd),
            Reach::Workspace,
            self.log.root().join("tool-output").join(&target.id),
            &[],
            Some(self.capability_lease(&target.id)),
        )
        .map_err(|error| error.to_string())?;
        files
            .iter()
            .map(|(name, bytes)| {
                workspace
                    .receive_file(folder, name, bytes)
                    .map_err(|error| {
                        format!(
                            "{name} could not be put in {}'s working directory: {error}",
                            target.name
                        )
                    })
            })
            .collect()
    }

    /// Copies the files held for request `id` to its recipient and lets go
    /// of them, and answers the line its message gains.
    pub(super) fn deliver_held(
        &self,
        id: &str,
        from: &str,
        to: &str,
        names: &[String],
    ) -> Result<String, String> {
        let files = self.held(id, names)?;
        let paths = self.deliver_files(from, to, &files)?;
        self.drop_held(id);
        let sender = self.participant(from).map_or_else(
            |_| from.to_string(),
            |persona| super::links::shown(&persona),
        );
        Ok(files_line(&sender, &paths))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linked_desks_files_are_checked_as_this_desk_sends_them() {
        let file = |name: &str, bytes: &[u8]| LinkedFile {
            name: name.into(),
            data: STANDARD.encode(bytes),
        };
        let taken = from_link(&[file("q3.csv", b"a,b")]).unwrap();
        assert_eq!(taken, [("q3.csv".to_string(), b"a,b".to_vec())]);
        for bad in ["../q3.csv", "a/b", "..", "", "a\\b", "a\nb"] {
            assert!(from_link(&[file(bad, b"x")]).is_err(), "{bad:?}");
        }
        assert!(from_link(&[file("a", b"x"), file("a", b"y")]).is_err());
        let half = vec![0u8; LINKED_FILES_MAX as usize / 2 + 1];
        assert_eq!(
            from_link(&[file("a", &half), file("b", &half)]).unwrap_err(),
            over_the_link()
        );
        let many: Vec<LinkedFile> = (0..=FILES_COUNT_MAX)
            .map(|n| file(&format!("{n}"), b"x"))
            .collect();
        assert!(from_link(&many).is_err());
        assert!(
            from_link(&[LinkedFile {
                name: "a".into(),
                data: "not base64!".into()
            }])
            .is_err()
        );
    }

    #[test]
    fn a_senders_files_land_in_a_folder_of_its_own_under_inbox() {
        assert_eq!(sender_folder("pork-chop"), Path::new("inbox/pork-chop"));
        assert_eq!(
            sender_folder("laptop-desk/clementine"),
            Path::new("inbox/clementine@laptop-desk")
        );
    }
}
