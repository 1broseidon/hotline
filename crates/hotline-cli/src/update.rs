//! The script-installed Linux server updater. No unit or room is ever written.
//! Release URLs and the installation path are intentionally not configurable.

use semver::Version;
use std::process::{Command, ExitCode};

const RELEASES: &str = "https://github.com/1broseidon/hotline/releases";
#[cfg(any(target_os = "linux", test))]
const BINARY: &str = "/usr/local/bin/hotline";
const GUIDANCE: &str = "self-update supports only the script-installed Linux server at /usr/local/bin/hotline; use your package manager or the desktop app's updater for desktop/AppImage/package installations";
type Result<T> = std::result::Result<T, String>;

#[derive(Debug, PartialEq)]
struct Options {
    check: bool,
    version: Option<Version>,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self {
            check: false,
            version: None,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--check" && !options.check {
                options.check = true;
            } else if (arg == "--version" || arg.starts_with("--version="))
                && options.version.is_none()
            {
                let value = if arg == "--version" {
                    args.next()
                        .map(String::as_str)
                        .ok_or("--version needs X.Y.Z")?
                } else {
                    &arg["--version=".len()..]
                };
                let version = parse_version(value)?;
                if !version.pre.is_empty() || !version.build.is_empty() {
                    return Err("--version needs a stable X.Y.Z release".into());
                }
                options.version = Some(version);
            } else {
                return Err(format!(
                    "unknown or duplicate update option: {arg}\nusage: hotline update [--check] [--version X.Y.Z]"
                ));
            }
        }
        Ok(options)
    }

    #[cfg(any(target_os = "linux", test))]
    fn sudo_command(&self) -> String {
        let mut command = format!("sudo {BINARY} update");
        if let Some(version) = &self.version {
            command.push_str(&format!(" --version {version}"));
        }
        command
    }
}

fn parse_version(text: &str) -> Result<Version> {
    Version::parse(text).map_err(|error| format!("invalid release version {text:?}: {error}"))
}

fn curl(url: &str) -> Command {
    let mut command = Command::new("curl");
    // Ignore ~/.curlrc; neither an override nor a redirect may permit HTTP.
    command.args([
        "--disable",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--tlsv1.2",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--connect-timeout",
        "15",
        "--max-time",
        "300",
        url,
    ]);
    command
}

fn output(command: &mut Command) -> Result<Vec<u8>> {
    let result = command
        .output()
        .map_err(|error| format!("could not run {:?}: {error}", command.get_program()))?;
    if !result.status.success() {
        return Err(format!(
            "{:?} failed ({}): {}",
            command.get_program(),
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.stdout)
}

fn latest() -> Result<Version> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        version: String,
    }
    let bytes = output(&mut curl(&format!(
        "{RELEASES}/latest/download/latest.json"
    )))?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid latest.json: {error}"))?;
    let version = parse_version(&manifest.version)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err("latest.json does not name a stable X.Y.Z release".into());
    }
    Ok(version)
}

fn should_update(current: &Version, target: &Version, explicit: bool) -> bool {
    // Build metadata does not change precedence. An automatic update can never downgrade.
    let order = target.cmp_precedence(current);
    order.is_gt() || (explicit && order.is_lt())
}

pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hotline update: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: &[String]) -> Result<()> {
    let options = Options::parse(args)?;
    #[cfg(not(target_os = "linux"))]
    if !options.check {
        return Err(format!(
            "{GUIDANCE}. On macOS or Windows, update the Hotline app instead."
        ));
    }
    #[cfg(target_os = "linux")]
    if !options.check {
        linux::validate_installation()?;
        // SAFETY: geteuid has no arguments or memory effects.
        if unsafe { libc::geteuid() } != 0 {
            return Err(format!(
                "root is required to replace the server binary. Run:\n  {}",
                options.sudo_command()
            ));
        }
        return linux::update(&options);
    }

    // Read-only, including on a developer build: no tempfiles, lock, unit calls or privilege checks.
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let target = match &options.version {
        Some(version) => version.clone(),
        None => latest()?,
    };
    report(&current, &target, options.version.is_some());
    Ok(())
}

fn report(current: &Version, target: &Version, explicit: bool) {
    println!(
        "Installed: {current}\n{}: {target}",
        if explicit { "Requested" } else { "Latest" }
    );
    if should_update(current, target, explicit) {
        println!(
            "{} available{}.",
            if target < current {
                "Downgrade"
            } else {
                "Update"
            },
            if explicit { " (explicit target)" } else { "" }
        );
    } else if target.cmp_precedence(current).is_lt() {
        println!("Installed version is newer; no automatic downgrade.");
    } else {
        println!("Already up to date.");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs::{self, File, OpenOptions};
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    const UNIT: &str = "hotline.service";

    fn io<T>(result: std::io::Result<T>) -> Result<T> {
        result.map_err(|error| error.to_string())
    }

    fn same_file(a: &Path, b: &Path) -> Result<bool> {
        let a = io(fs::metadata(a))?;
        let b = io(fs::metadata(b))?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }

    fn validate_path(executable: &Path, target: &Path) -> Result<()> {
        if executable != target
            || io(fs::canonicalize(target))? != target
            || !io(fs::symlink_metadata(target))?.is_file()
        {
            return Err(GUIDANCE.into());
        }
        Ok(())
    }

    pub(super) fn validate_installation() -> Result<()> {
        let executable = io(std::env::current_exe())?;
        validate_path(&executable, Path::new(BINARY))?;
        let metadata = io(fs::metadata(BINARY))?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "{BINARY} must be root-owned and not group/world-writable"
            ));
        }
        if std::env::var_os("APPIMAGE").is_some() || std::env::var_os("APPDIR").is_some() {
            return Err(GUIDANCE.into());
        }
        // A process waiting on another update must not replace the winner with its stale version.
        if !same_file(Path::new("/proc/self/exe"), Path::new(BINARY))? {
            return Err(
                "the installed executable changed; rerun /usr/local/bin/hotline update".into(),
            );
        }
        for (program, flag) in [("dpkg-query", "-S"), ("rpm", "-qf")] {
            match Command::new(program).args([flag, BINARY]).output() {
                Ok(result) if result.status.success() => {
                    return Err(format!(
                        "{BINARY} is owned by a package; use your package manager, not hotline update"
                    ));
                }
                // Both tools use 1 to report an unowned path. Other failures are not proof of safety.
                Ok(result) if result.status.code() == Some(1) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Ok(result) => {
                    return Err(format!(
                        "cannot establish package ownership: {program} returned {}",
                        result.status
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "cannot establish package ownership: {program}: {error}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn lock_parent(parent: &Path) -> Result<File> {
        let directory = io(File::open(parent))?;
        // Lock the directory inode, not a removable lockfile: concurrent updaters cannot split locks.
        // SAFETY: a live file descriptor and valid flock flags.
        if unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!(
                "another update is running (cannot lock {}): {}",
                parent.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(directory)
    }

    fn secure_parent(parent: &Path) -> Result<()> {
        // All ancestors must be root-owned and unwritable by other users.
        for path in parent.ancestors() {
            let metadata = io(fs::metadata(path))?;
            if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err(format!(
                    "{} must be root-owned and not group/world-writable",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    pub(super) fn update(options: &Options) -> Result<()> {
        let target_path = Path::new(BINARY);
        let parent = target_path.parent().unwrap();
        secure_parent(parent)?;
        let _lock = lock_parent(parent)?;
        validate_installation()?;
        let mut service = SystemService;
        let before = service.snapshot()?;
        let current = parse_version(env!("CARGO_PKG_VERSION"))?;
        if before.active {
            // Refuse an unhealthy or unsupported custom unit before writing anything.
            service
                .probe(target_path, &current, &crate::door::runtime())
                .map_err(|error| {
                    format!(
                        "the existing service is not ready for an update: {error}; nothing changed"
                    )
                })?;
        }
        let target = match &options.version {
            Some(version) => version.clone(),
            None => latest()?,
        };
        report(&current, &target, options.version.is_some());
        if !should_update(&current, &target, options.version.is_some()) {
            return Ok(());
        }
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => return Err(format!("no server release for Linux {other}")),
        };
        let stem = format!("hotline-server_{target}_linux_{arch}");
        let asset = format!("{stem}.tar.gz");
        let work = io(tempfile::Builder::new()
            .prefix(".hotline-update-")
            .tempdir_in(parent))?;
        let archive = work.path().join(&asset);
        let base = format!("{RELEASES}/download/desktop-v{target}");
        let sums = output(&mut curl(&format!("{base}/checksums.txt")))?;
        output(
            curl(&format!("{base}/{asset}"))
                .arg("--output")
                .arg(&archive),
        )?;
        verify_checksum(&archive, &sums, &asset)?;
        let staged = work.path().join("hotline.new");
        extract_binary(&archive, &format!("{stem}/hotline"), &staged)?;
        replace(
            target_path,
            &staged,
            work,
            &mut service,
            &before,
            &target,
            &current,
        )?;
        println!(
            "Hotline {target} installed; {}. Room, pairings and unit unchanged.",
            if before.active {
                "service restarted and ready"
            } else {
                "service left stopped"
            }
        );
        Ok(())
    }

    fn verify_checksum(archive: &Path, sums: &[u8], asset: &str) -> Result<()> {
        let sums = std::str::from_utf8(sums).map_err(|_| "checksums.txt is not UTF-8")?;
        let mut matches = sums.lines().filter_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim_start().trim_start_matches('*') == asset).then_some(hash)
        });
        let expected = matches
            .next()
            .ok_or_else(|| format!("checksums.txt has no {asset}"))?;
        if matches.next().is_some()
            || expected.len() != 64
            || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("invalid or duplicate checksum for {asset}"));
        }
        let mut file = io(File::open(archive))?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = io(file.read(&mut buffer))?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        if !hex::encode(hash.finalize()).eq_ignore_ascii_case(expected) {
            return Err(format!(
                "SHA256 mismatch for {asset}; the installed binary was not changed"
            ));
        }
        Ok(())
    }

    fn extract_binary(archive: &Path, member: &str, staged: &Path) -> Result<()> {
        let file = io(OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(staged))?;
        // stdout extraction creates no archive paths, links, unit or room files. No wildcards.
        // Ignore TAR_OPTIONS, which could otherwise change selection or execute a checkpoint command.
        let result = io(Command::new("tar")
            .env_remove("TAR_OPTIONS")
            .arg("--extract")
            .arg("--gzip")
            .arg("--to-stdout")
            .arg("--no-recursion")
            .arg("--occurrence=1")
            .arg("--file")
            .arg(archive)
            .arg("--")
            .arg(member)
            .stdout(Stdio::from(file))
            .stderr(Stdio::piped())
            .output())?;
        if !result.status.success() {
            return Err(format!(
                "cannot extract {member}: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        let mut file = io(File::open(staged))?;
        let mut magic = [0; 4];
        io(file.read_exact(&mut magic))
            .map_err(|_| "archive member is not a server executable".to_string())?;
        if &magic != b"\x7fELF" {
            return Err("archive member is not a Linux ELF executable".into());
        }
        Ok(())
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Snapshot {
        active: bool,
        pid: u32,
        exec: String,
    }

    trait Service {
        fn snapshot(&mut self) -> Result<Snapshot>;
        fn restart_ready(&mut self, binary: &Path, version: &Version) -> Result<()>;
    }

    struct SystemService;

    fn systemctl(args: &[&str]) -> Result<String> {
        let bytes = output(
            Command::new("systemctl")
                .env("LC_ALL", "C")
                .arg("--no-pager")
                .args(args)
                .arg(UNIT),
        )?;
        String::from_utf8(bytes).map_err(|_| "systemctl returned invalid UTF-8".into())
    }

    fn parse_snapshot(text: &str) -> Result<Snapshot> {
        let properties: std::collections::HashMap<_, _> = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let get = |key| properties.get(key).copied().unwrap_or("");
        if get("LoadState") != "loaded"
            || get("FragmentPath") != "/etc/systemd/system/hotline.service"
            || get("NeedDaemonReload") != "no"
        {
            return Err("hotline.service must be the loaded script-installed unit, with no pending daemon-reload; use systemctl status hotline.service to inspect it".into());
        }
        let exec = get("ExecStart");
        if exec.matches("{ path=").count() != 1
            || !exec.starts_with(&format!("{{ path={BINARY} ; argv[]={BINARY} serve "))
        {
            return Err(format!(
                "hotline.service ExecStart must directly run {BINARY} serve; {GUIDANCE}"
            ));
        }
        let pid: u32 = get("MainPID")
            .parse()
            .map_err(|_| "invalid hotline.service MainPID")?;
        let active = match (get("ActiveState"), get("SubState"), pid) {
            ("active", "running", 1..) => true,
            ("inactive", "dead", 0) => false,
            _ => {
                return Err(format!(
                    "hotline.service is in ambiguous state {}/{} (PID {pid}); wait for a stable running or stopped service before updating",
                    get("ActiveState"),
                    get("SubState")
                ));
            }
        };
        // systemctl includes volatile timing/status fields after argv[]. They must not influence identity.
        let exec = exec
            .split(" ; ignore_errors=")
            .next()
            .unwrap_or(exec)
            .to_owned();
        Ok(Snapshot { active, pid, exec })
    }

    impl Service for SystemService {
        fn snapshot(&mut self) -> Result<Snapshot> {
            parse_snapshot(&systemctl(&[
                "show",
                "--property=LoadState,FragmentPath,NeedDaemonReload,ExecStart,ActiveState,SubState,MainPID",
            ])?)
        }

        fn restart_ready(&mut self, binary: &Path, version: &Version) -> Result<()> {
            systemctl(&["restart"])?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let runtime = crate::door::runtime();
            let mut stable_pid = 0;
            let mut successes = 0;
            let mut last_error = "service has not become ready".to_owned();
            while Instant::now() < deadline {
                let probe = self.probe(binary, version, &runtime);
                match probe {
                    Ok(pid) => {
                        successes = if stable_pid == pid { successes + 1 } else { 1 };
                        stable_pid = pid;
                        // Type=exec being active alone isn't ready. Require three successful Door probes.
                        if successes >= 3 {
                            return Ok(());
                        }
                    }
                    Err(error) => {
                        last_error = error;
                        successes = 0;
                        stable_pid = 0;
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(format!(
                "service did not become ready within 30 seconds: {last_error}"
            ))
        }
    }

    impl SystemService {
        fn probe(
            &mut self,
            binary: &Path,
            version: &Version,
            runtime: &tokio::runtime::Runtime,
        ) -> Result<u32> {
            let state = self.snapshot()?;
            if !state.active {
                return Err("service is stopped".into());
            }
            verify_process(state.pid, binary)?;
            let data = process_data(state.pid)?;
            let door = crate::door::running(&data)?;
            if door.pid != state.pid || door.version != version.to_string() {
                return Err("Door does not belong to the expected service process/version".into());
            }
            runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(2),
                    crate::door::ask(&door, "welcome", serde_json::json!({})),
                )
                .await
                .map_err(|_| "service Door readiness timed out".to_string())?
            })?;
            if self.snapshot()? != state {
                return Err("service changed during readiness probe".into());
            }
            Ok(state.pid)
        }
    }

    fn verify_process(pid: u32, binary: &Path) -> Result<()> {
        if !same_file(&PathBuf::from(format!("/proc/{pid}/exe")), binary)? {
            return Err("hotline.service MainPID is not running the installed executable".into());
        }
        Ok(())
    }

    fn data_from_process(args: &[u8], environment: &[u8]) -> Result<PathBuf> {
        let args = std::str::from_utf8(args).map_err(|_| "service command line is not UTF-8")?;
        let environment =
            std::str::from_utf8(environment).map_err(|_| "service environment is not UTF-8")?;
        let mut args = args.split_terminator('\0');
        let mut data = None;
        while let Some(arg) = args.next() {
            let value = if arg == "--data" {
                Some(args.next().ok_or("service --data has no value")?)
            } else {
                arg.strip_prefix("--data=")
            };
            if let Some(value) = value
                && data.replace(value).is_some()
            {
                return Err("service has duplicate --data flags".into());
            }
        }
        let data = data.or_else(|| environment.split('\0').find_map(|entry| entry.strip_prefix("HOTLINE_DATA_DIR=")))
            .filter(|path| !path.is_empty()).ok_or("cannot safely discover the service room; its unit must set HOTLINE_DATA_DIR or --data (the script-installed unit does)")?;
        Ok(PathBuf::from(data))
    }

    fn process_data(pid: u32) -> Result<PathBuf> {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let data = data_from_process(
            &io(fs::read(proc.join("cmdline")))?,
            &io(fs::read(proc.join("environ")))?,
        )?;
        if data.is_absolute() {
            Ok(data)
        } else {
            Ok(io(fs::read_link(proc.join("cwd")))?.join(data))
        }
    }

    fn sync_directory(path: &Path) -> Result<()> {
        io(File::open(path))?
            .sync_all()
            .map_err(|error| error.to_string())
    }

    fn prepare_binary(path: &Path) -> Result<()> {
        io(fs::set_permissions(path, fs::Permissions::from_mode(0o755)))?;
        io(File::open(path))?
            .sync_all()
            .map_err(|error| error.to_string())
    }

    fn restore(target: &Path, work: &Path) -> Result<()> {
        // Never consume the backup: even a failed directory fsync or recovery restart leaves it available.
        let rollback = work.join("hotline.rollback");
        // This name is reserved before replacement, so recovery allocates no
        // executable contents even when the filesystem has filled up.
        io(fs::rename(&rollback, target))?;
        sync_directory(target.parent().unwrap())?;
        sync_directory(work)
    }

    fn replace(
        target: &Path,
        staged: &Path,
        work: tempfile::TempDir,
        service: &mut impl Service,
        before: &Snapshot,
        version: &Version,
        previous: &Version,
    ) -> Result<()> {
        prepare_binary(staged)?;
        if service.snapshot()? != *before {
            return Err(
                "hotline.service changed while downloading; nothing replaced, retry when stable"
                    .into(),
            );
        }
        let backup = work.path().join("hotline.previous");
        if fs::hard_link(target, &backup).is_err() {
            io(fs::copy(target, &backup))?;
        }
        io(File::open(&backup))?
            .sync_all()
            .map_err(|error| error.to_string())?;
        // Reserve a second name now, not during an out-of-space recovery. On
        // filesystems without hard links, reserve the copy before risking the
        // installed executable; failure here leaves the installation untouched.
        let rollback = work.path().join("hotline.rollback");
        if fs::hard_link(&backup, &rollback).is_err() {
            io(fs::copy(&backup, &rollback))?;
        }
        io(File::open(&rollback))?
            .sync_all()
            .map_err(|error| error.to_string())?;
        sync_directory(work.path())?;
        sync_directory(target.parent().unwrap())?;
        // From this point onward, failed recovery must not allow TempDir's destructor to erase the backup.
        let work = work.keep();
        if let Err(error) = fs::rename(staged, target) {
            let _ = fs::remove_dir_all(&work);
            return Err(format!(
                "cannot replace {}: {error}; original binary unchanged",
                target.display()
            ));
        }
        let mut restart_attempted = false;
        let activation: Result<()> = (|| {
            sync_directory(target.parent().unwrap())?;
            sync_directory(&work)?;
            // Do not turn an externally stopped/stopping service back on. The lock serializes
            // updaters, not an administrator's concurrent systemctl commands.
            if service.snapshot()? != *before {
                return Err("service changed during replacement; refusing to restart it".into());
            }
            if before.active {
                restart_attempted = true;
                service.restart_ready(target, version)?;
            }
            Ok(())
        })();
        if let Err(error) = activation {
            let recovery = (|| {
                restore(target, &work)?;
                if restart_attempted {
                    service.restart_ready(target, previous)?;
                }
                Ok::<(), String>(())
            })();
            return match recovery {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&work);
                    let _ = sync_directory(target.parent().unwrap());
                    Err(format!(
                        "update failed: {error}; previous binary restored{}",
                        if restart_attempted {
                            " and service is ready again"
                        } else {
                            " (service was not restarted)"
                        }
                    ))
                }
                Err(recovery) => Err(format!(
                    "update failed: {error}; RECOVERY FAILED: {recovery}. Backup preserved at {}. Inspect systemctl status hotline.service and recover manually; room and unit were not changed",
                    backup.display()
                )),
            };
        }
        io(fs::remove_dir_all(&work))?;
        sync_directory(target.parent().unwrap())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;

        struct FakeService {
            state: Snapshot,
            restarts: Vec<String>,
            results: VecDeque<Result<()>>,
        }
        impl Service for FakeService {
            fn snapshot(&mut self) -> Result<Snapshot> {
                Ok(self.state.clone())
            }
            fn restart_ready(&mut self, _: &Path, version: &Version) -> Result<()> {
                self.restarts.push(version.to_string());
                self.results.pop_front().unwrap_or(Ok(()))
            }
        }
        fn fixture(
            active: bool,
        ) -> (
            tempfile::TempDir,
            PathBuf,
            tempfile::TempDir,
            PathBuf,
            FakeService,
        ) {
            let parent = tempfile::tempdir().unwrap();
            let target = parent.path().join("hotline");
            fs::write(&target, b"old binary").unwrap();
            let work = tempfile::tempdir_in(parent.path()).unwrap();
            let staged = work.path().join("new");
            fs::write(&staged, b"new binary").unwrap();
            let service = FakeService {
                state: Snapshot {
                    active,
                    pid: if active { 123 } else { 0 },
                    exec: BINARY.into(),
                },
                restarts: vec![],
                results: VecDeque::new(),
            };
            (parent, target, work, staged, service)
        }
        fn apply(
            target: &Path,
            staged: &Path,
            work: tempfile::TempDir,
            service: &mut FakeService,
        ) -> Result<()> {
            let before = service.state.clone();
            replace(
                target,
                staged,
                work,
                service,
                &before,
                &Version::new(2, 0, 0),
                &Version::new(1, 0, 0),
            )
        }
        #[test]
        fn stopped_service_stays_stopped_and_replacement_is_atomic() {
            let (_parent, target, work, staged, mut service) = fixture(false);
            let mut old = File::open(&target).unwrap();
            apply(&target, &staged, work, &mut service).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"new binary");
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);
            let mut bytes = Vec::new();
            old.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"old binary");
            assert!(service.restarts.is_empty());
        }
        #[test]
        fn active_service_restarts_into_the_requested_version() {
            let (_parent, target, work, staged, mut service) = fixture(true);
            apply(&target, &staged, work, &mut service).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"new binary");
            assert_eq!(service.restarts, ["2.0.0"]);
        }
        #[test]
        fn service_change_during_download_prevents_replacement() {
            let (_parent, target, work, staged, mut service) = fixture(true);
            let before = service.state.clone();
            service.state.active = false;
            service.state.pid = 0;
            assert!(
                replace(
                    &target,
                    &staged,
                    work,
                    &mut service,
                    &before,
                    &Version::new(2, 0, 0),
                    &Version::new(1, 0, 0)
                )
                .is_err()
            );
            assert_eq!(fs::read(&target).unwrap(), b"old binary");
            assert!(service.restarts.is_empty());
        }
        #[test]
        fn failed_atomic_restore_does_not_consume_backup() {
            let dir = tempfile::tempdir().unwrap();
            let backup = dir.path().join("previous");
            fs::write(&backup, b"old binary").unwrap();
            let invalid_target = dir.path().join("directory-not-a-binary");
            fs::create_dir(&invalid_target).unwrap();
            fs::hard_link(&backup, dir.path().join("hotline.rollback")).unwrap();
            assert!(restore(&invalid_target, dir.path()).is_err());
            assert_eq!(fs::read(&backup).unwrap(), b"old binary");
        }
        #[test]
        fn active_service_restarts_and_failed_readiness_rolls_back() {
            let (_parent, target, work, staged, mut service) = fixture(true);
            service.results.push_back(Err("not ready".into()));
            let original_inode = fs::metadata(&target).unwrap().ino();
            let error = apply(&target, &staged, work, &mut service).unwrap_err();
            assert!(error.contains("previous binary restored"));
            assert_eq!(fs::read(&target).unwrap(), b"old binary");
            // Recovery reuses the reserved inode rather than allocating another
            // executable's worth of space after activation has failed.
            assert_eq!(fs::metadata(&target).unwrap().ino(), original_inode);
            assert_eq!(service.restarts, ["2.0.0", "1.0.0"]);
        }
        #[test]
        fn failed_recovery_preserves_backup() {
            let (_parent, target, work, staged, mut service) = fixture(true);
            let backup = work.path().join("hotline.previous");
            service
                .results
                .extend([Err("new failed".into()), Err("old failed".into())]);
            let error = apply(&target, &staged, work, &mut service).unwrap_err();
            assert!(error.contains("RECOVERY FAILED"));
            assert_eq!(fs::read(backup).unwrap(), b"old binary");
        }
        #[test]
        fn checksum_must_match_one_exact_asset() {
            let dir = tempfile::tempdir().unwrap();
            let archive = dir.path().join("asset");
            fs::write(&archive, b"abc").unwrap();
            let sum = hex::encode(Sha256::digest(b"abc"));
            let line = format!("{sum}  archive.tar.gz\n");
            assert!(verify_checksum(&archive, line.as_bytes(), "archive.tar.gz").is_ok());
            for sums in [
                format!("{sum}  other\n"),
                line.repeat(2),
                format!("{}  archive.tar.gz", "0".repeat(64)),
                "bad  archive.tar.gz".into(),
            ] {
                assert!(verify_checksum(&archive, sums.as_bytes(), "archive.tar.gz").is_err());
            }
        }
        #[test]
        fn validates_self_path_and_rejects_symlinks() {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("hotline");
            fs::write(&target, b"binary").unwrap();
            assert!(validate_path(&target, &target).is_ok());
            assert!(validate_path(Path::new("/usr/bin/hotline-app"), &target).is_err());
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(validate_path(&link, &link).is_err());
        }
        #[test]
        fn directory_lock_blocks_a_second_updater() {
            let dir = tempfile::tempdir().unwrap();
            let first = lock_parent(dir.path()).unwrap();
            assert!(lock_parent(dir.path()).is_err());
            drop(first);
            assert!(lock_parent(dir.path()).is_ok());
        }
        fn properties(active: &str, sub: &str, pid: u32) -> String {
            format!(
                "LoadState=loaded\nFragmentPath=/etc/systemd/system/hotline.service\nNeedDaemonReload=no\nExecStart={{ path={BINARY} ; argv[]={BINARY} serve --store file ; ignore_errors=no ; }}\nActiveState={active}\nSubState={sub}\nMainPID={pid}\n"
            )
        }
        #[test]
        fn service_must_be_unambiguous_and_use_our_binary() {
            assert!(
                parse_snapshot(&properties("active", "running", 123))
                    .unwrap()
                    .active
            );
            assert!(
                !parse_snapshot(&properties("inactive", "dead", 0))
                    .unwrap()
                    .active
            );
            for (active, sub, pid) in [
                ("activating", "start", 123),
                ("deactivating", "stop", 123),
                ("failed", "failed", 0),
                ("active", "exited", 0),
                ("inactive", "dead", 123),
            ] {
                assert!(parse_snapshot(&properties(active, sub, pid)).is_err());
            }
            assert!(
                parse_snapshot(
                    &properties("active", "running", 123).replace(BINARY, "/usr/bin/hotline-app")
                )
                .is_err()
            );
            assert!(
                parse_snapshot(
                    &properties("active", "running", 123)
                        .replace("NeedDaemonReload=no", "NeedDaemonReload=yes")
                )
                .is_err()
            );
        }
        #[test]
        fn room_discovery_respects_data_override() {
            let env = b"HOME=/var/lib/hotline\0HOTLINE_DATA_DIR=/var/lib/hotline/room\0";
            assert_eq!(
                data_from_process(b"hotline\0serve\0", env).unwrap(),
                Path::new("/var/lib/hotline/room")
            );
            for args in [
                b"hotline\0serve\0--data\0/custom\0".as_slice(),
                b"hotline\0serve\0--data=/custom\0",
            ] {
                assert_eq!(data_from_process(args, env).unwrap(), Path::new("/custom"));
            }
            assert!(data_from_process(b"hotline\0serve\0", b"HOME=/home/hotline\0").is_err());
        }
        #[test]
        fn only_exact_archive_binary_is_extracted() {
            let dir = tempfile::tempdir().unwrap();
            let stem = "hotline-server_2.0.0_linux_x86_64";
            let pack = dir.path().join(stem);
            fs::create_dir(&pack).unwrap();
            fs::write(pack.join("hotline"), b"\x7fELFtest").unwrap();
            fs::write(pack.join("hotline.service"), b"do not install").unwrap();
            let archive = dir.path().join("archive.tar.gz");
            assert!(
                Command::new("tar")
                    .env_remove("TAR_OPTIONS")
                    .arg("-czf")
                    .arg(&archive)
                    .arg("-C")
                    .arg(dir.path())
                    .arg(stem)
                    .status()
                    .unwrap()
                    .success()
            );
            let staged = dir.path().join("staged");
            extract_binary(&archive, &format!("{stem}/hotline"), &staged).unwrap();
            assert_eq!(fs::read(staged).unwrap(), b"\x7fELFtest");
            assert!(!dir.path().join("hotline.service").exists());
            assert!(
                extract_binary(&archive, "other/hotline", &dir.path().join("missing")).is_err()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Result<Options> {
        Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }
    #[test]
    fn strict_update_options() {
        assert_eq!(
            options(&[]).unwrap(),
            Options {
                check: false,
                version: None
            }
        );
        assert!(options(&["--check", "--version=1.2.3"]).unwrap().check);
        assert_eq!(
            options(&["--version", "1.2.3"]).unwrap().sudo_command(),
            "sudo /usr/local/bin/hotline update --version 1.2.3"
        );
        for args in [
            vec!["--version"],
            vec!["--version", "v1.2.3"],
            vec!["--version", "1.2"],
            vec!["--version", "1.2.3-beta"],
            vec!["--version", "../foo"],
            vec!["--version", "1.2.3", "--version", "2.0.0"],
            vec!["--check", "--check"],
            vec!["--data", "/room"],
            vec!["--help"],
        ] {
            assert!(options(&args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn semantic_versions_never_accidentally_downgrade() {
        let current = Version::new(1, 9, 0);
        assert!(should_update(&current, &Version::new(1, 10, 0), false));
        assert!(!should_update(&current, &Version::new(1, 8, 0), false));
        assert!(should_update(&current, &Version::new(1, 8, 0), true));
        assert!(!should_update(&current, &current, true));
        assert!(!should_update(
            &parse_version("1.9.0+dev").unwrap(),
            &current,
            false
        ));
    }
    #[test]
    fn explicit_check_is_read_only_even_in_a_developer_build() {
        assert!(run_inner(&["--check".into(), "--version".into(), "999.0.0".into()]).is_ok());
    }
}
