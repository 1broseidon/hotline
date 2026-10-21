//! A link is never followed here: one in the source is skipped and counted,
//! and one in the account is replaced by what the source holds.

use super::support::{AccountSupport, SetupCategory};
use crate::contract::{AccountSetupOrigin, AccountSetupView, SetupCategoryView, SetupCopyReport};
use crate::credentials::{atomic_write, private_read};
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.json";
const SETTINGS_ENV: &str = "env";

#[derive(Debug, PartialEq)]
pub(crate) struct SetupSource {
    pub(crate) dir: PathBuf,
    pub(crate) state_file: Option<PathBuf>,
}

pub(crate) fn nothing_to_copy(agent: &str) -> String {
    format!("There is no {agent} setup on this computer to copy.")
}

pub(crate) fn system_default_source(support: &AccountSupport) -> Option<SetupSource> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    source_from(support, std::env::var_os(support.env_var), home)
}

/// A folder the environment names that is not there is no source: the home
/// default does not stand in for it.
fn source_from(
    support: &AccountSupport,
    env_folder: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<SetupSource> {
    let (dir, state_parent) = if let Some(folder) = env_folder.filter(|folder| !folder.is_empty()) {
        let dir = PathBuf::from(folder);
        (dir.clone(), dir)
    } else {
        let home = home?;
        (home.join(support.home_dir), home)
    };
    dir.is_dir().then(|| SetupSource {
        dir,
        state_file: support.state_file.map(|file| state_parent.join(file)),
    })
}

/// `account_dir` must be the folder `checked_account_dir` returned.
pub(crate) fn copy_setup(
    source: &SetupSource,
    account_dir: &Path,
    support: &AccountSupport,
    categories: &[String],
) -> io::Result<SetupCopyReport> {
    if let Some(unknown) = categories
        .iter()
        .find(|id| !support.setup.iter().any(|category| id == &category.id))
    {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Unknown setup item: {unknown}"),
        ));
    }
    let mut report = SetupCopyReport {
        copied: Vec::new(),
        removed_keys: Vec::new(),
        links_skipped: 0,
    };
    let chosen = support
        .setup
        .iter()
        .filter(|category| categories.iter().any(|id| id == category.id));
    for category in chosen {
        // First, so a file that cannot be read refuses the category before it changes anything.
        for (file, keys) in category.json_keys {
            merge_keys(source, account_dir, support, file, keys, &mut report)?;
        }
        for path in category.paths {
            if *path == SETTINGS_FILE {
                copy_settings(source, account_dir, support, &mut report)?;
            } else {
                mirror(&source.dir.join(path), &account_dir.join(path), &mut report)?;
            }
        }
        report.copied.push(category.id.to_string());
    }
    Ok(report)
}

fn copy_settings(
    source: &SetupSource,
    account_dir: &Path,
    support: &AccountSupport,
    report: &mut SetupCopyReport,
) -> io::Result<()> {
    let target = account_dir.join(SETTINGS_FILE);
    let Some(mut settings) = read_source_object(&source.dir.join(SETTINGS_FILE), report)? else {
        return remove(&target);
    };
    for key in support.settings_auth_keys {
        if settings.shift_remove(*key).is_some() {
            report.removed_keys.push((*key).to_string());
        }
    }
    if let Some(Value::Object(env)) = settings.get_mut(SETTINGS_ENV) {
        let credentials: Vec<String> = env
            .keys()
            .filter(|name| {
                support
                    .scrub
                    .iter()
                    .any(|scrubbed| scrubbed.eq_ignore_ascii_case(name))
            })
            .cloned()
            .collect();
        for name in credentials {
            env.shift_remove(&name);
            report.removed_keys.push(format!("{SETTINGS_ENV}.{name}"));
        }
    }
    write_json(&target, settings)
}

fn merge_keys(
    source: &SetupSource,
    account_dir: &Path,
    support: &AccountSupport,
    file: &str,
    keys: &[&str],
    report: &mut SetupCopyReport,
) -> io::Result<()> {
    let source_file = match &source.state_file {
        Some(state_file) if support.state_file == Some(file) => state_file.clone(),
        _ => source.dir.join(file),
    };
    let from = read_source_object(&source_file, report)?.unwrap_or_default();
    let target = account_dir.join(file);
    let existing = read_account_object(&target)?;
    let nothing_there = existing.is_none();
    let mut merged = existing.unwrap_or_default();
    for key in keys {
        match from.get(*key) {
            Some(value) => {
                merged.insert((*key).to_string(), value.clone());
            }
            None => {
                merged.shift_remove(*key);
            }
        }
    }
    if nothing_there && merged.is_empty() {
        return Ok(());
    }
    write_json(&target, merged)
}

fn mirror(src: &Path, dst: &Path, report: &mut SetupCopyReport) -> io::Result<()> {
    let kind = match fs::symlink_metadata(src) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == ErrorKind::NotFound => return remove(dst),
        Err(error) => return Err(error),
    };
    if kind.is_symlink() {
        report.links_skipped += 1;
        remove(dst)
    } else if kind.is_file() {
        replace_file(dst, &fs::read(src)?)
    } else if kind.is_dir() {
        mirror_folder(src, dst, report)
    } else {
        remove(dst)
    }
}

fn mirror_folder(src: &Path, dst: &Path, report: &mut SetupCopyReport) -> io::Result<()> {
    if !fs::symlink_metadata(dst).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        remove(dst)?;
    }
    fs::create_dir_all(dst)?;
    let names = fs::read_dir(src)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<OsString>>>()?;
    // Before anything is written: on a case-insensitive disk an old `Review`
    // is the entry a new `review` lands on, and sweeping after would delete it.
    for entry in fs::read_dir(dst)? {
        let name = entry?.file_name();
        if !names.contains(&name) {
            remove(&dst.join(name))?;
        }
    }
    for name in &names {
        mirror(&src.join(name), &dst.join(name), report)?;
    }
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    let kind = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let removed = if kind.is_symlink() {
        unlink(path)
    } else if kind.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match removed {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(windows)]
fn unlink(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if fs::symlink_metadata(path)?.file_type().is_symlink_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(not(windows))]
fn unlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        remove(path)?;
    }
    atomic_write(path, bytes)
}

fn write_json(path: &Path, object: Map<String, Value>) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(object))?;
    bytes.push(b'\n');
    replace_file(path, &bytes)
}

fn parse_object(text: &str, path: &Path) -> io::Result<Map<String, Value>> {
    match serde_json::from_str(text) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "{} is not a JSON object.",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
        )),
    }
}

fn read_source_object(
    path: &Path,
    report: &mut SetupCopyReport,
) -> io::Result<Option<Map<String, Value>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            report.links_skipped += 1;
            Ok(None)
        }
        Ok(_) => parse_object(&fs::read_to_string(path)?, path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_account_object(path: &Path) -> io::Result<Option<Map<String, Value>>> {
    let file = match private_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    parse_object(&io::read_to_string(file)?, path).map(Some)
}

/// Creates nothing: a folder that holds no setup yet answers zeros.
pub(crate) fn setup_view(
    account_dir: &Path,
    support: &AccountSupport,
    origin: Option<AccountSetupOrigin>,
    with_paths: bool,
) -> AccountSetupView {
    let categories = support
        .setup
        .iter()
        .map(|category| SetupCategoryView {
            id: category.id.to_string(),
            count: count_items(account_dir, category),
            path: with_paths.then(|| {
                account_dir
                    .join(category.open)
                    .to_string_lossy()
                    .into_owned()
            }),
        })
        .collect();
    AccountSetupView {
        setup: origin,
        categories,
    }
}

fn count_items(account_dir: &Path, category: &SetupCategory) -> u32 {
    let count = match category.json_keys.first() {
        Some((file, keys)) => keys
            .first()
            .map_or(0, |key| entries_under_key(&account_dir.join(file), key)),
        None => category
            .paths
            .iter()
            .map(|path| entries_in(&account_dir.join(path)))
            .sum(),
    };
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn entries_under_key(file: &Path, key: &str) -> usize {
    let Ok(Some(object)) = read_account_object(file) else {
        return 0;
    };
    match object.get(key) {
        Some(Value::Object(entries)) => entries.len(),
        Some(Value::Array(entries)) => entries.len(),
        _ => 0,
    }
}

fn entries_in(path: &Path) -> usize {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::read_dir(path).map_or(0, Iterator::count)
        }
        Ok(metadata) if metadata.file_type().is_file() => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
