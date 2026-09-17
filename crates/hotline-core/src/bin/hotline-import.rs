//! Import an existing Hotline data directory into a new one, without a desk.
//!
//! `hotline-import <from> <to>` reads the previous edition's layout at `<from>`
//! and writes this tree's streams and vault at `<to>`. The source is never
//! written. The two paths must not be the same directory, and `<to>` must
//! not live inside `<from>` — either would make the importer a writer of
//! the tree it is reading.

use hotline_core::import;
use hotline_core::log::Log;
use hotline_core::vault::Vault;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(from) = args.next() else {
        return usage();
    };
    let Some(to) = args.next() else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }

    if let Err(error) = run(PathBuf::from(from), PathBuf::from(to)) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run(from: PathBuf, to: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(reason) = overlap_reason(&from, &to) {
        return Err(reason.into());
    }
    let log = Log::open(&to);
    let vault = Vault::open(&to, log.clone())?;
    let report = import::import(&from, &log, &vault)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Why these two paths must not be used together, or nothing when they
/// resolve to distinct trees.
///
/// Canonicalize so a symlink or a `..` cannot hide that `to` is `from`.
/// A `to` that does not exist yet is resolved against the nearest
/// ancestor that does, so `hotline-import X X/new` is refused before the
/// log creates `X/new` inside the source.
fn overlap_reason(from: &Path, to: &Path) -> Option<String> {
    if from == to {
        return Some(format!(
            "{} is the source directory or lives inside it; the importer would write into the Hotline it is reading",
            to.display()
        ));
    }
    let from = from.canonicalize().ok()?;
    let to = resolve_existing_prefix(to)?;
    if to == from || to.starts_with(&from) {
        return Some(format!(
            "{} is the source directory or lives inside it; the importer would write into the Hotline it is reading",
            to.display()
        ));
    }
    None
}

/// The canonical path, or — when the last components do not exist yet —
/// the canonical existing ancestor with those components joined back on.
fn resolve_existing_prefix(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return path.canonicalize().ok();
    }
    let mut current = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut missing = PathBuf::new();
    loop {
        if current.exists() {
            let mut resolved = current.canonicalize().ok()?;
            resolved.push(&missing);
            return Some(resolved);
        }
        match current.file_name() {
            Some(name) => {
                missing = Path::new(name).join(missing);
                if !current.pop() {
                    return None;
                }
            }
            None => return Some(current.join(missing)),
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: hotline-import <from> <to>");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hotline-import-bin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_same_directory_is_refused() {
        let dir = scratch("same");
        let reason = overlap_reason(&dir, &dir).expect("same path must be refused");
        assert!(
            reason.contains("source directory") || reason.contains("inside"),
            "{reason}"
        );
    }

    #[test]
    fn a_destination_inside_the_source_is_refused() {
        let from = scratch("inside");
        let to = from.join("nested");
        assert!(
            overlap_reason(&from, &to).is_some(),
            "nested must be refused"
        );
        fs::create_dir_all(&to).unwrap();
        assert!(
            overlap_reason(&from, &to).is_some(),
            "an existing nested dest must be refused"
        );
    }

    #[test]
    fn a_neighbour_is_allowed() {
        let parent = scratch("neighbour");
        let from = parent.join("from");
        let to = parent.join("to");
        fs::create_dir_all(&from).unwrap();
        assert!(overlap_reason(&from, &to).is_none());
        fs::create_dir_all(&to).unwrap();
        assert!(overlap_reason(&from, &to).is_none());
    }
}
