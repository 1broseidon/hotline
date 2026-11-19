//! The workspace is the only writable host tree. Runtime mounts deliberately
//! name installations, never arbitrary PATH parents or a whole home directory.
//! Missing dependencies stay missing rather than widening the boundary.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

pub(super) fn command(
    command: &str,
    workspace: &Path,
    folders: &[(PathBuf, bool)],
    identities: &[(PathBuf, String)],
    run: &super::RunEnv,
) -> Result<Command, String> {
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("Cannot open the shell workspace: {error}"))?;
    let mut process = launcher(Some(&workspace), folders)?;
    hand(&mut process, run)?;
    let ca = if run.ca_pem.is_some() {
        super::ca_setup(
            "cat /etc/ssl/certs/ca-certificates.crt 2>/dev/null || cat /etc/pki/tls/certs/ca-bundle.crt",
        )
    } else {
        String::new()
    };
    // Create the home *inside* the sandbox: a project-controlled symlink here
    // must never cause Hotline to create directories elsewhere on the host.
    let setup = format!(
        "{}mkdir -p -- \"$HOME\" || exit; {ca}{} exec /bin/sh -c \"$1\"",
        super::identity_checks(identities)?,
        super::RUSTUP_SETUP
    );
    process.args(["/bin/sh", "-c", &setup, "hotline-shell", command]);
    Ok(process)
}

/// The teammate's grants for its shell, and the proxy's CA, as `--setenv`
/// options bubblewrap reads from a private file it is handed open
/// (`--args`), never from its command line, which every account on the
/// machine can list for as long as the command runs. The file is unlinked
/// and only this user's; the command holds every value in its environment
/// anyway. The fixed names were set by the launcher, and the vault refuses
/// them as secret names.
fn hand(process: &mut Command, run: &super::RunEnv) -> Result<(), String> {
    use std::io::{Seek, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let ca = run
        .ca_pem
        .as_ref()
        .map(|pem| ("HOTLINE_PROXY_CA".to_owned(), pem.clone()));
    let pairs: Vec<(String, String)> = run.vars.iter().cloned().chain(ca).collect();
    if pairs.is_empty() {
        return Ok(());
    }
    let mut options = Vec::new();
    for (name, value) in &pairs {
        for part in [OsStr::new("--setenv"), OsStr::new(name), OsStr::new(value)] {
            options.extend_from_slice(part.as_bytes());
            options.push(0);
        }
    }
    let unreadable = |error: std::io::Error| format!("Cannot hand the shell its secrets: {error}");
    let mut file = tempfile::tempfile().map_err(unreadable)?;
    file.write_all(&options).map_err(unreadable)?;
    file.rewind().map_err(unreadable)?;
    let fd = file.as_raw_fd();
    process.arg("--args").arg(fd.to_string());
    // The file is close-on-exec here, so no other child of this process
    // inherits it; bubblewrap, alone, gets it open. The closure owns it, so
    // it is open until the command is spawned.
    // SAFETY: fcntl is async-signal-safe and touches only this descriptor,
    // which the closure keeps open.
    unsafe {
        process.pre_exec(move || {
            if libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

pub(super) fn available() -> Result<(), String> {
    let mut process = launcher(None, &[])?;
    process.arg("/bin/true");
    let output = process
        .as_std_mut()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(super::BWRAP_UNUSABLE.to_string()),
    }
}

fn launcher(workspace: Option<&Path>, folders: &[(PathBuf, bool)]) -> Result<Command, String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let host_home = std::env::var_os("HOME").map(PathBuf::from);
    launcher_with(workspace, &path, host_home.as_deref(), folders)
}

fn launcher_with(
    workspace: Option<&Path>,
    path: &OsStr,
    host_home: Option<&Path>,
    folders: &[(PathBuf, bool)],
) -> Result<Command, String> {
    // Resolve the sandbox launcher from system installations, never a
    // project-controlled PATH entry that would run before the wall exists.
    let bwrap = ["/usr/bin/bwrap", "/bin/bwrap"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .ok_or_else(|| super::BWRAP_MISSING.to_string())?;
    let mut mounts = runtime_mounts(host_home);
    add_path_executables(&mut mounts, path, workspace);
    let home = workspace.map_or_else(
        || PathBuf::from("/tmp/hotline-home"),
        |root| root.join(".hotline-home"),
    );
    let mut process = Command::new(bwrap);
    // Clear the launcher's environment too: loader variables must not affect
    // bwrap before it has established the sandbox.
    process.env_clear().args([
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--tmpfs",
        "/tmp",
    ]);
    for mount in &mounts {
        process.arg("--ro-bind").arg(mount).arg(mount);
    }
    // Only public configuration required by the runtime. In particular, do
    // not mount /etc, /run, host /tmp, or any credential or control socket.
    for config in [
        "/etc/ld.so.cache",
        "/etc/alternatives",
        "/etc/ssl/certs",
        "/etc/pki/tls/certs",
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/localtime",
    ] {
        if Path::new(config).exists() {
            process.args(["--ro-bind", config, config]);
        }
    }
    process.args(["--dev", "/dev", "--proc", "/proc"]);
    process.args(folder_binds(folders));
    // Bind last so a workspace under /tmp is not hidden by private scratch,
    // nor by a granted folder that holds it.
    if let Some(root) = workspace {
        process.arg("--bind").arg(root).arg(root);
        process.arg("--chdir").arg(root);
    } else {
        process.args(["--chdir", "/tmp"]);
    }
    // Synthetic ancestors exist to hold mounts, not as extra working space.
    // This remount affects only the root tmpfs; the workspace and private
    // /tmp are separate mounts and remain writable.
    process.args(["--remount-ro", "/"]);
    let path = sandbox_path(path, &mounts, workspace, &home)?;
    process.arg("--setenv").arg("PATH").arg(path);
    process.arg("--setenv").arg("HOME").arg(&home);
    process.args(["--setenv", "TMPDIR", "/tmp", "--setenv", "LANG", "C.UTF-8"]);
    for (name, relative) in [
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("CARGO_HOME", ".cargo"),
        ("RUSTUP_HOME", ".rustup"),
    ] {
        process.arg("--setenv").arg(name).arg(home.join(relative));
    }
    if let Some(host_home) = host_home {
        let rustup = host_home.join(".rustup");
        if mounts.contains(&rustup.join("toolchains"))
            && mounts.contains(&rustup.join("settings.toml"))
        {
            process
                .arg("--setenv")
                .arg("HOTLINE_INSTALLED_RUSTUP")
                .arg(rustup);
        }
    }
    Ok(process)
}

/// Each granted folder at its own path, read-only unless it may be changed.
/// After `/tmp` and `/dev`, so neither hides one; before the workspace, so a
/// read-only folder that holds the workspace does not make it read-only.
fn folder_binds(folders: &[(PathBuf, bool)]) -> Vec<OsString> {
    let mut args = Vec::new();
    for (path, writable) in folders {
        args.push(OsString::from(if *writable {
            "--bind"
        } else {
            "--ro-bind"
        }));
        args.push(path.clone().into_os_string());
        args.push(path.clone().into_os_string());
    }
    args
}

fn runtime_mounts(home: Option<&Path>) -> Vec<PathBuf> {
    let mut mounts = Vec::new();
    // /usr/local/src and other non-runtime trees must remain hidden too.
    for path in [
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/usr/lib64",
        "/usr/libexec",
        "/usr/share",
        "/usr/include",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/local/lib",
        "/usr/local/lib64",
        "/usr/local/libexec",
        "/usr/local/share",
        "/usr/local/include",
        "/usr/local/go",
        "/usr/local/swift",
    ] {
        if Path::new(path).exists() {
            mounts.push(PathBuf::from(path));
        }
    }
    // Linuxbrew keeps the runtime loader and linked libraries in its prefix.
    // Its etc and var directories may hold local service state or secrets.
    let mut brew = vec![PathBuf::from("/home/linuxbrew/.linuxbrew")];
    if let Some(home) = home.filter(|home| home.is_absolute() && *home != Path::new("/")) {
        brew.push(home.join(".linuxbrew"));
        for relative in [
            ".cargo/bin",
            ".rustup/toolchains",
            ".rustup/settings.toml",
            ".nvm/versions/node",
            ".pyenv/versions",
            ".pyenv/shims",
            ".pyenv/bin",
            ".pyenv/libexec",
            ".pyenv/plugins",
            ".pyenv/version",
            ".local/share/mise/installs",
            ".local/share/mise/shims",
            ".local/share/uv/python",
            ".bun/bin",
        ] {
            add_installation(&mut mounts, &home.join(relative));
        }
    }
    for prefix in brew {
        for relative in ["bin", "sbin", "lib", "libexec", "share", "opt", "Cellar"] {
            add_installation(&mut mounts, &prefix.join(relative));
        }
    }
    mounts
}

fn add_installation(mounts: &mut Vec<PathBuf>, path: &Path) {
    // A redirect to a project or home must not turn a known installation
    // location into permission to expose an unrelated tree.
    if path.canonicalize().is_ok_and(|resolved| resolved == path)
        && !mounts.contains(&path.to_path_buf())
    {
        mounts.push(path.to_path_buf());
    }
}

/// PATH grants execution of programs, not access to everything alongside
/// them. Bind standalone ELF files and interpreter scripts individually;
/// a symlink is mounted as its target file, without exposing the target tree.
fn add_path_executables(mounts: &mut Vec<PathBuf>, path: &OsStr, workspace: Option<&Path>) {
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute()
            || dir
                .components()
                .any(|part| part == std::path::Component::ParentDir)
            || mounts.iter().any(|root| dir.starts_with(root))
            || workspace.is_some_and(|root| dir.starts_with(root))
        {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
            let mut magic = [0; 4];
            let is_program = std::fs::File::open(&path)
                .and_then(|mut file| file.read_exact(&mut magic))
                .is_ok()
                && (magic == *b"\x7fELF" || magic.starts_with(b"#!"));
            if is_program && !mounts.contains(&path) {
                mounts.push(path);
            }
        }
    }
}

fn sandbox_path(
    path: &OsStr,
    mounts: &[PathBuf],
    workspace: Option<&Path>,
    home: &Path,
) -> Result<OsString, String> {
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
    ];
    for dir in std::env::split_paths(path) {
        if dir.is_absolute()
            && (mounts.iter().any(|root| {
                dir.starts_with(root) || (root.is_file() && root.parent() == Some(dir.as_path()))
            }) || workspace.is_some_and(|root| dir.starts_with(root)))
            && !dirs.contains(&dir)
        {
            dirs.push(dir);
        }
    }
    for dir in ["/usr/local/bin", "/usr/bin", "/bin"] {
        let dir = PathBuf::from(dir);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).map_err(|error| format!("Cannot build the shell PATH: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a command is handed is on no command line: bubblewrap reads it
    /// from the file it is given open.
    #[test]
    fn no_value_is_on_bubblewraps_command_line() {
        let run = crate::shell::RunEnv {
            vars: vec![
                ("PLAIN".into(), "readable-value-0001".into()),
                (
                    "HTTPS_PROXY".into(),
                    "http://hl-run:proxy-password-0002@127.0.0.1:1".into(),
                ),
                ("API_KEY".into(), "ghp_standin0003".into()),
            ],
            ca_pem: Some("-----BEGIN CERTIFICATE-----ca-0004".into()),
            held: None,
        };
        let mut process = Command::new("/usr/bin/bwrap");
        hand(&mut process, &run).unwrap();
        let line: Vec<String> = process
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for value in [
            "readable-value-0001",
            "proxy-password-0002",
            "ghp_standin0003",
            "ca-0004",
            "PLAIN",
        ] {
            assert!(!line.iter().any(|arg| arg.contains(value)), "{line:?}");
        }
        assert_eq!(line.len(), 2, "{line:?}");
        assert_eq!(line[0], "--args");
        // The descriptor holds the options, NUL-separated, from the start.
        let fd: i32 = line[1].parse().unwrap();
        let mut held = String::new();
        std::io::Read::read_to_string(
            &mut std::fs::File::open(format!("/proc/self/fd/{fd}")).unwrap(),
            &mut held,
        )
        .unwrap();
        assert!(
            held.starts_with("--setenv\0PLAIN\0readable-value-0001\0--setenv\0HTTPS_PROXY\0"),
            "{held:?}"
        );
        assert!(
            held.ends_with("--setenv\0HOTLINE_PROXY_CA\0-----BEGIN CERTIFICATE-----ca-0004\0"),
            "{held:?}"
        );
        // Nothing handed, nothing added.
        let mut bare = Command::new("/usr/bin/bwrap");
        hand(&mut bare, &crate::shell::RunEnv::default()).unwrap();
        assert_eq!(bare.as_std().get_args().count(), 0);
    }

    /// With bubblewrap here, the command really has what it was handed.
    #[tokio::test]
    async fn the_command_has_what_it_was_handed() {
        if available().is_err() {
            eprintln!("skipped: no usable bubblewrap");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let run = crate::shell::RunEnv {
            vars: vec![("PLAIN".into(), "readable value with spaces".into())],
            ..crate::shell::RunEnv::default()
        };
        let output = command("printf %s \"$PLAIN\"", root.path(), &[], &[], &run)
            .unwrap()
            .output()
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "readable value with spaces"
        );
    }

    #[test]
    fn arbitrary_path_directories_do_not_grant_read_access() {
        let mounts = vec![PathBuf::from("/usr"), PathBuf::from("/home/me/.cargo/bin")];
        let path = sandbox_path(
            OsStr::new("/home/me/other-project:/home/me/.cargo/bin:/usr/bin:.:/workspace/bin"),
            &mounts,
            Some(Path::new("/workspace")),
            Path::new("/workspace/.hotline-home"),
        )
        .unwrap();
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert!(!dirs.contains(&PathBuf::from("/home/me/other-project")));
        assert!(!dirs.contains(&PathBuf::from(".")));
        assert!(dirs.contains(&PathBuf::from("/home/me/.cargo/bin")));
        assert!(dirs.contains(&PathBuf::from("/workspace/bin")));
    }

    #[test]
    #[ignore = "needs bubblewrap"]
    fn a_path_program_does_not_expose_its_neighbors_or_symlink_target_directory() {
        super::super::tests::require_sandbox();
        let root = super::super::tests::TestDirectory::new();
        let workspace = root.path().join("workspace");
        let bin = root.path().join("bin");
        let package = root.path().join("package");
        for dir in [&workspace, &bin, &package] {
            std::fs::create_dir(dir).unwrap();
        }
        let script = package.join("hello");
        std::fs::write(&script, "#!/bin/sh\nprintf standalone-ok\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&script, bin.join("hello")).unwrap();
        std::fs::write(bin.join(".env"), "neighbor-secret").unwrap();
        std::fs::write(package.join(".env"), "package-secret").unwrap();
        let path = std::env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")])
            .unwrap();
        let mut process = launcher_with(Some(&workspace), &path, None, &[]).unwrap();
        process
            .args([
                "/bin/sh",
                "-c",
                "hello && test ! -e \"$1/.env\" && test ! -e \"$2/.env\"",
                "probe",
            ])
            .arg(&bin)
            .arg(&package);
        let output = process.as_std_mut().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"standalone-ok");
    }

    #[test]
    #[ignore = "needs bubblewrap"]
    fn home_toolchains_are_read_only_and_do_not_expose_credentials() {
        super::super::tests::require_sandbox();
        let root = super::super::tests::TestDirectory::new();
        let home = root.path().join("host-home");
        let workspace = root.path().join("workspace");
        let bin = home.join(".cargo/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        let script = bin.join("hello");
        std::fs::write(&script, "#!/bin/sh\nprintf toolchain-ok\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(home.join(".cargo/credentials.toml"), "credential-canary").unwrap();
        let path = std::env::join_paths([bin.as_path(), Path::new("/usr/bin")]).unwrap();
        let mut process = launcher_with(Some(&workspace), &path, Some(&home), &[]).unwrap();
        process.args(["/bin/sh", "-c", "hello && test ! -e \"$1/.cargo/credentials.toml\" && ! (echo changed > \"$1/.cargo/bin/hello\")", "probe"]).arg(&home);
        let output = process.as_std_mut().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"toolchain-ok");
        assert!(
            std::fs::read_to_string(&script)
                .unwrap()
                .contains("toolchain-ok")
        );
    }

    #[test]
    fn granted_folders_are_bound_with_their_modes_before_the_workspace() {
        let binds = folder_binds(&[
            (PathBuf::from("/granted/read"), false),
            (PathBuf::from("/granted/write"), true),
        ]);
        assert_eq!(
            binds,
            [
                "--ro-bind",
                "/granted/read",
                "/granted/read",
                "--bind",
                "/granted/write",
                "/granted/write"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    #[ignore = "needs bubblewrap"]
    fn granted_folders_are_open_to_the_shell_as_granted() {
        super::super::tests::require_sandbox();
        let root = super::super::tests::TestDirectory::new();
        let [workspace, read_only, writable, outside] =
            ["workspace", "read", "write", "outside"].map(|name| root.path().join(name));
        for dir in [&workspace, &read_only, &writable, &outside] {
            std::fs::create_dir(dir).unwrap();
        }
        std::fs::write(read_only.join("notes.txt"), "granted").unwrap();
        std::fs::write(outside.join("secret.txt"), "host-canary").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), read_only.join("escape")).unwrap();
        let folders = [(read_only.clone(), false), (writable.clone(), true)];
        let path = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        let mut process = launcher_with(Some(&workspace), &path, None, &folders).unwrap();
        process
            .args([
                "/bin/sh",
                "-c",
                "cat \"$1/notes.txt\" && echo x > \"$2/made.txt\" && ! (echo x > \"$1/made.txt\") 2>/dev/null && ! cat \"$1/escape\" 2>/dev/null",
                "probe",
            ])
            .arg(&read_only)
            .arg(&writable);
        let output = process.as_std_mut().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"granted");
        assert!(writable.join("made.txt").exists());
        assert!(!read_only.join("made.txt").exists());
    }

    #[test]
    fn redirected_installations_do_not_expose_other_projects() {
        let root = super::super::tests::TestDirectory::new();
        let install = root.path().join("bin");
        std::os::unix::fs::symlink("/etc", &install).unwrap();
        let mut mounts = Vec::new();
        add_installation(&mut mounts, &install);
        assert_eq!(mounts, [] as [PathBuf; 0]);
    }
}
