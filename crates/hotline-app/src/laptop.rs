//! This computer's browsers, for a teammate on a server (BRO-145).
//!
//! On this computer's desk the desk itself reads the person's browsers when
//! they bring cookies into a teammate's computer. A desk on a server can only
//! see the server's, so the shell reads this computer's and sends the
//! cookies for the sites the person ticked to the desk itself, as
//! `computer.cookies.push` over the bridge; no value passes through the
//! page. Listing and preview carry domains and counts only, as on the local
//! desk.

use hotline_core::computer::cookies;
use hotline_core::contract::{CookieSite, CookieTransfer, HostBrowser};
use std::path::Path;

#[tauri::command]
pub async fn laptop_browsers() -> Result<Vec<HostBrowser>, String> {
    // Looking for browsers walks the disk; it is not the window's thread's to do.
    tauri::async_runtime::spawn_blocking(cookies::detect)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn laptop_cookies_preview(
    browser_id: String,
    profile_id: String,
) -> Result<Vec<CookieSite>, String> {
    cookies::preview(&browser_id, &profile_id).await
}

/// Brings the ticked sites' cookies from this computer's browser into a
/// teammate's computer on a server. The values go from the browser to the
/// desk in the shell; the page only ever sees what came back, domains and counts.
#[tauri::command]
pub async fn laptop_cookies_push(
    desks: tauri::State<'_, std::sync::Arc<crate::desks::Host>>,
    desk_id: String,
    persona_id: String,
    browser_id: String,
    profile_id: String,
    domains: Vec<String>,
) -> Result<Vec<CookieSite>, String> {
    let cookies = cookies::select(&browser_id, &profile_id, &domains).await?;
    if cookies.is_empty() {
        return Err("None of the chosen sites had cookies to import.".to_string());
    }
    let transfer = CookieTransfer {
        source_id: source_id(desks.root()),
        browser_id,
        profile_id,
        domains,
        cookies,
    };
    let params = serde_json::json!({"personaId": persona_id, "transfer": transfer});
    let imported = desks
        .command(&desk_id, "computer.cookies.push", params)
        .await?;
    serde_json::from_value(imported)
        .map_err(|_| "The desk answered in a way this window does not understand.".to_string())
}

/// A stable id for this computer, so its imports stay apart from the
/// server's own browsers and from another laptop's. Made once and kept.
fn source_id(root: &Path) -> String {
    let path = root.join("laptop-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return id.to_string();
        }
    }
    let id = format!("{:032x}", rand::random::<u128>());
    let _ = std::fs::write(&path, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_id_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let first = source_id(dir.path());
        assert_eq!(first.len(), 32);
        assert_eq!(source_id(dir.path()), first);
    }
}
