//! Local invalidation precedes a bounded best-effort provider sign-out.

use super::*;
use crate::contract::CredentialKind;
use crate::vault::Vault;
use std::time::Duration;

/// Serialize deletion of one login so concurrent owner sockets cannot
/// revoke the same tokens twice. No vault writer lock crosses an await.
pub(crate) async fn delete(vault: &Vault, id: &str) -> io::Result<()> {
    let dir = vault.login_dir(id);
    let lock = record_lock(&dir.join("delete"));
    let _deleting = lock.lock().await;
    let credential = vault.find(id)?;
    if credential.credential_kind == CredentialKind::Oauth
        && matches!(
            credential.provider_id.as_str(),
            "openai-codex" | "github-copilot"
        )
    {
        vault.revoke(id)?;
        if credential.provider_id == "openai-codex" {
            let auth = chatgpt::auth::Authenticator::new(
                chatgpt::auth::AuthSource::OAuth,
                LoginDirStore::new(&dir),
                DeviceCodeHandler::default(),
                false,
            );
            let _ = tokio::time::timeout(Duration::from_secs(10), sign_out(&auth, &dir)).await;
            auth.forget();
        }
    }
    vault.delete(id)
}

async fn sign_out(auth: &chatgpt::auth::Authenticator, dir: &Path) {
    #[cfg(not(feature = "test-support"))]
    let _ = dir;
    #[cfg(feature = "test-support")]
    if let Some(origin) = crate::provider_fixture::origin(&dir.to_string_lossy()) {
        let _ = auth
            .sign_out(&crate::provider_fixture::AuthHttp::new(origin))
            .await;
        return;
    }
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let _ = auth.sign_out(&ReqwestClient::from(client)).await;
}
