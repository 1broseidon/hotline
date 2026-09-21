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

use std::path::{Path, PathBuf};

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
    include_str!("../../skills/hotline-room/SKILL.md"),
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

fn entry(directory: &str, path: &Path, source: SkillSource) -> SkillEntry {
    let file = path.join(FILE);
    let parsed = std::fs::read_to_string(&file)
        .map_err(|_| format!("{directory} has no {FILE}."))
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
pub fn write_computer(cwd: &Path, version: &str, sha256: &str, skill: &str) -> Result<(), String> {
    let target = cwd.join(DIRECTORY);
    for dir in [cwd.join(".agents"), target.clone()] {
        refuse_link(&dir)?;
    }
    let entry = target.join(COMPUTER);
    refuse_link(&entry)?;
    if entry.exists() && !entry.join(MANAGED_MARKER).exists() {
        // A folder the teammate or the person wrote with this name shadows
        // the computer's, exactly as it shadows a grant.
        return Ok(());
    }
    std::fs::create_dir_all(&entry).map_err(|error| made(&entry, error))?;
    std::fs::write(entry.join(FILE), skill).map_err(|error| made(&entry, error))?;
    std::fs::write(
        entry.join(MANAGED_MARKER),
        format!("computer {version} {sha256}\n"),
    )
    .map_err(|error| made(&entry, error))?;
    Ok(())
}

/// The computer's guide as the catalog lists it, when a running computer
/// wrote one into this workspace: source `computer`, with the release it
/// came from. Nothing when there is no such entry or the marker is not the
/// computer's.
pub fn computer_entry(cwd: &Path) -> Option<SkillEntry> {
    let folder = cwd.join(DIRECTORY).join(COMPUTER);
    let marker = std::fs::read_to_string(folder.join(MANAGED_MARKER)).ok()?;
    let version = marker
        .strip_prefix("computer ")?
        .split_whitespace()
        .next()?;
    let mut found = entry(COMPUTER, &folder, SkillSource::Computer);
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
pub fn visible(cwd: &Path) -> Vec<SkillEntry> {
    let mut out = builtin_entries();
    for found in read_folder(&cwd.join(DIRECTORY), SkillSource::Workspace, false) {
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
        out.push_str(&format!(
            "\n- {}: {} ({})",
            skill.name,
            skill.description,
            workspace_path(&skill.name)
        ));
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
/// This is a write into a workspace without the confined handle the tools
/// use, because it copies whole directories the operator put in the gateway
/// or keeps in their own folder. It is kept safe the plain way: `.agents`,
/// `.agents/skills` and each entry are refused if they are symbolic links,
/// and links inside an offered skill are not copied.
pub fn materialize(cwd: &Path, offering: &Offering, policy: &SkillPolicy) -> Result<(), String> {
    let target = cwd.join(DIRECTORY);
    for dir in [cwd.join(".agents"), target.clone()] {
        refuse_link(&dir)?;
    }
    std::fs::create_dir_all(&target).map_err(|error| made(&target, error))?;

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

    for existing in std::fs::read_dir(&target)
        .map_err(|error| made(&target, error))?
        .flatten()
    {
        let name = existing.file_name().to_string_lossy().into_owned();
        let path = existing.path();
        if path.join(MANAGED_MARKER).exists() && !wanted.iter().any(|(wanted, _)| *wanted == name) {
            remove_tree(&path)?;
        }
    }

    // Two sessions can share a working directory and start at once, so this
    // is written to be run twice at the same time with the same answer: a
    // built-in is overwritten in place rather than removed and remade, a
    // removal that finds nothing is not an error, and every directory is
    // made with create_dir_all.
    for (name, source) in wanted {
        let entry = target.join(&name);
        refuse_link(&entry)?;
        let ours = entry.join(MANAGED_MARKER).exists();
        if entry.exists() && !ours {
            continue;
        }
        if ours && matches!(source, Source::Tree(_)) {
            // An offered skill may have changed or lost a file since it was
            // copied; the copy is remade whole so nothing stale is read.
            remove_tree(&entry)?;
        }
        std::fs::create_dir_all(&entry).map_err(|error| made(&entry, error))?;
        match source {
            Source::Text(text) => {
                std::fs::write(entry.join(FILE), text).map_err(|error| made(&entry, error))?;
            }
            Source::Tree(from) => copy_tree(&from, &entry)?,
        }
        std::fs::write(entry.join(MANAGED_MARKER), "").map_err(|error| made(&entry, error))?;
    }
    Ok(())
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
    copy_tree(from, &destination)?;
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

/// Copies a gateway skill's files into the workspace entry. Links are
/// skipped, and so is a marker the operator happened to leave in the gateway.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    for item in std::fs::read_dir(from)
        .map_err(|error| made(from, error))?
        .flatten()
    {
        let kind = item
            .file_type()
            .map_err(|error| made(&item.path(), error))?;
        let name = item.file_name();
        if kind.is_symlink() || name == MANAGED_MARKER {
            continue;
        }
        let destination = to.join(&name);
        if kind.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|error| made(&destination, error))?;
            copy_tree(&item.path(), &destination)?;
        } else {
            std::fs::copy(item.path(), &destination).map_err(|error| made(&destination, error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(read_folder(&root.join("nowhere"), SkillSource::Gateway, false).is_empty());
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
        materialize(&cwd, &gateway_only(&gateway), &some).unwrap();
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
        materialize(&cwd, &gateway_only(&gateway), &all).unwrap();
        assert_eq!(
            names(&target),
            ["cut-release", "hotline-room", "later", "mine", "other"]
        );

        // Revoking leaves the built-ins and the teammate's own, nothing else.
        materialize(&cwd, &gateway_only(&gateway), &SkillPolicy::default()).unwrap();
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
        materialize(&cwd, &gateway_only(&gateway), &all).unwrap();
        assert!(!target.join("cut-release").join(MANAGED_MARKER).exists());
        let seen = visible(&cwd);
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
            gateway: gateway.clone(),
            home: Some(home.clone()),
            offered: Vec::new(),
        };
        materialize(&cwd, &none, &all).unwrap();
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
        materialize(&cwd, &offered, &all).unwrap();
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
        materialize(&cwd, &offered, &all).unwrap();
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
        materialize(&cwd, &withdrawn, &all).unwrap();
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
}
