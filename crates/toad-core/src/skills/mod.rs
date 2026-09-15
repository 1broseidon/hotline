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

use crate::contract::{SkillEntry, SkillSource};

/// Where skills live inside a workspace, relative to the working directory.
pub const DIRECTORY: &str = ".agents/skills";

/// The file every skill is.
pub const FILE: &str = "SKILL.md";

/// A file inside an entry that says Toad copied it, and may replace or
/// remove it. An entry without one belongs to the person or the teammate and
/// is never touched. A file rather than a marker line because `SKILL.md` has
/// to begin with its frontmatter.
pub const MANAGED_MARKER: &str = ".managed-by-toad";

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

const BUILTIN: &[(&str, &str)] = &[("toad-room", include_str!("../../skills/toad-room/SKILL.md"))];

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
/// until the operator adds a skill. Entries Toad copied there (carrying the
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
        },
        Err(reason) => SkillEntry {
            source,
            name: directory.to_string(),
            description: String::new(),
            path: path.to_string_lossy().into_owned(),
            invalid: Some(reason),
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
        })
        .collect()
}

/// Where a skill sits inside a workspace, as the agent is told it.
pub fn workspace_path(name: &str) -> String {
    format!("{DIRECTORY}/{name}/{FILE}")
}

/// What the preamble says about skills: one line each, name, description and
/// the path to read. Nothing else, so the body is read only when the task
/// calls for it. Empty when there are none, so the preamble says nothing.
pub fn index(skills: &[Skill]) -> String {
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
        assert!(skills.iter().any(|skill| skill.name == "toad-room"));
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
        let text = "---\nname: \"cut-release\"\ndescription: >\n  Cut a release.\n  Use when asked.\nmetadata:\n  author: toad\n  version: \"1\"\nallowed-tools: Bash(git:*)\n---\nbody\n";
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
        let root = std::env::temp_dir().join(format!("toad-skills-{}", uuid::Uuid::new_v4()));
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
        let lines = index(&builtin());
        assert!(lines.starts_with("You have skills:"));
        assert!(lines.contains("\n- toad-room: How to work inside a Toad room"));
        assert!(lines.contains("(.agents/skills/toad-room/SKILL.md)"));
    }
}
