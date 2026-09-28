//! Finder does not inherit the user's terminal PATH, and neither does a
//! Linux desktop session: what `.zshrc` adds — Homebrew, Linuxbrew, a
//! language's own bin — is there in a terminal and missing from an app
//! launched by a click. A stdio MCP server or an ACP harness named by a bare
//! command then "could not be started" for no reason the person can see.
//! Restore it before any runtime threads start so discovery and
//! grandchildren (notably Docker's credential helpers and npx's Node
//! interpreter) see the same directories a terminal would.
//!
//! Asking the login shell takes as long as its startup files do, often a
//! second or two with a busy `.zshrc`, and nothing can be drawn until PATH
//! is set. So the answer is kept in the room (`shell-path`): a launch uses
//! the last one at once and asks the shell again behind the window. A
//! different answer is kept for the next launch; this process's PATH cannot
//! change once its threads are running. Only the very first launch waits.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARKER: &[u8] = b"\0HOTLINE_PATH\0";
/// The login shell's PATH as the last launch found it, in the room.
const CACHE: &str = "shell-path";
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

pub fn restore(root: &Path) {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let (recovered, known) = recovered_path(root, &shell, SHELL_TIMEOUT);
    apply(&recovered);
    if known {
        // After `apply`: the process's PATH is settled before this thread,
        // or any other, exists.
        let root = root.to_path_buf();
        std::thread::spawn(move || refresh(&root, &shell, &recovered));
    }
}

/// The shell's PATH, and whether it came from the last launch rather than
/// from the shell just now. The first launch asks and waits, and keeps the
/// answer.
fn recovered_path(root: &Path, shell: &OsStr, timeout: Duration) -> (OsString, bool) {
    if let Ok(bytes) = fs::read(root.join(CACHE))
        && !bytes.is_empty()
    {
        return (OsString::from_vec(bytes), true);
    }
    match read_shell_path(shell, timeout) {
        Ok(path) => {
            keep(root, &path);
            (path, false)
        }
        Err(error) => {
            eprintln!("[startup] could not read shell PATH: {error}");
            (OsString::new(), false)
        }
    }
}

/// Asks the shell again, behind the window, and keeps a changed answer for
/// the next launch.
fn refresh(root: &Path, shell: &OsStr, used: &OsStr) {
    match read_shell_path(shell, SHELL_TIMEOUT) {
        Ok(fresh) if fresh != used => {
            keep(root, &fresh);
            eprintln!("[startup] the login shell's PATH changed; it applies from the next launch");
        }
        Ok(_) => {}
        Err(error) => eprintln!("[startup] could not re-read shell PATH: {error}"),
    }
}

fn keep(root: &Path, path: &OsStr) {
    let staged = root.join(format!("{CACHE}.{}", std::process::id()));
    let written =
        fs::write(&staged, path.as_bytes()).and_then(|()| fs::rename(&staged, root.join(CACHE)));
    if let Err(error) = written {
        let _ = fs::remove_file(&staged);
        eprintln!("[startup] could not keep the shell PATH: {error}");
    }
}

fn apply(recovered: &OsStr) {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut directories: Vec<PathBuf> = Vec::new();
    for path in [recovered, inherited.as_os_str()] {
        for directory in std::env::split_paths(path).filter(|p| !p.as_os_str().is_empty()) {
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        }
    }
    directories.extend(
        [
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/home/linuxbrew/.linuxbrew/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(PathBuf::from),
    );
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(PathBuf::from(&home).join(".local/bin"));
        directories.push(PathBuf::from(home).join(".docker/bin"));
    }
    directories.push(PathBuf::from(
        "/Applications/Docker.app/Contents/Resources/bin",
    ));
    if let Ok(path) = std::env::join_paths(directories) {
        // SAFETY: restore calls this before creating Tokio, Tauri, core, or
        // its own refresh thread.
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn read_shell_path(shell: &OsStr, timeout: Duration) -> Result<OsString, String> {
    // A file avoids pipe deadlocks from noisy shell startup files, and lets us
    // stop waiting even if a startup script leaves a background child behind.
    let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\0HOTLINE_PATH\\0%s\\0' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => return Err(format!("shell exited with {status}")),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => error.to_string(),
                    _ => "shell PATH lookup timed out".to_string(),
                });
            }
        }
    }
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let start = bytes
        .windows(MARKER.len())
        .rposition(|w| w == MARKER)
        .ok_or("shell did not return PATH")?
        + MARKER.len();
    let end = bytes[start..]
        .iter()
        .position(|b| *b == 0)
        .ok_or("unterminated shell PATH")?
        + start;
    if end == start {
        return Err("shell returned an empty PATH".to_string());
    }
    Ok(OsString::from_vec(bytes[start..end].to_vec()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn shell_startup_noise_does_not_become_path_and_children_find_helpers() {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let shell = root.path().join("shell");
        let helper = root.path().join("docker-credential-test");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\nprintf 'welcome\\n\\0HOTLINE_PATH\\0{}:/usr/bin:/bin\\0goodbye\\n'\n",
                root.path().display()
            ),
        )
        .unwrap();
        std::fs::write(&helper, "#!/bin/sh\nprintf credential-helper-found\n").unwrap();
        for file in [&shell, &helper] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The other test in this module forks its own shell on another
        // thread; a fork that lands while this file is still open for
        // writing leaves the child holding the descriptor until it execs,
        // and an exec of this script in that window is "Text file busy".
        // Waiting the window out is the fix; the script is not.
        let mut attempts = 0;
        let path = loop {
            match read_shell_path(shell.as_os_str(), Duration::from_secs(1)) {
                Ok(path) => break path,
                Err(busy) if busy.contains("Text file busy") && attempts < 50 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(other) => panic!("{other}"),
            }
        };
        let result = Command::new("/bin/sh")
            .args(["-c", "docker-credential-test"])
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"credential-helper-found");
    }

    #[test]
    fn a_launch_uses_the_last_answer_without_waiting_on_the_shell() {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let shell = root.path().join("shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\nprintf '\\0HOTLINE_PATH\\0/from/the/shell\\0'\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

        // The first launch asks the shell, waits, and keeps the answer.
        let mut attempts = 0;
        let first = loop {
            let (path, known) =
                recovered_path(root.path(), shell.as_os_str(), Duration::from_secs(1));
            if !path.is_empty() || attempts >= 50 {
                break (path, known);
            }
            attempts += 1;
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(first, (OsString::from("/from/the/shell"), false));

        // The next one takes it from the room: a shell that would hang is never waited on.
        std::fs::write(&shell, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        let started = Instant::now();
        let second = recovered_path(root.path(), shell.as_os_str(), Duration::from_secs(5));
        assert_eq!(second, (OsString::from("/from/the/shell"), true));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_changed_answer_is_kept_for_the_next_launch() {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        keep(root.path(), OsStr::new("/old"));
        let shell = root.path().join("shell");
        std::fs::write(&shell, "#!/bin/sh\nprintf '\\0HOTLINE_PATH\\0/new\\0'\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut attempts = 0;
        while std::fs::read(root.path().join(CACHE)).unwrap() != b"/new" && attempts < 50 {
            refresh(root.path(), shell.as_os_str(), OsStr::new("/old"));
            attempts += 1;
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(root.path().join(CACHE)).unwrap(), b"/new");
    }

    #[test]
    fn a_stuck_startup_script_is_bounded() {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let shell = root.path().join("shell");
        std::fs::write(&shell, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            read_shell_path(shell.as_os_str(), Duration::from_millis(50))
                .unwrap_err()
                .contains("timed out")
        );
    }
}
