//! Run startup in a fresh process: mutating PATH in a threaded test runner
//! would race other tests and would not model a Finder launch.
#[cfg(target_os = "macos")]
#[path = "../src/shell_path.rs"]
mod shell_path;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use toad_core::driver::acp::registry::cached_backends;

    if std::env::var_os("TOAD_PATH_TEST_CHILD").is_some() {
        let root = std::env::var_os("TOAD_DATA_DIR").unwrap();
        let root = std::path::Path::new(&root);
        assert!(
            cached_backends(root)
                .iter()
                .find(|b| b.id == "claude-acp")
                .unwrap()
                .unavailable
                .is_some()
        );
        shell_path::restore();
        assert!(
            cached_backends(root)
                .iter()
                .find(|b| b.id == "claude-acp")
                .unwrap()
                .unavailable
                .is_none()
        );
        let output = Command::new("docker")
            .arg("pull")
            .arg("test-image")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"helper found");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("version manager bin");
    std::fs::create_dir(&bin).unwrap();
    let shell = root.path().join("login-shell");
    let scripts = [
        (
            shell.clone(),
            format!(
                "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 1\nexport PATH='{}':/usr/bin:/bin\nprintf 'startup noise\\n'\neval \"$2\"\n",
                bin.display()
            ),
        ),
        (bin.join("claude"), "#!/bin/sh\nexit 0\n".into()),
        (bin.join("npx"), "#!/bin/sh\nexit 0\n".into()),
        (
            bin.join("docker"),
            "#!/bin/sh\nexec docker-credential-test\n".into(),
        ),
        (
            bin.join("docker-credential-test"),
            "#!/bin/sh\nprintf 'helper found'\n".into(),
        ),
    ];
    for (path, contents) in scripts {
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .env("TOAD_PATH_TEST_CHILD", "1")
        .env("TOAD_DATA_DIR", root.path())
        .env("HOME", root.path())
        .env("SHELL", shell)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .status()
        .unwrap();
    assert!(status.success());
    println!("Finder PATH: ACP discovery and Docker credential helper passed");
}
