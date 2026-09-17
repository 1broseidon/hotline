//! Listener controls are desktop-only; a phone cannot issue pairing invitations.
use hotline_core::remote::{Remote, RemotePairing, RemoteStatus};
use std::sync::Arc;

#[tauri::command]
pub fn remote_status(remote: tauri::State<'_, Arc<Remote>>) -> RemoteStatus {
    remote.status()
}

#[tauri::command]
pub async fn remote_configure(
    remote: tauri::State<'_, Arc<Remote>>,
    enabled: bool,
    host: String,
) -> Result<RemoteStatus, String> {
    remote.inner().configure(enabled, &host).await
}

#[tauri::command]
pub fn remote_pairing(remote: tauri::State<'_, Arc<Remote>>) -> Result<RemotePairing, String> {
    remote.pairing()
}

#[tauri::command]
pub fn remote_revoke(
    remote: tauri::State<'_, Arc<Remote>>,
    device_id: String,
) -> Result<RemoteStatus, String> {
    remote.revoke(&device_id)
}
