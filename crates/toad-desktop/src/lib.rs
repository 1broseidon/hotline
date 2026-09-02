//! The desktop shell: one window onto the room, which runs in this process.
//!
//! There is no child process. The core is a library; this binds its door on
//! a loopback port, hands the page the port and a token before it loads, and
//! opens a window on it. Everything the window does from then on is the
//! wire's business. Plugins remember the window's place, post toasts, pick
//! folders, open links, and write the clipboard; the judgement for those
//! lives in the page, not here. The menu bar is this process's: its items
//! emit an event the window handles.

use rand::RngCore;
use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, WebviewUrl, WebviewWindowBuilder};
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

/// The chords the window already listens for, shown on the menu so a shortcut
/// nobody can see is no shortcut. Ctrl, not Cmd, because the window's own
/// listener is Ctrl on every platform. The rows themselves live in
/// `ui/src/chords.ts`; a change here that is not a change there is a drift.
fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItemBuilder::with_id("settings", "Settings")
        .accelerator("Ctrl+,")
        .build(app)?;
    let app_title = if cfg!(target_os = "macos") {
        "Toad"
    } else {
        "App"
    };
    let about = MenuItemBuilder::with_id("about", "About").build(app)?;
    let app_menu = SubmenuBuilder::new(app, app_title)
        .item(&settings)
        .item(&about)
        .separator()
        .quit()
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let search = MenuItemBuilder::with_id("search", "Search")
        .accelerator("Ctrl+F")
        .build(app)?;
    let new_teammate = MenuItemBuilder::with_id("new-teammate", "New Teammate")
        .accelerator("Ctrl+N")
        .build(app)?;
    let teammate = MenuItemBuilder::with_id("teammate", "Teammate")
        .accelerator("Ctrl+I")
        .build(app)?;
    let mut view = SubmenuBuilder::new(app, "View")
        .item(&search)
        .separator()
        .item(&new_teammate)
        .item(&teammate)
        .separator();
    for n in 1..=9 {
        let item = MenuItemBuilder::with_id(format!("teammate-{n}"), format!("Teammate {n}"))
            .accelerator(format!("Ctrl+{n}"))
            .build(app)?;
        view = view.item(&item);
    }
    let view = view.build()?;
    let shortcuts = MenuItemBuilder::with_id("shortcuts", "Keyboard shortcuts").build(app)?;
    let about_toad = MenuItemBuilder::with_id("about", "About Toad").build(app)?;
    let github = MenuItemBuilder::with_id("github", "Toad on GitHub").build(app)?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&shortcuts)
        .item(&about_toad)
        .item(&github)
        .build()?;
    let menu = MenuBuilder::new(app)
        .item(&app_menu)
        .item(&edit)
        .item(&view)
        .item(&help)
        .build()?;
    app.set_menu(menu)?;

    let handle = app.handle().clone();
    app.on_menu_event(move |_app, event| {
        let id = event.id().as_ref();
        if matches!(
            id,
            "settings" | "search" | "new-teammate" | "teammate" | "about" | "shortcuts" | "github"
        ) || id.starts_with("teammate-")
        {
            let _ = handle.emit("toad://menu", id);
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Opened inside the async runtime because the room it stands up owns
    // background work — the idle chapter sweep — and a task has to be spawned
    // onto a runtime that is already there.
    let root = data_root();
    let desk =
        tauri::async_runtime::block_on(async { Desk::open(&root) }).expect("the desk did not open");
    let token = random_token();
    let door = Door::bind(desk.log.clone(), token.clone(), Arc::new(desk))
        .expect("the room's door did not bind");
    let port = door.port();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = door.run().await {
            eprintln!("[door] stopped: {error}");
        }
    });

    let injected = serde_json::json!({
        "platform": PLATFORM,
        "origin": format!("http://127.0.0.1:{port}"),
        "token": token,
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": root.display().to_string(),
    });
    let script = format!("window.__toadDesk = {injected};");

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            install_menu(app)?;
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Toad")
                .inner_size(1280.0, 860.0)
                .min_inner_size(520.0, 420.0)
                .center()
                .initialization_script(&script);
            // Overlay puts the traffic lights on the page so the rail header
            // can sit on their centre line. Elsewhere the OS keeps a plain bar
            // and this builder leaves decorations alone.
            #[cfg(target_os = "macos")]
            let window = window
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
            window.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
