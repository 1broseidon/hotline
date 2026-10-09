//! Tauri's default Windows manifest reaches the app binary as a compiled
//! resource for bins only. The library's test binary links Tauri without it,
//! so it loads the Common Controls that predate `TaskDialogIndirect` and dies
//! with STATUS_ENTRYPOINT_NOT_FOUND before a single test runs. So the same
//! manifest is linked into every binary this crate produces instead.
fn main() {
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let attributes = if msvc {
        let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    // Without an app manifest Tauri allows every registered app command from
    // every window, including the deliberately unprivileged computer viewer.
    let attributes = attributes.app_manifest(tauri_build::AppManifest::new().commands(&[
        "notify",
        "desk_pair_link",
        "desk_pair_ssh",
        "desk_forget",
        "desk_list",
        "desk_viewer_token",
        "open_sent_file",
        "save_sent_file",
        "transfer_pick",
        "transfer_read",
        "transfer_begin",
        "transfer_write",
        "transfer_end",
        "laptop_browsers",
        "laptop_cookies_preview",
        "laptop_cookies_push",
        "get_update_status",
        "check_update",
        "install_update",
        "cancel_update",
        "speech_capability",
        "speech_permit",
        "speech_start",
        "speech_stop",
        "speech_cancel",
    ]));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_speech();
    }
}

/// On-device speech is Apple's Speech framework, reachable only from Swift,
/// so the Swift in `macos/speech` is compiled here into a static library the
/// crate links (`src/speech.rs` calls its C entry points). The Swift runtime
/// itself is the system's, in `/usr/lib/swift` on every macOS this app runs on.
fn build_speech() {
    use std::path::PathBuf;
    use std::process::Command;

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sources_dir = manifest.join("macos").join("speech");
    println!("cargo:rerun-if-changed={}", sources_dir.display());
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&sources_dir)
        .expect("macos/speech is readable")
        .map(|entry| entry.expect("macos/speech is listable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "swift")
        })
        .collect();
    sources.sort();

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("no Swift target for macOS on {other}"),
    };
    // The bundle's minimumSystemVersion in tauri.conf.json.
    let target = format!("{arch}-apple-macos13.0");

    let run = |command: &mut Command| -> Vec<u8> {
        let output = command
            .output()
            .unwrap_or_else(|error| panic!("{command:?} did not run: {error}"));
        if !output.status.success() {
            panic!(
                "{command:?} failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        output.stdout
    };
    let sdk = run(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]));
    let sdk = String::from_utf8(sdk).unwrap().trim().to_string();

    run(Command::new("xcrun")
        .args(["--sdk", "macosx", "swiftc"])
        .args(["-parse-as-library", "-emit-library", "-static"])
        .args(["-module-name", "HotlineSpeech", "-swift-version", "5", "-O"])
        .args(["-target", &target, "-sdk", &sdk])
        .arg("-module-cache-path")
        .arg(out.join("swift-module-cache"))
        .arg("-o")
        .arg(out.join("libhotline_speech.a"))
        .args(&sources));
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=hotline_speech");

    // Where the linker finds the Swift runtime and its back-deployment
    // shims, as the compiler reports them for this target.
    let info = run(Command::new("xcrun")
        .args(["--sdk", "macosx", "swiftc", "-print-target-info"])
        .args(["-target", &target]));
    let info: serde_json::Value = serde_json::from_slice(&info).expect("target info is JSON");
    for path in info["paths"]["runtimeLibraryPaths"]
        .as_array()
        .expect("target info lists runtime library paths")
    {
        println!(
            "cargo:rustc-link-search=native={}",
            path.as_str().expect("a runtime library path is a string")
        );
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
