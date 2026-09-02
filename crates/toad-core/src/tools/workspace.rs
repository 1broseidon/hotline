use super::ToolError;
use crate::contract::Reach;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};
use ignore::WalkBuilder;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// How many lines a read returns when the agent does not ask. Paging is always
/// available: the answer names how many lines remain.
const DEFAULT_READ_LINES: usize = 2000;
const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const DEFAULT_SEARCH_RESULTS: usize = 100;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCH_LINE_CHARS: usize = 500;
const DEFAULT_FIND_RESULTS: usize = 500;
const MAX_FIND_RESULTS: usize = 1_000;
const MAX_PATTERN_CHARS: usize = 1_000;
/// An absurd ceiling, not a working limit: a teammate writing source and notes
/// never hits it, and a call that does is a mistake we refuse to land.
const MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

struct WorkspaceInner {
    /// The teammate's working directory: where relative paths start and
    /// where commands run.
    cwd: PathBuf,
    /// The directory every path is resolved inside. The working directory
    /// itself for `Reach::Workspace`; the filesystem root for `Reach::Machine`.
    root: PathBuf,
    dir: Dir,
    reach: Reach,
    /// Where a long tool result is written. Under workspace reach, a
    /// read-only tool may open a path here after the working directory;
    /// writes may not. The first overflow creates it, so it may be
    /// missing when the turn starts.
    overflow: PathBuf,
}

#[derive(Clone)]
pub struct Workspace {
    inner: Arc<WorkspaceInner>,
}

impl Workspace {
    pub fn open(cwd: PathBuf, reach: Reach, overflow: PathBuf) -> Result<Self, ToolError> {
        let cwd = cwd.canonicalize().map_err(|error| {
            ToolError::new(format!(
                "The Toad workspace {} is unavailable: {error}",
                cwd.display()
            ))
        })?;
        let root = match reach {
            Reach::Workspace => cwd.clone(),
            Reach::Machine => cwd
                .ancestors()
                .last()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/")),
        };
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|error| {
            ToolError::new(format!(
                "The Toad workspace {} could not be opened: {error}",
                root.display()
            ))
        })?;
        Ok(Self {
            inner: Arc::new(WorkspaceInner {
                cwd,
                root,
                dir,
                reach,
                overflow,
            }),
        })
    }

    pub fn display_root(&self) -> &Path {
        &self.inner.cwd
    }

    /// The teammate's one policy, so the shell can confine a command or not.
    pub fn reach(&self) -> Reach {
        self.inner.reach
    }

    /// Where a path may point, as the tools describe it to the model. The
    /// description is the policy the model reads, so it has to tell the truth
    /// about the wall — or the absence of one. The overflow directory is
    /// named here so every tool that quotes this sentence says the same
    /// thing, including that a long tool result lives there.
    fn paths_reach(&self) -> String {
        match self.inner.reach {
            Reach::Workspace => format!(
                "Paths are relative to the working directory and may not leave it, except to read a long tool result written under {}.",
                self.inner.overflow.display()
            ),
            Reach::Machine => {
                "Paths may be absolute or relative to the working directory; anywhere on this machine is allowed.".to_string()
            }
        }
    }

    /// A requested path as a path inside `dir`.
    ///
    /// Confined to the workspace, that is the request itself, relative and
    /// without `..`. Reaching the machine, absolute paths and `..` are fine:
    /// the request is resolved against the working directory lexically and
    /// then taken from the filesystem root.
    fn requested_relative(&self, requested: &str, allow_root: bool) -> Result<PathBuf, ToolError> {
        if self.inner.reach == Reach::Workspace {
            return normalize_relative_path(requested, allow_root);
        }
        if requested.contains('\0') {
            return Err(ToolError::new("Paths may not contain NUL bytes."));
        }
        let joined = self.inner.cwd.join(requested);
        let mut resolved = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::Normal(part) => resolved.push(part),
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
        if resolved.as_os_str().is_empty() {
            return if allow_root {
                Ok(PathBuf::from("."))
            } else {
                Err(ToolError::new("Name a file."))
            };
        }
        Ok(resolved)
    }

    fn canonical_subpath(&self, requested: &str, allow_root: bool) -> Result<PathBuf, ToolError> {
        let relative = self.requested_relative(requested, allow_root)?;
        let canonical = self
            .inner
            .dir
            .canonicalize(&relative)
            .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
        normalize_relative_path(canonical.to_string_lossy().as_ref(), allow_root)
    }

    /// A path a read-only tool may open: the working directory first, then
    /// the overflow directory. Writes never call this, so they cannot land
    /// there. Machine reach never looks at overflow; it has no wall.
    fn resolve_readable(
        &self,
        requested: &str,
        allow_root: bool,
    ) -> Result<(Dir, PathBuf, PathBuf), ToolError> {
        match self.canonical_subpath(requested, allow_root) {
            Ok(relative) => {
                let dir = self.inner.dir.try_clone().map_err(|error| {
                    ToolError::new(format!(
                        "The Toad workspace {} could not be opened: {error}",
                        self.inner.root.display()
                    ))
                })?;
                Ok((dir, relative, self.inner.root.clone()))
            }
            Err(error) => {
                if self.inner.reach != Reach::Workspace {
                    return Err(error);
                }
                self.resolve_overflow(requested, allow_root)
                    .map_err(|_| error)
            }
        }
    }

    fn resolve_overflow(
        &self,
        requested: &str,
        allow_root: bool,
    ) -> Result<(Dir, PathBuf, PathBuf), ToolError> {
        let root = self
            .inner
            .overflow
            .canonicalize()
            .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
        let dir = Dir::open_ambient_dir(&root, ambient_authority())
            .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
        let relative = if Path::new(requested).is_absolute() {
            let asked = Path::new(requested)
                .canonicalize()
                .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
            let stripped = asked
                .strip_prefix(&root)
                .map_err(|_| ToolError::new("Use a path relative to the active workspace."))?;
            if stripped.as_os_str().is_empty() {
                if allow_root {
                    PathBuf::from(".")
                } else {
                    return Err(ToolError::new("Name a file inside the workspace."));
                }
            } else {
                stripped.to_path_buf()
            }
        } else {
            normalize_relative_path(requested, allow_root)?
        };
        let canonical = dir
            .canonicalize(&relative)
            .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
        let relative = normalize_relative_path(canonical.to_string_lossy().as_ref(), allow_root)?;
        Ok((dir, relative, root))
    }

    fn read_file(&self, args: ReadFileArgs) -> Result<String, ToolError> {
        let (dir, relative, _) = self.resolve_readable(&args.path, false)?;
        let file = dir
            .open(&relative)
            .map_err(|error| ToolError::new(format!("Cannot open {}: {error}", args.path)))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ToolError::new(format!("{} is not a file.", args.path)));
        }

        let start_line = args.start_line.unwrap_or(1);
        let max_lines = args.max_lines.unwrap_or(DEFAULT_READ_LINES);
        if start_line == 0 {
            return Err(ToolError::new("start_line must be at least 1."));
        }
        if max_lines == 0 {
            return Err(ToolError::new("max_lines must be at least 1."));
        }

        let mut reader = BufReader::new(file);
        let mut raw = Vec::new();
        let mut line_no = 0usize;
        let mut selected = Vec::new();
        let mut remaining = 0usize;
        loop {
            raw.clear();
            let n = reader.read_until(b'\n', &mut raw)?;
            if n == 0 {
                break;
            }
            if raw.contains(&0) {
                return Err(ToolError::new(format!(
                    "{} is binary, not a UTF-8 text file.",
                    args.path
                )));
            }
            if raw.last() == Some(&b'\n') {
                raw.pop();
                if raw.last() == Some(&b'\r') {
                    raw.pop();
                }
            }
            let line = std::str::from_utf8(&raw).map_err(|_| {
                ToolError::new(format!("{} is binary, not a UTF-8 text file.", args.path))
            })?;
            line_no += 1;
            if line_no < start_line {
                continue;
            }
            if selected.len() < max_lines {
                selected.push(format!("{line_no}|{line}"));
            } else {
                remaining += 1;
            }
        }

        if selected.is_empty() {
            return Ok("(no lines in the requested range)".to_string());
        }
        if remaining > 0 {
            selected.push(remaining_line(remaining));
        }
        Ok(selected.join("\n"))
    }

    fn list_directory(&self, args: ListDirectoryArgs) -> Result<String, ToolError> {
        let requested = args.path.as_deref().unwrap_or(".");
        let (dir, relative, _) = self.resolve_readable(requested, true)?;
        let directory = dir.open_dir(&relative).map_err(|error| {
            ToolError::new(format!("Cannot open directory {requested}: {error}"))
        })?;

        let mut entries = Vec::new();
        for entry in directory.entries()? {
            let entry = entry?;
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() {
                name.push('/');
            }
            entries.push(name);
            if entries.len() > MAX_DIRECTORY_ENTRIES {
                break;
            }
        }
        entries.sort_unstable();

        let truncated = entries.len() > MAX_DIRECTORY_ENTRIES;
        entries.truncate(MAX_DIRECTORY_ENTRIES);
        if entries.is_empty() {
            return Ok("(empty directory)".to_string());
        }
        if truncated {
            entries.push("… directory listing truncated".to_string());
        }
        Ok(entries.join("\n"))
    }

    fn search_files(&self, args: SearchFilesArgs) -> Result<String, ToolError> {
        validate_pattern(&args.pattern)?;
        let limit = bounded_limit(
            args.max_results,
            DEFAULT_SEARCH_RESULTS,
            MAX_SEARCH_RESULTS,
            "max_results",
        )?;
        let (dir, start, root) = self.walk_start(args.path.as_deref().unwrap_or("."))?;
        let glob = build_glob_set(args.glob.as_deref())?;
        let mut matcher_builder = RegexMatcherBuilder::new();
        matcher_builder
            .case_insensitive(args.case_insensitive.unwrap_or(false))
            .fixed_strings(args.literal.unwrap_or(false));
        let matcher = matcher_builder
            .build(&args.pattern)
            .map_err(|error| ToolError::new(format!("Invalid search pattern: {error}")))?;

        let mut results = Vec::new();
        let mut truncated = false;
        for entry in walk(
            &start,
            Some(MAX_SEARCH_FILE_BYTES),
            args.include_hidden.unwrap_or(false),
        ) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = match entry.path().strip_prefix(&root) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if !glob_matches(&glob, relative) {
                continue;
            }

            let file = match dir.open(relative) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let shown_path = display_relative(relative);
            let mut searcher = SearcherBuilder::new()
                .line_number(true)
                .binary_detection(BinaryDetection::quit(b'\0'))
                .build();
            searcher
                .search_reader(
                    &matcher,
                    file,
                    UTF8(|line_number, line| {
                        results.push(format!("{shown_path}:{line_number}:{}", clip_line(line)));
                        if results.len() >= limit {
                            truncated = true;
                            return Ok(false);
                        }
                        Ok(true)
                    }),
                )
                .map_err(|error| {
                    ToolError::new(format!("Could not search {shown_path}: {error}"))
                })?;
            if truncated {
                break;
            }
        }

        if results.is_empty() {
            return Ok("(no matches)".to_string());
        }
        if truncated {
            results.push("… search results truncated".to_string());
        }
        Ok(results.join("\n"))
    }

    fn find_files(&self, args: FindFilesArgs) -> Result<String, ToolError> {
        validate_pattern(&args.pattern)?;
        let limit = bounded_limit(
            args.max_results,
            DEFAULT_FIND_RESULTS,
            MAX_FIND_RESULTS,
            "max_results",
        )?;
        let (_dir, start, root) = self.walk_start(args.path.as_deref().unwrap_or("."))?;
        let matcher = Glob::new(&args.pattern)
            .map_err(|error| ToolError::new(format!("Invalid glob pattern: {error}")))?
            .compile_matcher();

        let mut results = Vec::new();
        let mut truncated = false;
        for entry in walk(&start, None, args.include_hidden.unwrap_or(false)) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let relative = match entry.path().strip_prefix(&root) {
                Ok(path) if !path.as_os_str().is_empty() => path,
                _ => continue,
            };
            let shown = display_relative(relative);
            if !matcher.is_match(&shown) {
                continue;
            }
            let result = if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                format!("{shown}/")
            } else {
                shown
            };
            results.push(result);
            if results.len() >= limit {
                truncated = true;
                break;
            }
        }

        if results.is_empty() {
            return Ok("(no matching paths)".to_string());
        }
        if truncated {
            results.push("… path results truncated".to_string());
        }
        Ok(results.join("\n"))
    }

    fn prepare_write(&self, args: WriteFileArgs) -> Result<PreparedMutation, ToolError> {
        if args.content.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "Writes are limited to {MAX_WRITE_BYTES} bytes."
            )));
        }
        let relative = self.writable_path(&args.path)?;
        let before = self.read_optional(&relative)?;
        if before.is_some() && !args.overwrite.unwrap_or(true) {
            return Err(ToolError::new(format!(
                "{} already exists and overwrite is false.",
                args.path
            )));
        }
        Ok(PreparedMutation {
            requested: args.path,
            relative,
            before,
            after: args.content.into_bytes(),
        })
    }

    fn prepare_edit(&self, args: EditFileArgs) -> Result<PreparedMutation, ToolError> {
        if args.old_text.is_empty() {
            return Err(ToolError::new("old_text may not be empty."));
        }
        let relative = self.writable_path(&args.path)?;
        let before = self
            .read_optional(&relative)?
            .ok_or_else(|| ToolError::new(format!("{} does not exist.", args.path)))?;
        let content = String::from_utf8(before.clone())
            .map_err(|_| ToolError::new(format!("{} is not a UTF-8 text file.", args.path)))?;
        let matches = content.matches(&args.old_text).count();
        if matches == 0 {
            return Err(ToolError::new(format!(
                "old_text was not found in {}.",
                args.path
            )));
        }
        let replace_all = args.replace_all.unwrap_or(false);
        if matches > 1 && !replace_all {
            return Err(ToolError::new(format!(
                "old_text occurs {matches} times in {}; provide more context or set replace_all.",
                args.path
            )));
        }
        let edited = if replace_all {
            content.replace(&args.old_text, &args.new_text)
        } else {
            content.replacen(&args.old_text, &args.new_text, 1)
        };
        if edited.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "The edited file would exceed the {MAX_WRITE_BYTES}-byte write limit."
            )));
        }
        let after = edited.into_bytes();
        Ok(PreparedMutation {
            requested: args.path,
            relative,
            before: Some(before),
            after,
        })
    }

    fn commit_mutation(&self, mutation: PreparedMutation) -> Result<String, ToolError> {
        let current = self.read_optional(&mutation.relative)?;
        if current != mutation.before {
            return Err(ToolError::new(format!(
                "{} changed while the edit was being prepared; inspect it again before retrying.",
                mutation.requested
            )));
        }
        let bytes = mutation.after.len();
        let lines = String::from_utf8_lossy(&mutation.after).lines().count();
        self.atomic_write(&mutation.relative, &mutation.after)?;
        Ok(format!(
            "Wrote {bytes} bytes ({lines} lines) to {}.",
            display_relative(&mutation.relative)
        ))
    }

    fn writable_path(&self, requested: &str) -> Result<PathBuf, ToolError> {
        let normalized = self.requested_relative(requested, false)?;
        let file_name = normalized
            .file_name()
            .ok_or_else(|| ToolError::new("Name a file inside the workspace."))?;
        let parent = normalized
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let canonical_parent = self.inner.dir.canonicalize(parent).map_err(|error| {
            ToolError::new(format!("Cannot access the parent of {requested}: {error}"))
        })?;
        let canonical_parent =
            normalize_relative_path(canonical_parent.to_string_lossy().as_ref(), true)?;
        Ok(canonical_parent.join(file_name))
    }

    fn read_optional(&self, relative: &Path) -> Result<Option<Vec<u8>>, ToolError> {
        let metadata = match self.inner.dir.symlink_metadata(relative) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(ToolError::new(
                "Mutating files through symbolic links is not allowed.",
            ));
        }
        if !metadata.is_file() {
            return Err(ToolError::new(format!(
                "{} is not a file.",
                display_relative(relative)
            )));
        }

        let mut file = self.inner.dir.open(relative)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_WRITE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "{} exceeds the {MAX_WRITE_BYTES}-byte mutation limit.",
                display_relative(relative)
            )));
        }
        Ok(Some(bytes))
    }

    fn atomic_write(&self, relative: &Path, content: &[u8]) -> Result<(), ToolError> {
        let parent = relative
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = relative
            .file_name()
            .ok_or_else(|| ToolError::new("Name a file inside the workspace."))?;
        let parent_dir = self.inner.dir.open_dir(parent)?;
        let permissions = parent_dir
            .symlink_metadata(file_name)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.permissions());

        let mut temporary = None;
        for _ in 0..16 {
            let candidate = format!(
                ".toad-{}-{}.tmp",
                std::process::id(),
                NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
            );
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            match parent_dir.open_with(&candidate, &options) {
                Ok(file) => {
                    temporary = Some((candidate, file));
                    break;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        let (temporary_name, mut file) =
            temporary.ok_or_else(|| ToolError::other("Could not allocate a temporary file."))?;

        let result = (|| -> Result<(), ToolError> {
            file.write_all(content)?;
            file.sync_all()?;
            drop(file);
            if let Some(permissions) = permissions {
                parent_dir.set_permissions(&temporary_name, permissions)?;
            }
            parent_dir.rename(&temporary_name, &parent_dir, file_name)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = parent_dir.remove_file(&temporary_name);
        }
        result
    }

    fn walk_start(&self, requested: &str) -> Result<(Dir, PathBuf, PathBuf), ToolError> {
        let (dir, relative, root) = self.resolve_readable(requested, true)?;
        Ok((dir, root.join(relative), root))
    }
}

#[derive(Debug)]
struct PreparedMutation {
    requested: String,
    relative: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}

fn normalize_relative_path(requested: &str, allow_root: bool) -> Result<PathBuf, ToolError> {
    if requested.contains('\0') {
        return Err(ToolError::new("Paths may not contain NUL bytes."));
    }
    let path = Path::new(requested);
    if path.is_absolute() {
        return Err(ToolError::new(
            "Use a path relative to the active workspace.",
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                validate_platform_component(part.to_string_lossy().as_ref())?;
                normalized.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ToolError::new(
                    "Parent path components are not allowed inside the workspace.",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::new(
                    "Use a path relative to the active workspace.",
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        if allow_root {
            Ok(PathBuf::from("."))
        } else {
            Err(ToolError::new("Name a file inside the workspace."))
        }
    } else {
        Ok(normalized)
    }
}

#[cfg(windows)]
fn validate_platform_component(component: &str) -> Result<(), ToolError> {
    let trimmed = component.trim_end_matches(['.', ' ']);
    let stem = trimmed.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if component.contains(':') || trimmed != component || reserved {
        return Err(ToolError::new(
            "The path contains a Windows-reserved component.",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_platform_component(_component: &str) -> Result<(), ToolError> {
    Ok(())
}

fn walk(start: &Path, max_filesize: Option<u64>, include_hidden: bool) -> ignore::Walk {
    let mut builder = WalkBuilder::new(start);
    builder
        .hidden(!include_hidden)
        .parents(false)
        .follow_links(false)
        .max_filesize(max_filesize)
        .sort_by_file_path(|left, right| left.cmp(right));
    builder.build()
}

fn build_glob_set(pattern: Option<&str>) -> Result<Option<GlobSet>, ToolError> {
    let Some(pattern) = pattern else {
        return Ok(None);
    };
    validate_pattern(pattern)?;
    let mut builder = GlobSetBuilder::new();
    builder.add(
        Glob::new(pattern)
            .map_err(|error| ToolError::new(format!("Invalid glob pattern: {error}")))?,
    );
    builder
        .build()
        .map(Some)
        .map_err(|error| ToolError::new(format!("Invalid glob pattern: {error}")))
}

fn glob_matches(glob: &Option<GlobSet>, relative: &Path) -> bool {
    glob.as_ref()
        .is_none_or(|matcher| matcher.is_match(display_relative(relative)))
}

fn validate_pattern(pattern: &str) -> Result<(), ToolError> {
    let length = pattern.chars().count();
    if length == 0 {
        return Err(ToolError::new("The pattern may not be empty."));
    }
    if length > MAX_PATTERN_CHARS {
        return Err(ToolError::new(format!(
            "Patterns are limited to {MAX_PATTERN_CHARS} characters."
        )));
    }
    Ok(())
}

fn bounded_limit(
    requested: Option<usize>,
    default: usize,
    maximum: usize,
    name: &str,
) -> Result<usize, ToolError> {
    let value = requested.unwrap_or(default);
    if value == 0 || value > maximum {
        return Err(ToolError::new(format!(
            "{name} must be between 1 and {maximum}."
        )));
    }
    Ok(value)
}

fn display_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn clip_line(line: &str) -> String {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut clipped = line.chars().take(MAX_SEARCH_LINE_CHARS).collect::<String>();
    if line.chars().count() > MAX_SEARCH_LINE_CHARS {
        clipped.push('…');
    }
    clipped
}

fn remaining_line(count: usize) -> String {
    match count {
        1 => "1 line remains".to_string(),
        n => format!("{n} lines remain"),
    }
}

#[derive(Deserialize)]
pub struct ListDirectoryArgs {
    path: Option<String>,
}

pub struct ListDirectory {
    workspace: Workspace,
}

impl ListDirectory {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for ListDirectory {
    const NAME: &'static str = "ls";
    type Error = ToolError;
    type Args = ListDirectoryArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "List files and directories at a path. Directories end with a slash. {}",
            self.workspace.paths_reach()
        )
        .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory to list. Defaults to the working directory."
                }
            },
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.list_directory(args))
            .await
            .map_err(|error| ToolError::new(format!("Directory task failed: {error}")))?
    }
}

#[derive(Deserialize)]
pub struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    max_lines: Option<usize>,
}

pub struct ReadFile {
    workspace: Workspace,
}

impl ReadFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for ReadFile {
    const NAME: &'static str = "read";
    type Error = ToolError;
    type Args = ReadFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Read a UTF-8 text file with line numbers; each call answers a slice (default {DEFAULT_READ_LINES} lines) and says how many lines remain, and binary content is refused. {}",
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to read."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First line to return, starting at 1."
                },
                "max_lines": {
                    "type": "integer",
                    "minimum": 1,
                    "description": format!("Lines to return from start_line. Defaults to {DEFAULT_READ_LINES}.")
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.read_file(args))
            .await
            .map_err(|error| ToolError::new(format!("File task failed: {error}")))?
    }
}

#[derive(Deserialize)]
pub struct SearchFilesArgs {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    case_insensitive: Option<bool>,
    literal: Option<bool>,
    max_results: Option<usize>,
    include_hidden: Option<bool>,
}

pub struct SearchFiles {
    workspace: Workspace,
}

impl SearchFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for SearchFiles {
    const NAME: &'static str = "grep";
    type Error = ToolError;
    type Args = SearchFilesArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Search text files under a directory with a regular expression or literal string. Respects ignore files and returns path, line number, and matching line. {}",
            self.workspace.paths_reach()
        )
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression to search for, or text when literal is true."
                },
                "path": {
                    "type": "string",
                    "description": "The file or directory to search. Defaults to the working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob limiting searched files, for example '**/*.rs'."
                },
                "case_insensitive": { "type": "boolean" },
                "literal": { "type": "boolean" },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SEARCH_RESULTS
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Search hidden files and directories. Defaults to false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.search_files(args))
            .await
            .map_err(|error| ToolError::new(format!("Search task failed: {error}")))?
    }
}

#[derive(Deserialize)]
pub struct FindFilesArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
    include_hidden: Option<bool>,
}

pub struct FindFiles {
    workspace: Workspace,
}

impl FindFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for FindFiles {
    const NAME: &'static str = "glob";
    type Error = ToolError;
    type Args = FindFilesArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Find files and directories by glob under a directory. Respects ignore files and does not follow directory symlinks. {}",
            self.workspace.paths_reach()
        )
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob matched against paths under the searched directory, for example '**/*.rs'."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search. Defaults to the working directory."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_FIND_RESULTS
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Include hidden files and directories. Defaults to false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.find_files(args))
            .await
            .map_err(|error| ToolError::new(format!("Find task failed: {error}")))?
    }
}

#[derive(Deserialize)]
pub struct WriteFileArgs {
    path: String,
    content: String,
    overwrite: Option<bool>,
}

pub struct WriteFile {
    workspace: Workspace,
}

impl WriteFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for WriteFile {
    const NAME: &'static str = "write";
    type Error = ToolError;
    type Args = WriteFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Create or replace a UTF-8 file of up to {} MiB and say how many bytes and lines were written. {}",
            MAX_WRITE_BYTES / (1024 * 1024),
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Complete new UTF-8 file content."
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Allow replacing an existing file. Defaults to true."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || {
            let mutation = workspace.prepare_write(args)?;
            workspace.commit_mutation(mutation)
        })
        .await
        .map_err(|error| ToolError::other(format!("Write task failed: {error}")))?
    }
}

#[derive(Deserialize)]
pub struct EditFileArgs {
    path: String,
    old_text: String,
    new_text: String,
    replace_all: Option<bool>,
}

pub struct EditFile {
    workspace: Workspace,
}

impl EditFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for EditFile {
    const NAME: &'static str = "edit";
    type Error = ToolError;
    type Args = EditFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Edit a UTF-8 text file by replacing exact text; a non-unique match is rejected unless replace_all is true, writes are capped at {} MiB, and the result says how many bytes and lines were written. {}",
            MAX_WRITE_BYTES / (1024 * 1024),
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to edit."
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to replace. Include enough context to make it unique."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every exact match. Defaults to false."
                }
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || {
            let mutation = workspace.prepare_edit(args)?;
            workspace.commit_mutation(mutation)
        })
        .await
        .map_err(|error| ToolError::other(format!("Edit task failed: {error}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_READ_LINES, EditFile, EditFileArgs, FindFiles, FindFilesArgs, ListDirectory, Reach,
        ReadFile, ReadFileArgs, SearchFiles, SearchFilesArgs, Workspace, WriteFile, WriteFileArgs,
        normalize_relative_path,
    };
    use crate::tools::RunCommand;
    use rig::tool::Tool;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("toad-tools-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn opened(cwd: impl AsRef<Path>, reach: Reach) -> Workspace {
        let cwd = cwd.as_ref();
        Workspace::open(
            cwd.to_path_buf(),
            reach,
            cwd.with_extension("overflow-unused"),
        )
        .unwrap()
    }

    #[test]
    fn file_reads_are_line_numbered_and_say_what_remains() {
        let directory = TestDirectory::new();
        fs::write(
            directory.path().join("notes.txt"),
            "one\ntwo\nthree\nfour\n",
        )
        .unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);
        let result = workspace
            .read_file(ReadFileArgs {
                path: "notes.txt".to_string(),
                start_line: Some(2),
                max_lines: Some(2),
            })
            .unwrap();
        assert_eq!(result, "2|two\n3|three\n1 line remains");
    }

    #[test]
    fn a_read_of_a_file_longer_than_the_default_slice_answers_the_slice_and_the_remainder() {
        let directory = TestDirectory::new();
        let total = DEFAULT_READ_LINES + 5;
        let mut content = String::new();
        for index in 1..=total {
            content.push_str(&format!("line-{index}\n"));
        }
        fs::write(directory.path().join("long.txt"), content).unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);
        let result = workspace
            .read_file(ReadFileArgs {
                path: "long.txt".to_string(),
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), DEFAULT_READ_LINES + 1);
        assert_eq!(lines[0], "1|line-1");
        assert_eq!(
            lines[DEFAULT_READ_LINES - 1],
            format!("{DEFAULT_READ_LINES}|line-{DEFAULT_READ_LINES}")
        );
        assert_eq!(lines[DEFAULT_READ_LINES], "5 lines remain");
        assert!(!result.contains(&format!(
            "{}|line-{}",
            DEFAULT_READ_LINES + 1,
            DEFAULT_READ_LINES + 1
        )));
    }

    #[test]
    fn a_binary_file_is_refused_with_a_sentence() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("blob.bin"), [b'a', 0, b'b']).unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);
        let error = workspace
            .read_file(ReadFileArgs {
                path: "blob.bin".to_string(),
                start_line: None,
                max_lines: None,
            })
            .unwrap_err();
        assert!(error.to_string().contains("binary"), "{}", error);
    }

    #[test]
    fn parent_paths_are_rejected() {
        let result = normalize_relative_path("../secret", false);
        assert!(result.is_err());
    }

    #[test]
    fn workspace_reads_and_searches_files() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("src/lib.rs"),
            "fn alpha() {}\n// needle\n",
        )
        .unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);

        let read = workspace
            .read_file(ReadFileArgs {
                path: "src/lib.rs".to_string(),
                start_line: Some(2),
                max_lines: Some(1),
            })
            .unwrap();
        assert_eq!(read, "2|// needle");

        let searched = workspace
            .search_files(SearchFilesArgs {
                pattern: "needle".to_string(),
                path: None,
                glob: Some("**/*.rs".to_string()),
                case_insensitive: None,
                literal: Some(true),
                max_results: None,
                include_hidden: None,
            })
            .unwrap();
        assert!(searched.contains("src/lib.rs:2:// needle"));

        let found = workspace
            .find_files(FindFilesArgs {
                pattern: "**/*.rs".to_string(),
                path: None,
                max_results: None,
                include_hidden: None,
            })
            .unwrap();
        assert_eq!(found, "src/lib.rs");
    }

    #[test]
    fn workspace_prepares_and_commits_atomic_mutations() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("notes.txt"), "before\n").unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);

        let edit = workspace
            .prepare_edit(EditFileArgs {
                path: "notes.txt".to_string(),
                old_text: "before".to_string(),
                new_text: "after".to_string(),
                replace_all: None,
            })
            .unwrap();
        workspace.commit_mutation(edit).unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("notes.txt")).unwrap(),
            "after\n"
        );

        let write = workspace
            .prepare_write(WriteFileArgs {
                path: "new.txt".to_string(),
                content: "created\n".to_string(),
                overwrite: None,
            })
            .unwrap();
        let wrote = workspace.commit_mutation(write).unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("new.txt")).unwrap(),
            "created\n"
        );
        assert!(
            wrote.contains("bytes") && wrote.contains("lines"),
            "{wrote}"
        );
    }

    #[test]
    fn write_of_a_large_file_succeeds() {
        let directory = TestDirectory::new();
        let workspace = opened(directory.path(), Reach::Workspace);
        let content = "hello world\n".repeat(30_000);
        let bytes = content.len();
        let lines = content.lines().count();
        let write = workspace
            .prepare_write(WriteFileArgs {
                path: "big.txt".to_string(),
                content: content.clone(),
                overwrite: None,
            })
            .unwrap();
        let result = workspace.commit_mutation(write).unwrap();
        assert!(
            result.contains(&format!("{bytes} bytes"))
                && result.contains(&format!("{lines} lines")),
            "{result}"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("big.txt")).unwrap(),
            content
        );
    }

    #[test]
    fn descriptions_mention_no_stale_limits() {
        let directory = TestDirectory::new();
        let workspace = opened(directory.path(), Reach::Workspace);
        let descriptions = [
            ListDirectory::new(workspace.clone()).description(),
            ReadFile::new(workspace.clone()).description(),
            SearchFiles::new(workspace.clone()).description(),
            FindFiles::new(workspace.clone()).description(),
            WriteFile::new(workspace.clone()).description(),
            EditFile::new(workspace.clone()).description(),
            RunCommand::new(workspace).description(),
        ];
        for text in descriptions {
            for stale in ["256 KiB", "16 KiB", "50 KiB", "400 lines", "200 lines"] {
                assert!(
                    !text.contains(stale),
                    "stale limit {stale:?} still in: {text}"
                );
            }
        }
    }

    #[test]
    fn exact_edit_rejects_ambiguous_matches() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("notes.txt"), "same\nsame\n").unwrap();
        let workspace = opened(directory.path(), Reach::Workspace);

        let result = workspace.prepare_edit(EditFileArgs {
            path: "notes.txt".to_string(),
            old_text: "same".to_string(),
            new_text: "changed".to_string(),
            replace_all: None,
        });

        assert!(result.unwrap_err().to_string().contains("occurs 2 times"));
    }

    #[test]
    fn reaching_the_machine_resolves_absolute_paths_and_parents() {
        let directory = TestDirectory::new();
        let inside = directory.path().join("inside");
        fs::create_dir_all(&inside).unwrap();
        fs::write(directory.path().join("outside.txt"), "outside\n").unwrap();

        let confined = opened(inside.clone(), Reach::Workspace);
        assert!(
            confined
                .read_file(ReadFileArgs {
                    path: "../outside.txt".into(),
                    start_line: None,
                    max_lines: None
                })
                .is_err()
        );

        let open = opened(inside, Reach::Machine);
        let by_parent = open
            .read_file(ReadFileArgs {
                path: "../outside.txt".into(),
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        assert!(by_parent.contains("outside"));
        let absolute = directory
            .path()
            .join("outside.txt")
            .to_string_lossy()
            .to_string();
        let by_absolute = open
            .read_file(ReadFileArgs {
                path: absolute,
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        assert!(by_absolute.contains("outside"));
    }

    #[test]
    fn workspace_reach_can_read_the_teammates_overflow_directory() {
        let directory = TestDirectory::new();
        let overflow = TestDirectory::new();
        fs::write(overflow.path().join("call-1.txt"), "spilled\nneedle\n").unwrap();
        let workspace = Workspace::open(
            directory.path().to_path_buf(),
            Reach::Workspace,
            overflow.path().to_path_buf(),
        )
        .unwrap();

        let spilled = overflow
            .path()
            .join("call-1.txt")
            .to_string_lossy()
            .into_owned();
        let read = workspace
            .read_file(ReadFileArgs {
                path: spilled,
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        assert!(read.contains("spilled"), "{read}");

        let searched = workspace
            .search_files(SearchFilesArgs {
                pattern: "needle".to_string(),
                path: Some(overflow.path().to_string_lossy().into_owned()),
                glob: None,
                case_insensitive: None,
                literal: Some(true),
                max_results: None,
                include_hidden: None,
            })
            .unwrap();
        assert!(searched.contains("needle"), "{searched}");

        let write = workspace.prepare_write(WriteFileArgs {
            path: overflow
                .path()
                .join("nope.txt")
                .to_string_lossy()
                .into_owned(),
            content: "x\n".to_string(),
            overwrite: None,
        });
        let outside = workspace.prepare_write(WriteFileArgs {
            path: directory
                .path()
                .parent()
                .unwrap()
                .join("elsewhere.txt")
                .to_string_lossy()
                .into_owned(),
            content: "x\n".to_string(),
            overwrite: None,
        });
        assert_eq!(
            write.unwrap_err().to_string(),
            outside.unwrap_err().to_string()
        );

        let elsewhere = TestDirectory::new();
        fs::write(elsewhere.path().join("secret.txt"), "nope\n").unwrap();
        assert!(
            workspace
                .read_file(ReadFileArgs {
                    path: elsewhere
                        .path()
                        .join("secret.txt")
                        .to_string_lossy()
                        .into_owned(),
                    start_line: None,
                    max_lines: None,
                })
                .is_err()
        );

        let description = ReadFile::new(workspace).description();
        assert!(
            description.contains(&overflow.path().display().to_string())
                && description.contains("long tool result"),
            "{description}"
        );
    }

    #[test]
    fn a_missing_overflow_directory_is_a_refusal_not_a_panic() {
        let directory = TestDirectory::new();
        let missing = directory.path().with_extension("does-not-exist-overflow");
        assert!(!missing.exists());
        let workspace = Workspace::open(
            directory.path().to_path_buf(),
            Reach::Workspace,
            missing.clone(),
        )
        .unwrap();
        let error = workspace
            .read_file(ReadFileArgs {
                path: missing.join("call-1.txt").to_string_lossy().into_owned(),
                start_line: None,
                max_lines: None,
            })
            .unwrap_err();
        assert!(!error.to_string().is_empty(), "{error}");
    }

    #[test]
    fn machine_reach_ignores_the_overflow_root() {
        let directory = TestDirectory::new();
        let inside = directory.path().join("inside");
        let overflow = directory.path().join("overflow");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&overflow).unwrap();
        fs::write(directory.path().join("outside.txt"), "outside\n").unwrap();
        fs::write(overflow.join("call.txt"), "spilled\n").unwrap();

        let workspace = Workspace::open(inside, Reach::Machine, overflow.clone()).unwrap();
        let by_parent = workspace
            .read_file(ReadFileArgs {
                path: "../outside.txt".into(),
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        assert!(by_parent.contains("outside"));

        let spilled = workspace
            .read_file(ReadFileArgs {
                path: overflow.join("call.txt").to_string_lossy().into_owned(),
                start_line: None,
                max_lines: None,
            })
            .unwrap();
        assert!(spilled.contains("spilled"));

        let description = ReadFile::new(workspace).description();
        assert!(!description.contains("long tool result"), "{description}");
    }
}
