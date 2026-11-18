//! Which directory a teammate's workspace is (H-64).
//!
//! Every holder of the workspace opens `persona.cwd` again by its path: the
//! file tools and the confined shell on every turn, `send_file`, generated
//! images, the `AGENTS.md` and skills writers, ACP callbacks and the
//! computer's mount. A confined teammate could once remove its own emptied
//! workspace and make a link in its place (macOS), or rename a folder above
//! its workspace inside a writable folder and put another in its place
//! (Linux), and the next open went wherever that led. So the record keeps
//! which directory the person chose (`cwdIdentity`, set when `cwd` is set),
//! and under workspace reach each holder compares the directory it actually
//! opened, read from its own handle, with that, refusing another before it
//! acts.
//!
//! A record from before the field has none. Its teammate's first start
//! adopts the directory it finds only if nothing a confined teammate could
//! have planted lies in its path: the folder itself is not a link, and no
//! folder above it that is a link sits inside a folder some confined
//! teammate may change (its workspace or a writable grant). Otherwise the
//! start is refused and the person is asked to choose the folder again. A
//! folder that is missing is made again where it was and adopted, by the
//! same rule: nothing about it can then lead elsewhere.

use crate::contract::{Persona, Reach};
use std::path::{Path, PathBuf};

/// The identity holders of `persona`'s workspace compare the root they open
/// with: the record's, under workspace reach. With whole-machine reach there
/// is no wall for the root to move.
pub(crate) fn bound_identity(persona: &Persona) -> Option<&str> {
    if persona.reach.unwrap_or_default() == Reach::Workspace {
        persona.cwd_identity.as_deref()
    } else {
        None
    }
}

/// How a holder that always confines itself to the workspace (Hotline's
/// ACP callbacks and `AGENTS.md`, a generated image's output) opens it for
/// `persona`: as the wall, refusing a root that is a link, unless the person
/// gave the teammate the whole machine, whose root may keep following a link
/// it names. Either way a recorded identity is checked on the opened handle.
pub(crate) type OpenRoot = fn(
    PathBuf,
    Reach,
    PathBuf,
    &[crate::contract::FolderGrant],
    Option<&str>,
    Option<crate::tools::capability::CapabilityLease>,
) -> Result<crate::tools::Workspace, crate::tools::ToolError>;

pub(crate) fn root_opener(persona: &Persona) -> OpenRoot {
    if persona.reach.unwrap_or_default() == Reach::Machine {
        crate::tools::Workspace::open_following_root
    } else {
        crate::tools::Workspace::open_with_capability
    }
}

/// Why `persona`'s workspace at `cwd` is not to be used now, if it is not,
/// for a holder that has no handle of its own to compare (the computer's
/// runtime binds the path itself): a link at the root, or a directory there
/// that is not the recorded one.
pub(crate) fn moved_workspace(persona: &Persona, cwd: &str) -> Option<String> {
    if persona.reach.unwrap_or_default() != Reach::Workspace {
        return None;
    }
    let path = Path::new(cwd);
    if let Some(why) = crate::tools::linked_root(path) {
        return Some(why);
    }
    let expected = persona.cwd_identity.as_deref()?;
    match crate::tools::path_identity(path) {
        Some(found) if found == expected => None,
        // Not made yet: the session start makes it, by its own rule.
        None if !path.exists() => None,
        _ => Some(crate::tools::not_the_chosen_folder(path)),
    }
}

/// Why `persona`'s granted folders are not to be used now, if one is not:
/// one that resolves somewhere other than its own path (a link at it or
/// above it), or a directory there that is not the one granted. The person
/// is asked to grant it again rather than have its new target adopted. A
/// folder that is gone is left out as before; Hotline's received folder is
/// never in the record and is not asked about.
pub(crate) fn moved_folder(persona: &Persona) -> Option<String> {
    if persona.reach.unwrap_or_default() != Reach::Workspace {
        return None;
    }
    persona.folders.iter().flatten().find_map(|grant| {
        let path = Path::new(&grant.path);
        std::fs::symlink_metadata(path).ok()?;
        let resolved = dunce::canonicalize(path).ok();
        let moved = resolved.as_deref() != Some(path)
            || grant
                .identity
                .as_deref()
                .is_some_and(|expected| crate::tools::path_identity(path).as_deref() != Some(expected));
        moved.then(|| {
            format!(
                "The granted folder {} is no longer the folder that was granted to {}, so Hotline will not open it. Grant the folder again, or take it away, in the teammate's settings.",
                path.display(),
                persona.name
            )
        })
    })
}

/// Why a computer may not be made with `persona`'s workspace and folders: one
/// of them sits inside a folder a confined teammate may change (its own
/// writable folder holding its workspace, or another teammate's workspace or
/// writable folder). A container runtime binds each by its path, after
/// Hotline's check, so whoever can rename a folder above it could have the
/// runtime bind another directory; Hotline's own tools and shell compare the
/// directory they opened and need no such rule.
pub(crate) fn overlapping_roots(roster: &[Persona], persona: &Persona) -> Option<String> {
    if persona.reach.unwrap_or_default() != Reach::Workspace {
        return None;
    }
    let confined = |other: &&Persona| other.reach.unwrap_or_default() == Reach::Workspace;
    let resolved = |path: &str| dunce::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let roots: Vec<PathBuf> = std::iter::once(resolved(&persona.cwd))
        .chain(
            persona
                .folders
                .iter()
                .flatten()
                .map(|grant| PathBuf::from(&grant.path)),
        )
        .collect();
    let areas: Vec<PathBuf> = roster
        .iter()
        .filter(confined)
        .flat_map(|other| {
            let workspace = (other.id != persona.id).then(|| resolved(&other.cwd));
            let writable = other
                .folders
                .iter()
                .flatten()
                .filter(|grant| grant.writable)
                .map(|grant| PathBuf::from(&grant.path));
            workspace.into_iter().chain(writable)
        })
        .collect();
    roots.iter().find_map(|root| {
        let area = areas
            .iter()
            .find(|area| *area != root && root.starts_with(area))?;
        Some(format!(
            "{}'s computer is not started: {} is inside {}, a folder a teammate can change, and a computer opens its folders by path. Move the working directory or folder out of it, or take away write access to it.",
            persona.name,
            root.display(),
            area.display()
        ))
    })
}

/// What a session start does with `persona`'s workspace, before anything is
/// written there: makes it if missing, then refuses it if it is a link or
/// not the recorded directory. `Ok(Some(identity))` is a directory to adopt
/// as the record's: an older record without one, or a missing folder made
/// again, either only when nothing in its path could have been planted.
pub(super) fn bind_root(roster: &[Persona], persona: &Persona) -> Result<Option<String>, String> {
    let path = Path::new(&persona.cwd);
    let confined = persona.reach.unwrap_or_default() == Reach::Workspace;
    if confined && let Some(why) = crate::tools::linked_root(path) {
        return Err(why);
    }
    if let Some(why) = moved_folder(persona) {
        return Err(why);
    }
    let missing = std::fs::symlink_metadata(path).is_err();
    if confined && missing && planted_link(roster, path) {
        return Err(reselect(path));
    }
    std::fs::create_dir_all(path).map_err(|error| {
        format!(
            "{}'s working directory {} could not be made: {error}",
            persona.name, persona.cwd
        )
    })?;
    if !confined {
        return Ok(None);
    }
    let Some(found) = crate::tools::path_identity(path) else {
        // No identity on this platform: the link check is all there is.
        return Ok(None);
    };
    match persona.cwd_identity.as_deref() {
        Some(expected) if expected == found => Ok(None),
        Some(_) if missing => Ok(Some(found)),
        Some(_) => Err(crate::tools::not_the_chosen_folder(path)),
        None if planted_link(roster, path) => Err(reselect(path)),
        None => Ok(Some(found)),
    }
}

/// The identities to adopt for `persona`'s granted folders that have none:
/// each that still resolves to its own path (as a session opens it), by
/// grant path. An older grant that no longer does is left out by the session
/// anyway.
pub(super) fn unbound_folders(persona: &Persona) -> Vec<(String, String)> {
    persona
        .folders
        .iter()
        .flatten()
        .filter(|grant| grant.identity.is_none())
        .filter_map(|grant| {
            let path = Path::new(&grant.path);
            let canonical = dunce::canonicalize(path).ok()?;
            if canonical != path {
                return None;
            }
            Some((grant.path.clone(), crate::tools::path_identity(path)?))
        })
        .collect()
}

/// Whether a folder above `cwd` is a link inside a folder a confined
/// teammate may change: its workspace or one of its writable grants. Such a
/// link may have been planted to send `cwd` elsewhere. Links elsewhere in the
/// path (`/tmp`, a linked projects folder) are the person's own layout.
fn planted_link(roster: &[Persona], cwd: &Path) -> bool {
    let changeable: Vec<PathBuf> = roster
        .iter()
        .filter(|persona| persona.reach.unwrap_or_default() == Reach::Workspace)
        .flat_map(|persona| {
            let workspace = dunce::canonicalize(&persona.cwd).ok();
            let writable = persona
                .folders
                .iter()
                .flatten()
                .filter(|grant| grant.writable)
                .map(|grant| PathBuf::from(&grant.path));
            workspace.into_iter().chain(writable)
        })
        .collect();
    cwd.ancestors().any(|step| {
        let linked =
            std::fs::symlink_metadata(step).is_ok_and(|found| found.file_type().is_symlink());
        linked
            && step
                .parent()
                .and_then(|parent| dunce::canonicalize(parent).ok())
                .is_some_and(|parent| changeable.iter().any(|area| parent.starts_with(area)))
    })
}

fn reselect(cwd: &Path) -> String {
    format!(
        "The working directory {} passes through a link inside a folder a teammate can change, so Hotline cannot tell it is still the folder that was chosen and will not open it. Choose the folder again in the teammate's settings.",
        cwd.display()
    )
}

impl super::Room {
    /// [`overlapping_roots`] against the desk's roster, before a computer
    /// is made or started with `persona`'s folders.
    pub(crate) fn computer_overlap(&self, persona: &Persona) -> Result<(), String> {
        overlapping_roots(&crate::room::roster(&self.log), persona).map_or(Ok(()), Err)
    }

    /// Records which directories a starting teammate's workspace and granted
    /// folders are, where [`bind_root`] and [`unbound_folders`] found ones to
    /// adopt, in the session's view at once and on its record when the
    /// record still names the same paths. The record is written under the
    /// policy lock only if it is free: a start never waits on a policy change
    /// in flight, and an identity not written now is adopted at the next.
    pub(super) async fn adopt_roots(&self, persona: &mut Persona, root: Option<String>) {
        let folders = if persona.reach.unwrap_or_default() == Reach::Workspace {
            unbound_folders(persona)
        } else {
            Vec::new()
        };
        if root.is_none() && folders.is_empty() {
            return;
        }
        let adopt = |persona: &mut Persona| {
            if let Some(root) = &root {
                persona.cwd_identity = Some(root.clone());
            }
            for grant in persona.folders.iter_mut().flatten() {
                if let Some((_, identity)) = folders.iter().find(|(path, _)| *path == grant.path)
                    && grant.identity.is_none()
                {
                    grant.identity = Some(identity.clone());
                }
            }
        };
        adopt(persona);
        let lock = self.policy_update_lock();
        let Ok(_held) = lock.try_lock() else {
            return;
        };
        let Some(mut record) = crate::room::roster(&self.log)
            .into_iter()
            .find(|record| record.id == persona.id && record.cwd == persona.cwd)
        else {
            return;
        };
        let before = record.clone();
        adopt(&mut record);
        // The session's own received folder is in its view, never its record.
        if record == before {
            return;
        }
        if let Err(error) = crate::room::append_persona(&self.log, &record) {
            eprintln!("{}'s workspace could not be recorded: {error}", record.name);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The computer's runtime binds the workspace by its path, so its start
    /// reads that path's directory now: a link there, or another directory
    /// than the record's, keeps the computer from starting with it; the
    /// recorded one, one not made yet, or whole-machine reach does not.
    #[test]
    fn a_computer_is_not_started_on_a_changed_workspace() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let mut ada = crate::session::tests::persona("ada");
        ada.reach = None;
        ada.cwd = work.to_string_lossy().into_owned();
        ada.cwd_identity = crate::tools::path_identity(&work);
        assert_eq!(moved_workspace(&ada, &ada.cwd), None);
        std::fs::rename(&work, root.path().join("work-was")).unwrap();
        assert_eq!(moved_workspace(&ada, &ada.cwd), None, "not made yet");
        std::fs::create_dir(&work).unwrap();
        assert!(moved_workspace(&ada, &ada.cwd).is_some_and(|why| why.contains("no longer")));
        std::fs::remove_dir(&work).unwrap();
        std::os::unix::fs::symlink(root.path().join("work-was"), &work).unwrap();
        assert!(moved_workspace(&ada, &ada.cwd).is_some_and(|why| why.contains("link")));
        ada.reach = Some(Reach::Machine);
        assert_eq!(moved_workspace(&ada, &ada.cwd), None);
    }

    /// A computer opens its folders by path, so one is not made while the
    /// workspace or a granted folder sits inside a folder a confined
    /// teammate may change: the teammate's own writable folder holding its
    /// workspace, or another teammate's workspace or writable folder. Two
    /// teammates sharing one workspace, or whole-machine reach, are not.
    #[test]
    fn a_computer_is_not_made_inside_a_folder_a_teammate_can_change() {
        let grant = |path: &str, writable| crate::contract::FolderGrant {
            path: path.to_string(),
            writable,
            identity: None,
        };
        let mut ada = crate::session::tests::persona("ada");
        ada.reach = None;
        ada.cwd = "/nowhere/projects/app".to_string();
        let mut bob = crate::session::tests::persona("bob");
        bob.reach = None;
        bob.cwd = "/nowhere/elsewhere".to_string();
        let roster = |ada: &Persona, bob: &Persona| vec![ada.clone(), bob.clone()];
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);

        ada.folders = Some(vec![grant("/nowhere/projects", true)]);
        assert!(overlapping_roots(&roster(&ada, &bob), &ada).is_some());
        ada.folders = Some(vec![grant("/nowhere/projects", false)]);
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);

        bob.folders = Some(vec![grant("/nowhere/projects", true)]);
        assert!(overlapping_roots(&roster(&ada, &bob), &ada).is_some());
        bob.folders = None;
        ada.folders = Some(vec![grant("/nowhere/elsewhere/lib", false)]);
        assert!(
            overlapping_roots(&roster(&ada, &bob), &ada).is_some(),
            "inside Bob's workspace"
        );
        bob.reach = Some(Reach::Machine);
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);

        bob.reach = None;
        bob.cwd = ada.cwd.clone();
        ada.folders = None;
        assert_eq!(
            overlapping_roots(&roster(&ada, &bob), &ada),
            None,
            "a shared workspace"
        );
        ada.reach = Some(Reach::Machine);
        ada.folders = Some(vec![grant("/nowhere/projects", true)]);
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);
    }
}
