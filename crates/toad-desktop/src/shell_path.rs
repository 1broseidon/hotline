//! Finder does not inherit the user's terminal PATH, and neither does a
//! Linux desktop session: what `.zshrc` adds — Homebrew, Linuxbrew, a
//! language's own bin — is there in a terminal and missing from an app
//! launched by a click. A stdio MCP server or an ACP harness named by a bare
//! command then "could not be started" for no reason the person can see.
//! Restore it before any runtime threads start so discovery and
//! grandchildren (notably Docker's credential helpers and npx's Node
//! interpreter) see the same directories a terminal would.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARKER: &[u8] = b"\0TOAD_PATH\0";

pub fn restore() {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let recovered = match read_shell_path(&shell, Duration::from_secs(5)) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[startup] could not read shell PATH: {error}");
            OsString::new()
        }
    };
    let mut directories: Vec<PathBuf> = Vec::new();
    for path in [&recovered, &inherited] {
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
        // SAFETY: run calls this before creating Tokio, Tauri, or core threads.
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn read_shell_path(shell: &OsStr, timeout: Duration) -> Result<OsString, String> {
    // A file avoids pipe deadlocks from noisy shell startup files, and lets us
    // stop waiting even if a startup script leaves a background child behind.
    let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\0TOAD_PATH\\0%s\\0' \"$PATH\""])
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
                "#!/bin/sh\nprintf 'welcome\\n\\0TOAD_PATH\\0{}:/usr/bin:/bin\\0goodbye\\n'\n",
                root.path().display()
            ),
        )
        .unwrap();
        std::fs::write(&helper, "#!/bin/sh\nprintf credential-helper-found\n").unwrap();
        for file in [&shell, &helper] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = read_shell_path(shell.as_os_str(), Duration::from_secs(1)).unwrap();
        let result = Command::new("/bin/sh")
            .args(["-c", "docker-credential-test"])
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"credential-helper-found");
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
