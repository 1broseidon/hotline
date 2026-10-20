//! Hotline's two halves of MCP.
//!
//! The client — the servers the room knows, the grant a teammate gets, the
//! live connections a session holds and OAuth sign-in — is the `hotline-mcp`
//! crate, re-exported here so `crate::mcp::McpServer` still holds. That
//! crate never opens the vault: what the vault keeps for a server is read
//! here and handed in, and a registration an OAuth sign-in makes is saved
//! back through the one call it is given.
//!
//! Hotline's own teammate tools are the other half of MCP, a server that
//! calls into the live room, and they live in [`server`].

pub mod server;

pub use hotline_mcp::{
    CallContent, CallError, CallImage, Connections, FailedServer, Grant, HttpAuth, McpServer,
    McpTool, McpTransport, NO_SAVED_TOKEN, connect, grant, missing_reason, servers, unsupported,
};
pub(crate) use hotline_mcp::{
    McpOAuthRegistration, Presented, launch_values_pending, manager_for_server, public_room_event,
    public_servers, validate_http_url,
};
pub use hotline_room::mcp_servers::normalize_servers;

use crate::driver::CapabilityLease;
use crate::vault::{McpCredentialStore, Vault};
use hotline_mcp::{Granted, McpOAuthService, OAuthVault, Vaulted};
use serde_json::Value;
use std::sync::Arc;

/// Connect granted servers with access to the protected MCP OAuth store.
/// Standalone callers without a vault retain the old refusal behavior.
///
/// Each server's launch values are resolved from the vault here, and what
/// it keeps for the server read, before the client connects any of them; a
/// server the vault cannot resolve fails alone.
pub(crate) async fn connect_with_capability_and_vault(
    persona_id: &str,
    servers: &[McpServer],
    capability: Option<CapabilityLease>,
    vault: Option<Arc<Vault>>,
) -> Connections {
    let granted = servers
        .iter()
        .map(|server| match &vault {
            None => Granted {
                server: server.clone(),
                vaulted: Ok(Vaulted::empty()),
            },
            Some(vault) => match vault.resolve_mcp_server(server) {
                Ok(resolved) => Granted {
                    vaulted: Ok(vaulted(vault, &resolved)),
                    server: resolved,
                },
                Err(error) => Granted {
                    server: server.clone(),
                    vaulted: Err(error.to_string()),
                },
            },
        })
        .collect();
    hotline_mcp::connect_granted(persona_id, granted, capability).await
}

/// What the vault keeps for one server: a bearer or header server's saved
/// token, or an OAuth server's tokens, lock and registration.
fn vaulted(vault: &Arc<Vault>, server: &McpServer) -> Vaulted<McpCredentialStore> {
    match &server.transport {
        McpTransport::Http {
            url,
            auth: HttpAuth::Secret { .. },
        } => Vaulted {
            secret: vault
                .mcp_secret(&server.id, url)
                .map_err(|error| error.to_string()),
            oauth: None,
        },
        McpTransport::Http {
            auth: HttpAuth::Oauth | HttpAuth::OauthConfigured { .. },
            ..
        } => Vaulted {
            secret: Ok(None),
            oauth: Some(oauth_vault(vault, server)),
        },
        _ => Vaulted::empty(),
    }
}

/// An OAuth server's credential store, token lock and saved registration,
/// each bound to the server's id and URL.
pub(crate) fn oauth_vault(
    vault: &Arc<Vault>,
    server: &McpServer,
) -> OAuthVault<McpCredentialStore> {
    let url = url_of(server);
    OAuthVault {
        store: vault.mcp_credential_store(&server.id, url),
        token_lock: vault.mcp_token_lock(&server.id),
        registration: vault
            .mcp_oauth_registration(&server.id, url)
            .map_err(|error| error.to_string()),
    }
}

/// The URL the vault binds a server's record to. A stdio server has none,
/// and the client refuses to sign one in before it looks at what was read.
fn url_of(server: &McpServer) -> &str {
    match &server.transport {
        McpTransport::Http { url, .. } => url,
        McpTransport::Stdio { .. } => "",
    }
}

/// The MCP OAuth sign-ins the window drives, with the vault each one reads
/// and saves through.
pub(crate) struct McpOAuth {
    vault: Arc<Vault>,
    logins: McpOAuthService,
}

impl McpOAuth {
    pub(crate) fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            logins: McpOAuthService::default(),
        }
    }

    pub(crate) fn set_on_complete(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        self.logins.set_on_complete(callback);
    }

    pub(crate) async fn start(&self, server: &McpServer) -> Result<Value, String> {
        let url = url_of(server);
        self.logins
            .start(server, oauth_vault(&self.vault, server), |registration| {
                self.vault
                    .save_mcp_oauth_registration(&server.id, url, registration)
            })
            .await
    }

    pub(crate) async fn complete(
        &self,
        login_id: &str,
        callback_url: &str,
    ) -> Result<Value, String> {
        self.logins.complete(login_id, callback_url).await
    }

    pub(crate) async fn status(&self, server: &McpServer) -> Result<Value, String> {
        self.logins
            .status(server, vaulted(&self.vault, server))
            .await
    }

    /// The store clears the server's whole record, whatever URL it is bound
    /// to: its registration, its tokens and a saved bearer or header token.
    pub(crate) async fn sign_out(&self, server: &McpServer) -> Result<(), String> {
        self.logins
            .sign_out(
                &server.id,
                self.vault.mcp_token_lock(&server.id),
                self.vault.mcp_credential_store(&server.id, url_of(server)),
            )
            .await
    }
}

#[cfg(test)]
mod tests;
