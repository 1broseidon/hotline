//! Which directory a teammate's workspace is (H-64).
//!
//! Every holder of the workspace opens `persona.cwd` again by its path: the
//! file tools and the confined shell on every turn, `send_file`, generated
//! images, the `AGENTS.md` and skills writers and readers, ACP callbacks and
//! the computer's mount. A confined teammate could once remove its own
//! emptied workspace and make a link in its place (macOS), or rename a folder
//! above its workspace inside a writable folder and put another in its place
//! (Linux), and the next open went wherever that led. So the record keeps
//! which directory the person chose (`cwdIdentity`, set when `cwd` is set),
//! and under workspace reach each holder compares the directory it actually
//! opened, read from its own handle, with that, refusing another before it
//! acts.
//!
//! A bound workspace (one with an identity) that is missing, unreadable or
//! another directory stops the start before anything is made or written,
//! and the person chooses it again. A record without one (older, or a new
//! teammate whose folder is not made yet) is bound at its first start, under
//! the policy lock and written to the record before the session goes on,
//! only when no confined teammate could have replaced it or any folder
//! leading to it: no folder holding a step of its path, as spelled or as
//! resolved, is or lies inside a folder some workspace-reach teammate may
//! write (its workspace or a writable grant), compared by directory
//! identity, not by spelling. Otherwise the person chooses it again. Only
//! then is a missing folder made.

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
/// runtime binds the path itself, the desk's skill listing gates on it): a
/// link at the root, a workspace not bound yet, or one that is missing,
/// unreadable or another directory than the recorded one.
pub(crate) fn moved_workspace(persona: &Persona, cwd: &str) -> Option<String> {
    if persona.reach.unwrap_or_default() != Reach::Workspace {
        return None;
    }
    let path = Path::new(cwd);
    if let Some(why) = crate::tools::linked_root(path) {
        return Some(why);
    }
    let Some(expected) = persona.cwd_identity.as_deref() else {
        return Some(format!(
            "{}'s working directory {} is confirmed when the teammate next starts; start it first.",
            persona.name, cwd
        ));
    };
    match crate::tools::path_identity(path) {
        Some(found) if found == expected => None,
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
            || grant.identity.as_deref().is_some_and(|expected| {
                crate::tools::path_identity(path).as_deref() != Some(expected)
            });
        moved.then(|| grant_again(path, &persona.name))
    })
}

/// Why a computer may not be made with `persona`'s workspace and folders: a
/// confined teammate could replace one of them, or a folder leading to it
/// ([`replaceable`]). A container runtime binds each by its path, after
/// Hotline's check, so it could be handed another directory; Hotline's own
/// tools and shell compare the directory they opened and need no such rule.
pub(crate) fn overlapping_roots(roster: &[Persona], persona: &Persona) -> Option<String> {
    if persona.reach.unwrap_or_default() != Reach::Workspace {
        return None;
    }
    let areas = confined_areas(roster);
    std::iter::once(persona.cwd.as_str())
        .chain(persona.folders.iter().flatten().map(|grant| grant.path.as_str()))
        .map(Path::new)
        .find(|root| replaceable(&areas, root))
        .map(|root| {
            format!(
                "{}'s computer is not started: {} is inside a folder a teammate can change, and a computer opens its folders by path. Move the working directory or folder out of it, or take away write access to it.",
                persona.name,
                root.display()
            )
        })
}

/// The identity of each folder a workspace-reach teammate may write at its
/// root: its workspace and its writable grants. One that is gone holds
/// nothing and is left out.
fn confined_areas(roster: &[Persona]) -> Vec<String> {
    roster
        .iter()
        .filter(|persona| persona.reach.unwrap_or_default() == Reach::Workspace)
        .flat_map(|persona| {
            std::iter::once(persona.cwd.as_str()).chain(
                persona
                    .folders
                    .iter()
                    .flatten()
                    .filter(|grant| grant.writable)
                    .map(|grant| grant.path.as_str()),
            )
        })
        .filter_map(|path| crate::tools::path_identity(Path::new(path)))
        .collect()
}

/// Whether a confined teammate could replace `path` or a folder on the way to
/// it: a directory holding one of its steps, as spelled (through any link) or
/// as resolved, is one of `areas` or lies inside one. Compared by directory
/// identity, so another spelling of a folder (a case-insensitive volume, a
/// link) is the same folder; a directory whose identity cannot be read counts
/// as replaceable. Steps not made yet hold nothing.
fn replaceable(areas: &[String], path: &Path) -> bool {
    let spelled = path.ancestors().skip(1).map(Path::to_path_buf);
    let resolved = dunce::canonicalize(path).ok().into_iter().flat_map(|path| {
        path.ancestors()
            .skip(1)
            .map(Path::to_path_buf)
            .collect::<Vec<_>>()
    });
    let mut holders = std::collections::BTreeSet::new();
    for holder in spelled.chain(resolved) {
        if let Ok(holder) = dunce::canonicalize(&holder) {
            holders.extend(holder.ancestors().map(Path::to_path_buf));
        }
    }
    holders
        .iter()
        .any(|holder| match crate::tools::path_identity(holder) {
            Some(identity) => areas.contains(&identity),
            None => true,
        })
}

/// What a session start found for a teammate's workspace and granted
/// folders: the identities to record where the record has none.
#[derive(Default)]
pub(super) struct Binding {
    root: Option<String>,
    folders: Vec<(String, String)>,
}

/// What a session start does with `subject` (the latest record, with the
/// reach and folders the session will run with), before anything is made
/// or written: refuses a root that is a link, a bound root that is missing,
/// unreadable or another directory, and a granted folder that is now a link
/// or another directory; binds an unbound root or grant only when no confined
/// teammate could have replaced it ([`replaceable`]), making a missing root
/// only then.
pub(super) fn bind_root(roster: &[Persona], subject: &Persona) -> Result<Binding, String> {
    let path = Path::new(&subject.cwd);
    if subject.reach.unwrap_or_default() != Reach::Workspace {
        std::fs::create_dir_all(path).map_err(|error| could_not_make(subject, &error))?;
        return Ok(Binding::default());
    }
    if let Some(why) = crate::tools::linked_root(path) {
        return Err(why);
    }
    if let Some(why) = moved_folder(subject) {
        return Err(why);
    }
    let areas = confined_areas(roster);
    let mut binding = Binding::default();
    if let Some(expected) = subject.cwd_identity.as_deref() {
        // Bound: used as it is or not at all, never made again.
        let found = std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_dir)
            .and_then(|_| crate::tools::path_identity(path));
        if found.as_deref() != Some(expected) {
            return Err(crate::tools::not_the_chosen_folder(path));
        }
    } else {
        if replaceable(&areas, path) {
            return Err(reselect(path));
        }
        match std::fs::symlink_metadata(path) {
            Ok(found) if found.is_dir() => {}
            Ok(_) => return Err(reselect(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(path).map_err(|error| could_not_make(subject, &error))?;
            }
            Err(_) => return Err(reselect(path)),
        }
        binding.root = Some(crate::tools::path_identity(path).ok_or_else(|| reselect(path))?);
    }
    for grant in subject.folders.iter().flatten() {
        let path = Path::new(&grant.path);
        if grant.identity.is_some() || std::fs::symlink_metadata(path).is_err() {
            continue;
        }
        if replaceable(&areas, path) {
            return Err(grant_again(path, &subject.name));
        }
        let identity =
            crate::tools::path_identity(path).ok_or_else(|| grant_again(path, &subject.name))?;
        binding.folders.push((grant.path.clone(), identity));
    }
    Ok(binding)
}

fn could_not_make(persona: &Persona, error: &std::io::Error) -> String {
    format!(
        "{}'s working directory {} could not be made: {error}",
        persona.name, persona.cwd
    )
}

fn reselect(cwd: &Path) -> String {
    format!(
        "Hotline cannot confirm that the working directory {} is the folder that was chosen, since a teammate could have replaced it or a folder above it, so it will not open it. Choose the folder again in the teammate's settings.",
        cwd.display()
    )
}

fn grant_again(path: &Path, name: &str) -> String {
    format!(
        "The granted folder {} is no longer, or cannot be confirmed as, the folder that was granted to {name}, so Hotline will not open it. Grant the folder again, or take it away, in the teammate's settings.",
        path.display()
    )
}

impl super::Room {
    /// [`overlapping_roots`] against the desk's roster, before a computer
    /// is made or started with `persona`'s folders.
    pub(crate) fn computer_overlap(&self, persona: &Persona) -> Result<(), String> {
        overlapping_roots(&crate::room::roster(&self.log), persona).map_or(Ok(()), Err)
    }

    /// Binds a starting session's workspace and granted folders to the
    /// directories its record names ([`bind_root`]), under the policy lock,
    /// so no grant change is half-seen: the latest record is read under it,
    /// any identity it lacked and may adopt is written to it before the lock
    /// is let go, and only then does `view` (the session's own copy) take the
    /// record's identities. A record whose `cwd` changed since the session was
    /// opened stops the start; a newer choice at the same path, read here,
    /// wins over anything this start found. The one lock order is
    /// `start_gate -> policy_update_lock`, and no caller of a thread's agent
    /// holds the policy lock.
    pub(super) async fn bind_workspace(&self, view: &mut Persona) -> Result<(), String> {
        let gate = self.policy_update_lock();
        let _held = gate.lock().await;
        let roster = crate::room::roster(&self.log);
        let record = roster
            .iter()
            .find(|record| record.id == view.id)
            .cloned()
            .ok_or_else(|| format!("{} is no longer on this desk.", view.name))?;
        if record.cwd != view.cwd {
            return Err(format!(
                "{}'s working directory changed while it was starting; start it again.",
                view.name
            ));
        }
        let recorded = |path: &str| {
            record
                .folders
                .iter()
                .flatten()
                .find(|grant| grant.path == path)
                .map(|grant| grant.identity.clone())
        };
        // Bound are the grants on the record that this session runs with;
        // Hotline's own received folder is never on the record and opens as
        // before.
        let mut subject = record.clone();
        subject.reach = view.reach;
        subject.folders = view.folders.as_ref().map(|grants| {
            grants
                .iter()
                .filter_map(|grant| {
                    recorded(&grant.path).map(|identity| crate::contract::FolderGrant {
                        identity,
                        ..grant.clone()
                    })
                })
                .collect()
        });
        let binding = bind_root(&roster, &subject)?;
        let mut updated = record.clone();
        if binding.root.is_some() {
            updated.cwd_identity.clone_from(&binding.root);
        }
        let adopt = |grants: &mut Option<Vec<crate::contract::FolderGrant>>| {
            for grant in grants.iter_mut().flatten() {
                if let Some((_, identity)) =
                    binding.folders.iter().find(|(path, _)| *path == grant.path)
                    && grant.identity.is_none()
                {
                    grant.identity = Some(identity.clone());
                }
            }
        };
        adopt(&mut updated.folders);
        if updated != record {
            crate::room::append_persona(&self.log, &updated).map_err(|error| {
                format!("{}'s workspace could not be recorded: {error}", view.name)
            })?;
        }
        view.cwd_identity.clone_from(&updated.cwd_identity);
        for grant in view.folders.iter_mut().flatten() {
            if let Some(identity) = updated
                .folders
                .iter()
                .flatten()
                .find(|recorded| recorded.path == grant.path)
                .map(|recorded| recorded.identity.clone())
            {
                grant.identity = identity;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn confined(id: &str, cwd: &Path) -> Persona {
        let mut persona = crate::session::tests::persona(id);
        persona.reach = None;
        persona.cwd = cwd.to_string_lossy().into_owned();
        persona
    }

    fn grant(path: &Path, writable: bool) -> crate::contract::FolderGrant {
        crate::contract::FolderGrant {
            path: path.to_string_lossy().into_owned(),
            writable,
            identity: crate::tools::path_identity(path),
        }
    }

    /// The computer's runtime binds the workspace by its path, so its start
    /// reads that path's directory now: the recorded one starts; one not
    /// bound yet, gone, another directory, or a link does not; whole-machine
    /// reach is not asked.
    #[test]
    fn a_computer_is_not_started_on_a_changed_workspace() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let mut ada = confined("ada", &work);
        assert!(moved_workspace(&ada, &ada.cwd).is_some_and(|why| why.contains("start it first")));
        ada.cwd_identity = crate::tools::path_identity(&work);
        assert_eq!(moved_workspace(&ada, &ada.cwd), None);
        std::fs::rename(&work, root.path().join("work-was")).unwrap();
        assert!(moved_workspace(&ada, &ada.cwd).is_some(), "gone");
        std::fs::create_dir(&work).unwrap();
        assert!(moved_workspace(&ada, &ada.cwd).is_some_and(|why| why.contains("no longer")));
        std::fs::remove_dir(&work).unwrap();
        std::os::unix::fs::symlink(root.path().join("work-was"), &work).unwrap();
        assert!(moved_workspace(&ada, &ada.cwd).is_some_and(|why| why.contains("link")));
        ada.reach = Some(Reach::Machine);
        assert_eq!(moved_workspace(&ada, &ada.cwd), None);
    }

    /// A computer opens its folders by path, so one is not made while a
    /// confined teammate could replace its workspace or a granted folder or
    /// a folder leading to one: the teammate's own writable folder holding its
    /// workspace, another teammate's workspace or writable folder holding
    /// one, including through another spelling of that folder (a
    /// case-insensitive volume), compared by directory identity. A read-only
    /// grant, two teammates sharing one workspace, or whole-machine reach are
    /// not refused.
    #[test]
    fn a_computer_is_not_made_inside_a_folder_a_teammate_can_change() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let projects = base.join("projects");
        let app = projects.join("app");
        let elsewhere = base.join("elsewhere");
        let lib = elsewhere.join("lib");
        for directory in [&app, &lib] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let mut ada = confined("ada", &app);
        let mut bob = confined("bob", &base.join("bob"));
        std::fs::create_dir_all(base.join("bob")).unwrap();
        let roster = |ada: &Persona, bob: &Persona| vec![ada.clone(), bob.clone()];
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);

        ada.folders = Some(vec![grant(&projects, true)]);
        assert!(
            overlapping_roots(&roster(&ada, &bob), &ada).is_some(),
            "her own writable folder"
        );
        ada.folders = Some(vec![grant(&projects, false)]);
        assert_eq!(
            overlapping_roots(&roster(&ada, &bob), &ada),
            None,
            "read-only"
        );

        ada.folders = None;
        bob.folders = Some(vec![grant(&projects, true)]);
        assert!(
            overlapping_roots(&roster(&ada, &bob), &ada).is_some(),
            "Bob's writable folder"
        );
        // The same folder spelled in other case, where the volume does not
        // tell case apart (APFS by default): the same directory.
        let shouted = base.join("PROJECTS");
        if crate::tools::path_identity(&shouted) == crate::tools::path_identity(&projects) {
            bob.folders = Some(vec![crate::contract::FolderGrant {
                path: shouted.to_string_lossy().into_owned(),
                writable: true,
                identity: None,
            }]);
            assert!(
                overlapping_roots(&roster(&ada, &bob), &ada).is_some(),
                "case-aliased"
            );
            let shouting = confined("ada", &base.join("PROJECTS/app"));
            bob.folders = Some(vec![grant(&projects, true)]);
            assert!(overlapping_roots(&roster(&shouting, &bob), &shouting).is_some());
        }
        bob.folders = None;

        bob.cwd = elsewhere.to_string_lossy().into_owned();
        ada.folders = Some(vec![grant(&lib, false)]);
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
        ada.folders = Some(vec![grant(&projects, true)]);
        assert_eq!(overlapping_roots(&roster(&ada, &bob), &ada), None);
    }
}
