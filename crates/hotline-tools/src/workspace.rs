use super::ToolError;
use crate::capability::CapabilityLease;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};
use hotline_contract::{FolderGrant, Reach};
use ignore::WalkBuilder;
use serde::Deserialize;
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
    /// The path the caller supplied before `cwd` was canonicalized. ACP
    /// clients may echo that spelling in an absolute callback path (for
    /// example, when the selected directory is a symlink), so accepting it
    /// here still resolves the final path through the canonical capability.
    callback_cwd: PathBuf,
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
    /// The person's extra folders, opened once. Only workspace reach has
    /// them: whole-machine reach already opens everything from the root.
    folders: Vec<Folder>,
    capability: Option<CapabilityLease>,
}

/// One granted folder, held as its own cap-std handle so a path resolved in
/// it is confined to it exactly as a workspace path is to the workspace.
struct Folder {
    /// Its canonical path: an absolute request reaches the folder only by
    /// naming this.
    path: PathBuf,
    dir: Dir,
    writable: bool,
}

/// Which handle a resolved path is inside: the working directory (or, with
/// whole-machine reach, the root), or one granted folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Root,
    Folder(usize),
}

#[derive(Clone)]
pub struct Workspace {
    inner: Arc<WorkspaceInner>,
}

impl Workspace {
    pub fn open(cwd: PathBuf, reach: Reach, overflow: PathBuf) -> Result<Self, ToolError> {
        Self::open_with_capability(cwd, reach, overflow, &[], None)
    }

    pub fn open_with_capability(
        cwd: PathBuf,
        reach: Reach,
        overflow: PathBuf,
        folders: &[FolderGrant],
        capability: Option<CapabilityLease>,
    ) -> Result<Self, ToolError> {
        let callback_cwd = if cwd.is_absolute() {
            cwd.clone()
        } else {
            std::env::current_dir().map_or_else(|_| cwd.clone(), |directory| directory.join(&cwd))
        };
        let cwd = cwd.canonicalize().map_err(|error| {
            ToolError::new(format!(
                "The Hotline workspace {} is unavailable: {error}",
                cwd.display()
            ))
        })?;
        let root = match reach {
            Reach::Workspace => cwd.clone(),
            Reach::Machine => cwd
                .ancestors()
                .last()
                .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
        };
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|error| {
            ToolError::new(format!(
                "The Hotline workspace {} could not be opened: {error}",
                root.display()
            ))
        })?;
        let folders = match reach {
            Reach::Workspace => open_folders(folders, &cwd),
            Reach::Machine => Vec::new(),
        };
        Ok(Self {
            inner: Arc::new(WorkspaceInner {
                cwd,
                callback_cwd,
                root,
                dir,
                reach,
                overflow,
                folders,
                capability,
            }),
        })
    }

    /// The folders this handle opened, with whether each may be changed, for
    /// the shell to grant the same set.
    pub(crate) fn folders(&self) -> Vec<(PathBuf, bool)> {
        self.inner
            .folders
            .iter()
            .map(|folder| (folder.path.clone(), folder.writable))
            .collect()
    }

    /// Checks the authority immediately before a built-in operation starts.
    /// Cloned tools retain the same lease, so changing reach or revoking a
    /// session closes every already-built workspace too.
    pub fn check_capability(&self) -> Result<(), ToolError> {
        self.inner.capability.as_ref().map_or(Ok(()), |capability| {
            capability.check().map_err(ToolError::permission_denied)
        })
    }

    pub fn display_root(&self) -> &Path {
        &self.inner.cwd
    }

    /// The teammate's one policy, so the shell can confine a command or not.
    pub fn reach(&self) -> Reach {
        self.inner.reach
    }

    /// Reads a text file named by a protocol callback. ACP gives the client
    /// an absolute path, while the built-in tools deliberately take relative
    /// paths, so this adapter accepts an absolute path only when it names a
    /// file inside this workspace. Resolution still goes through the same
    /// canonical cap-std directory as every other read.
    pub fn read_text_path(
        &self,
        path: &Path,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, ToolError> {
        self.check_capability()?;
        let requested = self.callback_path(path)?;
        // Protocol callbacks have no overflow namespace: they are the
        // harness's view of the workspace itself, never a way to reach a
        // result spill directory owned by Hotline.
        let (place, relative) = self.canonical_subpath(&requested, false)?;
        let file = self.dir(place).open(&relative)?;
        let mut bytes = Vec::new();
        file.take(MAX_WRITE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "{} exceeds the {MAX_WRITE_BYTES}-byte read limit.",
                path.display()
            )));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| ToolError::new(format!("{} is not a UTF-8 text file.", path.display())))?;
        if line.is_none() && limit.is_none() {
            return Ok(text);
        }
        let from = line.unwrap_or(1).saturating_sub(1) as usize;
        let lines: Vec<&str> = text.split('\n').skip(from).collect();
        let kept = match limit {
            Some(limit) => &lines[..lines.len().min(limit as usize)],
            None => &lines[..],
        };
        Ok(kept.join("\n"))
    }

    /// Whether the entry at `path` is itself a symbolic link, without following
    /// it. Absent is not a link. Lets a caller that preserves handwritten files
    /// leave a linked one alone instead of refusing it.
    pub fn is_symlink_path(&self, path: &Path) -> Result<bool, ToolError> {
        self.check_capability()?;
        let requested = self.callback_path(path)?;
        let (place, relative) = self.locate(&requested, false)?;
        match self.dir(place).symlink_metadata(&relative) {
            Ok(metadata) => Ok(metadata.file_type().is_symlink()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Reads an optional managed file without following its final symlink.
    /// The distinction between absent and inaccessible matters to callers
    /// that preserve handwritten files, such as the ACP identity materializer.
    pub fn read_text_if_exists_path(&self, path: &Path) -> Result<Option<String>, ToolError> {
        self.check_capability()?;
        let requested = self.callback_path(path)?;
        let (place, relative) = self.locate(&requested, false)?;
        let Some(bytes) = self.read_optional(place, &relative)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes)
            .map_err(|_| ToolError::new(format!("{} is not a UTF-8 text file.", path.display())))?;
        Ok(Some(text))
    }

    /// Atomically writes a protocol callback file through the same mutation
    /// checks as the built-in write tool. Parent directories are made one at
    /// a time after rejecting symlinks, so an ACP request cannot create a
    /// directory through a link that points outside the workspace.
    pub fn write_text_path(&self, path: &Path, content: &str) -> Result<(), ToolError> {
        self.check_capability()?;
        if content.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "Writes are limited to {MAX_WRITE_BYTES} bytes."
            )));
        }
        let requested = self.callback_path(path)?;
        let (place, relative) = self.locate(&requested, false)?;
        // Refused before a parent directory is made, so a read-only folder
        // never gains an empty directory from a write it then refuses.
        self.require_writable(place, &requested)?;
        if let Some(parent) = relative.parent() {
            self.ensure_parent(place, parent)?;
        }
        let mutation = self.prepare_write(WriteFileArgs {
            path: requested,
            content: content.to_string(),
            overwrite: Some(true),
        })?;
        self.commit_mutation(mutation).map(|_| ())
    }

    /// Converts the path vocabulary of an external protocol to this
    /// Workspace's relative vocabulary. A Workspace never accepts a path
    /// that is merely lexically outside and hopes canonicalization will bring
    /// it back; callers have to name a path under the captured root.
    /// An absolute path in a granted folder is kept absolute, which is how
    /// [`Self::locate`] tells it is the folder's.
    fn callback_path(&self, path: &Path) -> Result<String, ToolError> {
        let relative = if self.inner.reach == Reach::Workspace && path.is_absolute() {
            match path
                .strip_prefix(&self.inner.cwd)
                .or_else(|_| path.strip_prefix(&self.inner.callback_cwd))
            {
                Ok(relative) => relative,
                Err(_) if self.folder_of(path).is_some() => path,
                Err(_) => {
                    return Err(ToolError::permission_denied(
                        "The ACP file callback may only access the teammate's workspace and the folders granted to it.",
                    ));
                }
            }
        } else {
            path
        };
        let relative = relative
            .to_str()
            .ok_or_else(|| ToolError::new("ACP file callback paths must be valid UTF-8."))?;
        if relative.is_empty() {
            return Err(ToolError::new("Name a file inside the workspace."));
        }
        Ok(relative.to_string())
    }

    fn ensure_parent(&self, place: Place, parent: &Path) -> Result<(), ToolError> {
        let dir = self.dir(place);
        let mut current = PathBuf::new();
        for component in parent.components() {
            let Component::Normal(part) = component else {
                continue;
            };
            validate_platform_component(part.to_string_lossy().as_ref())?;
            current.push(part);
            match dir.symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ToolError::new(
                        "Creating files through symbolic links is not allowed.",
                    ));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(ToolError::new(format!(
                        "{} is not a directory.",
                        display_relative(&current)
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    dir.create_dir(&current)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Where a path may point, as the tools describe it to the model. The
    /// description is the policy the model reads, so it has to tell the truth
    /// about the wall — or the absence of one. The overflow directory is
    /// named here so every tool that quotes this sentence says the same
    /// thing, including that a long tool result lives there.
    fn paths_reach(&self) -> String {
        match self.inner.reach {
            Reach::Workspace if !self.inner.folders.is_empty() => {
                let folders: Vec<String> = self
                    .inner
                    .folders
                    .iter()
                    .map(|folder| {
                        format!(
                            "{} ({})",
                            folder.path.display(),
                            if folder.writable {
                                "read and change"
                            } else {
                                "read only"
                            }
                        )
                    })
                    .collect();
                format!(
                    "Paths are relative to the working directory and may not leave it, except by absolute path into these granted folders: {}; and to read a long tool result written under {}.",
                    folders.join(", "),
                    self.inner.overflow.display()
                )
            }
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

    /// A requested path as the handle it is inside and a path within that
    /// handle, before anything on disk is followed. Under workspace reach an
    /// absolute path inside a granted folder is the folder's, and the rest of
    /// it obeys the workspace's rules: relative, and no `..`. Every other
    /// request is resolved as it always was.
    fn locate(&self, requested: &str, allow_root: bool) -> Result<(Place, PathBuf), ToolError> {
        if self.inner.reach == Reach::Workspace
            && let Some((index, rest)) = self.folder_of(Path::new(requested))
        {
            let rest = rest
                .to_str()
                .ok_or_else(|| ToolError::new("Paths must be valid UTF-8."))?;
            return Ok((
                Place::Folder(index),
                normalize_relative_path(rest, allow_root)?,
            ));
        }
        Ok((Place::Root, self.requested_relative(requested, allow_root)?))
    }

    /// The granted folder an absolute path is lexically inside, and the
    /// path within it. Folders never nest, so at most one matches.
    fn folder_of<'a>(&self, path: &'a Path) -> Option<(usize, &'a Path)> {
        if !path.is_absolute() {
            return None;
        }
        self.inner
            .folders
            .iter()
            .enumerate()
            .find_map(|(index, folder)| {
                path.strip_prefix(&folder.path)
                    .ok()
                    .map(|rest| (index, rest))
            })
    }

    fn dir(&self, place: Place) -> &Dir {
        match place {
            Place::Root => &self.inner.dir,
            Place::Folder(index) => &self.inner.folders[index].dir,
        }
    }

    fn root_of(&self, place: Place) -> &Path {
        match place {
            Place::Root => &self.inner.root,
            Place::Folder(index) => &self.inner.folders[index].path,
        }
    }

    /// Refuses a change inside a folder granted read-only. The workspace,
    /// and everything under whole-machine reach, may be changed.
    fn require_writable(&self, place: Place, requested: &str) -> Result<(), ToolError> {
        match place {
            Place::Folder(index) if !self.inner.folders[index].writable => {
                Err(ToolError::permission_denied(format!(
                    "{requested} is in {}, a folder this teammate may read but not change.",
                    self.inner.folders[index].path.display()
                )))
            }
            _ => Ok(()),
        }
    }

    /// A path as a read tool shows it: relative inside the working directory
    /// (or from the root under whole-machine reach), and absolute anywhere
    /// else, because that is how it can be asked for again.
    fn shown(&self, root: &Path, relative: &Path) -> String {
        if root == self.inner.root {
            display_relative(relative)
        } else {
            display_relative(&root.join(relative.strip_prefix(".").unwrap_or(relative)))
        }
    }

    fn canonical_subpath(
        &self,
        requested: &str,
        allow_root: bool,
    ) -> Result<(Place, PathBuf), ToolError> {
        let (place, relative) = self.locate(requested, allow_root)?;
        // cap-std resolves inside the handle and refuses a symlink or `..`
        // that would leave it, for a folder exactly as for the workspace.
        let canonical = self
            .dir(place)
            .canonicalize(&relative)
            .map_err(|error| ToolError::new(format!("Cannot access {requested}: {error}")))?;
        Ok((
            place,
            normalize_relative_path(canonical.to_string_lossy().as_ref(), allow_root)?,
        ))
    }

    /// A path a read-only tool may open: the working directory or a granted
    /// folder first, then the overflow directory. Writes never call this, so
    /// they cannot land in overflow. Machine reach never looks at overflow;
    /// it has no wall.
    fn resolve_readable(
        &self,
        requested: &str,
        allow_root: bool,
    ) -> Result<(Dir, PathBuf, PathBuf), ToolError> {
        match self.canonical_subpath(requested, allow_root) {
            Ok((place, relative)) => {
                let root = self.root_of(place);
                let dir = self.dir(place).try_clone().map_err(|error| {
                    ToolError::new(format!("{} could not be opened: {error}", root.display()))
                })?;
                Ok((dir, relative, root.to_path_buf()))
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
        self.check_capability()?;
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

    /// A file a teammate is sending the person, opened to be read whole. It
    /// is found where the read tools find one, so what a teammate may send
    /// is exactly what it may read. Answers the open file, its size, and
    /// where it is.
    pub fn open_to_send(
        &self,
        requested: &str,
    ) -> Result<(std::fs::File, u64, PathBuf), ToolError> {
        self.check_capability()?;
        let (dir, relative, root) = self.resolve_readable(requested, false)?;
        let file = dir
            .open(&relative)
            .map_err(|error| ToolError::new(format!("Cannot open {requested}: {error}")))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ToolError::new(format!("{requested} is not a file.")));
        }
        Ok((file.into_std(), metadata.len(), root.join(relative)))
    }

    pub fn create_file_path(&self, requested: &str, bytes: &[u8]) -> Result<PathBuf, ToolError> {
        self.check_capability()?;
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new("The file is too large to write."));
        }
        let (place, relative) = self.writable_path(requested)?;
        let dir = self.dir(place);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = dir.open_with(&relative, &options)?;
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = dir.remove_file(&relative);
            return Err(error.into());
        }
        Ok(self
            .root_of(place)
            .join(relative.strip_prefix(".").unwrap_or(&relative)))
    }

    fn list_directory(&self, args: ListDirectoryArgs) -> Result<String, ToolError> {
        self.check_capability()?;
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
        self.check_capability()?;
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
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&root) else {
                continue;
            };
            if !glob_matches(glob.as_ref(), relative) {
                continue;
            }

            let Ok(file) = dir.open(relative) else {
                continue;
            };
            let shown_path = self.shown(&root, relative);
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
        self.check_capability()?;
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
            let Ok(entry) = entry else { continue };
            let relative = match entry.path().strip_prefix(&root) {
                Ok(path) if !path.as_os_str().is_empty() => path,
                _ => continue,
            };
            // The pattern matches the path within the searched tree, as it
            // always has; the answer names it where it can be read again.
            if !matcher.is_match(display_relative(relative)) {
                continue;
            }
            let shown = self.shown(&root, relative);
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
        self.check_capability()?;
        if args.content.len() > MAX_WRITE_BYTES {
            return Err(ToolError::new(format!(
                "Writes are limited to {MAX_WRITE_BYTES} bytes."
            )));
        }
        let (place, relative) = self.writable_path(&args.path)?;
        let before = self.read_optional(place, &relative)?;
        if before.is_some() && !args.overwrite.unwrap_or(true) {
            return Err(ToolError::new(format!(
                "{} already exists and overwrite is false.",
                args.path
            )));
        }
        Ok(PreparedMutation {
            requested: args.path,
            place,
            relative,
            before,
            after: args.content.into_bytes(),
        })
    }

    fn prepare_edit(&self, args: EditFileArgs) -> Result<PreparedMutation, ToolError> {
        self.check_capability()?;
        if args.old_text.is_empty() {
            return Err(ToolError::new("old_text may not be empty."));
        }
        let (place, relative) = self.writable_path(&args.path)?;
        let before = self
            .read_optional(place, &relative)?
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
            place,
            relative,
            before: Some(before),
            after,
        })
    }

    fn commit_mutation(&self, mutation: PreparedMutation) -> Result<String, ToolError> {
        self.check_capability()?;
        let current = self.read_optional(mutation.place, &mutation.relative)?;
        if current != mutation.before {
            return Err(ToolError::new(format!(
                "{} changed while the edit was being prepared; inspect it again before retrying.",
                mutation.requested
            )));
        }
        let bytes = mutation.after.len();
        let lines = String::from_utf8_lossy(&mutation.after).lines().count();
        self.atomic_write(mutation.place, &mutation.relative, &mutation.after)?;
        Ok(format!(
            "Wrote {bytes} bytes ({lines} lines) to {}.",
            self.shown(self.root_of(mutation.place), &mutation.relative)
        ))
    }

    fn writable_path(&self, requested: &str) -> Result<(Place, PathBuf), ToolError> {
        let (place, normalized) = self.locate(requested, false)?;
        self.require_writable(place, requested)?;
        let file_name = normalized
            .file_name()
            .ok_or_else(|| ToolError::new("Name a file inside the workspace."))?;
        let parent = normalized
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let canonical_parent = self.dir(place).canonicalize(parent).map_err(|error| {
            ToolError::new(format!("Cannot access the parent of {requested}: {error}"))
        })?;
        let canonical_parent =
            normalize_relative_path(canonical_parent.to_string_lossy().as_ref(), true)?;
        Ok((place, canonical_parent.join(file_name)))
    }

    fn read_optional(&self, place: Place, relative: &Path) -> Result<Option<Vec<u8>>, ToolError> {
        let dir = self.dir(place);
        let metadata = match dir.symlink_metadata(relative) {
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

        let mut file = dir.open(relative)?;
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

    fn atomic_write(&self, place: Place, relative: &Path, content: &[u8]) -> Result<(), ToolError> {
        self.check_capability()?;
        let parent = relative
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = relative
            .file_name()
            .ok_or_else(|| ToolError::new("Name a file inside the workspace."))?;
        let parent_dir = self.dir(place).open_dir(parent)?;
        let permissions = parent_dir
            .symlink_metadata(file_name)
            .ok()
            .filter(cap_std::fs::Metadata::is_file)
            .map(|metadata| metadata.permissions());

        let mut temporary = None;
        for _ in 0..16 {
            let candidate = format!(
                ".hotline-{}-{}.tmp",
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
    place: Place,
    relative: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}

/// The granted folders this session may open. Each was canonical when the
/// person granted it; one whose name now resolves somewhere else (a symlink
/// put in its place) or that is gone is left out rather than followed, and
/// one inside the working directory is already the workspace's.
fn open_folders(grants: &[FolderGrant], cwd: &Path) -> Vec<Folder> {
    grants
        .iter()
        .filter_map(|grant| {
            let path = PathBuf::from(&grant.path);
            let canonical = dunce::canonicalize(&path).ok()?;
            if canonical != path || canonical.starts_with(cwd) {
                return None;
            }
            let dir = Dir::open_ambient_dir(&canonical, ambient_authority()).ok()?;
            Some(Folder {
                path: canonical,
                dir,
                writable: grant.writable,
            })
        })
        .collect()
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
        .sort_by_file_path(Ord::cmp);
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

fn glob_matches(glob: Option<&GlobSet>, relative: &Path) -> bool {
    glob.is_none_or(|matcher| matcher.is_match(display_relative(relative)))
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

#[derive(Deserialize)]
pub struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    max_lines: Option<usize>,
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

#[derive(Deserialize)]
pub struct FindFilesArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
    include_hidden: Option<bool>,
}

#[derive(Deserialize)]
pub struct WriteFileArgs {
    path: String,
    content: String,
    overwrite: Option<bool>,
}

#[derive(Deserialize)]
pub struct EditFileArgs {
    path: String,
    old_text: String,
    new_text: String,
    replace_all: Option<bool>,
}

mod rig_tools;
pub use rig_tools::{EditFile, FindFiles, ListDirectory, ReadFile, SearchFiles, WriteFile};

#[cfg(test)]
mod tests;
