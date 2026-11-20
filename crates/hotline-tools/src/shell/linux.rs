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

/// What the confined shell can reach, as the teammate is told it. The second
/// sentence depends on the kernel, because only Landlock's scoping closes
/// abstract sockets while the host network stays shared.
pub(super) fn wall() -> &'static str {
    if abstract_sockets_closed() {
        "The shell can access the workspace, selected read-only installed tools, and private /tmp. Other host files are hidden. HOME is .hotline-home inside the workspace; use it for persistent caches and user installs. Host credentials and environment variables are not inherited. Network access remains available. Unix sockets outside the sandbox, including the desktop's abstract ones, are not."
    } else {
        "The shell can access the workspace, selected read-only installed tools, and private /tmp. Other host files are hidden. HOME is .hotline-home inside the workspace; use it for persistent caches and user installs. Host credentials and environment variables are not inherited. Network access remains available. This kernel is older than Linux 6.12, so abstract Unix sockets outside the sandbox stay reachable too."
    }
}

/// The shell shares the host's network namespace so that it has the network,
/// and with it the host's abstract Unix sockets, which have no file to hide:
/// an X11 display (`@/tmp/.X11-unix/X0`), a D-Bus bus started by
/// `dbus-launch`, and anything else bound there. Landlock's scope (ABI 6,
/// Linux 6.12) refuses a connection to one bound outside the sandbox, and
/// leaves the network and the sockets the command binds itself alone.
const LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1;
const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
const LANDLOCK_ABI_WITH_SCOPES: libc::c_long = 6;

/// `struct landlock_ruleset_attr` as of ABI 6. Handling no file or network
/// access keeps Landlock out of bubblewrap's mounts, which it would refuse
/// to a process whose domain handles file access.
#[repr(C)]
struct LandlockRuleset {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

/// Whether this kernel can close the host's abstract sockets to the shell.
/// Landlock may be missing, off at boot, or older than ABI 6; each reads as
/// no.
pub(super) fn abstract_sockets_closed() -> bool {
    // SAFETY: a null attribute with the version flag only asks the kernel
    // for its Landlock ABI; nothing is read or written.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<LandlockRuleset>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    abi >= LANDLOCK_ABI_WITH_SCOPES
}

/// Puts bubblewrap, and so everything it starts, in a Landlock domain that
/// connects to no abstract socket bound outside it. A step that fails stops
/// the spawn, so the shell never runs believing it is closed when it is not.
fn close_abstract_sockets(process: &mut Command) {
    // SAFETY: prctl, the two Landlock calls and close are async-signal-safe,
    // and the closure allocates nothing.
    unsafe {
        process.pre_exec(|| {
            // Landlock asks for this before it confines an unprivileged
            // process; bubblewrap sets it for the command anyway.
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let no_flags: libc::c_uint = 0;
            let ruleset = LandlockRuleset {
                handled_access_fs: 0,
                handled_access_net: 0,
                scoped: LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET,
            };
            let fd = libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &raw const ruleset,
                std::mem::size_of::<LandlockRuleset>(),
                no_flags,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let Ok(fd) = libc::c_int::try_from(fd) else {
                return Err(std::io::Error::from_raw_os_error(libc::EBADF));
            };
            let restricted = libc::syscall(libc::SYS_landlock_restrict_self, fd, no_flags);
            let error = std::io::Error::last_os_error();
            libc::close(fd);
            if restricted != 0 {
                return Err(error);
            }
            Ok(())
        });
    }
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
    if abstract_sockets_closed() {
        close_abstract_sockets(&mut process);
    }
    // Clear the launcher's environment too: loader variables must not affect
    // bwrap before it has established the sandbox. That also keeps the
    // desktop session's addresses (DISPLAY, WAYLAND_DISPLAY, XAUTHORITY,
    // DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR) out of the shell.
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

    /// A probe the confined shell runs: for each `name=kind:address` it
    /// connects and prints `name:ok` and what the listener said, or `name:`
    /// and the errno that refused it. `abstract` addresses have no file.
    const SOCKET_PROBE: &str = r#"
import errno, socket, sys
for target in sys.argv[1:]:
    name, rest = target.split("=", 1)
    kind, address = rest.split(":", 1)
    if kind == "tcp":
        s = socket.socket(socket.AF_INET)
        address = ("127.0.0.1", int(address))
    else:
        s = socket.socket(socket.AF_UNIX)
        address = "\0" + address if kind == "abstract" else address
    s.settimeout(5)
    try:
        s.connect(address)
        print(name + ":ok:" + s.recv(16).decode())
    except OSError as e:
        print(name + ":" + errno.errorcode.get(e.errno, str(e.errno)))
"#;

    /// Runs the probe in the real confined shell, with the desktop session's
    /// variables set on bubblewrap itself, as a desk started from a desktop
    /// would have them.
    async fn probe(workspace: &Path, targets: &[String]) -> String {
        std::fs::write(workspace.join("probe.py"), SOCKET_PROBE).unwrap();
        let line = format!(
            "python3 probe.py {} && echo \"session:${{DISPLAY:-}}${{WAYLAND_DISPLAY:-}}${{XAUTHORITY:-}}${{DBUS_SESSION_BUS_ADDRESS:-}}${{XDG_RUNTIME_DIR:-}}\" && {{ ls -a /tmp /run 2>&1 || true; }}",
            targets.join(" ")
        );
        let mut process =
            command(&line, workspace, &[], &[], &crate::shell::RunEnv::default()).unwrap();
        for (name, value) in [
            ("DISPLAY", ":0"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XAUTHORITY", "/home/me/.Xauthority"),
            (
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:abstract=/tmp/dbus-standin",
            ),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ] {
            process.env(name, value);
        }
        let output = process.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// A listener that answers each connection with `reply`, on a thread.
    fn answer(listener: std::os::unix::net::UnixListener, reply: &'static str) {
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = std::io::Write::write_all(&mut stream, reply.as_bytes());
            }
        });
    }

    /// The shell shares the host's network namespace, and with it the
    /// abstract sockets a desktop's X server and a `dbus-launch` bus listen
    /// on. From Linux 6.12 Landlock refuses them; a path socket outside the
    /// workspace is not mounted, nor are `/run` and the host's `/tmp` with
    /// `.X11-unix`, and the session's addresses do not reach the shell.
    #[tokio::test]
    #[ignore = "needs bubblewrap"]
    async fn host_unix_sockets_and_the_desktop_session_are_out_of_reach() {
        use std::os::linux::net::SocketAddrExt;
        super::super::tests::require_sandbox();
        let root = super::super::tests::TestDirectory::new();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let name = format!("hotline-test-display-{}", std::process::id());
        let display = std::os::unix::net::UnixListener::bind_addr(
            &std::os::unix::net::SocketAddr::from_abstract_name(&name).unwrap(),
        )
        .unwrap();
        answer(display, "display");
        let bus = root.path().join("bus");
        answer(std::os::unix::net::UnixListener::bind(&bus).unwrap(), "bus");
        let output = probe(
            &workspace,
            &[
                format!("abstract=abstract:{name}"),
                format!("path=path:{}", bus.display()),
            ],
        )
        .await;
        assert!(output.contains("path:ENOENT"), "{output}");
        assert!(output.contains("session:\n"), "{output}");
        assert!(output.contains("ls: cannot access '/run'"), "{output}");
        assert!(!output.contains(".X11-unix"), "{output}");
        if abstract_sockets_closed() {
            assert!(output.contains("abstract:EPERM"), "{output}");
            assert!(wall().contains("including the desktop's abstract ones, are not"));
        } else {
            eprintln!(
                "abstract sockets not checked: this kernel has no Landlock ABI 6 (Linux 6.12), so nothing can close them; the shell says so"
            );
            assert!(output.contains("abstract:ok:display"), "{output}");
            assert!(wall().contains("stay reachable"));
        }
    }

    /// Closing the host's sockets leaves the network and the sockets the
    /// command makes itself: an abstract one, one in the workspace, one in
    /// its private /tmp, and TCP to a listener on the host.
    #[tokio::test]
    #[ignore = "needs bubblewrap"]
    async fn the_shell_keeps_tcp_and_the_sockets_it_makes_itself() {
        super::super::tests::require_sandbox();
        let root = super::super::tests::TestDirectory::new();
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in tcp.incoming().flatten() {
                let _ = std::io::Write::write_all(&mut stream, b"tcp");
            }
        });
        let serve = r#"
import socket, threading
def answer(listener):
    while True:
        connection, _ = listener.accept()
        connection.sendall(b"own")
        connection.close()
for address in ["\0hotline-own", "own.sock", "/tmp/own.sock"]:
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(address)
    listener.listen()
    threading.Thread(target=answer, args=(listener,), daemon=True).start()
import probe  # connects to what sys.argv names
"#;
        std::fs::write(root.path().join("serve.py"), serve).unwrap();
        std::fs::write(root.path().join("probe.py"), SOCKET_PROBE).unwrap();
        let line = format!(
            "python3 serve.py tcp=tcp:{port} abstract=abstract:hotline-own workspace=path:own.sock tmp=path:/tmp/own.sock"
        );
        let output = command(
            &line,
            root.path(),
            &[],
            &[],
            &crate::shell::RunEnv::default(),
        )
        .unwrap()
        .output()
        .await
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout,
            "tcp:ok:tcp\nabstract:ok:own\nworkspace:ok:own\ntmp:ok:own\n",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
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
