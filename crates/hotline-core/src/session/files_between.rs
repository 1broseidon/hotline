//! Files between teammates (`docs/design/thread-awareness.md` §3).
//!
//! An inbox item may carry files. The sender's are read where its read
//! tools find them, by the check `send_file` makes
//! ([`super::files::from_workspace`]): its working directory and the folders
//! granted it, or anywhere under whole-machine reach, and from a job's
//! thread only within what the job was narrowed to
//! ([`Room::reaching_from`]). They are read when the
//! teammate sends them, and land as the item does: at once when the two may
//! already reach each other, else held on the desk (`files-between/<key>/`)
//! until the person's first-contact card lets the item in. Then each is
//! copied, never over a file there, into a folder Hotline keeps on the desk
//! for the recipient, `received/<recipient>/<sender>/`, outside its working
//! directory, and the item says where. An inbound source's files arrive the
//! same way. Nothing new is approved: a file goes only where the item
//! carrying it may.
//!
//! The recipient reads them there and changes nothing: its sessions and read
//! tools are given `received/<recipient>/` as one more extra folder, read
//! only ([`Room::with_received`]), so a file it wants in its project it
//! copies there itself, with its own reach. A file a tool picks up by its
//! name (a test runner's config, an instructions file) is never put in a
//! project by a colleague.

use super::Room;
use crate::contract::{FolderGrant, Persona, Reach, ThreadId};
use crate::driver::CapabilityLease;
use crate::tools::Workspace;
use std::path::{Path, PathBuf};

/// The most a message's or an aside's files come to together on one desk:
/// what `send_file` sends the person.
pub(crate) const FILES_MAX: u64 = crate::sent::MAX_BYTES;

/// The most files one message or aside carries.
pub(crate) const FILES_COUNT_MAX: usize = 10;

/// Where the desk holds a message's files until it is let in.
const HELD: &str = "files-between";

/// Where on the desk each teammate's received files are kept, a folder for
/// each recipient.
const RECEIVED: &str = "received";

fn held_dir(root: &Path, id: &str) -> PathBuf {
    root.join(HELD).join(id)
}

/// Where a transport stages what an inbound source delivered, on the desk:
/// the one place a source's files are taken from.
pub(super) fn inbound_dir(root: &Path, source_id: &str) -> PathBuf {
    root.join("inbound").join(source_id)
}

/// The folder a teammate's received files are kept in on the desk, and the
/// folder it is granted read-only: `received/<its id>`, the id made one
/// plain name.
pub(crate) fn received_dir(root: &Path, persona_id: &str) -> PathBuf {
    root.join(RECEIVED)
        .join(crate::paths::encode_file_component(persona_id))
}

/// The folder in the recipient's received folder a sender's files land in:
/// its id, or for a teammate on a linked desk `personaId@deskId`, made one
/// plain name, so no id can name a folder above it.
fn sender_folder(from: &str) -> PathBuf {
    let folder = match crate::linked::split(from) {
        Some((desk, persona)) => format!("{persona}@{desk}"),
        None => from.to_string(),
    };
    PathBuf::from(crate::paths::encode_file_component(&folder))
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

/// What the message or the aside says of the files: where each now is, by
/// its whole path, and that the recipient reads them there and copies one
/// into its working directory to work on it.
pub(super) fn files_line(sender: &str, paths: &[PathBuf]) -> String {
    let paths: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    format!(
        "Files from {sender}, kept outside your working directory for you to read, not change; copy one into your working directory to work on it: {}",
        paths.join(", ")
    )
}

/// The grant a teammate's sessions are given for its received folder
/// ([`received_dir`], made if missing): read only, spelled as the directory
/// it is, as a folder the person grants is. No folder the person grants can
/// be inside the data directory, so this one is never theirs. None when the
/// desk cannot make it.
pub(crate) fn received_grant(root: &Path, persona_id: &str) -> Option<FolderGrant> {
    let dir = received_dir(root, persona_id);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dunce::canonicalize(&dir).ok()?;
    Some(FolderGrant {
        path: path.to_str()?.to_string(),
        writable: false,
    })
}

impl Room {
    /// Reads the files `from` asks to send from `thread`, at `paths`, as
    /// `send_file` reads one there, and answers each by name with its bytes:
    /// a job's thread reads only within what its job was narrowed to.
    /// Refused, with why: a path its read tools cannot reach, more than
    /// [`FILES_COUNT_MAX`] files, two of one name, and files that come to
    /// more than [`FILES_MAX`].
    pub(crate) async fn take_files(
        &self,
        from: &str,
        thread: &ThreadId,
        paths: &[String],
        capability: Option<CapabilityLease>,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        if paths.len() > FILES_COUNT_MAX {
            return Err(format!("At most {FILES_COUNT_MAX} files go with one item."));
        }
        let persona = self.reaching_from(from, thread)?;
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

    /// Copies files from `from` into `to`'s received folder, under its
    /// sender's own folder, under `to`'s own lease, and answers where each
    /// landed. A teammate of any reach gets them there, and nowhere else.
    pub(super) fn deliver_files(
        &self,
        from: &str,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        self.deliver_into(&sender_folder(from), to, files)
    }

    /// Copies an inbound source's files into `to`'s received folder, under
    /// `inbound-<source id>/`, as a colleague's are.
    pub(super) fn deliver_inbound(
        &self,
        source_id: &str,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        let folder = PathBuf::from(crate::paths::encode_file_component(&format!(
            "inbound-{source_id}"
        )));
        self.deliver_into(&folder, to, files)
    }

    /// Writes each file into `folder` of `to`'s received folder through a
    /// handle confined to that folder, so no name and no link in it reaches
    /// anywhere else, and answers each file's whole path.
    fn deliver_into(
        &self,
        folder: &Path,
        to: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<Vec<PathBuf>, String> {
        let target = self.persona(to)?;
        let could_not = |error: &dyn std::fmt::Display| {
            format!("The files could not be kept for {}: {error}", target.name)
        };
        let received = received_dir(self.log.root(), &target.id);
        std::fs::create_dir_all(&received).map_err(|error| could_not(&error))?;
        let kept = Workspace::open_with_capability(
            received,
            Reach::Workspace,
            self.log.root().join("tool-output").join(&target.id),
            &[],
            Some(self.capability_lease(&target.id)),
        )
        .map_err(|error| could_not(&error))?;
        files
            .iter()
            .map(|(name, bytes)| {
                kept.receive_file(folder, name, bytes)
                    .map(|landed| kept.display_root().join(landed))
                    .map_err(|error| {
                        format!("{name} could not be kept for {}: {error}", target.name)
                    })
            })
            .collect()
    }

    /// The teammate as its sessions and its read tools are given it: its
    /// record with its received folder as one more extra folder, read only
    /// ([`received_grant`]). The record is the person's and is not changed.
    /// A job or run made `without` folders is narrowed after this
    /// (`agent::clone_of`), and loses it with the rest.
    pub(super) fn with_received(&self, mut persona: Persona) -> Persona {
        if let Some(grant) = received_grant(self.log.root(), &persona.id) {
            persona.folders.get_or_insert_with(Vec::new).push(grant);
        }
        persona
    }

    /// `persona` without the grant [`Room::with_received`] added: what its
    /// computer is made with and its preamble lists, both of which hold the
    /// person's folders alone. A computer is the teammate's across its
    /// threads and is made again whenever its folders differ, so a folder
    /// some of its threads have and others lack would remake it between
    /// them; and each item names where its files are.
    pub(super) fn without_received(&self, mut persona: Persona) -> Persona {
        let dir = received_dir(self.log.root(), &persona.id);
        let received = dunce::canonicalize(&dir).unwrap_or(dir);
        if let Some(folders) = persona.folders.as_mut() {
            folders.retain(|folder| Path::new(&folder.path) != received);
            if folders.is_empty() {
                persona.folders = None;
            }
        }
        persona
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

    /// Each sender's files land in a folder of its own, one plain name
    /// whatever its id says, in a folder of the recipient's own.
    #[test]
    fn a_senders_files_land_in_a_folder_of_its_own_that_no_id_can_leave() {
        assert_eq!(sender_folder("pork-chop"), Path::new("pork-chop"));
        assert_eq!(
            sender_folder("laptop-desk/clementine"),
            Path::new("clementine@laptop-desk")
        );
        for id in ["..", ".", "../bob", "a/../../b", "..\\x"] {
            let folder = sender_folder(id);
            assert_eq!(folder.components().count(), 1, "{id}: {folder:?}");
            assert!(
                matches!(
                    folder.components().next(),
                    Some(std::path::Component::Normal(_))
                ),
                "{id}: {folder:?}"
            );
        }
        let root = Path::new("/data");
        assert_eq!(received_dir(root, "bob"), Path::new("/data/received/bob"));
        assert_eq!(received_dir(root, ".."), Path::new("/data/received/%2E%2E"));
    }
}
