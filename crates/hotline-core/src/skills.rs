//! Skills: procedures a teammate reads when the task calls for it.
//!
//! A skill is a directory named for the skill holding `SKILL.md` — `name`
//! and `description` frontmatter, then a Markdown body — in the Agent Skills
//! format, under `.agents/skills/<name>/`. The design record's section 7 says
//! why: three sources (built-in, the operator's gateway, the teammate's own
//! workspace) and one channel, the workspace, so both drivers read the same
//! files.
//!
//! This module is the catalog: it validates a skill, holds the built-ins, and
//! reads a folder of them. What the agent hears is [`index`], a line per
//! skill with the name, the description and the path — the format's
//! progressive disclosure, so a body costs nothing until it is read.

use std::ffi::OsStr;
use std::fmt::Write;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde_json::{Map, Value};

use crate::contract::{PolicyMode, SkillEntry, SkillPolicy, SkillSource};

/// Where skills live inside a workspace, relative to the working directory.
pub const DIRECTORY: &str = ".agents/skills";

/// The file every skill is.
pub const FILE: &str = "SKILL.md";

/// A file inside an entry that says Hotline copied it, and may replace or
/// remove it. An entry without one belongs to the person or the teammate and
/// is never touched. A file rather than a marker line because `SKILL.md` has
/// to begin with its frontmatter.
pub const MANAGED_MARKER: &str = ".managed-by-hotline";

const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 1024;
/// A body past this is not a skill, it is a document the skill should point
/// at; the format recommends staying under 500 lines.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// A skill that passed validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Everything after the frontmatter, trimmed.
    pub body: String,
}

/// The built-in skills, bundled with the binary and always on. A bad one
/// fails the test suite rather than a session, so this list is proven by
/// [`tests::every_builtin_is_a_valid_skill`].
pub fn builtin() -> Vec<Skill> {
    BUILTIN
        .iter()
        .map(|(name, text)| {
            parse(name, text).unwrap_or_else(|reason| panic!("built-in skill {name}: {reason}"))
        })
        .collect()
}

const BUILTIN: &[(&str, &str)] = &[(
    "hotline-room",
    include_str!("../skills/hotline-room/SKILL.md"),
)];

/// Parses one `SKILL.md`, given the name of the directory it sits in, and
/// says in one sentence what is wrong with it if anything is.
pub fn parse(directory: &str, text: &str) -> Result<Skill, String> {
    let (front, body) = split_frontmatter(text)?;
    let fields = fields(front)?;
    let name = fields
        .iter()
        .find(|(key, _)| key == "name")
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or("SKILL.md has no name.")?;
    valid_name(name)?;
    if name != directory {
        return Err(format!(
            "The name {name} does not match the folder {directory}."
        ));
    }
    let description = fields
        .iter()
        .find(|(key, _)| key == "description")
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or("SKILL.md has no description.")?;
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(format!(
            "The description is longer than {MAX_DESCRIPTION} characters."
        ));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "The body is larger than {} KiB; move the detail to a referenced file.",
            MAX_BODY_BYTES / 1024
        ));
    }
    Ok(Skill {
        name: name.to_string(),
        description: description.to_string(),
        body: body.trim().to_string(),
    })
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.chars().count() > MAX_NAME {
        return Err(format!("The name is longer than {MAX_NAME} characters."));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "The name {name} may only have lowercase letters, digits and hyphens."
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err(format!(
            "The name {name} may not start or end with a hyphen, or repeat one."
        ));
    }
    Ok(())
}

/// The frontmatter between the first two `---` lines, and the body after.
fn split_frontmatter(text: &str) -> Result<(&str, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---")
        .and_then(|rest| {
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
        })
        .ok_or("SKILL.md does not begin with --- frontmatter.")?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Ok((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    Err("The frontmatter never closes with ---.".to_string())
}

/// The top-level `key: value` pairs of the frontmatter. This is the subset of
/// YAML the format needs: plain and quoted scalars, `>` and `|` block scalars
/// for a long description, and nested mappings such as `metadata`, whose
/// indented lines belong to their key and are kept as they are.
fn fields(front: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for raw in front.lines() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // A continuation of the previous key: a block scalar's line or a
            // nested mapping's entry.
            let Some((_, value)) = out.last_mut() else {
                return Err("The frontmatter begins with an indented line.".to_string());
            };
            let (folded, literal) = (value.starts_with('>'), value.starts_with('|'));
            if folded || literal {
                let sep = if literal || value.len() == 1 {
                    "\n"
                } else {
                    " "
                };
                let head = if value.len() == 1 { "" } else { sep };
                *value = format!("{value}{head}{}", line.trim());
            } else {
                value.push('\n');
                value.push_str(line.trim());
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("The frontmatter line {line:?} is not key: value."));
        };
        out.push((key.trim().to_string(), unquote(value.trim())));
    }
    for (_, value) in &mut out {
        if let Some(rest) = value.strip_prefix('>').or_else(|| value.strip_prefix('|')) {
            *value = rest.trim().to_string();
        }
    }
    Ok(out)
}

fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        });
    match inner {
        Some(inner) if value.starts_with('"') => inner.replace("\\\"", "\""),
        Some(inner) => inner.replace("''", "'"),
        None => value.to_string(),
    }
}

/// Every entry of a folder of skills, valid or not. A folder that does not
/// exist is an empty catalog, not an error: the gateway has nothing in it
/// until the operator adds a skill. Entries Hotline copied there (carrying the
/// marker) are skipped when `own_only` is set, so a workspace listing shows
/// what the person and the teammate wrote, not the grant echoed back.
pub fn read_folder(folder: &Path, source: SkillSource, own_only: bool) -> Vec<SkillEntry> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter(|(_, path)| !(own_only && path.join(MANAGED_MARKER).exists()))
        .map(|(name, path)| entry(&name, &path, source))
        .collect()
}

/// The workspace's `.agents/skills`, opened through a handle on the
/// workspace that must be `identity` (the record's `cwdIdentity`, under
/// workspace reach) when one is given: a workspace swapped for another
/// directory since the person chose it (H-64) lists nothing. Every read
/// after is resolved inside the handle, never by the path again.
fn workspace_skills(cwd: &Path, identity: Option<&str>) -> Option<Dir> {
    let root = Dir::open_ambient_dir(cwd, ambient_authority()).ok()?;
    if let Some(expected) = identity
        && crate::tools::dir_identity(&root).as_deref() != Some(expected)
    {
        return None;
    }
    root.open_dir(DIRECTORY).ok()
}

/// [`read_folder`] for a teammate's own skills. With the record's
/// `identity` (workspace reach) they are read through [`workspace_skills`],
/// so the desk's listing and the preamble's index read only the directory
/// the person chose, and a skill folder linked out of the workspace, which
/// the teammate's tools cannot open either, is not listed. Without one
/// (whole-machine reach) the folder is read by its path as before.
pub fn read_workspace(cwd: &Path, identity: Option<&str>, own_only: bool) -> Vec<SkillEntry> {
    if identity.is_none() {
        return read_folder(&cwd.join(DIRECTORY), SkillSource::Workspace, own_only);
    }
    let Some(skills) = workspace_skills(cwd, identity) else {
        return Vec::new();
    };
    let Ok(entries) = skills.entries() else {
        return Vec::new();
    };
    let shown = cwd.join(DIRECTORY);
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .filter(|name| skills.metadata(name).is_ok_and(|found| found.is_dir()))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter(|name| !(own_only && skills.exists(Path::new(name).join(MANAGED_MARKER))))
        .map(|name| {
            let text = skills.read_to_string(Path::new(&name).join(FILE)).ok();
            entry_of(&name, &shown.join(&name), SkillSource::Workspace, text)
        })
        .collect()
}

fn entry(directory: &str, path: &Path, source: SkillSource) -> SkillEntry {
    entry_of(
        directory,
        path,
        source,
        std::fs::read_to_string(path.join(FILE)).ok(),
    )
}

/// An entry from its `SKILL.md`'s text, read however the caller reads it.
fn entry_of(directory: &str, path: &Path, source: SkillSource, text: Option<String>) -> SkillEntry {
    let parsed = text
        .ok_or_else(|| format!("{directory} has no {FILE}."))
        .and_then(|text| parse(directory, &text));
    match parsed {
        Ok(skill) => SkillEntry {
            source,
            name: skill.name,
            description: skill.description,
            path: path.to_string_lossy().into_owned(),
            invalid: None,
            offered: None,
            version: None,
        },
        Err(reason) => SkillEntry {
            source,
            name: directory.to_string(),
            description: String::new(),
            path: path.to_string_lossy().into_owned(),
            invalid: Some(reason),
            offered: None,
            version: None,
        },
    }
}

/// The built-ins as catalog entries, at the path they have inside a
/// workspace.
pub fn builtin_entries() -> Vec<SkillEntry> {
    builtin()
        .into_iter()
        .map(|skill| SkillEntry {
            source: SkillSource::Builtin,
            path: workspace_path(&skill.name),
            name: skill.name,
            description: skill.description,
            invalid: None,
            offered: None,
            version: None,
        })
        .collect()
}

/// The name of the skill a running computer serves as its guide.
pub const COMPUTER: &str = "hotline-computer";

/// Writes the running computer's guide into the workspace as the
/// `hotline-computer` skill, under Hotline's marker, so the catalog entry is the
/// release actually running and never a bundled copy that can drift. The
/// marker records the release and checksum it came from, which is how the
/// entry is told apart from a gateway folder of the same name.
///
/// It is written the way [`materialize`] writes, through handles on the
/// workspace that no link the teammate planted can lead out of.
pub fn write_computer(
    cwd: &Path,
    identity: Option<&str>,
    version: &str,
    sha256: &str,
    skill: &str,
) -> Result<(), String> {
    let skills = open_skills(cwd, identity)?;
    let path = cwd.join(DIRECTORY).join(COMPUTER);
    match entry_state(&skills, &path)? {
        // A folder the teammate or the person wrote with this name shadows
        // the computer's, exactly as it shadows a grant.
        EntryState::Theirs => return Ok(()),
        EntryState::Linked => return Err(format!("{} is a symbolic link.", path.display())),
        EntryState::Missing | EntryState::Ours => {}
    }
    let entry = child_dir(&skills, &path)?;
    replace_file(&entry, &path, FILE.as_ref(), |file| {
        io::Write::write_all(file, skill.as_bytes())
    })?;
    replace_file(&entry, &path, MANAGED_MARKER.as_ref(), |file| {
        io::Write::write_all(file, format!("computer {version} {sha256}\n").as_bytes())
    })?;
    Ok(())
}

/// The computer's guide as the catalog lists it, when a running computer
/// wrote one into this workspace: source `computer`, with the release it
/// came from. Nothing when there is no such entry or the marker is not the
/// computer's.
pub fn computer_entry(cwd: &Path, identity: Option<&str>) -> Option<SkillEntry> {
    // Hotline writes this entry as plain files, so it is read through the
    // workspace handle whatever the reach.
    let skills = workspace_skills(cwd, identity)?;
    let folder = cwd.join(DIRECTORY).join(COMPUTER);
    let marker = skills
        .read_to_string(Path::new(COMPUTER).join(MANAGED_MARKER))
        .ok()?;
    let version = marker
        .strip_prefix("computer ")?
        .split_whitespace()
        .next()?;
    let text = skills.read_to_string(Path::new(COMPUTER).join(FILE)).ok();
    let mut found = entry_of(COMPUTER, &folder, SkillSource::Computer, text);
    found.path = workspace_path(COMPUTER);
    found.version = Some(version.to_string());
    Some(found)
}

/// The room setting naming the person's own skills folder, when it is not
/// the standard one.
pub const HOME_SETTING: &str = "skillsHome";
/// The room setting listing which of the person's own skills are offered to
/// teammates, by name.
pub const OFFERED_SETTING: &str = "offeredSkills";

/// What the operator offers teammates beyond the built-ins: the gateway
/// folder, everything valid in it; and the person's own folder,
/// `~/.agents/skills` unless the room says otherwise, whose entries are
/// offered by name and read from where they are, so an edit there is what a
/// teammate reads at its next start.
pub struct Offering {
    pub gateway: PathBuf,
    pub home: Option<PathBuf>,
    pub offered: Vec<String>,
}

impl Offering {
    /// From the room's settings, over a data directory.
    pub fn from_settings(root: &Path, settings: &Map<String, Value>) -> Offering {
        let home = settings
            .get(HOME_SETTING)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(crate::paths::default_skills_home);
        let offered = settings
            .get(OFFERED_SETTING)
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Offering {
            gateway: crate::paths::skills_path(root),
            home,
            offered,
        }
    }

    /// The gateway's entries, valid or not, then the person's own, each
    /// saying whether it is offered.
    pub fn entries(&self) -> Vec<SkillEntry> {
        let mut out = read_folder(&self.gateway, SkillSource::Gateway, false);
        out.extend(self.home_entries());
        out
    }

    fn home_entries(&self) -> Vec<SkillEntry> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        read_folder(home, SkillSource::Home, false)
            .into_iter()
            .map(|entry| SkillEntry {
                offered: Some(self.offered.contains(&entry.name)),
                ..entry
            })
            .collect()
    }

    /// Whether teammates may be granted an entry: anything valid in the
    /// gateway, and a home entry switched on.
    pub fn offers(&self, entry: &SkillEntry) -> bool {
        entry.invalid.is_none()
            && (entry.source != SkillSource::Home || entry.offered == Some(true))
    }

    /// The person's own skill of that name, for the switch: refused when the
    /// name is not one, there is no folder, the folder has no such entry, or
    /// the entry is not a valid skill.
    pub fn home_entry(&self, name: &str) -> Result<SkillEntry, String> {
        valid_name(name)?;
        if self.home.is_none() {
            return Err("There is no folder of your own skills to read.".to_owned());
        }
        let entry = self
            .home_entries()
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| format!("Your skills folder has no skill named {name}."))?;
        match entry.invalid {
            Some(reason) => Err(reason),
            None => Ok(entry),
        }
    }

    /// The offered names with one switched on or off: sorted, each once.
    pub fn switched(&self, name: &str, offered: bool) -> Vec<String> {
        let mut names: Vec<String> = self
            .offered
            .iter()
            .filter(|one| *one != name)
            .cloned()
            .collect();
        if offered {
            names.push(name.to_owned());
        }
        names.sort();
        names.dedup();
        names
    }
}

/// Where a skill sits inside a workspace, as the agent is told it.
pub fn workspace_path(name: &str) -> String {
    format!("{DIRECTORY}/{name}/{FILE}")
}

/// What a teammate can read in its workspace: the built-ins, and every valid
/// entry in its `.agents/skills`, one per name. An entry on disk wins over a
/// built-in of the same name, because it is the file that will be read.
pub fn visible(cwd: &Path, identity: Option<&str>) -> Vec<SkillEntry> {
    let mut out = builtin_entries();
    for found in read_workspace(cwd, identity, false) {
        if found.invalid.is_some() {
            continue;
        }
        match out.iter_mut().find(|one| one.name == found.name) {
            Some(known) => known.description = found.description,
            None => out.push(SkillEntry {
                path: workspace_path(&found.name),
                ..found
            }),
        }
    }
    out
}

/// What the preamble says about skills: one line each, name, description and
/// the path to read. Nothing else, so the body is read only when the task
/// calls for it. Empty when there are none, so the preamble says nothing.
pub fn index(skills: &[SkillEntry]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "You have skills: procedures to read when the task calls for one, each a file in your working directory. Read the file before starting work it describes.",
    );
    for skill in skills {
        let _ = write!(
            out,
            "\n- {}: {} ({})",
            skill.name,
            skill.description,
            workspace_path(&skill.name)
        );
    }
    out
}

/// Writes the skills a teammate may read into its workspace: the built-ins,
/// and the offered skills its policy grants — the gateway's, and the
/// person's own that are switched on, copied fresh from where they are.
/// Every entry Hotline writes carries [`MANAGED_MARKER`]; an entry without
/// one is the teammate's or the person's, is left alone, and shadows a grant
/// of the same name. A marked entry the grant no longer covers is removed,
/// so a revoked grant leaves nothing of Hotline's behind.
///
/// The desk writes here unconfined, into a folder the teammate can change,
/// so every step goes through a directory handle opened on the workspace, as
/// `AGENTS.md` is written: the workspace, then `.agents`, `.agents/skills`
/// and each entry, each opened inside the one before and refused if it is a
/// symbolic link. A link swapped in after the check cannot lead out, because
/// a handle resolves nothing outside the directory it was opened on. Files
/// are written to a fresh temporary file renamed over the name, which
/// replaces a planted link instead of writing through it; an entry counts as
/// Hotline's only when its marker is a plain file; removal takes links away
/// without following them; and links inside an offered skill are not copied.
pub fn materialize(
    cwd: &Path,
    identity: Option<&str>,
    offering: &Offering,
    policy: &SkillPolicy,
) -> Result<(), String> {
    let target = cwd.join(DIRECTORY);
    let skills = open_skills(cwd, identity)?;

    enum Source<'a> {
        Text(&'a str),
        Tree(PathBuf),
    }
    let mut wanted: Vec<(String, Source)> = BUILTIN
        .iter()
        .map(|(name, text)| (name.to_string(), Source::Text(text)))
        .collect();
    for entry in offering.entries() {
        let granted = match policy.mode {
            PolicyMode::All => true,
            PolicyMode::Some => policy.names.contains(&entry.name),
            PolicyMode::None => false,
        };
        if granted && offering.offers(&entry) && !wanted.iter().any(|(name, _)| *name == entry.name)
        {
            wanted.push((entry.name, Source::Tree(PathBuf::from(entry.path))));
        }
    }

    for existing in skills
        .entries()
        .map_err(|error| made(&target, error))?
        .flatten()
    {
        let name = existing.file_name();
        let path = target.join(&name);
        let wanted_here = wanted
            .iter()
            .any(|(wanted, _)| OsStr::new(wanted) == name.as_os_str());
        if !wanted_here && entry_state(&skills, &path)? == EntryState::Ours {
            remove_entry(&skills, &path)?;
        }
    }

    // Two sessions can share a working directory and start at once, so this
    // is written to be run twice at the same time with the same answer: a
    // built-in is overwritten in place rather than removed and remade, a
    // removal that finds nothing is not an error, and a directory that is
    // already there is opened rather than made.
    for (name, source) in wanted {
        let path = target.join(&name);
        let state = entry_state(&skills, &path)?;
        match state {
            EntryState::Theirs => continue,
            EntryState::Linked => return Err(format!("{} is a symbolic link.", path.display())),
            EntryState::Missing | EntryState::Ours => {}
        }
        if state == EntryState::Ours && matches!(source, Source::Tree(_)) {
            // An offered skill may have changed or lost a file since it was
            // copied; the copy is remade whole so nothing stale is read.
            remove_entry(&skills, &path)?;
        }
        let entry = child_dir(&skills, &path)?;
        match source {
            Source::Text(text) => replace_file(&entry, &path, FILE.as_ref(), |file| {
                io::Write::write_all(file, text.as_bytes())
            })?,
            Source::Tree(from) => {
                let source = open_source(&from)?;
                copy_tree(&source, &from, &entry, &path)?;
            }
        }
        replace_file(&entry, &path, MANAGED_MARKER.as_ref(), |_| Ok(()))?;
    }
    Ok(())
}

/// The workspace's `.agents/skills`, made if missing and opened as a handle
/// that every later step is resolved inside. `.agents` and `.agents/skills`
/// are the teammate's to change, so either one being a symbolic link refuses
/// the write rather than following it.
///
/// The root is the working directory the teammate's record names, opened by
/// that path here as every other holder of the workspace opens it. Under
/// workspace reach the caller passes `identity`, the directory the record
/// says that is (`cwdIdentity`), and the root handle just opened must be that
/// directory, read from the handle itself, or nothing is written: a
/// workspace swapped for a link or another directory since it was chosen
/// (H-64) is refused here as by the teammate's own tools.
fn open_skills(cwd: &Path, identity: Option<&str>) -> Result<Dir, String> {
    std::fs::create_dir_all(cwd).map_err(|error| made(cwd, error))?;
    let root = Dir::open_ambient_dir(cwd, ambient_authority()).map_err(|error| made(cwd, error))?;
    if let Some(expected) = identity
        && crate::tools::dir_identity(&root).as_deref() != Some(expected)
    {
        return Err(crate::tools::not_the_chosen_folder(cwd));
    }
    let agents_path = cwd.join(".agents");
    let agents = child_dir(&root, &agents_path)?;
    child_dir(&agents, &cwd.join(DIRECTORY))
}

/// Makes the directory `path` names inside `parent` if it is missing, and
/// opens it there. A symbolic link is refused. The open happens inside
/// `parent`'s handle, which cannot resolve a path out of `parent`, so a link
/// swapped in after the check leads nowhere outside; on Unix the directory
/// opened must also be the one just checked, so it leads nowhere else
/// either.
fn child_dir(parent: &Dir, path: &Path) -> Result<Dir, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} is not a folder.", path.display()))?;
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(made(path, error)),
    }
    let checked = parent
        .symlink_metadata(name)
        .map_err(|error| made(path, error))?;
    if checked.file_type().is_symlink() {
        return Err(format!("{} is a symbolic link.", path.display()));
    }
    let opened = parent.open_dir(name).map_err(|error| made(path, error))?;
    let now = opened.dir_metadata().map_err(|error| made(path, error))?;
    if !same_object(&checked, &now) {
        return Err(format!(
            "{} changed while it was being written.",
            path.display()
        ));
    }
    Ok(opened)
}

/// Whether what was opened is what was checked before the open. On Unix this
/// compares device and inode, so an object swapped in between is caught. On
/// other systems there is no such identity to compare and it says yes: there
/// the handle's own confinement is the whole guard, which keeps a swap from
/// leading outside the folder the handle was opened on but does not refuse
/// it.
fn same_object(checked: &cap_std::fs::Metadata, opened: &cap_std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        (checked.dev(), checked.ino()) == (opened.dev(), opened.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (checked, opened);
        true
    }
}

/// Whose an entry of `.agents/skills` is, read without following any link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryState {
    /// Nothing is there yet.
    Missing,
    /// A directory whose marker is a plain file: Hotline's to replace or
    /// remove.
    Ours,
    /// Anything else: the person's or the teammate's, never touched. A
    /// linked marker is not Hotline's, because Hotline never writes a link.
    Theirs,
    /// The entry is itself a symbolic link: never followed, and refused when
    /// Hotline would write there, as a linked `.agents` is.
    Linked,
}

/// Reads an entry's state.
fn entry_state(skills: &Dir, path: &Path) -> Result<EntryState, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} is not a folder.", path.display()))?;
    let metadata = match skills.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(EntryState::Missing),
        Err(error) => return Err(made(path, error)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(EntryState::Linked);
    }
    if !metadata.is_dir() {
        return Ok(EntryState::Theirs);
    }
    let marker = Path::new(name).join(MANAGED_MARKER);
    match skills.symlink_metadata(&marker) {
        Ok(marker) if marker.is_file() => Ok(EntryState::Ours),
        _ => Ok(EntryState::Theirs),
    }
}

/// Removes an entry through the skills handle. A link anywhere inside it is
/// removed, never followed, and an entry already gone is not an error.
fn remove_entry(skills: &Dir, path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} is not a folder.", path.display()))?;
    match skills.remove_dir_all(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(removed(path, error)),
    }
}

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

/// Writes `name` inside `dir`: a temporary file made new beside it (so it is
/// never a link), filled, then renamed over the name. A rename replaces
/// whatever is there, a planted link included, and never writes through it.
/// `shown` is the entry's path, for the error.
fn replace_file(
    dir: &Dir,
    shown: &Path,
    name: &OsStr,
    fill: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> Result<(), String> {
    let mut made_one = None;
    for _ in 0..16 {
        let candidate = format!(
            ".hotline-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        );
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        match dir.open_with(&candidate, &options) {
            Ok(file) => {
                made_one = Some((candidate, file.into_std()));
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(made(shown, error)),
        }
    }
    let (temporary, mut file) =
        made_one.ok_or_else(|| format!("{} could not be written.", shown.display()))?;
    let result = fill(&mut file)
        .and_then(|()| {
            drop(file);
            dir.rename(&temporary, dir, name)
        })
        .map_err(|error| made(&shown.join(name), error));
    if result.is_err() {
        let _ = dir.remove_file(&temporary);
    }
    result
}

fn remove_tree(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(removed(path, error)),
    }
}

/// Copies a folder the person picked into the gateway under its own name.
/// The folder has to be a skill already — the name is its directory's, so a
/// renamed copy would be invalid on arrival — and the gateway may not have
/// one of that name yet, because replacing a skill somebody's teammate is
/// granted is a decision, not a side effect of adding.
pub fn add_to_gateway(gateway: &Path, from: &Path) -> Result<SkillEntry, String> {
    let name = from
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{} is not a folder.", from.display()))?;
    let picked = entry(&name, from, SkillSource::Gateway);
    if let Some(reason) = picked.invalid {
        return Err(reason);
    }
    let destination = gateway.join(&name);
    if destination.exists() {
        return Err(format!("The gateway already has a skill named {name}."));
    }
    std::fs::create_dir_all(&destination).map_err(|error| made(&destination, error))?;
    let source = open_source(from)?;
    let into = Dir::open_ambient_dir(&destination, ambient_authority())
        .map_err(|error| made(&destination, error))?;
    copy_tree(&source, from, &into, &destination)?;
    Ok(entry(&name, &destination, SkillSource::Gateway))
}

/// Removes a gateway skill. A name that is not there is not an error: the
/// person wanted it gone, and it is.
pub fn remove_from_gateway(gateway: &Path, name: &str) -> Result<(), String> {
    valid_name(name)?;
    let path = gateway.join(name);
    refuse_link(&path)?;
    remove_tree(&path)
}

fn refuse_link(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{} is a symbolic link.", path.display()))
        }
        _ => Ok(()),
    }
}

fn made(path: &Path, error: std::io::Error) -> String {
    format!("{} could not be written: {error}", path.display())
}

fn removed(path: &Path, error: std::io::Error) -> String {
    format!("{} could not be removed: {error}", path.display())
}

/// The folder a skill is copied from, opened once as the handle every file
/// and folder in it is then opened inside. It is the gateway's entry, the
/// person's own skill, or the folder the person picked to add: theirs, but
/// not still, so nothing in it is read by path after this.
fn open_source(from: &Path) -> Result<Dir, String> {
    Dir::open_ambient_dir(from, ambient_authority()).map_err(|error| unread(from, error))
}

/// Copies a skill's files from the source handle into the destination
/// handle. On the destination side every folder is made and opened inside the
/// one before, and every file is written by [`replace_file`], so a link
/// planted there is replaced or refused, never written through. On the
/// source side every entry is opened inside the source handle: a link is
/// skipped, and so is anything that is not a plain file or folder and a
/// marker the operator happened to leave; what is opened must be the object
/// that was listed, a file a file and a folder a folder, so a file swapped for
/// a link while the copy runs is refused rather than read. A file keeps its
/// permissions, so a script stays runnable.
fn copy_tree(from: &Dir, from_shown: &Path, to: &Dir, shown: &Path) -> Result<(), String> {
    for item in from
        .entries()
        .map_err(|error| unread(from_shown, error))?
        .flatten()
    {
        let name = item.file_name();
        if name == MANAGED_MARKER {
            continue;
        }
        let source = from_shown.join(&name);
        let listed = from
            .symlink_metadata(&name)
            .map_err(|error| unread(&source, error))?;
        #[cfg(test)]
        tests::between_listing_and_opening(&source);
        let destination = shown.join(&name);
        if listed.is_dir() {
            let inner_from = from
                .open_dir(&name)
                .map_err(|error| unread(&source, error))?;
            let opened = inner_from
                .dir_metadata()
                .map_err(|error| unread(&source, error))?;
            if !opened.is_dir() || !same_object(&listed, &opened) {
                return Err(changed(&source));
            }
            let inner_to = child_dir(to, &destination)?;
            copy_tree(&inner_from, &source, &inner_to, &destination)?;
        } else if listed.is_file() {
            let mut options = OpenOptions::new();
            options.read(true);
            // A FIFO swapped in after the listing would block an open
            // forever; this way it opens, and is refused as not a file.
            #[cfg(unix)]
            cap_std::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
            let file = from
                .open_with(&name, &options)
                .map_err(|error| unread(&source, error))?;
            let opened = file.metadata().map_err(|error| unread(&source, error))?;
            if !opened.is_file() || !same_object(&listed, &opened) {
                return Err(changed(&source));
            }
            let mut reading = file.into_std();
            let permissions = reading
                .metadata()
                .map_err(|error| unread(&source, error))?
                .permissions();
            replace_file(to, shown, &name, |file| {
                io::copy(&mut reading, file)?;
                file.set_permissions(permissions)
            })?;
        }
    }
    Ok(())
}

fn unread(path: &Path, error: std::io::Error) -> String {
    format!("{} could not be read: {error}", path.display())
}

fn changed(path: &Path) -> String {
    format!("{} changed while it was being copied.", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Swap = Option<Box<dyn FnMut(&Path)>>;

    thread_local! {
        /// What a test does to a skill's source between the copy listing an
        /// entry and opening it. Each test runs on its own thread.
        static BETWEEN: std::cell::RefCell<Swap> = const { std::cell::RefCell::new(None) };
    }

    pub(super) fn between_listing_and_opening(source: &Path) {
        BETWEEN.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(source);
            }
        });
    }

    const GOOD: &str = "---\nname: cut-release\ndescription: Cut a desktop release. Use when asked to release.\n---\n\n# Steps\n\n1. Bump.\n";

    #[test]
    fn a_valid_skill_parses_to_its_name_description_and_body() {
        let skill = parse("cut-release", GOOD).unwrap();
        assert_eq!(skill.name, "cut-release");
        assert_eq!(
            skill.description,
            "Cut a desktop release. Use when asked to release."
        );
        assert_eq!(skill.body, "# Steps\n\n1. Bump.");
    }

    #[test]
    fn every_builtin_is_a_valid_skill() {
        let skills = builtin();
        assert!(skills.iter().any(|skill| skill.name == "hotline-room"));
        for skill in &skills {
            assert!(!skill.body.is_empty(), "{} has no body", skill.name);
            assert!(
                skill.body.lines().count() < 500,
                "{} is too long",
                skill.name
            );
        }
    }

    #[test]
    fn the_name_must_match_the_folder_and_the_format_rules() {
        let renamed = GOOD.replace("cut-release", "Cut-Release");
        assert!(
            parse("Cut-Release", &renamed)
                .unwrap_err()
                .contains("lowercase")
        );
        assert!(parse("other", GOOD).unwrap_err().contains("does not match"));
        let doubled = GOOD.replace("cut-release", "cut--release");
        assert!(
            parse("cut--release", &doubled)
                .unwrap_err()
                .contains("hyphen")
        );
        let long = "a".repeat(65);
        assert!(
            parse(&long, &GOOD.replace("cut-release", &long))
                .unwrap_err()
                .contains("64")
        );
    }

    #[test]
    fn a_missing_description_or_frontmatter_is_said_in_a_sentence() {
        assert_eq!(
            parse("cut-release", "---\nname: cut-release\n---\nbody").unwrap_err(),
            "SKILL.md has no description."
        );
        assert_eq!(
            parse("cut-release", "# no frontmatter").unwrap_err(),
            "SKILL.md does not begin with --- frontmatter."
        );
        assert_eq!(
            parse("cut-release", "---\nname: cut-release\n").unwrap_err(),
            "The frontmatter never closes with ---."
        );
        let long = "x".repeat(1025);
        assert!(
            parse(
                "cut-release",
                &format!("---\nname: cut-release\ndescription: {long}\n---\n")
            )
            .unwrap_err()
            .contains("1024")
        );
    }

    #[test]
    fn quoted_folded_and_nested_frontmatter_are_read_as_the_format_writes_them() {
        let text = "---\nname: \"cut-release\"\ndescription: >\n  Cut a release.\n  Use when asked.\nmetadata:\n  author: hotline\n  version: \"1\"\nallowed-tools: Bash(git:*)\n---\nbody\n";
        let skill = parse("cut-release", text).unwrap();
        assert_eq!(skill.description, "Cut a release. Use when asked.");
        let literal = text.replace(">\n", "|\n");
        assert_eq!(
            parse("cut-release", &literal).unwrap().description,
            "Cut a release.\nUse when asked."
        );
        let crlf = GOOD.replace('\n', "\r\n");
        assert_eq!(
            parse("cut-release", &crlf).unwrap().body,
            "# Steps\r\n\r\n1. Bump."
        );
    }

    #[test]
    fn a_folder_lists_the_valid_and_the_invalid_with_its_reason() {
        let root = std::env::temp_dir().join(format!("hotline-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("cut-release")).unwrap();
        std::fs::write(root.join("cut-release").join(FILE), GOOD).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::create_dir_all(root.join("copied")).unwrap();
        std::fs::write(
            root.join("copied").join(FILE),
            GOOD.replace("cut-release", "copied"),
        )
        .unwrap();
        std::fs::write(root.join("copied").join(MANAGED_MARKER), "").unwrap();
        std::fs::write(root.join("stray.md"), "not a folder").unwrap();

        let all = read_folder(&root, SkillSource::Gateway, false);
        let names: Vec<&str> = all.iter().map(|one| one.name.as_str()).collect();
        assert_eq!(names, ["copied", "cut-release", "empty"]);
        assert_eq!(all[1].invalid, None);
        assert_eq!(all[2].invalid.as_deref(), Some("empty has no SKILL.md."));

        let own = read_folder(&root, SkillSource::Workspace, true);
        let names: Vec<&str> = own.iter().map(|one| one.name.as_str()).collect();
        assert_eq!(
            names,
            ["cut-release", "empty"],
            "a copied entry is the grant, not the teammate's own"
        );

        assert_eq!(
            read_folder(&root.join("nowhere"), SkillSource::Gateway, false),
            [] as [SkillEntry; 0]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_index_is_one_line_per_skill_with_the_path_to_read() {
        assert_eq!(index(&[]), "");
        let lines = index(&builtin_entries());
        assert!(lines.starts_with("You have skills:"));
        assert!(lines.contains("\n- hotline-room: How to work inside a Hotline room"));
        assert!(lines.contains("(.agents/skills/hotline-room/SKILL.md)"));
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hotline-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn put(folder: &Path, name: &str, marked: bool) {
        std::fs::create_dir_all(folder.join(name)).unwrap();
        std::fs::write(
            folder.join(name).join(FILE),
            GOOD.replace("cut-release", name),
        )
        .unwrap();
        if marked {
            std::fs::write(folder.join(name).join(MANAGED_MARKER), "").unwrap();
        }
    }

    /// An offering of the gateway alone: nothing of the person's own.
    fn gateway_only(folder: &Path) -> Offering {
        Offering {
            gateway: folder.to_path_buf(),
            home: None,
            offered: Vec::new(),
        }
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(folder)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    #[test]
    fn a_grant_is_copied_under_the_marker_and_a_revoked_one_leaves_nothing_of_hotlines() {
        let root = scratch("materialize");
        let gateway = root.join("gateway");
        put(&gateway, "cut-release", false);
        std::fs::create_dir_all(gateway.join("cut-release").join("scripts")).unwrap();
        std::fs::write(gateway.join("cut-release/scripts/bump.sh"), "#!/bin/sh\n").unwrap();
        put(&gateway, "other", false);
        put(&gateway, "broken", false);
        std::fs::write(gateway.join("broken").join(FILE), "no frontmatter").unwrap();
        let cwd = root.join("ws");
        let target = cwd.join(DIRECTORY);
        put(&target, "mine", false);
        put(&target, "stale", true);

        let some = SkillPolicy {
            mode: PolicyMode::Some,
            names: vec!["cut-release".into(), "broken".into()],
        };
        materialize(&cwd, None, &gateway_only(&gateway), &some).unwrap();
        assert_eq!(names(&target), ["cut-release", "hotline-room", "mine"]);
        assert!(target.join("cut-release").join(MANAGED_MARKER).exists());
        assert!(target.join("cut-release/scripts/bump.sh").exists());
        assert!(target.join("hotline-room").join(MANAGED_MARKER).exists());
        assert!(!target.join("mine").join(MANAGED_MARKER).exists());
        assert_eq!(
            std::fs::read_to_string(target.join("hotline-room").join(FILE)).unwrap(),
            BUILTIN[0].1
        );

        // All includes a skill added later; a broken one is never copied.
        put(&gateway, "later", false);
        let all = SkillPolicy {
            mode: PolicyMode::All,
            names: Vec::new(),
        };
        materialize(&cwd, None, &gateway_only(&gateway), &all).unwrap();
        assert_eq!(
            names(&target),
            ["cut-release", "hotline-room", "later", "mine", "other"]
        );

        // Revoking leaves the built-ins and the teammate's own, nothing else.
        materialize(&cwd, None, &gateway_only(&gateway), &SkillPolicy::default()).unwrap();
        assert_eq!(names(&target), ["hotline-room", "mine"]);

        // A skill of the teammate's own shadows a grant, and a built-in, by
        // name: what is on disk is what is listed.
        put(&target, "cut-release", false);
        std::fs::write(
            target.join("hotline-room").join(FILE),
            GOOD.replace("cut-release", "hotline-room")
                .replace("Cut a desktop release.", "My own room rules."),
        )
        .unwrap();
        std::fs::remove_file(target.join("hotline-room").join(MANAGED_MARKER)).unwrap();
        materialize(&cwd, None, &gateway_only(&gateway), &all).unwrap();
        assert!(!target.join("cut-release").join(MANAGED_MARKER).exists());
        let seen = visible(&cwd, None);
        let room = seen.iter().find(|one| one.name == "hotline-room").unwrap();
        assert!(room.description.starts_with("My own room rules."));
        assert_eq!(room.path, workspace_path("hotline-room"));
        assert!(seen.iter().any(|one| one.name == "cut-release"));
        assert_eq!(seen[0].name, "hotline-room", "built-ins are listed first");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_skill_of_the_persons_own_is_copied_only_when_offered_and_granted() {
        let root = scratch("offered");
        let gateway = root.join("gateway");
        put(&gateway, "triage", false);
        let home = root.join("home");
        put(&home, "cut-release", false);
        put(&home, "triage", false);
        put(&home, "Bad", false);
        let cwd = root.join("ws");
        let target = cwd.join(DIRECTORY);
        let all = SkillPolicy {
            mode: PolicyMode::All,
            names: Vec::new(),
        };

        // Nothing of the person's own is a grant until it is offered, even
        // under all; the catalog says what is offered and what is wrong.
        let none = Offering {
            gateway,
            home: Some(home.clone()),
            offered: Vec::new(),
        };
        materialize(&cwd, None, &none, &all).unwrap();
        assert_eq!(names(&target), ["hotline-room", "triage"]);
        let entries = none.entries();
        let listed: Vec<(SkillSource, &str, Option<bool>, bool)> = entries
            .iter()
            .map(|one| {
                (
                    one.source,
                    one.name.as_str(),
                    one.offered,
                    one.invalid.is_some(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (SkillSource::Gateway, "triage", None, false),
                (SkillSource::Home, "Bad", Some(false), true),
                (SkillSource::Home, "cut-release", Some(false), false),
                (SkillSource::Home, "triage", Some(false), false),
            ]
        );

        // The switch: a valid name that is there, listed sorted and once.
        assert_eq!(none.switched("cut-release", true), ["cut-release"]);
        assert_eq!(
            none.home_entry("nope").unwrap_err(),
            "Your skills folder has no skill named nope."
        );
        assert!(
            none.home_entry("Bad").is_err(),
            "an invalid entry is not offered"
        );
        assert_eq!(
            none.home_entry("cut-release").unwrap().source,
            SkillSource::Home
        );
        let offered = Offering {
            offered: vec!["triage".into(), "cut-release".into(), "cut-release".into()],
            ..none
        };
        assert_eq!(offered.switched("triage", false), ["cut-release"]);
        assert_eq!(
            offered.switched("nope", true),
            ["cut-release", "nope", "triage"]
        );

        // Offered and granted, it is copied from where it is, fresh each
        // start; a name the gateway has is the gateway's.
        materialize(&cwd, None, &offered, &all).unwrap();
        assert_eq!(names(&target), ["cut-release", "hotline-room", "triage"]);
        assert!(target.join("cut-release").join(MANAGED_MARKER).exists());
        std::fs::write(
            home.join("cut-release").join(FILE),
            GOOD.replace("Cut a desktop release.", "Cut a desktop release, revised."),
        )
        .unwrap();
        std::fs::write(
            home.join("triage").join(FILE),
            GOOD.replace("cut-release", "triage")
                .replace("Cut a desktop release.", "The person's own triage."),
        )
        .unwrap();
        materialize(&cwd, None, &offered, &all).unwrap();
        assert!(
            std::fs::read_to_string(target.join("cut-release").join(FILE))
                .unwrap()
                .contains("revised")
        );
        assert!(
            !std::fs::read_to_string(target.join("triage").join(FILE))
                .unwrap()
                .contains("person's own"),
            "the gateway's triage is the one copied"
        );
        assert!(
            !home.join("cut-release").join(MANAGED_MARKER).exists(),
            "the person's folder is never written"
        );

        // Withdrawn, it is gone at the next start.
        let withdrawn = Offering {
            offered: vec!["triage".into()],
            ..offered
        };
        materialize(&cwd, None, &withdrawn, &all).unwrap();
        assert_eq!(names(&target), ["hotline-room", "triage"]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_skills_folder_is_refused_not_followed() {
        let root = scratch("linked");
        let cwd = root.join("ws");
        std::fs::create_dir_all(cwd.join(".agents")).unwrap();
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, cwd.join(DIRECTORY)).unwrap();
        let refusal = materialize(
            &cwd,
            None,
            &gateway_only(&root.join("gateway")),
            &SkillPolicy::default(),
        )
        .unwrap_err();
        assert!(refusal.ends_with("is a symbolic link."), "{refusal}");
        assert!(
            names(&elsewhere).is_empty(),
            "nothing was written through the link"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A folder outside the workspace holding files the person cares about,
    /// to point planted links at.
    #[cfg(unix)]
    fn outside(root: &Path) -> PathBuf {
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("thesis.docx"), "the thesis").unwrap();
        std::fs::write(outside.join("room.jsonl"), "the roster").unwrap();
        outside
    }

    #[cfg(unix)]
    fn untouched(outside: &Path) {
        assert_eq!(names(outside), ["room.jsonl", "thesis.docx"]);
        assert_eq!(
            std::fs::read_to_string(outside.join("thesis.docx")).unwrap(),
            "the thesis"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("room.jsonl")).unwrap(),
            "the roster"
        );
    }

    #[cfg(unix)]
    fn plain_file(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
    }

    #[cfg(unix)]
    fn relink(link: &Path, to: &Path) {
        let _ = std::fs::remove_file(link);
        std::os::unix::fs::symlink(to, link).unwrap();
    }

    /// H-01: a teammate swaps `SKILL.md` in a skill Hotline wrote for a link
    /// to a file of the person's, or a link to nothing. The next start
    /// replaces the link with the skill and leaves the file as it was, and
    /// creates nothing where the dangling link pointed; the computer's guide
    /// is written the same way.
    #[cfg(unix)]
    #[test]
    fn a_linked_file_inside_a_managed_skill_is_never_written_through() {
        let root = scratch("linked-file");
        let outside = outside(&root);
        let cwd = root.join("ws");
        let gateway = gateway_only(&root.join("gateway"));
        let policy = SkillPolicy::default();
        materialize(&cwd, None, &gateway, &policy).unwrap();
        write_computer(&cwd, None, "0.11.0", "abc", "the guide").unwrap();

        for name in ["hotline-room", COMPUTER] {
            let entry = cwd.join(DIRECTORY).join(name);
            relink(&entry.join(FILE), &outside.join("thesis.docx"));
        }
        materialize(&cwd, None, &gateway, &policy).unwrap();
        write_computer(&cwd, None, "0.11.0", "abc", "the guide").unwrap();
        untouched(&outside);

        for name in ["hotline-room", COMPUTER] {
            let entry = cwd.join(DIRECTORY).join(name);
            relink(&entry.join(FILE), &outside.join("planted.txt"));
        }
        materialize(&cwd, None, &gateway, &policy).unwrap();
        write_computer(&cwd, None, "0.11.0", "abc", "the guide").unwrap();
        untouched(&outside);

        let room = cwd.join(DIRECTORY).join("hotline-room");
        assert!(plain_file(&room.join(FILE)));
        assert!(plain_file(&room.join(MANAGED_MARKER)));
        assert_eq!(
            std::fs::read_to_string(room.join(FILE)).unwrap(),
            BUILTIN[0].1
        );
        assert_eq!(names(&room), [MANAGED_MARKER, FILE], "no temporary is left");
        let computer = cwd.join(DIRECTORY).join(COMPUTER);
        assert!(plain_file(&computer.join(FILE)));
        assert_eq!(
            std::fs::read_to_string(computer.join(FILE)).unwrap(),
            "the guide"
        );
        assert_eq!(
            computer_entry(&cwd, None).unwrap().version.as_deref(),
            Some("0.11.0")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// H-01: a marker that is a link is not Hotline's, because Hotline never
    /// writes one. The entry is left alone, the file the link names is never
    /// emptied, and a linked marker on an entry no longer granted does not
    /// make Hotline remove it.
    #[cfg(unix)]
    #[test]
    fn a_linked_marker_is_not_hotlines_and_is_never_truncated() {
        let root = scratch("linked-marker");
        let outside = outside(&root);
        let cwd = root.join("ws");
        let target = cwd.join(DIRECTORY);
        let gateway = gateway_only(&root.join("gateway"));
        let policy = SkillPolicy::default();
        materialize(&cwd, None, &gateway, &policy).unwrap();
        write_computer(&cwd, None, "0.11.0", "abc", "the guide").unwrap();
        put(&target, "stale", true);

        for name in ["hotline-room", COMPUTER, "stale"] {
            let entry = target.join(name);
            relink(&entry.join(MANAGED_MARKER), &outside.join("room.jsonl"));
            relink(&entry.join(FILE), &outside.join("thesis.docx"));
        }
        materialize(&cwd, None, &gateway, &policy).unwrap();
        write_computer(&cwd, None, "0.12.0", "def", "another guide").unwrap();
        untouched(&outside);
        assert_eq!(names(&target), [COMPUTER, "hotline-room", "stale"]);
        for name in ["hotline-room", COMPUTER, "stale"] {
            assert!(
                std::fs::symlink_metadata(target.join(name).join(MANAGED_MARKER))
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{name}'s linked marker is the teammate's and is left as it is"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// H-01: `.agents`, `.agents/skills` or a skill's own folder swapped for
    /// a link to a folder outside refuses the write, for the skills and the
    /// computer's guide alike, and nothing lands where the link points.
    #[cfg(unix)]
    #[test]
    fn a_linked_folder_at_any_level_above_a_skill_is_refused_not_followed() {
        for level in [".agents", DIRECTORY, ".agents/skills/hotline-room"] {
            let root = scratch("linked-level");
            let outside = outside(&root);
            let cwd = root.join("ws");
            let link = cwd.join(level);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let refusal = materialize(
                &cwd,
                None,
                &gateway_only(&root.join("gateway")),
                &SkillPolicy::default(),
            )
            .unwrap_err();
            assert!(
                refusal.ends_with("is a symbolic link."),
                "{level}: {refusal}"
            );
            untouched(&outside);
            std::fs::remove_dir_all(root).unwrap();
        }
        for level in [".agents", DIRECTORY, ".agents/skills/hotline-computer"] {
            let root = scratch("linked-level");
            let outside = outside(&root);
            let cwd = root.join("ws");
            let link = cwd.join(level);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let refusal = write_computer(&cwd, None, "0.11.0", "abc", "the guide").unwrap_err();
            assert!(
                refusal.ends_with("is a symbolic link."),
                "{level}: {refusal}"
            );
            untouched(&outside);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// H-01: a parent swapped for a link after the check is not followed
    /// either. The skills handle is held on the folder it opened, so a write
    /// and a removal through it land in that folder, wherever the teammate
    /// moved it inside its workspace, and never where the new link points.
    /// The desk's skill listing and the preamble's index read only the
    /// directory the record says the workspace is: its skills and computer
    /// guide are listed as before; with a folder above it renamed away and
    /// replaced (as a confined shell on Linux can, inside a writable folder),
    /// nothing is read from the directory now at its path.
    #[cfg(unix)]
    #[test]
    fn skills_are_read_only_from_the_recorded_workspace() {
        let root = scratch("skills-bound-read");
        let cwd = root.join("holder/mid/work");
        std::fs::create_dir_all(&cwd).unwrap();
        let identity = crate::tools::path_identity(&cwd).unwrap();
        put(&cwd.join(DIRECTORY), "own-skill", false);
        write_computer(&cwd, Some(&identity), "0.11.0", "abc", "the guide").unwrap();
        let listed = |identity: Option<&str>| {
            let mut names: Vec<String> = read_workspace(&cwd, identity, false)
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            names.extend(computer_entry(&cwd, identity).map(|entry| entry.name));
            names
        };
        assert_eq!(
            listed(Some(&identity)),
            ["hotline-computer", "own-skill", "hotline-computer"]
        );
        assert!(
            visible(&cwd, Some(&identity))
                .iter()
                .any(|entry| entry.name == "own-skill")
        );

        let planted = root.join("planted");
        put(
            &planted.join("work").join(DIRECTORY),
            "planted-skill",
            false,
        );
        std::fs::rename(root.join("holder/mid"), root.join("holder/mid-was")).unwrap();
        std::os::unix::fs::symlink(&planted, root.join("holder/mid")).unwrap();
        assert_eq!(listed(Some(&identity)), Vec::<String>::new());
        assert!(
            !visible(&cwd, Some(&identity))
                .iter()
                .any(|entry| entry.name == "planted-skill")
        );
    }

    /// The skills writer writes only into the directory the record says the
    /// workspace is: the same one is written as before; another made at its
    /// path since (H-64) gets nothing, and neither does the one moved away.
    #[cfg(unix)]
    #[test]
    fn skills_are_written_only_into_the_recorded_workspace() {
        let root = scratch("skills-bound-root");
        let cwd = root.join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let identity = crate::tools::path_identity(&cwd).unwrap();
        let gateway = root.join("gateway");
        put(&gateway, "cut-release", false);
        let all = SkillPolicy::default();
        materialize(&cwd, Some(&identity), &gateway_only(&gateway), &all).unwrap();
        write_computer(&cwd, Some(&identity), "0.11.0", "abc", "the guide").unwrap();
        assert!(cwd.join(DIRECTORY).is_dir());

        std::fs::rename(&cwd, root.join("work-was")).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        let refused =
            materialize(&cwd, Some(&identity), &gateway_only(&gateway), &all).unwrap_err();
        assert!(refused.contains("no longer the folder chosen"), "{refused}");
        assert!(write_computer(&cwd, Some(&identity), "0.12.0", "def", "another").is_err());
        assert_eq!(names(&cwd), Vec::<String>::new());
        assert_eq!(
            std::fs::read_to_string(
                root.join("work-was")
                    .join(DIRECTORY)
                    .join(COMPUTER)
                    .join(FILE)
            )
            .unwrap(),
            "the guide"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_parent_swapped_for_a_link_after_it_was_opened_leads_nowhere_outside() {
        let root = scratch("swapped-parent");
        let outside = outside(&root);
        let cwd = root.join("ws");
        let target = cwd.join(DIRECTORY);
        put(&target, "stale", true);
        let skills = open_skills(&cwd, None).unwrap();

        std::fs::rename(cwd.join(".agents"), cwd.join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, cwd.join(".agents")).unwrap();
        std::fs::create_dir_all(outside.join("skills/stale")).unwrap();
        std::fs::write(outside.join("skills/stale/keep"), "kept").unwrap();

        let room = target.join("hotline-room");
        let entry = child_dir(&skills, &room).unwrap();
        replace_file(&entry, &room, FILE.as_ref(), |file| {
            io::Write::write_all(file, b"written")
        })
        .unwrap();
        remove_entry(&skills, &target.join("stale")).unwrap();

        assert_eq!(
            std::fs::read_to_string(cwd.join("moved/skills/hotline-room").join(FILE)).unwrap(),
            "written"
        );
        assert_eq!(names(&cwd.join("moved/skills")), ["hotline-room"]);
        assert!(!outside.join("skills/hotline-room").exists());
        assert_eq!(
            std::fs::read_to_string(outside.join("skills/stale/keep")).unwrap(),
            "kept"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// H-01: a granted skill is copied whole with its scripts still runnable.
    /// Links the teammate plants in the copy (a folder and a file) are taken
    /// away at the next start and nothing is written through them; the copy
    /// itself replaces a linked file and refuses a linked folder; and a
    /// revoked grant's entry is removed without following a link out.
    #[cfg(unix)]
    #[test]
    fn a_granted_skill_is_copied_and_removed_without_following_a_planted_link() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("copied-links");
        let outside = outside(&root);
        let gateway = root.join("gateway");
        put(&gateway, "cut-release", false);
        std::fs::create_dir_all(gateway.join("cut-release/scripts")).unwrap();
        let script = gateway.join("cut-release/scripts/bump.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cwd = root.join("ws");
        let entry = cwd.join(DIRECTORY).join("cut-release");
        let granted = SkillPolicy {
            mode: PolicyMode::Some,
            names: vec!["cut-release".into()],
        };
        let offering = gateway_only(&gateway);

        materialize(&cwd, None, &offering, &granted).unwrap();
        let copied = entry.join("scripts/bump.sh");
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "#!/bin/sh\n");
        assert_eq!(
            std::fs::metadata(&copied).unwrap().permissions().mode() & 0o777,
            0o755
        );

        std::fs::remove_dir_all(entry.join("scripts")).unwrap();
        std::os::unix::fs::symlink(&outside, entry.join("scripts")).unwrap();
        relink(&entry.join(FILE), &outside.join("thesis.docx"));
        materialize(&cwd, None, &offering, &granted).unwrap();
        untouched(&outside);
        assert!(plain_file(&entry.join(FILE)));
        assert!(plain_file(&entry.join("scripts/bump.sh")));

        // The copy on its own, into a folder that already holds the links.
        let into = root.join("into");
        std::fs::create_dir_all(&into).unwrap();
        std::os::unix::fs::symlink(&outside, into.join("scripts")).unwrap();
        std::os::unix::fs::symlink(outside.join("thesis.docx"), into.join(FILE)).unwrap();
        let handle = Dir::open_ambient_dir(&into, ambient_authority()).unwrap();
        let from = gateway.join("cut-release");
        let refusal = copy_tree(&open_source(&from).unwrap(), &from, &handle, &into);
        untouched(&outside);
        if let Err(refusal) = refusal {
            assert!(refusal.ends_with("is a symbolic link."), "{refusal}");
        }

        // Revoked, with links planted inside: removed, nothing outside goes.
        std::fs::remove_dir_all(entry.join("scripts")).unwrap();
        std::os::unix::fs::symlink(&outside, entry.join("scripts")).unwrap();
        std::os::unix::fs::symlink(outside.join("room.jsonl"), entry.join("notes")).unwrap();
        materialize(&cwd, None, &offering, &SkillPolicy::default()).unwrap();
        assert_eq!(names(&cwd.join(DIRECTORY)), ["hotline-room"]);
        untouched(&outside);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Every file under `folder`, read without following a link, holds none
    /// of the outside file's words.
    #[cfg(unix)]
    fn holds_nothing_from_outside(folder: &Path) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                holds_nothing_from_outside(&entry.path());
            } else if kind.is_file() {
                let bytes = std::fs::read(entry.path()).unwrap();
                assert!(
                    !String::from_utf8_lossy(&bytes).contains("the thesis"),
                    "{} holds outside bytes",
                    entry.path().display()
                );
            }
        }
    }

    /// H-01: the folder a skill is copied from is the person's, not still.
    /// A file in it swapped for a link (out of the folder, or to another file
    /// in it) or a subfolder swapped for a link out, after the copy listed it
    /// and before it opened it, refuses the copy, and no outside byte lands in
    /// the gateway or the workspace.
    #[cfg(unix)]
    #[test]
    fn a_source_swapped_for_a_link_while_it_is_copied_brings_no_outside_bytes() {
        type Case = (&'static str, fn(&Path, &Path));
        let cases: [Case; 3] = [
            ("notes.md", |source, outside| {
                relink(source, &outside.join("thesis.docx"));
            }),
            ("notes.md", |source, _| {
                relink(source, &source.with_file_name(FILE));
            }),
            ("scripts", |source, outside| {
                std::fs::remove_dir_all(source).unwrap();
                std::os::unix::fs::symlink(outside, source).unwrap();
            }),
        ];
        for (swapped, swap) in cases {
            for through in ["gateway", "workspace"] {
                let root = scratch("swapped-source");
                let outside = outside(&root);
                let picked = root.join("picked");
                put(&picked, "cut-release", false);
                let skill = picked.join("cut-release");
                std::fs::write(skill.join("notes.md"), "notes").unwrap();
                std::fs::create_dir_all(skill.join("scripts")).unwrap();
                std::fs::write(skill.join("scripts/bump.sh"), "#!/bin/sh\n").unwrap();

                let target = skill.join(swapped);
                let outside_for_hook = outside.clone();
                let mut armed = true;
                BETWEEN.with(|hook| {
                    *hook.borrow_mut() = Some(Box::new(move |source: &Path| {
                        if armed && source == target {
                            armed = false;
                            swap(source, &outside_for_hook);
                        }
                    }));
                });
                let gateway = root.join("gateway");
                let cwd = root.join("ws");
                let refusal = if through == "gateway" {
                    add_to_gateway(&gateway, &skill).map(|_| ())
                } else {
                    let granted = SkillPolicy {
                        mode: PolicyMode::All,
                        names: Vec::new(),
                    };
                    materialize(&cwd, None, &gateway_only(&picked), &granted)
                };
                BETWEEN.with(|hook| *hook.borrow_mut() = None);

                let refusal = refusal.unwrap_err();
                assert!(
                    refusal.contains("changed while it was being copied")
                        || refusal.contains("could not be read"),
                    "{swapped} via {through}: {refusal}"
                );
                holds_nothing_from_outside(&gateway);
                holds_nothing_from_outside(&cwd);
                untouched(&outside);
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }
}
