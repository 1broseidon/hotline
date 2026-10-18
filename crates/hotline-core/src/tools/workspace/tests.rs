use super::{
    DEFAULT_READ_LINES, EditFile, EditFileArgs, FindFiles, FindFilesArgs, ListDirectory, Reach,
    ReadFile, ReadFileArgs, SearchFiles, SearchFilesArgs, Workspace, WriteFile, WriteFileArgs,
    normalize_relative_path,
};
// The extra-folder tests build their fixture with symlinks, so they run where those do.
#[cfg(unix)]
use super::{FolderGrant, ListDirectoryArgs};
use crate::driver::CapabilityEpoch;
use crate::tools::RunCommand;
use rig::tool::Tool;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Two tests in one process may read the same clock; the counter keeps
/// their roots apart.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hotline-tools-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
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
        result.contains(&format!("{bytes} bytes")) && result.contains(&format!("{lines} lines")),
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
fn a_revoked_workspace_handle_refuses_reads_and_writes() {
    let directory = TestDirectory::new();
    fs::write(directory.path().join("canary.txt"), "still here\n").unwrap();
    let epoch = CapabilityEpoch::default();
    let workspace = Workspace::open_with_capability(
        directory.path().to_path_buf(),
        Reach::Workspace,
        directory.path().join("overflow"),
        &[],
        Some(epoch.lease()),
    )
    .unwrap();
    epoch.invalidate();

    let read = workspace.read_file(ReadFileArgs {
        path: "canary.txt".to_string(),
        start_line: None,
        max_lines: None,
    });
    assert!(
        read.unwrap_err()
            .to_string()
            .contains("capabilities have been revoked")
    );
    let write = workspace.prepare_write(WriteFileArgs {
        path: "new.txt".to_string(),
        content: "should not land\n".to_string(),
        overwrite: None,
    });
    assert!(
        write
            .unwrap_err()
            .to_string()
            .contains("capabilities have been revoked")
    );
    assert!(!directory.path().join("new.txt").exists());
}

/// A workspace with one read-only and one writable folder beside it,
/// and a file outside every grant that a link in the read-only one names.
#[cfg(unix)]
struct Granted {
    _scratch: TestDirectory,
    workspace: PathBuf,
    read_only: PathBuf,
    writable: PathBuf,
    outside: PathBuf,
}

#[cfg(unix)]
impl Granted {
    fn new() -> Self {
        let scratch = TestDirectory::new();
        let base = scratch.path().canonicalize().unwrap();
        let [workspace, read_only, writable, outside] =
            ["workspace", "read-only", "writable", "outside"].map(|name| base.join(name));
        for directory in [&workspace, &read_only, &writable, &outside] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(read_only.join("notes.txt"), "granted needle\n").unwrap();
        fs::write(outside.join("secret.txt"), "outside needle\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), read_only.join("escape.txt"))
            .unwrap();
        std::os::unix::fs::symlink(&outside, read_only.join("away")).unwrap();
        Self {
            _scratch: scratch,
            workspace,
            read_only,
            writable,
            outside,
        }
    }

    fn grants(&self) -> Vec<FolderGrant> {
        vec![
            FolderGrant {
                path: self.read_only.to_string_lossy().into_owned(),
                writable: false,
            },
            FolderGrant {
                path: self.writable.to_string_lossy().into_owned(),
                writable: true,
            },
        ]
    }

    fn open(&self, reach: Reach, capability: Option<crate::driver::CapabilityLease>) -> Workspace {
        Workspace::open_with_capability(
            self.workspace.clone(),
            reach,
            self.workspace.with_extension("overflow-unused"),
            &self.grants(),
            capability,
        )
        .unwrap()
    }
}

#[cfg(unix)]
fn read(workspace: &Workspace, path: &Path) -> Result<String, super::ToolError> {
    workspace.read_file(ReadFileArgs {
        path: path.to_string_lossy().into_owned(),
        start_line: None,
        max_lines: None,
    })
}

#[cfg(unix)]
fn write(workspace: &Workspace, path: &Path) -> Result<String, super::ToolError> {
    workspace
        .prepare_write(WriteFileArgs {
            path: path.to_string_lossy().into_owned(),
            content: "written\n".to_string(),
            overwrite: None,
        })
        .and_then(|mutation| workspace.commit_mutation(mutation))
}

#[cfg(unix)]
#[test]
fn a_granted_folder_is_read_by_absolute_path_and_changed_only_when_writable() {
    let granted = Granted::new();
    let workspace = granted.open(Reach::Workspace, None);

    assert!(
        read(&workspace, &granted.read_only.join("notes.txt"))
            .unwrap()
            .contains("granted")
    );
    let listed = workspace
        .list_directory(ListDirectoryArgs {
            path: Some(granted.read_only.to_string_lossy().into_owned()),
        })
        .unwrap();
    assert!(listed.contains("notes.txt"), "{listed}");
    let searched = workspace
        .search_files(SearchFilesArgs {
            pattern: "needle".to_string(),
            path: Some(granted.read_only.to_string_lossy().into_owned()),
            glob: None,
            case_insensitive: None,
            literal: Some(true),
            max_results: None,
            include_hidden: None,
        })
        .unwrap();
    // Named where it can be read again, and never through the link out.
    assert!(
        searched.contains(&format!(
            "{}:1:",
            granted.read_only.join("notes.txt").display()
        )),
        "{searched}"
    );
    assert!(!searched.contains("outside needle"), "{searched}");

    let refused = write(&workspace, &granted.read_only.join("new.txt")).unwrap_err();
    assert!(
        refused.to_string().contains("read but not change"),
        "{refused}"
    );
    assert!(!granted.read_only.join("new.txt").exists());
    let edit = workspace.prepare_edit(EditFileArgs {
        path: granted
            .read_only
            .join("notes.txt")
            .to_string_lossy()
            .into_owned(),
        old_text: "granted".to_string(),
        new_text: "changed".to_string(),
        replace_all: None,
    });
    assert!(edit.is_err());
    assert_eq!(
        fs::read_to_string(granted.read_only.join("notes.txt")).unwrap(),
        "granted needle\n"
    );

    let wrote = write(&workspace, &granted.writable.join("new.txt")).unwrap();
    assert!(
        wrote.contains(&granted.writable.join("new.txt").display().to_string()),
        "{wrote}"
    );
    assert_eq!(
        fs::read_to_string(granted.writable.join("new.txt")).unwrap(),
        "written\n"
    );
    // The workspace itself is as it was.
    write(&workspace, Path::new("own.txt")).unwrap();
    assert!(granted.workspace.join("own.txt").exists());

    let description = ReadFile::new(workspace).description();
    assert!(
        description.contains(&format!("{} (read only)", granted.read_only.display()))
            && description.contains(&format!("{} (read and change)", granted.writable.display())),
        "{description}"
    );
}

#[cfg(unix)]
#[test]
fn a_path_outside_every_grant_or_out_through_a_link_is_refused() {
    let granted = Granted::new();
    let workspace = granted.open(Reach::Workspace, None);
    for path in [
        granted.outside.join("secret.txt"),
        granted.read_only.join("escape.txt"),
        granted.read_only.join("away/secret.txt"),
        granted.read_only.join("../outside/secret.txt"),
        granted.writable.join("../outside/secret.txt"),
    ] {
        assert!(read(&workspace, &path).is_err(), "{}", path.display());
    }
    for path in [
        granted.outside.join("new.txt"),
        granted.read_only.join("away/new.txt"),
        granted.writable.join("../outside/new.txt"),
    ] {
        assert!(write(&workspace, &path).is_err(), "{}", path.display());
    }
    assert!(!granted.outside.join("new.txt").exists());
    assert_eq!(fs::read_dir(&granted.outside).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_or_never_granted_opens_nothing() {
    let granted = Granted::new();
    // No grants: the record from before the field.
    let without = Workspace::open(
        granted.workspace.clone(),
        Reach::Workspace,
        granted.workspace.with_extension("overflow-unused"),
    )
    .unwrap();
    assert!(read(&without, &granted.read_only.join("notes.txt")).is_err());

    // A granted folder whose name now leads elsewhere is not followed.
    fs::remove_dir_all(&granted.writable).unwrap();
    std::os::unix::fs::symlink(&granted.outside, &granted.writable).unwrap();
    let workspace = granted.open(Reach::Workspace, None);
    assert!(read(&workspace, &granted.writable.join("secret.txt")).is_err());
    assert!(write(&workspace, &granted.writable.join("new.txt")).is_err());
    assert!(!granted.outside.join("new.txt").exists());
    assert_eq!(workspace.folders().len(), 1);
}

#[cfg(unix)]
#[test]
fn a_revoked_handle_refuses_its_folders_too() {
    let granted = Granted::new();
    let epoch = CapabilityEpoch::default();
    let workspace = granted.open(Reach::Workspace, Some(epoch.lease()));
    assert!(read(&workspace, &granted.read_only.join("notes.txt")).is_ok());
    epoch.invalidate();
    for result in [
        read(&workspace, &granted.read_only.join("notes.txt")),
        write(&workspace, &granted.writable.join("late.txt")),
    ] {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("capabilities have been revoked")
        );
    }
    assert!(!granted.writable.join("late.txt").exists());
}

#[cfg(unix)]
#[test]
fn whole_machine_reach_keeps_no_folder_wall() {
    let granted = Granted::new();
    let workspace = granted.open(Reach::Machine, None);
    assert!(workspace.folders().is_empty());
    write(&workspace, &granted.read_only.join("machine.txt")).unwrap();
    assert!(granted.read_only.join("machine.txt").exists());
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
