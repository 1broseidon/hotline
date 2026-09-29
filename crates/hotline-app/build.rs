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
    ]));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
