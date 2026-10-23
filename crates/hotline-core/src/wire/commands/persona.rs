//! A teammate on the room stream: made, changed, pinned, made lead and
//! removed.

use super::{append, now};
use crate::contract::SkillPolicy;
use crate::contract::{
    FolderGrant, McpPolicy, Persona, PersonaComputer, PersonaDraft, PolicyMode, Reach, SessionInfo,
};
use crate::driver::HOTLINE_BACKEND_ID;
use crate::log::Log;
use crate::wire::RoomHandle;
use crate::{paths, room};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// A drafted field that is present and not blank, trimmed.
fn given(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The backend a draft takes when it names none: the room's chosen default,
/// or Hotline Agent itself.
fn default_backend_id(log: &Log) -> String {
    room::settings(log)
        .get("defaultBackendId")
        .and_then(Value::as_str)
        .unwrap_or(HOTLINE_BACKEND_ID)
        .to_string()
}

/// A teammate as the previous edition's `createPersona` made one: a fresh uuid,
/// the room's default backend, a workspace under the data directory, and
/// every capability its policy can give.
pub(super) fn create_persona(log: &Log, draft: PersonaDraft) -> Result<Value, String> {
    build_persona(log, Uuid::new_v4().to_string(), draft)
}

/// The write behind both `persona.create` and `mobile.persona_create`: the
/// id is the caller's, a fresh uuid for the desk and a phone's own
/// `requestId` for a mobile create, so a retried create finds the teammate
/// already made instead of a second one.
fn build_persona(log: &Log, id: String, draft: PersonaDraft) -> Result<Value, String> {
    let stamped = now();
    let backend_id = given(draft.backend_id).unwrap_or_else(|| default_backend_id(log));
    // A Hotline Agent draft that leaves the model blank takes the room's
    // standing choice, so the teammate starts on what Settings named
    // rather than whichever model happens to lead the catalogue.
    let model_id = given(draft.model_id).or_else(|| {
        if backend_id != HOTLINE_BACKEND_ID {
            return None;
        }
        crate::models::preferred_model(&room::settings(log))
    });
    let cwd = given(draft.cwd).unwrap_or_else(|| {
        paths::default_workspace(log.root(), &id)
            .to_string_lossy()
            .into_owned()
    });
    let folders = checked_folders(draft.folders, &cwd, log.root())?;
    crate::accounts::check_choice(
        log,
        &backend_id,
        model_id.as_deref(),
        draft.account_id.as_deref(),
    )?;
    let persona = Persona {
        node: None,
        id,
        name: given(Some(draft.name)).unwrap_or_else(|| "Untitled".to_string()),
        goal: given(draft.goal).unwrap_or_default(),
        avatar: None,
        team: given(draft.team),
        backend_id,
        cwd,
        // The workspace is the wall unless the draft asked for the machine,
        // and an absent reach is the workspace, so only the wider one is
        // written down.
        reach: draft.reach.filter(|reach| *reach == Reach::Machine),
        folders,
        model_id,
        mode_id: None,
        effort_id: given(draft.effort_id),
        harness_override: None,
        account_id: draft.account_id,
        hop_notice: None,
        mcp_policy: McpPolicy {
            mode: PolicyMode::None,
            server_ids: Vec::new(),
        },
        skill_policy: SkillPolicy::default(),
        background_work: draft.background_work.unwrap_or(false),
        allowed_senders: Vec::new(),
        web_search_policy: None,
        computer: draft.computer,
        voice: None,
        session_checkpoints: Vec::new(),
        last_session_id: None,
        created_at: stamped,
        updated_at: stamped,
    };
    room::append_persona(log, &persona)?;
    // The desk's first teammate is its lead.
    room::settle_lead(log)?;
    Ok(json!(persona))
}

/// All a phone may say about a teammate it creates. The type has no room for
/// reach, cwd, computer or background work, so those cannot arrive here.
pub(super) struct MobileDraft {
    pub(super) name: String,
    pub(super) goal: Option<String>,
    pub(super) backend_id: Option<String>,
    pub(super) model_id: Option<String>,
    pub(super) effort_id: Option<String>,
}

/// `mobile.persona_create`: the phone's own narrow create. `requestId`
/// becomes the id, so a lost acknowledgement's retry finds the teammate
/// already made rather than making a second one. Everything the phone
/// cannot express — reach, cwd, computer, background work — is left at
/// [`build_persona`]'s safest default by never being named in the draft.
pub(super) async fn mobile_persona_create(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    request_id: &str,
    asked: MobileDraft,
) -> Result<Value, String> {
    let MobileDraft {
        name,
        goal,
        backend_id,
        model_id,
        effort_id,
    } = asked;
    let id = Uuid::parse_str(request_id)
        .map_err(|_| "requestId must be a uuid.".to_string())?
        .to_string();
    if let Some(existing) = room::roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
    {
        return Ok(json!(existing));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A teammate needs a name.".to_string());
    }
    let backend_id = given(backend_id).unwrap_or_else(|| default_backend_id(log));
    match room
        .backends()
        .await
        .into_iter()
        .find(|backend| backend.id == backend_id)
    {
        None => return Err(format!("There is no harness {backend_id} on this desk.")),
        Some(backend) if backend.unavailable.is_some() => {
            return Err(format!(
                "{} is not ready: {}",
                backend.name,
                backend.unavailable.unwrap_or_default()
            ));
        }
        Some(_) => {}
    }
    let draft = PersonaDraft {
        name,
        goal,
        team: None,
        backend_id: Some(backend_id),
        cwd: None,
        reach: None,
        folders: None,
        model_id,
        effort_id,
        computer: None,
        background_work: None,
        account_id: None,
    };
    build_persona(log, id, draft)
}

/// `mobile.persona_update`'s whole patch: only `name` and `goal` can be in
/// it, because the command has nowhere to carry anything else. A name is
/// trimmed and must not be blank; a goal is trimmed and may be, which
/// clears it.
pub(super) fn mobile_persona_patch(
    name: Option<String>,
    goal: Option<String>,
) -> Result<Value, String> {
    let mut patch = Map::new();
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() {
            return Err("A teammate needs a name.".to_string());
        }
        patch.insert("name".into(), Value::from(name));
    }
    if let Some(goal) = goal {
        patch.insert("goal".into(), Value::from(goal.trim()));
    }
    if patch.is_empty() {
        return Err("Nothing to change: name a new name or goal.".to_string());
    }
    Ok(Value::Object(patch))
}

/// `mobile.persona_access`: the owner phone's access controls. Reach is
/// Hotline Agent's and a mode a harness's, so each is refused for the other
/// kind of teammate rather than stored where nothing reads it.
pub(super) async fn mobile_persona_access(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    reach: Option<Reach>,
    mode_id: Option<String>,
    background_work: Option<bool>,
) -> Result<Value, String> {
    let persona = living(log, id)?;
    let hotline = persona.backend_id == HOTLINE_BACKEND_ID;
    if reach.is_none() && mode_id.is_none() && background_work.is_none() {
        return Err("Nothing to change: name a reach, a mode or background work.".to_string());
    }
    if reach.is_some() && !hotline {
        return Err(
            "Only a Hotline Agent teammate has a reach; a harness's access is its mode."
                .to_string(),
        );
    }
    if mode_id.is_some() && hotline {
        return Err("A Hotline Agent teammate has no mode; its access is its reach.".to_string());
    }
    let mut patch = Map::new();
    if let Some(reach) = reach {
        patch.insert("reach".into(), json!(reach));
    }
    if let Some(background_work) = background_work {
        patch.insert("backgroundWork".into(), json!(background_work));
    }
    if !patch.is_empty() {
        apply_persona_update(log, room, id, &Value::Object(patch)).await?;
    }
    if let Some(mode_id) = mode_id {
        set_mode(log, room, id, &mode_id).await?;
    }
    living(log, id).map(|persona| json!(persona))
}

/// Merge only the owner's resource controls while holding the same gate as
/// desktop patches, so a concurrent image/mount/secret edit cannot be lost.
pub(super) async fn mobile_persona_computer(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    enabled: Option<bool>,
    memory: Option<String>,
    cpus: Option<Option<f64>>,
) -> Result<Value, String> {
    if enabled.is_none() && memory.is_none() && cpus.is_none() {
        return Err("Nothing to change: name enabled, memory or CPUs.".into());
    }
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    let persona = living(log, id)?;
    // Enabling, disabling and clearing a CPU limit do not need a probe.
    if memory.is_some() || cpus.flatten().is_some() {
        let capacity = room.computer_capacity().await;
        if let Some(Some(cpus)) = cpus {
            validate_computer_cpus(cpus, capacity.cpus)?;
        }
        if let Some(memory) = &memory {
            validate_computer_memory(memory, capacity.memory_bytes)?;
        }
    }
    let mut computer = persona.computer.unwrap_or(PersonaComputer {
        enabled: false,
        image: None,
        memory: None,
        cpus: None,
        pids: None,
        mounts: None,
        secrets: None,
    });
    if let Some(enabled) = enabled {
        computer.enabled = enabled;
    }
    if let Some(memory) = memory {
        computer.memory = Some(memory);
    }
    if let Some(cpus) = cpus {
        computer.cpus = cpus;
    }
    apply_persona_update_locked(log, room, id, &json!({ "computer": computer })).await
}

fn validate_computer_cpus(cpus: f64, capacity: u32) -> Result<(), String> {
    if !cpus.is_finite() || cpus <= 0.0 || cpus > f64::from(capacity) || (cpus * 2.0).fract() != 0.0
    {
        return Err(format!(
            "CPUs must be positive half-core increments, at most {capacity}."
        ));
    }
    Ok(())
}

/// Whole m/M (MiB) and g/G (GiB) runtime spellings only. In particular,
/// decimal/fractional values are not rounded into an unintended grant.
fn validate_computer_memory(memory: &str, capacity: u64) -> Result<(), String> {
    const STEP: u64 = 512 * 1024 * 1024;
    let invalid = || {
        "Memory must be whole MiB or GiB (such as 4608m or 4g), at least 512 MiB and in 512 MiB increments.".to_string()
    };
    let (digits, scale) = match memory.as_bytes().last() {
        Some(b'm' | b'M') => (&memory[..memory.len() - 1], 1024_u64 * 1024),
        Some(b'g' | b'G') => (&memory[..memory.len() - 1], 1024_u64 * 1024 * 1024),
        _ => return Err(invalid()),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let bytes = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(invalid)?;
    if bytes < STEP || !bytes.is_multiple_of(STEP) {
        return Err(invalid());
    }
    if bytes > capacity {
        return Err(format!(
            "Memory exceeds this computer's capacity of {} MiB.",
            capacity / (1024 * 1024)
        ));
    }
    Ok(())
}

/// Switches a harness teammate's mode and keeps it, so the teammate comes
/// back in it after a restart. A resting teammate advertises no modes, so its
/// choice is kept unchecked and offered to the harness at its next start,
/// which refuses one it does not have.
pub(super) async fn set_mode(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    persona_id: &str,
    mode_id: &str,
) -> Result<SessionInfo, String> {
    living(log, persona_id)?;
    if mode_id.trim().is_empty() {
        return Err("A mode needs an id.".to_string());
    }
    let offered = room.info(persona_id);
    let info = if offered.modes.is_empty() {
        offered
    } else {
        if !offered.modes.iter().any(|mode| mode.id == mode_id) {
            return Err(format!("{mode_id} is not a mode this teammate offers."));
        }
        room.set_mode(persona_id, mode_id).await?
    };
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    if living(log, persona_id)?.mode_id.as_deref() != Some(mode_id) {
        update_persona(log, room, persona_id, &json!({ "modeId": mode_id }))?;
    }
    Ok(info)
}

/// `persona.update` and `mobile.persona_update` both land here, under the
/// policy gate, restarting a live session when the patch asks for it.
pub(super) async fn apply_persona_update(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<Value, String> {
    let gate = room.policy_update_lock();
    let _held = gate.lock().await;
    apply_persona_update_locked(log, room, id, patch).await
}

/// The caller holds `policy_update_lock` for read/merge/write and effects.
async fn apply_persona_update_locked(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<Value, String> {
    let (updated, reattaches) = update_persona(log, room, id, patch)?;
    if reattaches {
        room.reattach(id).await?;
    } else if patch.get("computer").is_some() {
        room.computer_settings_changed(id).await;
    }
    Ok(updated)
}

/// The patch over the record, and the whole record written again: a stream
/// folds by id, so a line carrying only what changed would leave the fold
/// holding only what changed.
pub(super) fn update_persona(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
    patch: &Value,
) -> Result<(Value, bool), String> {
    let previous = living(log, id)?;
    if patch.get("avatar").is_some_and(|avatar| !avatar.is_null()) {
        return Err("A picture can only be cleared here, with `avatar: null`.".into());
    }
    if patch.get("accountId").is_some() {
        return Err("A teammate's account is set with persona.account.".into());
    }
    let mut checked = patched(&previous, id, patch)?;
    // Checked here, before anything is revoked or written, so a refused
    // folder leaves the teammate exactly as it was.
    if patch.get("folders").is_some() {
        checked.folders = checked_folders(checked.folders.take(), &checked.cwd, log.root())?;
    }
    let reattaches = persona_update_reattaches(patch, &previous, &checked);
    if reattaches {
        room.invalidate(id)?;
    }
    let updated = {
        // A checkpoint may have been written or cleared since the first read:
        // the patch goes over the record as it is now, under its writers' lock.
        let _writes = room::persona_write_lock();
        let mut updated = patched(&living(log, id)?, id, patch)?;
        if patch.get("folders").is_some() {
            updated.folders = checked.folders;
        }
        room::append_persona(log, &updated)?;
        updated
    };
    if updated.avatar.is_none() {
        crate::session::avatar::remove_except(log.root(), id, None);
    }
    Ok((json!(updated), reattaches))
}

fn patched(record: &Persona, id: &str, patch: &Value) -> Result<Persona, String> {
    let mut record = json!(record);
    let fields = record
        .as_object_mut()
        .expect("a teammate serializes as an object");
    for (key, value) in patch
        .as_object()
        .ok_or("A patch is an object of fields to change.")?
    {
        fields.insert(key.clone(), value.clone());
    }
    fields.insert("id".into(), Value::from(id));
    fields.insert("updatedAt".into(), Value::from(now()));
    serde_json::from_value(record)
        .map_err(|error| format!("That patch does not leave a teammate behind: {error}."))
}

/// Whether this update restarts a live session. The computer's own
/// settings do not, short of turning it on or off: its limits, mounts and
/// image take effect when the container is next made, and its secrets are
/// handed to it where it runs. A turn in flight is not cut short for them.
fn persona_update_reattaches(patch: &Value, previous: &Persona, updated: &Persona) -> bool {
    let enabled = |persona: &Persona| {
        persona
            .computer
            .as_ref()
            .is_some_and(|computer| computer.enabled)
    };
    let mut rest = patch.clone();
    if let Some(fields) = rest.as_object_mut() {
        fields.remove("computer");
    }
    persona_patch_reattaches(&rest) || enabled(previous) != enabled(updated)
}

/// A patch of these fields rebuilds the driver, so a live session has to
/// restart for the new tools to take effect. `name`, `team`, `avatar`,
/// `modelId`, `modeId` and `effortId` do not: model, mode and effort already
/// switch live. `folders` does because every handle holds the folders it was
/// opened with: the restart revokes those handles, and the computer is made
/// again with the new set when the session starts it.
fn persona_patch_reattaches(patch: &Value) -> bool {
    const KEYS: [&str; 11] = [
        "cwd",
        "reach",
        "folders",
        "goal",
        "mcpPolicy",
        "skillPolicy",
        "computer",
        "backendId",
        "harnessOverride",
        "backgroundWork",
        "allowedSenders",
    ];
    patch
        .as_object()
        .is_some_and(|fields| KEYS.iter().any(|key| fields.contains_key(*key)))
}

/// The most extra folders one teammate has. A list longer than this is not
/// a few repositories beside the work; it is a wish for whole-machine reach.
const MAX_FOLDERS: usize = 16;

/// A teammate's extra folders, each checked and spelled as the directory it
/// is, or the first reason one cannot be granted. Empty is none. A folder is
/// refused when it is not an absolute path to an existing directory, when it
/// is `/`, the home directory or holds it, Hotline's data directory or holds
/// it or is inside it — another teammate's workspace there included, whose
/// `.hotline-home` is that teammate's own — or when it is this teammate's
/// workspace or inside it. Two folders may not nest; the
/// same folder named twice is one grant, which may be changed if either said
/// so.
fn checked_folders(
    grants: Option<Vec<FolderGrant>>,
    cwd: &str,
    data: &Path,
) -> Result<Option<Vec<FolderGrant>>, String> {
    let Some(grants) = grants.filter(|grants| !grants.is_empty()) else {
        return Ok(None);
    };
    let resolved = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| resolved(Path::new(&home)));
    let data = resolved(data);
    // A teammate's workspace is made when its session first starts, so a
    // new one may not be on disk yet; its spelling is then what it will be.
    let workspace = resolved(Path::new(&expand_home(cwd)));
    let mut checked: Vec<FolderGrant> = Vec::new();
    for grant in grants {
        let asked = grant.path.trim();
        let expanded = PathBuf::from(expand_home(asked));
        if !expanded.is_absolute() {
            return Err(format!("{asked} is not an absolute path to a folder."));
        }
        let path = dunce::canonicalize(&expanded)
            .ok()
            .filter(|path| path.is_dir())
            .ok_or_else(|| format!("{asked} is not a folder on this machine."))?;
        let Some(spelled) = path.to_str().map(str::to_string) else {
            return Err(format!("{asked} is not a path Hotline can spell."));
        };
        if path.parent().is_none() {
            return Err(format!(
                "{asked} is the whole disk; Whole machine is the switch for that."
            ));
        }
        if home.as_ref().is_some_and(|home| home.starts_with(&path)) {
            return Err(format!(
                "{asked} is the home folder or holds it; choose a folder inside it."
            ));
        }
        if data.starts_with(&path) {
            return Err(format!(
                "{asked} holds Hotline's own data; choose a folder that does not."
            ));
        }
        if path.starts_with(&data) {
            return Err(format!(
                "{asked} is inside Hotline's own data, where each teammate's workspace is its own; choose a folder outside it."
            ));
        }
        if path.starts_with(&workspace) {
            return Err(format!(
                "{asked} is this teammate's own workspace or inside it, which it already has."
            ));
        }
        if let Some(same) = checked.iter_mut().find(|folder| folder.path == spelled) {
            same.writable |= grant.writable;
            continue;
        }
        if let Some(other) = checked.iter().find(|folder| {
            let other = Path::new(&folder.path);
            other.starts_with(&path) || path.starts_with(other)
        }) {
            return Err(format!(
                "{asked} and {} overlap; grant one of them.",
                other.path
            ));
        }
        checked.push(FolderGrant {
            path: spelled,
            writable: grant.writable,
        });
    }
    if checked.len() > MAX_FOLDERS {
        return Err(format!(
            "A teammate can have up to {MAX_FOLDERS} extra folders."
        ));
    }
    Ok(Some(checked))
}

/// `~` and `~/…` as the home directory, as a person types a folder.
fn expand_home(path: &str) -> String {
    if (path == "~" || path.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        return format!("{}{}", home.to_string_lossy(), &path[1..]);
    }
    path.to_string()
}

/// The tombstone is the same kind and id again, so the fold finds it instead
/// of the teammate and a mirror has a line to ship rather than an absence to
/// notice.
pub(super) fn delete_persona(
    log: &Log,
    room: &Arc<dyn RoomHandle>,
    id: &str,
) -> Result<Value, String> {
    living(log, id)?;
    room.invalidate(id)?;
    {
        let _writes = room::persona_write_lock();
        append(
            log,
            &json!({ "kind": "persona", "id": id, "deleted": true }),
        )?;
    }
    // The roster already ignores a pin on a teammate who is gone; settling
    // the stored list keeps it honest and frees the slot.
    let settled = room::pinned_teammates(log);
    let stored = room::settings(log).remove("pinnedTeammates");
    if stored.unwrap_or_else(|| json!([])) != json!(settled) {
        write_pins(log, &settled)?;
    }
    // A lead who has gone is no lead, and the one teammate left, if one is,
    // leads.
    room::settle_lead(log)?;
    crate::session::ledger::forget(id);
    room.forget(id);
    Ok(Value::Null)
}

/// Pins `id` at `slot`, or unpins it. The list is the setting's whole value,
/// so the answer is the list as it now stands.
pub(super) fn pin_persona(log: &Log, id: &str, slot: Option<u8>) -> Result<Value, String> {
    living(log, id)?;
    let mut pins = room::pinned_teammates(log);
    let was_pinned = pins.iter().any(|pin| pin == id);
    pins.retain(|pin| pin != id);
    if let Some(slot) = slot {
        if !was_pinned && pins.len() >= room::MAX_PINS {
            return Err(room::too_many_pins());
        }
        pins.insert(usize::from(slot).min(pins.len()), id.to_string());
    }
    write_pins(log, &pins)?;
    Ok(json!(pins))
}

/// Makes `id` the desk's lead, which moves the role from whoever had it,
/// or clears the lead when no teammate is named. The answer is the lead as
/// it now stands.
pub(super) fn lead_persona(log: &Log, id: Option<&str>) -> Result<Value, String> {
    if let Some(id) = id {
        living(log, id)?;
        room::write_lead(log, Some(id))?;
        return Ok(json!(id));
    }
    // `settle_lead` would make the one teammate lead again at the next
    // start, so a cleared lead there would not stay cleared.
    if room::roster(log).len() == 1 {
        return Err("A desk with one teammate has it as its lead.".to_string());
    }
    room::write_lead(log, None)?;
    Ok(Value::Null)
}

fn write_pins(log: &Log, pins: &[String]) -> Result<(), String> {
    let event = if pins.is_empty() {
        json!({ "kind": "setting", "id": "pinnedTeammates", "deleted": true })
    } else {
        json!({ "kind": "setting", "id": "pinnedTeammates", "value": pins })
    };
    append(log, &event)
}

pub(in crate::wire) fn living(log: &Log, id: &str) -> Result<Persona, String> {
    room::roster(log)
        .into_iter()
        .find(|persona| persona.id == id)
        .ok_or_else(|| format!("There is no teammate {id}."))
}
