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
    io::{ErrorKind, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_FILE_BYTES: u64 = 256 * 1024;
const DEFAULT_READ_LINES: usize = 200;
const MAX_READ_LINES: usize = 400;
const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const DEFAULT_SEARCH_RESULTS: usize = 100;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCH_LINE_CHARS: usize = 500;
const DEFAULT_FIND_RESULTS: usize = 500;
const MAX_FIND_RESULTS: usize = 1_000;
const MAX_PATTERN_CHARS: usize = 1_000;
const MAX_TOOL_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_WRITE_BYTES: usize = 256 * 1024;
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
}

#[derive(Clone)]
pub struct Workspace {
    inner: Arc<WorkspaceInner>,
}

impl Workspace {
    pub fn open(cwd: PathBuf, reach: Reach) -> Result<Self, ToolError> {
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
            }),
        })
    }

    pub fn display_root(&self) -> &Path {
        &self.inner.cwd
    }

    /// Where a path may point, as the tools describe it to the model. The
    /// description is the policy the model reads, so it has to tell the truth
    /// about the wall — or the absence of one.
    fn paths_reach(&self) -> &'static str {
        match self.inner.reach {
            Reach::Workspace => "Paths are relative to the working directory and may not leave it.",
            Reach::Machine => {
                "Paths may be absolute or relative to the working directory; anywhere on this machine is allowed."
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

    fn read_file(&self, args: ReadFileArgs) -> Result<String, ToolError> {
        let relative = self.canonical_subpath(&args.path, false)?;
        let mut file = self
            .inner
            .dir
            .open(&relative)
            .map_err(|error| ToolError::new(format!("Cannot open {}: {error}", args.path)))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ToolError::new(format!("{} is not a file.", args.path)));
        }

        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(ToolError::new(format!(
                "{} exceeds the {MAX_FILE_BYTES}-byte read limit.",
                args.path
            )));
        }

        let content = String::from_utf8(bytes)
            .map_err(|_| ToolError::new(format!("{} is not a UTF-8 text file.", args.path)))?;
        render_lines(
            &content,
            args.start_line.unwrap_or(1),
            args.max_lines.unwrap_or(DEFAULT_READ_LINES),
        )
    }

    fn list_directory(&self, args: ListDirectoryArgs) -> Result<String, ToolError> {
        let requested = args.path.as_deref().unwrap_or(".");
        let relative = self.canonical_subpath(requested, true)?;
        let directory = self.inner.dir.open_dir(&relative).map_err(|error| {
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

        let mut truncated = entries.len() > MAX_DIRECTORY_ENTRIES;
        entries.truncate(MAX_DIRECTORY_ENTRIES);
        if entries.is_empty() {
            return Ok("(empty directory)".to_string());
        }
        let mut output = Vec::new();
        let mut output_bytes = 0;
        for entry in entries {
            if !push_bounded(&mut output, &mut output_bytes, entry) {
                truncated = true;
                break;
            }
        }
        if truncated {
            push_truncation_notice(&mut output, "… directory listing truncated");
        }
        Ok(output.join("\n"))
    }

    fn search_files(&self, args: SearchFilesArgs) -> Result<String, ToolError> {
        validate_pattern(&args.pattern)?;
        let limit = bounded_limit(
            args.max_results,
            DEFAULT_SEARCH_RESULTS,
            MAX_SEARCH_RESULTS,
            "max_results",
        )?;
        let start = self.walk_start(args.path.as_deref().unwrap_or("."))?;
        let glob = build_glob_set(args.glob.as_deref())?;
        let mut matcher_builder = RegexMatcherBuilder::new();
        matcher_builder
            .case_insensitive(args.case_insensitive.unwrap_or(false))
            .fixed_strings(args.literal.unwrap_or(false));
        let matcher = matcher_builder
            .build(&args.pattern)
            .map_err(|error| ToolError::new(format!("Invalid search pattern: {error}")))?;

        let mut results = Vec::new();
        let mut output_bytes = 0;
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
            let relative = match entry.path().strip_prefix(&self.inner.root) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if !glob_matches(&glob, relative) {
                continue;
            }

            let file = match self.inner.dir.open(relative) {
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
                        let result = format!("{shown_path}:{line_number}:{}", clip_line(line));
                        if !push_bounded(&mut results, &mut output_bytes, result)
                            || results.len() >= limit
                        {
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
            push_truncation_notice(&mut results, "… search results truncated");
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
        let start = self.walk_start(args.path.as_deref().unwrap_or("."))?;
        let matcher = Glob::new(&args.pattern)
            .map_err(|error| ToolError::new(format!("Invalid glob pattern: {error}")))?
            .compile_matcher();

        let mut results = Vec::new();
        let mut output_bytes = 0;
        let mut truncated = false;
        for entry in walk(&start, None, args.include_hidden.unwrap_or(false)) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let relative = match entry.path().strip_prefix(&self.inner.root) {
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
            if !push_bounded(&mut results, &mut output_bytes, result) || results.len() >= limit {
                truncated = true;
                break;
            }
        }

        if results.is_empty() {
            return Ok("(no matching paths)".to_string());
        }
        if truncated {
            push_truncation_notice(&mut results, "… path results truncated");
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
        self.atomic_write(&mutation.relative, &mutation.after)?;
        Ok(format!(
            "Wrote {bytes} bytes to {}.",
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

    fn walk_start(&self, requested: &str) -> Result<PathBuf, ToolError> {
        let relative = self.canonical_subpath(requested, true)?;
        Ok(self.inner.root.join(relative))
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

fn push_bounded(lines: &mut Vec<String>, bytes: &mut usize, line: String) -> bool {
    let needed = line.len() + usize::from(!lines.is_empty());
    if *bytes + needed > MAX_TOOL_OUTPUT_BYTES {
        return false;
    }
    *bytes += needed;
    lines.push(line);
    true
}

fn push_truncation_notice(lines: &mut Vec<String>, notice: &str) {
    while !lines.is_empty()
        && lines.iter().map(String::len).sum::<usize>() + lines.len() + notice.len()
            > MAX_TOOL_OUTPUT_BYTES
    {
        lines.pop();
    }
    lines.push(notice.to_string());
}

fn render_lines(content: &str, start_line: usize, max_lines: usize) -> Result<String, ToolError> {
    if start_line == 0 {
        return Err(ToolError::new("start_line must be at least 1."));
    }
    if max_lines == 0 || max_lines > MAX_READ_LINES {
        return Err(ToolError::new(format!(
            "max_lines must be between 1 and {MAX_READ_LINES}."
        )));
    }

    let mut lines = Vec::new();
    let mut output_bytes = 0;
    let mut truncated = false;
    for (index, line) in content
        .lines()
        .enumerate()
        .skip(start_line - 1)
        .take(max_lines)
    {
        if !push_bounded(
            &mut lines,
            &mut output_bytes,
            format!("{}|{line}", index + 1),
        ) {
            truncated = true;
            break;
        }
    }
    if truncated {
        push_truncation_notice(&mut lines, "… file output truncated");
    }
    let rendered = lines.join("\n");

    Ok(if rendered.is_empty() {
        "(no lines in the requested range)".to_string()
    } else {
        rendered
    })
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
            "Read a UTF-8 text file with line numbers. Files are limited to {} KiB and each call returns at most {MAX_READ_LINES} lines. {}",
            MAX_FILE_BYTES / 1024,
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
                    "maximum": MAX_READ_LINES,
                    "description": "Maximum number of lines to return."
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
            "Create or replace a UTF-8 file. {}",
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
            "Edit a UTF-8 text file by replacing exact text. A non-unique match is rejected unless replace_all is true. {}",
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
        EditFileArgs, FindFilesArgs, Reach, ReadFileArgs, SearchFilesArgs, Workspace,
        WriteFileArgs, normalize_relative_path, render_lines,
    };
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

    #[test]
    fn file_reads_are_line_numbered_and_bounded() {
        let result = render_lines("one\ntwo\nthree\nfour", 2, 2).unwrap();
        assert_eq!(result, "2|two\n3|three");
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
        let workspace = Workspace::open(directory.path().to_path_buf(), Reach::Workspace).unwrap();

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
        let workspace = Workspace::open(directory.path().to_path_buf(), Reach::Workspace).unwrap();

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
        workspace.commit_mutation(write).unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("new.txt")).unwrap(),
            "created\n"
        );
    }

    #[test]
    fn exact_edit_rejects_ambiguous_matches() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("notes.txt"), "same\nsame\n").unwrap();
        let workspace = Workspace::open(directory.path().to_path_buf(), Reach::Workspace).unwrap();

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

        let confined = Workspace::open(inside.clone(), Reach::Workspace).unwrap();
        assert!(
            confined
                .read_file(ReadFileArgs {
                    path: "../outside.txt".into(),
                    start_line: None,
                    max_lines: None
                })
                .is_err()
        );

        let open = Workspace::open(inside, Reach::Machine).unwrap();
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
}
