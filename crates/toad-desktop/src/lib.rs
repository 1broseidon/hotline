//! The desktop shell: one window onto the room, which runs in this process.
//!
//! There is no child process. The core is a library; this binds its door on
//! a loopback port, hands the page the port and a token before it loads, and
//! opens a window on it once the page has loaded. Everything the window
//! does from then on is the
//! wire's business. Plugins remember the window's place, pick folders, open
//! links, and write the clipboard; on Linux and Windows one posts toasts too,
//! while on macOS the notification center does, through `notify`, because it
//! is the one that hands a click back. The judgement for all of it lives in
//! the page, not here. The page draws the window's top strip on
//! every platform. On macOS the menu bar is this process's, and its items
//! emit an event the window handles. On Linux and Windows there is no menu
//! bar and no system frame: the strip carries the window's controls too,
//! so the window is one material to its edge.
//! Closing the window hides it; the tray is how the person gets it back and
//! how they actually quit, because the room keeps working either way.

#[cfg(target_os = "macos")]
mod notify;

mod remote;
mod updater;

#[cfg(target_os = "macos")]
mod shell_path;

use rand::RngCore;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use tauri::Emitter;
#[cfg(target_os = "macos")]
use tauri::menu::SubmenuBuilder;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
#[cfg(not(target_os = "macos"))]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
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
/// nobody can see is no shortcut. `CmdOrCtrl` is Cmd here, matching the
/// window's own listener, which is Cmd on macOS and Ctrl elsewhere. The rows
/// themselves live in `ui/src/chords.ts`; a change here that is not a change
/// there is a drift. macOS only: a Mac app without a menu bar has no Quit and
/// no Paste, while elsewhere the webview answers those chords itself and the
/// page lists the rest under the rail's More menu.
///
/// The app and Window menus carry the items every Mac app has — Hide, Hide
/// Others, Services, Minimize, Zoom — because a hand that presses Cmd+H or
/// Cmd+M expects them to work, and AppKit adds Emoji & Symbols to an Edit
/// menu and a search field to a Help menu on its own once each is
/// registered as such.
#[cfg(target_os = "macos")]
fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItemBuilder::with_id("settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let about = MenuItemBuilder::with_id("about", "About Toad").build(app)?;
    let app_menu = SubmenuBuilder::new(app, "Toad")
        .item(&about)
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let new_teammate = MenuItemBuilder::with_id("new-teammate", "New Teammate")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&new_teammate)
        .separator()
        .close_window()
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
        .accelerator("CmdOrCtrl+F")
        .build(app)?;
    let teammate = MenuItemBuilder::with_id("teammate", "Teammate")
        .accelerator("CmdOrCtrl+I")
        .build(app)?;
    let mut view = SubmenuBuilder::new(app, "View")
        .item(&search)
        .item(&teammate)
        .separator();
    for n in 1..=9 {
        let item = MenuItemBuilder::with_id(format!("teammate-{n}"), format!("Teammate {n}"))
            .accelerator(format!("CmdOrCtrl+{n}"))
            .build(app)?;
        view = view.item(&item);
    }
    let view = view.build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize_with_text("Zoom")
        .separator()
        .bring_all_to_front()
        .build()?;
    window.set_as_windows_menu_for_nsapp()?;

    let shortcuts = MenuItemBuilder::with_id("shortcuts", "Keyboard Shortcuts").build(app)?;
    let github = MenuItemBuilder::with_id("github", "Toad on GitHub").build(app)?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&shortcuts)
        .item(&github)
        .build()?;
    help.set_as_help_menu_for_nsapp()?;

    let menu = MenuBuilder::new(app)
        .item(&app_menu)
        .item(&file)
        .item(&edit)
        .item(&view)
        .item(&window)
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

/// Show, unminimize and focus the main window. The tray's Open, a left click
/// on Linux and Windows, and a macOS dock reopen of a hidden app all mean
/// the same thing: the room is still here, bring the window back.
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// The tray is how the person gets the window back and how they actually
/// quit. There is no setting for this: a teammate mid-build that dies
/// because someone closed a window is the failure this exists to stop.
fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItemBuilder::with_id("open", "Open Toad").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Toad").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&open)
        .separator()
        .item(&quit)
        .build()?;

    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Toad")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        });
    #[cfg(target_os = "macos")]
    let tray = tray.icon_as_template(true);
    // Linux and Windows: a left click is Open. macOS shows the menu, the
    // platform's convention. Tauri 2 does not emit tray clicks on Linux, so
    // the menu is the way back there.
    #[cfg(not(target_os = "macos"))]
    let tray = tray
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    tray.build(app)?;
    Ok(())
}

/// Size and place, not whether the window is on screen: a hidden window is
/// not a preference, and the next launch still opens the room.
fn window_state_flags() -> StateFlags {
    StateFlags::all() & !StateFlags::VISIBLE
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    shell_path::restore();

    // Opened inside the async runtime because the room it stands up owns
    // background work — the idle chapter sweep — and a task has to be spawned
    // onto a runtime that is already there.
    let root = data_root();
    let desk = Arc::new(
        tauri::async_runtime::block_on(async { Desk::open(&root) }).expect("the desk did not open"),
    );
    let token = random_token();
    let remote = toad_core::remote::Remote::open(&root, desk.log.clone(), desk.clone())
        .expect("remote access settings did not open");
    tauri::async_runtime::block_on(remote.restore());
    let door = Door::bind(desk.log.clone(), token.clone(), desk.clone())
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
        "computerImage": toad_core::computer::default_image(),
    });
    let script = format!("window.__toadDesk = {injected};");

    let builder = tauri::Builder::default()
        .manage(desk)
        .manage(remote)
        .manage(updater::Updates::new(
            &root,
            env!("CARGO_PKG_VERSION").to_string(),
            tauri::is_dev() || cfg!(debug_assertions),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_notification::init());
    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        notify::notify,
        updater::get_update_status,
        updater::check_update,
        updater::install_update,
        updater::cancel_update,
        remote::remote_status,
        remote::remote_configure,
        remote::remote_pairing,
        remote::remote_revoke
    ]);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        updater::get_update_status,
        updater::check_update,
        updater::install_update,
        updater::cancel_update,
        remote::remote_status,
        remote::remote_configure,
        remote::remote_pairing,
        remote::remote_revoke
    ]);
    builder
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            {
                install_menu(app)?;
                notify::install(app.handle());
            }
            // The window stays hidden until the page has loaded, because a
            // window shown first is a white frame before the well paints,
            // and a dark theme notices. Shown once: a reload of the page
            // while the window is hidden in the tray must not raise it.
            let shown = std::sync::Once::new();
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title(if tauri::is_dev() {
                    "Toad · Development"
                } else {
                    "Toad"
                })
                .inner_size(1280.0, 860.0)
                .min_inner_size(520.0, 420.0)
                .center()
                .visible(false)
                .on_page_load(move |window, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        shown.call_once(|| {
                            let _ = window.show();
                        });
                    }
                })
                .initialization_script(&script);
            // Overlay puts the traffic lights on the page, and the inset
            // puts their centre on the top strip's centre line: 16 logical
            // pixels down, half the strip's 32. The number is tao's, not a
            // centre — it resizes AppKit's title bar container and the
            // buttons keep their seat in it, so the centre lands 2 above
            // the given y; measured, not derived. The strip leaves 80 for
            // them (index.css). A click into an inactive
            // window lands on what it hit, the way Finder's does, rather
            // than only waking the window. Elsewhere the system frame is
            // dropped and the page draws its own strip and controls.
            #[cfg(target_os = "macos")]
            let window = window
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .traffic_light_position(tauri::LogicalPosition::new(12.0, 18.0))
                .accept_first_mouse(true);
            #[cfg(not(target_os = "macos"))]
            let window = window.decorations(false);
            // A frameless window on Linux is a rectangle unless it is see-through
            // and the page rounds itself; Windows rounds a top-level window on
            // its own and would lose its shadow for the transparency.
            #[cfg(target_os = "linux")]
            let window = window.transparent(true);
            let window = window.build()?;
            // The plugin writes size and place to disk on exit, and only
            // updates its cache on close, so a hide that is not an exit
            // would lose the last place unless we save now.
            window.on_window_event({
                let window = window.clone();
                move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let _ = window.app_handle().save_window_state(window_state_flags());
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            });
            install_tray(app)?;
            updater::start(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            {
                // A dock click of a running app with no visible window is Reopen,
                // not a new launch; without this the icon bounces and nothing
                // appears.
                if let tauri::RunEvent::Reopen {
                    has_visible_windows: false,
                    ..
                } = event
                {
                    show_main_window(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
