//! Which teammate has more access than another, and the person's standing
//! consent to one asking another with more to work, with the one thing that
//! clears it: the recipient's access growing.
//!
//! A teammate's access is the level its settings give it: its own folders,
//! or the whole machine (Hotline Agent's Whole machine switch, `reach`). Only a request
//! up in access asks the person ([`more_access`]); between equals, or down,
//! the receiver can do nothing the sender could not, so nothing asks.
//!
//! "Always" on that card is two records. The caller's id in the recipient's
//! `allowedSenders`, which lets it ask, and a `collaboration_informed` room
//! event for the direction, which says the person was shown what a handoff
//! means and lets it hand work over. Both stand until the person takes them
//! away (Permissions › Collaboration, Revoke) or the recipient comes to reach
//! more than it did when they were given: then the work the person agreed
//! to could do more than they agreed to, so both go, and the next request up
//! in access asks again. A goal edited, a model or agent switched, a
//! restart, a chapter, a rebuild or a sign-out that narrows nothing leaves
//! them alone.

use crate::contract::{Persona, PolicyMode, Reach};
use crate::log::{Log, StreamId};
use serde_json::{Value, json};
use std::collections::HashSet;

const INFORMED: &str = "collaboration_informed";

fn informed_id(caller: &str, target: &str) -> String {
    format!("collaboration:{caller}:{target}")
}

/// The direction's informed-consent record as the room stream folds it: the
/// latest line under its id, so a tombstone counts the moment it is written.
fn record<'a>(room: &'a [Value], caller: &str, target: &str) -> Option<&'a Value> {
    let id = informed_id(caller, target);
    room.iter()
        .find(|event| event["kind"] == INFORMED && event["id"] == id.as_str())
}

/// Whether the person has been shown what a handoff from `caller` to
/// `target` means and allowed it for good. `room` is the room stream's fold.
pub(crate) fn informed(room: &[Value], caller: &str, target: &str) -> bool {
    record(room, caller, target).is_some_and(|event| event["deleted"] != true)
}

/// Records the person's informed, standing consent for one direction.
pub(crate) fn record_informed(log: &Log, caller: &str, target: &str) -> Result<(), String> {
    log.append(
        &StreamId::Room,
        &json!({"kind": INFORMED, "id": informed_id(caller, target), "from": caller, "to": target}),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Whether `target` has more access than `caller`, by the level each one's
/// settings give it: the whole machine is more than its own folders.
pub(crate) fn more_access(caller: &Persona, target: &Persona) -> bool {
    whole_machine(target) && !whole_machine(caller)
}

/// Whether a teammate has the whole machine: Hotline Agent with Whole
/// machine on. Another agent's callbacks stay in its workspace whatever its
/// record says (`driver/acp/session.rs`), as its settings show no such switch.
fn whole_machine(persona: &Persona) -> bool {
    persona.backend_id == crate::driver::HOTLINE_BACKEND_ID
        && persona.reach.unwrap_or_default() == Reach::Machine
}

/// A teammate's access in the words the person knows it by from its
/// settings, for the card that asks before one with more of it does work:
/// the whole machine (the Whole machine switch), else its own folders (its
/// Folders). A card up in access names the whole machine; only a caller on
/// a linked desk, always asked about, can meet the other.
pub(crate) fn access_words(target: &Persona) -> &'static str {
    if !whole_machine(target) {
        "its own folders"
    } else if cfg!(target_os = "macos") {
        "your whole Mac"
    } else {
        "the whole machine"
    }
}

/// How a teammate's access grew, kept on the record of the consent it
/// cleared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Widening {
    Machine,
    Workspace,
    Folder,
    McpServer,
    Computer,
    Secret,
}

impl Widening {
    fn code(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Workspace => "workspace",
            Self::Folder => "folder",
            Self::McpServer => "mcp",
            Self::Computer => "computer",
            Self::Secret => "secret",
        }
    }
}

/// The servers a policy grants out of those the desk has.
fn granted<'a>(persona: &'a Persona, servers: &'a [String]) -> HashSet<&'a str> {
    match persona.mcp_policy.mode {
        PolicyMode::All => servers.iter().map(String::as_str).collect(),
        PolicyMode::Some => persona
            .mcp_policy
            .server_ids
            .iter()
            .map(String::as_str)
            .collect(),
        PolicyMode::None => HashSet::new(),
    }
}

/// Whether `updated` reaches more than `previous` did, and how. `before` and
/// `after` are the desk's MCP servers around the change: the same list for a
/// teammate's own update, two for a server added in settings. Only what lets
/// the teammate touch more counts; a narrowing, its goal, its model or its
/// agent do not.
pub(crate) fn widened(
    previous: &Persona,
    updated: &Persona,
    before: &[String],
    after: &[String],
) -> Option<Widening> {
    let machine = |persona: &Persona| persona.reach.unwrap_or_default() == Reach::Machine;
    if machine(updated) && !machine(previous) {
        return Some(Widening::Machine);
    }
    if updated.cwd != previous.cwd {
        return Some(Widening::Workspace);
    }
    if folder_grown(previous, updated) {
        return Some(Widening::Folder);
    }
    let on = |persona: &Persona| persona.computer.clone().filter(|computer| computer.enabled);
    match (on(previous), on(updated)) {
        (None, Some(_)) => return Some(Widening::Computer),
        (Some(was), Some(now)) => {
            let mounts = was.mounts.unwrap_or_default();
            let mount_grown = now.mounts.iter().flatten().any(|mount| {
                !mounts.iter().any(|old| {
                    old.host == mount.host
                        && old.path == mount.path
                        && (!old.readonly || mount.readonly)
                })
            });
            if mount_grown {
                return Some(Widening::Folder);
            }
        }
        _ => {}
    }
    let secrets = previous.secrets.clone().unwrap_or_default();
    if updated
        .secrets
        .iter()
        .flatten()
        .any(|name| !secrets.contains(name))
    {
        return Some(Widening::Secret);
    }
    let had = granted(previous, before);
    if !granted(updated, after).is_subset(&had) {
        return Some(Widening::McpServer);
    }
    None
}

/// A folder the teammate did not have, or may now change and could not.
fn folder_grown(previous: &Persona, updated: &Persona) -> bool {
    let had = previous.folders.clone().unwrap_or_default();
    updated.folders.iter().flatten().any(|folder| {
        !had.iter()
            .any(|old| old.path == folder.path && (old.writable || !folder.writable))
    })
}

/// Takes every standing consent to hand `target` work away, because its
/// access grew: each sender leaves `allowedSenders` on the record the caller
/// is about to write, and each direction's informed consent is tombstoned
/// with why, for the next card to say.
pub(crate) fn clear(log: &Log, target: &mut Persona, why: Widening) -> Result<(), String> {
    let room = log.load(&StreamId::Room);
    let mut senders: Vec<String> = std::mem::take(&mut target.allowed_senders);
    for event in &room {
        if event["kind"] == INFORMED
            && event["to"] == target.id.as_str()
            && event["deleted"] != true
            && let Some(from) = event["from"].as_str()
            && !senders.iter().any(|sender| sender == from)
        {
            senders.push(from.to_string());
        }
    }
    for sender in senders {
        log.append(
            &StreamId::Room,
            &json!({"kind": INFORMED, "id": informed_id(&sender, &target.id),
                "from": sender, "to": target.id, "deleted": true, "widened": why.code()}),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// A deleted teammate's informed consent, in both directions: it names
/// nobody any more.
pub(crate) fn forget(log: &Log, persona: &str) {
    for mut event in log.load(&StreamId::Room) {
        if event["kind"] == INFORMED
            && event["deleted"] != true
            && (event["from"] == persona || event["to"] == persona)
        {
            event["deleted"] = json!(true);
            if let Err(error) = log.append(&StreamId::Room, &event) {
                eprintln!("forget informed collaboration: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests;
