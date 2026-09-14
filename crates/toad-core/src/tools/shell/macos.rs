//! Seatbelt is a filesystem and service boundary, not a mount/PID namespace.
//! Runtime exceptions name installations; PATH never grants host file access.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;

const UNAVAILABLE: &str = "The protected shell needs working macOS Seatbelt enforcement, but its isolation probe failed. Restart Toad after checking macOS support, or explicitly enable Whole machine to use an unrestricted shell.";

pub(super) fn command(command: &str, workspace: &Path) -> Result<Command, String> {
    let workspace = workspace
        .canonicalize()
        .map_err(|error| format!("Cannot open the shell workspace: {error}"))?;
    if workspace.to_str().is_none() {
        return Err("Protected shell workspace paths must be UTF-8.".into());
    }
    let host_home = std::env::var_os("HOME").map(PathBuf::from);
    let runtimes = runtimes(host_home.as_deref());
    let mut process = launcher(&workspace, &runtimes);
    let home = workspace.join(".toad-home");
    let mut env = vec![
        ("PATH", sandbox_path(&runtimes, &home)?),
        ("HOME", home.clone().into_os_string()),
        ("TMPDIR", home.join(".tmp").into_os_string()),
        ("LANG", "en_US.UTF-8".into()),
        // Homebrew OpenSSL otherwise reads prefix/etc, which can hold secrets.
        // Built-in providers and the public system CA bundle are sufficient.
        ("OPENSSL_CONF", "/dev/null".into()),
        ("SSL_CERT_FILE", "/private/etc/ssl/cert.pem".into()),
    ];
    for (name, relative) in [
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("CARGO_HOME", ".cargo"),
        ("RUSTUP_HOME", ".rustup"),
        ("GOPATH", "go"),
        ("GOCACHE", ".cache/go-build"),
        ("npm_config_cache", ".cache/npm"),
        ("PYTHONUSERBASE", ".local"),
    ] {
        env.push((name, home.join(relative).into_os_string()));
    }
    let mut setup = String::from("umask 077; mkdir -p -- \"$HOME\" \"$TMPDIR\" || exit; ");
    if let Some(host_home) = host_home {
        if runtimes.contains(&host_home.join(".rustup/toolchains"))
            && runtimes.contains(&host_home.join(".rustup/settings.toml"))
        {
            env.push((
                "TOAD_INSTALLED_RUSTUP",
                host_home.join(".rustup").into_os_string(),
            ));
        }
        if runtimes.contains(&host_home.join(".pyenv/versions")) {
            env.push(("PYENV_ROOT", host_home.join(".pyenv").into_os_string()));
        }
    }
    // Prefer the standalone tools. Xcode's conventional symlink may point
    // to a versioned bundle, but never grant an arbitrary redirected tree.
    for developer in [
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app/Contents/Developer",
    ] {
        if let Ok(developer) = Path::new(developer).canonicalize()
            && runtimes.iter().any(|root| developer.starts_with(root))
        {
            env.push(("DEVELOPER_DIR", developer.into_os_string()));
            break;
        }
    }
    // env runs after Seatbelt, so even an environment value added by a caller
    // cannot reach the command. The launcher's own environment is also empty.
    process.args(["/usr/bin/env", "-i"]);
    for (name, value) in env {
        let mut assignment = std::ffi::OsString::from(format!("{name}="));
        assignment.push(value);
        process.arg(assignment);
    }
    // All setup writes happen after confinement. A hostile HOME or scratch
    // symlink cannot make the unsandboxed core create a host directory.
    setup.push_str(super::RUSTUP_SETUP);
    setup.push_str("exec /bin/sh -c \"$1\"");
    process.args(["/bin/sh", "-c", &setup, "toad-shell", command]);
    Ok(process)
}

pub(super) fn available() -> Result<(), String> {
    probe(Path::new("/usr/bin/sandbox-exec"))
}

fn probe(executable: &Path) -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|_| UNAVAILABLE.to_string())?;
    let root = root
        .path()
        .canonicalize()
        .map_err(|_| UNAVAILABLE.to_string())?;
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).map_err(|_| UNAVAILABLE.to_string())?;
    std::fs::write(root.join("denied"), "probe").map_err(|_| UNAVAILABLE.to_string())?;
    let mut process = std::process::Command::new(executable);
    process
        .env_clear()
        .args(["-p", &profile(&workspace, &runtimes(None))]);
    process.current_dir(&workspace).args([
        "/bin/sh", "-c",
        "echo probe > allowed && /bin/cat allowed >/dev/null && ! /bin/cat ../denied >/dev/null 2>&1 && ! /bin/sh -c 'echo changed > ../denied' 2>/dev/null",
    ]);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = process.spawn().map_err(|_| UNAVAILABLE.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status))
                if status.success()
                    && std::fs::read(workspace.join("allowed"))
                        .is_ok_and(|data| data == b"probe\n")
                    && std::fs::read(root.join("denied")).is_ok_and(|data| data == b"probe") =>
            {
                return Ok(());
            }
            Ok(Some(_)) | Err(_) => return Err(UNAVAILABLE.to_string()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UNAVAILABLE.to_string());
            }
        }
    }
}

fn launcher(workspace: &Path, runtimes: &[PathBuf]) -> Command {
    // Never resolve the unsandboxed launcher through a project-controlled PATH.
    let mut process = Command::new("/usr/bin/sandbox-exec");
    process
        .env_clear()
        .args(["-p", &profile(workspace, runtimes)]);
    process.current_dir(workspace);
    process
}

fn runtimes(home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in [
        "/bin",
        "/sbin",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/usr/libexec",
        "/usr/share",
        "/System/Library",
        "/Library/Apple",
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app",
        "/usr/local/bin",
        "/usr/local/lib",
        "/usr/local/include",
        "/usr/local/share",
        "/usr/local/go",
    ] {
        add_installation(&mut paths, Path::new(path));
    }
    if let Ok(entries) = std::fs::read_dir("/Applications") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(version) = name
                .to_str()
                .and_then(|name| name.strip_prefix("Xcode_"))
                .and_then(|name| name.strip_suffix(".app"))
            else {
                continue;
            };
            if !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.') {
                add_installation(&mut paths, &entry.path());
            }
        }
    }
    for prefix in ["/opt/homebrew", "/usr/local"] {
        for relative in [
            "Cellar", "opt", "bin", "sbin", "lib", "libexec", "share", "include",
        ] {
            add_installation(&mut paths, &Path::new(prefix).join(relative));
        }
    }
    if let Some(home) = home.filter(|home| home.is_absolute() && *home != Path::new("/")) {
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
            add_installation(&mut paths, &home.join(relative));
        }
    }
    paths
}

fn add_installation(paths: &mut Vec<PathBuf>, path: &Path) {
    // A symlinked installation must not grant its target tree. Seatbelt also
    // resolves nested symlinks against the target's own permissions.
    if path.canonicalize().is_ok_and(|resolved| resolved == path)
        && !paths.contains(&path.to_path_buf())
    {
        paths.push(path.to_path_buf());
    }
}

#[cfg(test)]
pub(super) fn supported_tool_on_path(name: &str) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let runtimes = runtimes(home.as_deref());
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter_map(|path| path.join(name).canonicalize().ok())
        .any(|path| runtimes.iter().any(|root| path.starts_with(root)))
}

fn sandbox_path(runtimes: &[PathBuf], home: &Path) -> Result<std::ffi::OsString, String> {
    let mut paths = vec![
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
    ];
    for path in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if path.is_absolute()
            && runtimes.iter().any(|root| path.starts_with(root))
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    for path in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !paths.contains(&PathBuf::from(path)) {
            paths.push(PathBuf::from(path));
        }
    }
    std::env::join_paths(paths)
        .map_err(|error| format!("Cannot build the protected shell PATH: {error}"))
}

fn quote(path: &Path) -> String {
    // Non-UTF-8 paths cannot be represented faithfully by a Seatbelt string.
    path.as_os_str()
        .to_str()
        .unwrap_or("")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn profile(workspace: &Path, runtimes: &[PathBuf]) -> String {
    let mut policy = String::from(
        r#"(version 1)
(deny default)
(allow process-fork process-exec)
(allow signal (target same-sandbox))
(allow sysctl-read
    (sysctl-name-prefix "hw.") (sysctl-name-prefix "net.routetable.")
    (sysctl-name "kern.ostype") (sysctl-name "kern.osrelease")
    (sysctl-name "kern.osversion") (sysctl-name "kern.version")
    (sysctl-name "kern.hostname") (sysctl-name "machdep.cpu.brand_string")
    (sysctl-name "kern.argmax") (sysctl-name "kern.maxfilesperproc")
    (sysctl-name "kern.usrstack64") (sysctl-name "kern.shreg_private")
    (sysctl-name "kern.osproductversion")
    (sysctl-name "security.mac.lockdown_mode_state"))
; IP networking stays available. Unix sockets and host helper services do not.
(allow network* (local ip "*:*") (remote ip "*:*"))
; DNSServiceQueryRecord uses this one system Unix socket, not a Mach port.
(allow network-outbound (literal "/private/var/run/mDNSResponder"))
(allow mach-lookup
    (global-name "com.apple.system.logger")
    (global-name "com.apple.system.opendirectoryd.libinfo")
    (global-name "com.apple.SystemConfiguration.configd")
    (global-name "com.apple.mDNSResponder")
    ; Go uses SecTrustEvaluate for HTTPS. This is not the securityd keychain service.
    (global-name "com.apple.trustd.agent"))
; dyld reads / itself; this does not grant its descendants.
(allow file-read* (literal "/"))
(allow file-read-metadata (literal "/var") (literal "/tmp") (literal "/etc"))
(allow file-read* (subpath "/private/var/select")
    (literal "/dev/null") (literal "/dev/random") (literal "/dev/urandom")
    (literal "/private/etc/localtime") (literal "/private/etc/hosts")
    (literal "/private/etc/resolv.conf")
    (literal "/Library/Preferences/com.apple.dt.Xcode.plist")
    (literal "/private/etc/ssl/openssl.cnf") (literal "/private/etc/ssl/cert.pem"))
(allow file-write* (literal "/dev/null"))
"#,
    );
    for path in runtimes {
        policy.push_str(&format!(
            "(allow file-read* file-map-executable (subpath \"{}\"))\n",
            quote(path)
        ));
    }
    policy.push_str(&format!(
        "(allow file-read* file-write* file-map-executable (subpath \"{}\"))\n",
        quote(workspace)
    ));
    // getcwd and runtime path resolution need ancestor metadata, never data.
    for path in runtimes.iter().map(PathBuf::as_path).chain([workspace]) {
        for ancestor in path.ancestors().skip(1) {
            policy.push_str(&format!(
                "(allow file-read-metadata (literal \"{}\"))\n",
                quote(ancestor)
            ));
        }
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    async fn output(workspace: &Path, script: &str) -> std::process::Output {
        command(script, workspace).unwrap().output().await.unwrap()
    }

    #[test]
    fn availability_requires_real_enforcement() {
        available().unwrap();
        assert!(probe(Path::new("/does-not-exist/sandbox-exec")).is_err());
        assert!(probe(Path::new("/usr/bin/true")).is_err());
        let root = tempfile::tempdir().unwrap();
        let fake = root.path().join("fake-sandbox");
        std::fs::write(&fake, "#!/bin/sh\nshift 2\nexec \"$@\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            probe(&fake).is_err(),
            "an unrestricted launcher passed the probe"
        );
    }

    #[tokio::test]
    async fn aliases_symlinks_and_children_cannot_access_host_data() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("work space \"quoted\"");
        let other = root.path().join("other");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&other).unwrap();
        for name in [".env", "credentials", "vault", "teammate-tape"] {
            let secret = other.join(name);
            std::fs::write(&secret, "host-canary").unwrap();
            symlink(&secret, workspace.join("absolute")).unwrap();
            symlink(format!("../other/{name}"), workspace.join("relative")).unwrap();
            for path in [secret.to_str().unwrap(), "absolute", "relative"] {
                for script in [
                    format!("cat '{path}'"),
                    format!("/bin/sh -c \"cat '{path}'\""),
                    format!("echo changed > '{path}'"),
                ] {
                    let result = output(&workspace, &script).await;
                    assert!(!result.status.success(), "{script}");
                    assert!(!String::from_utf8_lossy(&result.stdout).contains("host-canary"));
                    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "host-canary");
                }
            }
            std::fs::remove_file(workspace.join("absolute")).unwrap();
            std::fs::remove_file(workspace.join("relative")).unwrap();
        }
    }

    #[tokio::test]
    async fn host_process_environment_and_signals_are_not_available() {
        let root = tempfile::tempdir().unwrap();
        let mut host = std::process::Command::new("/bin/sleep")
            .arg("30")
            .env("TOAD_HOST_CANARY", "host-process-secret")
            .spawn()
            .unwrap();
        let pid = host.id();
        let read = output(root.path(), &format!("/bin/ps eww -p {pid}")).await;
        let signal = output(root.path(), &format!("kill -0 {pid}")).await;
        let _ = host.kill();
        let _ = host.wait();
        assert!(!String::from_utf8_lossy(&read.stdout).contains("host-process-secret"));
        assert!(!signal.status.success(), "{signal:?}");
    }

    #[tokio::test]
    async fn private_scratch_and_hostile_setup_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let outside = root.path().join("never-created");
        symlink(&outside, work.join(".toad-home")).unwrap();
        assert!(!output(&work, "echo should-not-run").await.status.success());
        assert!(!outside.exists());
        std::fs::remove_file(work.join(".toad-home")).unwrap();
        std::fs::create_dir(work.join(".toad-home")).unwrap();
        symlink(&outside, work.join(".toad-home/.tmp")).unwrap();
        assert!(!output(&work, "echo should-not-run").await.status.success());
        assert!(!outside.exists());
        std::fs::remove_file(work.join(".toad-home/.tmp")).unwrap();
        let result = output(
            &work,
            "echo private > \"$TMPDIR/marker\" && cat \"$TMPDIR/marker\"",
        )
        .await;
        assert!(result.status.success(), "{result:?}");
        let host_tmp = tempfile::NamedTempFile::new_in("/tmp").unwrap();
        for path in [
            host_tmp.path().to_path_buf(),
            host_tmp.path().canonicalize().unwrap(),
        ] {
            let result = output(&work, &format!("echo changed > '{}'", path.display())).await;
            assert!(!result.status.success(), "{result:?}");
        }
    }

    #[tokio::test]
    async fn host_helper_services_and_unix_control_sockets_are_denied() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        symlink(&socket, work.join("socket")).unwrap();
        // Even a socket placed directly in the allowed workspace cannot
        // delegate work to an unsandboxed listener.
        let inside = std::os::unix::net::UnixListener::bind(work.join("inside.sock")).unwrap();
        inside.set_nonblocking(true).unwrap();
        for script in [
            "/usr/bin/open -a Terminal".to_string(),
            "/usr/bin/osascript -e 'tell application \"System Events\" to get name'".to_string(),
            "/bin/launchctl list".to_string(),
            "python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.connect(\"socket\")'"
                .to_string(),
        ] {
            let result = output(&work, &script).await;
            assert!(
                !result.status.success(),
                "helper escaped: {script}: {result:?}"
            );
        }
        assert!(listener.accept().is_err());
        assert!(inside.accept().is_err());
    }

    #[tokio::test]
    async fn ip_networking_still_works() {
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nnetwork-ok")
                .await
                .unwrap();
        });
        let result = output(
            root.path(),
            &format!("/usr/bin/curl --noproxy '*' --max-time 5 -fsS http://{address}"),
        )
        .await;
        server.abort();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, b"network-ok");
    }

    #[tokio::test]
    #[ignore = "requires public DNS and HTTPS"]
    async fn public_dns_and_https_remain_available() {
        let root = tempfile::tempdir().unwrap();
        let result = output(
            root.path(),
            "/usr/bin/curl --max-time 15 -fsS https://example.com",
        )
        .await;
        assert!(result.status.success(), "{result:?}");
    }

    #[tokio::test]
    #[ignore = "requires public package registries"]
    async fn package_downloads_use_private_caches() {
        let root = tempfile::tempdir().unwrap();
        for (tool, script, cache) in [
            ("npm", "npm view is-number version", ".cache/npm"),
            (
                "go",
                "go mod init example.com/smoke && go get golang.org/x/text@v0.3.8",
                "go/pkg/mod",
            ),
        ] {
            if !supported_tool_on_path(tool) {
                continue;
            }
            let result = output(root.path(), script).await;
            assert!(result.status.success(), "{tool}: {result:?}");
            assert!(root.path().join(".toad-home").join(cache).exists());
        }
    }

    #[tokio::test]
    async fn home_runtime_exceptions_do_not_include_credentials_or_neighbors() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let host_home = root.join("host");
        let work = root.join("work");
        std::fs::create_dir_all(host_home.join(".cargo/bin")).unwrap();
        std::fs::create_dir_all(host_home.join("project")).unwrap();
        std::fs::create_dir(&work).unwrap();
        std::fs::write(host_home.join(".cargo/credentials.toml"), "host-canary").unwrap();
        std::fs::write(host_home.join("project/.env"), "host-canary").unwrap();
        symlink(
            host_home.join("project"),
            host_home.join(".cargo/bin/redirect"),
        )
        .unwrap();
        let runtimes = runtimes(Some(&host_home));
        for relative in [
            ".cargo/credentials.toml",
            "project/.env",
            ".cargo/bin/redirect/.env",
        ] {
            let mut process = launcher(&work, &runtimes);
            process.arg("/bin/cat").arg(host_home.join(relative));
            let result = process.output().await.unwrap();
            assert!(!result.status.success(), "{result:?}");
            assert!(!String::from_utf8_lossy(&result.stdout).contains("host-canary"));
        }
    }

    #[test]
    fn redirected_installations_never_grant_the_target() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let other = root.path().join("other");
        std::fs::create_dir_all(home.join(".rustup")).unwrap();
        std::fs::create_dir(&other).unwrap();
        symlink(&other, home.join(".rustup/toolchains")).unwrap();
        let paths = runtimes(Some(&home));
        assert!(!paths.contains(&other));
        assert!(!paths.contains(&home.join(".rustup/toolchains")));
    }
}
