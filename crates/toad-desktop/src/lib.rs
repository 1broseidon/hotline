//! The desktop shell: one window onto the room, which runs in this process.
//!
//! There is no child process. The core is a library; this binds its door on
//! a loopback port, hands the page the port and a token before it loads, and
//! opens a window on it. Everything the window does from then on is the
//! wire's business.

use rand::RngCore;
use std::sync::Arc;
use tauri::{WebviewUrl, WebviewWindowBuilder};
use toad_core::desk::Desk;
use toad_core::paths::data_root;
use toad_core::wire::Door;

const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Opened inside the async runtime because the room it stands up owns
    // background work — the idle chapter sweep — and a task has to be spawned
    // onto a runtime that is already there.
    let desk = tauri::async_runtime::block_on(async { Desk::open(&data_root()) })
        .expect("the desk did not open");
    let token = random_token();
    let door = Door::bind(desk.log.clone(), token.clone(), Arc::new(desk))
        .expect("the room's door did not bind");
    let port = door.port();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = door.run().await {
            eprintln!("[door] stopped: {error}");
        }
    });

    let desk = serde_json::json!({
        "platform": PLATFORM,
        "origin": format!("http://127.0.0.1:{port}"),
        "token": token,
    });
    let script = format!("window.__toadDesk = {desk};");

    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Toad")
                .inner_size(1280.0, 860.0)
                .min_inner_size(520.0, 420.0)
                .center()
                .initialization_script(&script)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
