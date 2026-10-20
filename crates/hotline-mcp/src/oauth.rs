//! OAuth integration for HTTP MCP servers.
//!
//! rmcp owns the OAuth protocol state machine: protected-resource and
//! authorization-server discovery, DCR, PKCE, token exchange and refresh.
//! This module supplies Hotline's policy around that state machine. In particular,
//! it refuses rmcp's legacy endpoint fallback, requires advertised S256 PKCE,
//! binds registrations and credentials to the configured URL and issuer, and
//! never lets a missing token turn a configured connection into an anonymous
//! one.

use crate::{HttpAuth, McpServer, McpTransport, NO_VAULT, Vaulted};
use rmcp::ServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationMetadata,
    AuthorizationMetadataResolution, AuthorizationMetadataSource, AuthorizationRequest,
    AuthorizationSession, CredentialStore, OAuthClientConfig,
};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    StreamableHttpError, StreamableHttpPostResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;
use url::Url;
use uuid::Uuid;

type OAuthRunningService = (
    RunningService<rmcp::RoleClient, rmcp::model::ClientInfo>,
    Vec<rmcp::model::Tool>,
);
type OAuthCompleteCallback = Arc<dyn Fn() + Send + Sync>;
type OAuthCompleteSlot = Arc<StdMutex<Option<OAuthCompleteCallback>>>;

/// The client identity Hotline received from an MCP authorization server. The
/// secret is kept beside the token in the vault record; this type never crosses
/// the room or wire boundary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct McpOAuthRegistration {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub issuer: Option<String>,
    pub resource: String,
    pub scopes: Vec<String>,
}

/// What the vault keeps for one OAuth server, as the core reads it and hands
/// it in. The tokens stay behind `store`, loaded for one SDK operation at a
/// time; the registration is read once, by the caller, for the call it is
/// handed to.
pub struct OAuthVault<S> {
    /// rmcp's credential store, bound to the server's id and URL, so a token
    /// minted for one origin is never sent to another.
    pub store: S,
    /// Held across a token's whole read, refresh and save. The vault hands
    /// out one per server, shared by every manager in the process.
    pub token_lock: Arc<AsyncMutex<()>>,
    /// The client registration saved for this URL, or why it could not be
    /// read.
    pub registration: Result<Option<McpOAuthRegistration>, String>,
}

/// Connect one already signed-in OAuth server through rmcp's auth-aware
/// Streamable HTTP transport.
pub async fn connect_oauth<S: CredentialStore + Clone + 'static>(
    server: &McpServer,
    vault: &OAuthVault<S>,
) -> Result<OAuthRunningService, String> {
    let token_lock = vault.token_lock.clone();
    let manager = manager_for_server(server, vault, None).await?;
    {
        let _token_guard = token_lock.lock().await;
        manager
            .get_access_token()
            .await
            .map_err(auth_error_message)?;
    }

    let McpTransport::Http { url, .. } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    // A stalled MCP request must eventually release the shared token lock,
    // otherwise even sign-out would wait indefinitely for the remote server.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "MCP OAuth HTTP client could not be created".to_string())?;
    let auth_client = AuthClient::new(client, manager);
    let auth_client = LockedOAuthClient {
        inner: auth_client,
        token_lock,
    };
    let transport = StreamableHttpClientTransport::with_client(
        auth_client,
        StreamableHttpClientTransportConfig::with_uri(url.clone()),
    );
    super::handshake(super::hotline_client().serve(transport)).await
}

/// rmcp's refresh guard prevents two callers from entering the refresh HTTP
/// request at once, but its expiry decision happens before that guard is
/// acquired. Independent long-lived sessions can therefore both decide that
/// the same token is stale. Serialize the whole auth-aware transport call for
/// this server so a later manager reloads the rotated credential before it
/// makes its own expiry decision. The lock is per server and shared by Rig,
/// ACP proxies and all managers created from this vault.
#[derive(Clone)]
struct LockedOAuthClient {
    inner: AuthClient<reqwest::Client>,
    token_lock: Arc<AsyncMutex<()>>,
}

impl LockedOAuthClient {
    async fn current_token(&self) -> Result<String, StreamableHttpError<reqwest::Error>> {
        self.inner
            .get_access_token()
            .await
            .map_err(StreamableHttpError::Auth)
    }
}

impl StreamableHttpClient for LockedOAuthClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<http::HeaderName, http::HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let _token_guard = self.token_lock.lock().await;
        let token = self.current_token().await?;
        self.inner
            .post_message(uri, message, session_id, Some(token), custom_headers)
            .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<http::HeaderName, http::HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let _token_guard = self.token_lock.lock().await;
        let token = self.current_token().await?;
        self.inner
            .post_message_with_max_sse_event_size(
                uri,
                message,
                session_id,
                Some(token),
                custom_headers,
                max_sse_event_size,
            )
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        _auth_header: Option<String>,
        custom_headers: HashMap<http::HeaderName, http::HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let _token_guard = self.token_lock.lock().await;
        let token = self.current_token().await?;
        self.inner
            .delete_session(uri, session_id, Some(token), custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<http::HeaderName, http::HeaderValue>,
    ) -> Result<
        rmcp::transport::common::client_side_sse::BoxedSseResponse,
        StreamableHttpError<Self::Error>,
    > {
        let _token_guard = self.token_lock.lock().await;
        let token = self.current_token().await?;
        self.inner
            .get_stream(uri, session_id, last_event_id, Some(token), custom_headers)
            .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<http::HeaderName, http::HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<
        rmcp::transport::common::client_side_sse::BoxedSseResponse,
        StreamableHttpError<Self::Error>,
    > {
        let _token_guard = self.token_lock.lock().await;
        let token = self.current_token().await?;
        self.inner
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                Some(token),
                custom_headers,
                max_sse_event_size,
            )
            .await
    }
}

/// Build a manager for a configured server and restore its protected
/// credentials when they are present. The redirect is supplied only while a
/// browser authorization flow is being prepared.
pub async fn manager_for_server<S: CredentialStore + Clone + 'static>(
    server: &McpServer,
    vault: &OAuthVault<S>,
    redirect_uri: Option<&str>,
) -> Result<AuthorizationManager, String> {
    let McpTransport::Http { url, auth } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    validate_server_url(url, "MCP server URL")?;

    let store = vault.store.clone();
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(auth_error_message)?;
    manager.set_credential_store(store.clone());

    let resolution = resolve_metadata_with_challenge(&manager, url).await?;
    validate_resolution(url, &resolution)?;
    let metadata = resolution.metadata.clone();
    manager.set_metadata(metadata.clone());

    let configured = oauth_config(auth);
    if let Some(resource) = configured
        .as_ref()
        .and_then(|config| config.resource.as_deref())
    {
        validate_resource(resource)?;
        validate_resource_binding(url, resource)?;
    }
    let saved = vault.registration.clone()?;
    if let Some(registration) = &saved
        && registration.issuer.as_deref() != metadata.issuer.as_deref()
    {
        return Err(
            "saved MCP OAuth registration belongs to a different authorization server; sign in again"
                .to_string(),
        );
    }
    if let Some(registration) = &saved {
        validate_resource(&registration.resource)?;
        validate_resource_binding(url, &registration.resource)?;
        if let Some(configured_resource) = configured
            .as_ref()
            .and_then(|config| config.resource.as_deref())
            && registration.resource != configured_resource
        {
            return Err(
                "saved MCP OAuth registration targets a different resource; sign in again"
                    .to_string(),
            );
        }
    }

    // When discovery reaches the authorization server directly, rmcp has no
    // protected-resource document from which to populate its resource
    // indicator. Rebase the manager to the operator's already validated
    // resource (or the saved DCR resource) so RFC 8707's `resource` parameter
    // still names the intended MCP resource. Protected-resource discovery
    // keeps rmcp's authoritative resource value instead.
    let configured_resource = configured
        .as_ref()
        .and_then(|config| config.resource.clone());
    let manager_resource = configured_resource.or_else(|| {
        saved
            .as_ref()
            .map(|registration| registration.resource.clone())
    });
    if matches!(
        resolution.source,
        AuthorizationMetadataSource::AuthorizationServerMetadata
    ) && manager_resource
        .as_deref()
        .is_some_and(|resource| resource != url)
    {
        let resource = manager_resource.expect("resource was checked above");
        let mut rebased = AuthorizationManager::new(&resource)
            .await
            .map_err(auth_error_message)?;
        rebased.set_credential_store(store);
        rebased.set_metadata(metadata.clone());
        manager = rebased;
    }

    // A saved DCR response takes precedence over a client id in settings. The
    // latter is public configuration and may be used for preregistered
    // servers, but its secret can only come from the vault.
    let registration = saved.or_else(|| {
        configured
            .as_ref()
            .and_then(|config| config.client_id.as_ref())
            .map(|client_id| McpOAuthRegistration {
                client_id: client_id.clone(),
                client_secret: None,
                redirect_uri: redirect_uri.map_or_else(loopback_redirect, str::to_string),
                issuer: metadata.issuer.clone(),
                resource: configured
                    .as_ref()
                    .and_then(|config| config.resource.clone())
                    .unwrap_or_else(|| url.clone()),
                scopes: configured
                    .as_ref()
                    .map(|config| config.scopes.clone())
                    .unwrap_or_default(),
            })
    });

    let redirect = redirect_uri
        .map(str::to_string)
        .or_else(|| {
            registration
                .as_ref()
                .map(|registration| registration.redirect_uri.clone())
        })
        .unwrap_or_else(loopback_redirect);
    validate_redirect_uri(&redirect)?;

    if let Some(registration) = &registration {
        let mut config = OAuthClientConfig::new(registration.client_id.clone(), redirect.clone());
        if let Some(secret) = registration.client_secret.clone() {
            config = config.with_client_secret(secret);
        }
        config = config.with_scopes(registration.scopes.clone());
        manager
            .configure_client(config)
            .map_err(auth_error_message)?;
    }

    // rmcp's initializer checks the stored issuer and configures the saved
    // client id. It deliberately clears tokens minted by another issuer.
    manager
        .initialize_from_store()
        .await
        .map_err(auth_error_message)?;

    // initialize_from_store configures only a client id. Reapply the protected
    // secret after it so confidential registrations can refresh and rotate.
    if let Some(registration) = &registration {
        let mut config = OAuthClientConfig::new(registration.client_id.clone(), redirect);
        if let Some(secret) = registration.client_secret.clone() {
            config = config.with_client_secret(secret);
        }
        config = config.with_scopes(registration.scopes.clone());
        manager
            .configure_client(config)
            .map_err(auth_error_message)?;
    }
    Ok(manager)
}

/// Start a browser authorization flow. The returned session owns rmcp's
/// in-memory PKCE/state store and must stay alive until the native callback is
/// handled. A registration this flow makes is saved through
/// `save_registration` before the browser opens.
async fn authorization_session<S: CredentialStore + Clone + 'static>(
    server: &McpServer,
    vault: &OAuthVault<S>,
    redirect_uri: &str,
    save_registration: impl FnOnce(McpOAuthRegistration) -> io::Result<()>,
) -> Result<AuthorizationSession, String> {
    let mut manager = manager_for_server(server, vault, Some(redirect_uri)).await?;
    let McpTransport::Http { url, auth } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    let metadata = discover_metadata(server).await?;
    let registration = vault.registration.clone()?;
    let configured = oauth_config(auth);
    let mut request = AuthorizationRequest::new(redirect_uri)
        .with_client_name("Hotline MCP Gateway")
        .with_application_type("native");
    let scopes = registration
        .as_ref()
        .map(|registration| registration.scopes.clone())
        .or_else(|| configured.as_ref().map(|config| config.scopes.clone()))
        .unwrap_or_default();
    if !scopes.is_empty() {
        request = request.with_scopes(scopes.clone());
    }
    if let Some(registration) = registration {
        request = request.with_preregistered_client(registration.client_id);
        if let Some(secret) = registration.client_secret {
            request = request.with_client_secret(secret);
        }
    } else if let Some(client_id) = configured.clone().and_then(|config| config.client_id) {
        // Keep the public preregistered identity and its resource binding in
        // the protected record too. This makes a configured client survive a
        // restart with the same issuer/resource checks as a DCR client; only
        // the client id is taken from settings.
        let configured_registration = McpOAuthRegistration {
            client_id: client_id.clone(),
            client_secret: None,
            redirect_uri: redirect_uri.to_string(),
            issuer: metadata.issuer.clone(),
            resource: configured
                .as_ref()
                .and_then(|config| config.resource.clone())
                .unwrap_or_else(|| url.clone()),
            scopes: scopes.clone(),
        };
        save_registration(configured_registration).map_err(|error| error.to_string())?;
        request = request.with_preregistered_client(client_id);
    } else {
        // Register here rather than hiding the response inside
        // AuthorizationSession::new: rmcp intentionally exposes only the
        // resulting token, while Hotline must retain a returned client secret in
        // its protected boundary for future refreshes.
        let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        let config = manager
            .register_client("Hotline MCP Gateway", redirect_uri, &scope_refs)
            .await
            .map_err(auth_error_message)?;
        let registration = McpOAuthRegistration {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            redirect_uri: redirect_uri.to_string(),
            issuer: metadata.issuer.clone(),
            resource: configured
                .as_ref()
                .and_then(|config| config.resource.clone())
                .unwrap_or_else(|| url.clone()),
            scopes: scopes.clone(),
        };
        save_registration(registration).map_err(|error| error.to_string())?;
        request = request.with_preregistered_client(config.client_id);
        if let Some(secret) = config.client_secret {
            request = request.with_client_secret(secret);
        }
    }

    AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, error)| auth_error_message(error))
}

/// A browser login in progress. The session and its state store remain in
/// memory; only the resulting registration and token are persisted, through
/// what the core hands each call.
#[derive(Default)]
pub struct McpOAuthService {
    pending: Arc<StdMutex<HashMap<String, Arc<PendingLogin>>>>,
    start_locks: Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    on_complete: OAuthCompleteSlot,
}

struct PendingLogin {
    login_id: String,
    server_id: String,
    server_url: String,
    redirect_uri: String,
    authorization_url: String,
    csrf_state: String,
    /// The vault's lock for this server, taken across the token exchange.
    token_lock: Arc<AsyncMutex<()>>,
    session: AsyncMutex<Option<AuthorizationSession>>,
    state: AsyncMutex<OAuthLoginState>,
    shutdown: tokio_util::sync::CancellationToken,
}

const CALLBACK_DEADLINE: Duration = Duration::from_secs(600);
const CALLBACK_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum OAuthLoginState {
    Pending,
    Complete,
    Failed(String),
    SignedOut,
}

impl McpOAuthService {
    pub fn set_on_complete(&self, callback: OAuthCompleteCallback) {
        *self
            .on_complete
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(callback);
    }

    pub async fn start<S: CredentialStore + Clone + 'static>(
        &self,
        server: &McpServer,
        vault: OAuthVault<S>,
        save_registration: impl FnOnce(McpOAuthRegistration) -> io::Result<()>,
    ) -> Result<Value, String> {
        let McpTransport::Http { url, auth } = &server.transport else {
            return Err("MCP OAuth sign-in is only available for HTTP servers".to_string());
        };
        if !matches!(auth, HttpAuth::Oauth | HttpAuth::OauthConfigured { .. }) {
            return Err(
                "MCP OAuth sign-in requires this HTTP server's authentication mode to be OAuth"
                    .to_string(),
            );
        }
        let start_lock = self.start_lock(&server.id);
        let _start_guard = start_lock.lock().await;
        let existing = {
            let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending
                .values()
                .find(|login| login.server_id == server.id && login.server_url == *url)
                .cloned()
        };
        if let Some(existing) = existing
            && *existing.state.lock().await == OAuthLoginState::Pending
        {
            return Ok(existing.to_value("pending").await);
        }

        let saved = vault.registration.clone()?;
        let saved_redirect = saved
            .as_ref()
            .map(|registration| registration.redirect_uri.clone());
        let saved_url = saved_redirect
            .as_deref()
            .map(Url::parse)
            .transpose()
            .map_err(|error| format!("saved MCP OAuth redirect URI is invalid: {error}"))?;
        let port = saved_url.as_ref().and_then(Url::port);
        let path = saved_url
            .as_ref()
            .filter(|redirect| is_loopback_host(redirect.host_str()))
            .map(|redirect| redirect.path().to_string())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| callback_path(&server.id));
        let listener = match port {
            Some(port) => TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|error| {
                    format!(
                        "saved MCP OAuth callback port {port} is unavailable; sign out and sign in again ({error})"
                    )
                })?,
            None => TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|error| format!("could not open the native OAuth callback: {error}"))?,
        };
        let callback_port = listener
            .local_addr()
            .map_err(|error| format!("could not read the native OAuth callback address: {error}"))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{callback_port}{path}");
        validate_redirect_uri(&redirect_uri)?;
        let session =
            authorization_session(server, &vault, &redirect_uri, save_registration).await?;
        let authorization_url = session.get_authorization_url().to_string();
        let csrf_state = Url::parse(&authorization_url)
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "state")
                    .map(|(_, value)| value.into_owned())
            })
            .ok_or_else(|| "MCP OAuth authorization response did not include state".to_string())?;
        let login = Arc::new(PendingLogin {
            login_id: Uuid::new_v4().to_string(),
            server_id: server.id.clone(),
            server_url: url.clone(),
            redirect_uri,
            authorization_url,
            csrf_state,
            token_lock: vault.token_lock.clone(),
            session: AsyncMutex::new(Some(session)),
            state: AsyncMutex::new(OAuthLoginState::Pending),
            shutdown: tokio_util::sync::CancellationToken::new(),
        });
        let login_id = login.login_id.clone();
        let replaced = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let replaced = pending
                .values()
                .filter(|existing| existing.server_id == server.id)
                .cloned()
                .collect::<Vec<_>>();
            pending.retain(|_, existing| existing.server_id != server.id);
            pending.insert(login_id, login.clone());
            replaced
        };
        for old_login in replaced {
            old_login.shutdown.cancel();
        }
        let service = self.clone_for_task();
        let callback_login = login.clone();
        tokio::spawn(async move {
            service.callback_loop(listener, callback_login).await;
        });
        Ok(login.to_value("pending").await)
    }

    pub async fn complete(&self, login_id: &str, callback_url: &str) -> Result<Value, String> {
        let login = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(login_id)
            .cloned()
            .ok_or_else(|| "MCP OAuth login is no longer pending".to_string())?;
        self.complete_login(&login, callback_url).await?;
        Ok(login.to_value("signed_in").await)
    }

    pub async fn status<S: CredentialStore + Clone + 'static>(
        &self,
        server: &McpServer,
        vaulted: Vaulted<S>,
    ) -> Result<Value, String> {
        let McpTransport::Http { url, auth } = &server.transport else {
            return Err("MCP OAuth status is only available for HTTP servers".to_string());
        };
        // A secret server has no login: its status is whether the vault
        // holds its token, in the same words the window already reads.
        if let HttpAuth::Secret { .. } = auth {
            let saved = vaulted
                .secret
                .map_err(|error| format!("could not read the saved MCP token: {error}"))?
                .is_some();
            return Ok(json!({
                "serverId": server.id,
                "status": if saved { "signed_in" } else { "signed_out" },
            }));
        }
        if !matches!(auth, HttpAuth::Oauth | HttpAuth::OauthConfigured { .. }) {
            return Err(
                "MCP OAuth status requires this HTTP server's authentication mode to be OAuth"
                    .to_string(),
            );
        }
        let pending = {
            let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            pending
                .values()
                .find(|login| login.server_id == server.id && login.server_url == *url)
                .cloned()
        };
        if let Some(login) = pending {
            let state = login.state.lock().await.clone();
            match state {
                OAuthLoginState::Pending => return Ok(login.to_value("pending").await),
                OAuthLoginState::Failed(_) => return Ok(login.to_value("failed").await),
                OAuthLoginState::SignedOut => return Ok(login.to_value("signed_out").await),
                // A completed in-memory session is only a record of how the
                // last login happened. Read the vault below so status polling
                // still notices expiry and a rejected refresh in this process.
                OAuthLoginState::Complete => {}
            }
        }
        let Some(vault) = vaulted.oauth else {
            return Err(NO_VAULT.to_string());
        };
        let Some(credentials) = vault.store.load().await.map_err(auth_error_message)? else {
            return Ok(json!({
                "serverId": server.id,
                "status": "signed_out",
            }));
        };
        if credentials.token_response.is_none() {
            return Ok(json!({
                "serverId": server.id,
                "status": "signed_out",
            }));
        }
        if !token_needs_refresh(&credentials) {
            return Ok(json!({
                "serverId": server.id,
                "status": "signed_in",
            }));
        }

        match manager_for_server(server, &vault, None).await {
            Ok(manager) => {
                let result = {
                    let _token_guard = vault.token_lock.lock().await;
                    manager.get_access_token().await
                };
                match result {
                    Ok(_) => Ok(json!({
                        "serverId": server.id,
                        "status": "signed_in",
                    })),
                    Err(error) => Ok(json!({
                        "serverId": server.id,
                        "status": "failed",
                        "error": auth_error_message(error),
                    })),
                }
            }
            Err(error) => Ok(json!({
                "serverId": server.id,
                "status": "failed",
                "error": error,
            })),
        }
    }

    /// Forgets the server's sign-in: `store` clears its whole record, the
    /// registration and a saved bearer or header token with it, once
    /// `token_lock` is held.
    pub async fn sign_out(
        &self,
        server_id: &str,
        token_lock: Arc<AsyncMutex<()>>,
        store: impl CredentialStore,
    ) -> Result<(), String> {
        let start_lock = self.start_lock(server_id);
        let _start_guard = start_lock.lock().await;
        let pending: Vec<Arc<PendingLogin>> = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|login| login.server_id == server_id)
            .cloned()
            .collect();
        for login in pending {
            *login.state.lock().await = OAuthLoginState::SignedOut;
            login.shutdown.cancel();
        }
        // A refresh can already have loaded the old token when sign-out is
        // requested. Wait for the same outer lock used by every gateway
        // connection before deleting the record, so a rotated token cannot be
        // written back after sign-out.
        let _token_guard = token_lock.lock().await;
        store.clear().await.map_err(|error| {
            let error = match error {
                AuthError::CredentialStoreError(error) => error,
                error => error.to_string(),
            };
            format!("could not sign out of MCP OAuth: {error}")
        })
    }

    fn clone_for_task(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            start_locks: self.start_locks.clone(),
            on_complete: self.on_complete.clone(),
        }
    }

    fn start_lock(&self, server_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .start_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        locks
            .entry(server_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    async fn callback_loop(&self, listener: TcpListener, login: Arc<PendingLogin>) {
        let callback_port = match listener.local_addr() {
            Ok(address) => address.port(),
            Err(error) => {
                *login.state.lock().await = OAuthLoginState::Failed(format!(
                    "MCP OAuth callback listener could not be inspected: {error}"
                ));
                return;
            }
        };
        let deadline_at = Instant::now() + CALLBACK_DEADLINE;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline_at) => {
                    let mut state = login.state.lock().await;
                    if *state == OAuthLoginState::Pending {
                        *state = OAuthLoginState::Failed("MCP OAuth sign-in timed out.".to_string());
                    }
                    break;
                }
                () = login.shutdown.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((mut socket, _)) = accepted else { break; };
                    let request = read_callback_request(
                        &mut socket,
                        callback_port,
                        deadline_at,
                        &login.shutdown,
                    ).await;
                    match request {
                        Ok(url) => match self.complete_login(&login, &url).await {
                            Ok(()) => {
                                let body = callback_page("Signed in", "You can close this tab.", false);
                                write_callback_response(&mut socket, "200 OK", &body).await;
                                break;
                            }
                            Err(error) => {
                                let body = callback_page("Sign-in failed", &error, true);
                                write_callback_response(&mut socket, "400 Bad Request", &body).await;
                                let terminal = matches!(
                                    &*login.state.lock().await,
                                    OAuthLoginState::Complete
                                        | OAuthLoginState::Failed(_)
                                        | OAuthLoginState::SignedOut
                                );
                                if terminal {
                                    break;
                                }
                            }
                        },
                        Err(error) => {
                            let body = callback_page("Sign-in failed", &error, true);
                            write_callback_response(&mut socket, "400 Bad Request", &body).await;
                        }
                    }
                }
            }
        }
    }

    async fn complete_login(&self, login: &PendingLogin, callback_url: &str) -> Result<(), String> {
        validate_callback_url(&login.redirect_uri, callback_url)?;
        // Starting/replacing a login and signing out must not race an
        // in-flight callback. In particular, a callback that was accepted just
        // before sign-out must not recreate the protected record afterward.
        let start_lock = self.start_lock(&login.server_id);
        let _start_guard = start_lock.lock().await;
        let parsed = Url::parse(callback_url).map_err(|error| error.to_string())?;
        let callback_state = parsed
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| "MCP OAuth callback did not include state".to_string())?;
        if callback_state != login.csrf_state {
            return Err("MCP OAuth callback state did not match this sign-in.".to_string());
        }
        let mut state = login.state.lock().await;
        match &*state {
            OAuthLoginState::Pending => {}
            OAuthLoginState::Complete => return Ok(()),
            OAuthLoginState::SignedOut => {
                return Err(
                    "MCP OAuth sign-in was signed out before the callback arrived.".to_string(),
                );
            }
            OAuthLoginState::Failed(error) => return Err(error.clone()),
        }
        if parsed.query_pairs().any(|(key, _)| key == "error") {
            *state = OAuthLoginState::Failed(
                "MCP OAuth consent was denied; sign in again to grant access.".to_string(),
            );
            return Err("MCP OAuth consent was denied; sign in again to grant access.".to_string());
        }
        let session = login.session.lock().await;
        let Some(session) = session.as_ref() else {
            return Err("MCP OAuth login is no longer pending".to_string());
        };
        // Token exchange persists credentials. Serialize it with refresh and
        // sign-out so a concurrent sign-out cannot be undone by this save.
        let _token_guard = login.token_lock.lock().await;
        match session.handle_callback_url(callback_url).await {
            Ok(_) => {
                *state = OAuthLoginState::Complete;
                login.shutdown.cancel();
                let callback = self
                    .on_complete
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let Some(callback) = callback {
                    callback();
                }
                Ok(())
            }
            Err(AuthError::InternalError(message))
                if message.to_ascii_lowercase().contains("state not found") =>
            {
                Err("MCP OAuth callback state did not match this sign-in.".to_string())
            }
            Err(error) => {
                let message = auth_error_message(error);
                *state = OAuthLoginState::Failed(message.clone());
                Err(message)
            }
        }
    }
}

impl PendingLogin {
    async fn to_value(&self, status: &str) -> Value {
        let error = match &*self.state.lock().await {
            OAuthLoginState::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let mut value = json!({
            "loginId": self.login_id,
            "serverId": self.server_id,
            "status": status,
            "authorizationUrl": self.authorization_url,
            "redirectUri": self.redirect_uri,
        });
        if let Some(error) = error {
            value["error"] = Value::String(error);
        }
        value
    }
}

async fn discover_metadata(server: &McpServer) -> Result<AuthorizationMetadata, String> {
    let McpTransport::Http { url, .. } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    let manager = AuthorizationManager::new(url)
        .await
        .map_err(auth_error_message)?;
    let resolution = resolve_metadata_with_challenge(&manager, url).await?;
    validate_resolution(url, &resolution)?;
    Ok(resolution.metadata)
}

async fn resolve_metadata_with_challenge(
    manager: &AuthorizationManager,
    server_url: &str,
) -> Result<AuthorizationMetadataResolution, String> {
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(auth_error_message)?;
    if resolution.source.is_discovered() {
        return Ok(resolution);
    }

    // Some compliant resource servers publish only the protected-resource
    // pointer in a WWW-Authenticate challenge. This request is discovery
    // traffic with no credentials; a configured MCP connection never takes
    // this path as an anonymous fallback.
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "MCP OAuth discovery request could not be created".to_string())?
        .get(server_url)
        .send()
        .await
        .map_err(|_| "MCP OAuth challenge discovery request failed".to_string())?;
    let Some(challenge) = response
        .headers()
        .get("www-authenticate")
        .and_then(|value| value.to_str().ok())
    else {
        return Err(
            "this MCP server did not publish OAuth metadata or an authorization challenge"
                .to_string(),
        );
    };
    let discovered = manager
        .resolve_metadata_from_challenge(Some(challenge))
        .await
        .map_err(auth_error_message)?;
    if !discovered.source.is_discovered() {
        return Err(
            "this MCP server's OAuth challenge did not point to discoverable metadata".to_string(),
        );
    }
    Ok(discovered)
}

fn callback_path(server_id: &str) -> String {
    let mut path = String::from("/oauth/callback/");
    for character in server_id.chars().take(48) {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            path.push(character);
        } else {
            path.push('_');
        }
    }
    if path.ends_with('/') {
        path.push_str("server");
    }
    path
}

async fn read_callback_request(
    socket: &mut TcpStream,
    callback_port: u16,
    deadline_at: Instant,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Result<String, String> {
    let read = async {
        let mut bytes = Vec::with_capacity(1024);
        let mut chunk = [0_u8; 1024];
        loop {
            let read = socket
                .read(&mut chunk)
                .await
                .map_err(|error| format!("could not read callback: {error}"))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            if bytes.len() > 8192 {
                return Err("OAuth callback request was too large".to_string());
            }
        }
        let request =
            String::from_utf8(bytes).map_err(|_| "OAuth callback was not HTTP".to_string())?;
        let first_line = request
            .lines()
            .next()
            .ok_or_else(|| "OAuth callback request was malformed".to_string())?;
        let mut fields = first_line.split_whitespace();
        if fields.next() != Some("GET") {
            return Err("OAuth callback must use GET".to_string());
        }
        let target = fields
            .next()
            .ok_or_else(|| "OAuth callback request was malformed".to_string())?;
        if !target.starts_with('/') {
            return Err("OAuth callback request path was invalid".to_string());
        }
        Url::parse(&format!("http://127.0.0.1:{callback_port}{target}"))
            .map(|url| url.to_string())
            .map_err(|error| format!("OAuth callback URL was invalid: {error}"))
    };
    tokio::select! {
        () = shutdown.cancelled() => Err("MCP OAuth callback listener was closed".to_string()),
        result = tokio::time::timeout_at(deadline_at, read) => result
            .map_err(|_| "MCP OAuth callback request timed out".to_string())?,
    }
}

/// The callback page runs nothing and loads nothing: its one style sheet is
/// inline, so the policy allows inline style and refuses everything else,
/// and an error message that slipped past escaping still could not run.
const CALLBACK_CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The window's dark and light planes, ink and signals, copied from
/// `ui/src/tokens.css`, because the browser tab cannot reach the window's
/// stylesheet and the policy would refuse it if it could.
const CALLBACK_PAGE_STYLE: &str = "\
:root{color-scheme:dark light;--well:oklch(12% 0.006 250);--raised:oklch(20% 0.007 250);\
--line:oklch(100% 0 0 / 0.09);--ink:oklch(95% 0.004 250);--ink-2:oklch(78% 0.006 250);\
--ink-3:oklch(62% 0.008 250);--accent:oklch(78.7% 0.128 163);--danger:oklch(72% 0.17 25)}\
@media (prefers-color-scheme:light){:root{--well:oklch(92.5% 0.005 250);\
--raised:oklch(100% 0 0);--line:oklch(20% 0.01 250 / 0.1);--ink:oklch(20% 0.01 250);\
--ink-2:oklch(42% 0.01 250);--ink-3:oklch(56% 0.01 250);--danger:oklch(55% 0.19 27)}}\
*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;\
background:var(--well);color:var(--ink);\
font:14px/1.5 \"IBM Plex Sans\",-apple-system,\"Segoe UI Variable Text\",system-ui,sans-serif}\
main{width:min(360px,100%);padding:24px;border:1px solid var(--line);border-radius:12px;\
background:var(--raised)}\
p{margin:0}\
.brand{color:var(--ink-3);font-size:12px;letter-spacing:.02em}\
h1{display:flex;align-items:center;gap:8px;margin:8px 0 4px;font-size:16px;font-weight:600}\
h1::before{content:\"\";width:8px;height:8px;border-radius:50%;background:var(--accent)}\
.failed h1::before{background:var(--danger)}\
.detail{color:var(--ink-2);overflow-wrap:anywhere}";

/// The page the browser lands on after the provider sends it back: what
/// happened in a heading, and a line saying what to do or what went wrong.
fn callback_page(title: &str, detail: &str, failed: bool) -> String {
    let class = if failed { " class=\"failed\"" } else { "" };
    let title = html_escape(title);
    let detail = html_escape(detail);
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Hotline · {title}</title><style>{CALLBACK_PAGE_STYLE}</style></head>\
<body><main{class}><p class=\"brand\">Hotline</p><h1>{title}</h1>\
<p class=\"detail\">{detail}</p></main></body></html>"
    )
}

async fn write_callback_response(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: {CALLBACK_CONTENT_SECURITY_POLICY}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(
        CALLBACK_WRITE_TIMEOUT,
        socket.write_all(response.as_bytes()),
    )
    .await;
}

fn validate_callback_url(expected: &str, actual: &str) -> Result<(), String> {
    let expected = Url::parse(expected).map_err(|error| error.to_string())?;
    let actual = Url::parse(actual).map_err(|error| error.to_string())?;
    if actual.scheme() != expected.scheme()
        || actual.host_str() != expected.host_str()
        || actual.port_or_known_default() != expected.port_or_known_default()
        || actual.path() != expected.path()
        || !actual.username().is_empty()
        || actual.password().is_some()
        || actual.fragment().is_some()
    {
        return Err("OAuth callback did not match the pending native sign-in".to_string());
    }
    Ok(())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn oauth_config(auth: &HttpAuth) -> Option<OAuthConfig> {
    match auth {
        HttpAuth::OauthConfigured {
            scopes,
            resource,
            client_id,
            token_endpoint_auth_method: _,
        } => Some(OAuthConfig {
            scopes: scopes.clone(),
            resource: resource.clone(),
            client_id: client_id.clone(),
        }),
        HttpAuth::Oauth => Some(OAuthConfig {
            scopes: Vec::new(),
            resource: None,
            client_id: None,
        }),
        _ => None,
    }
}

#[derive(Clone)]
struct OAuthConfig {
    scopes: Vec<String>,
    resource: Option<String>,
    client_id: Option<String>,
}

fn validate_resolution(
    server_url: &str,
    resolution: &AuthorizationMetadataResolution,
) -> Result<(), String> {
    if !resolution.source.is_discovered() {
        return Err(
            "this MCP server did not publish OAuth metadata; Hotline requires discovery and will not guess /authorize or /token endpoints"
                .to_string(),
        );
    }
    let metadata = &resolution.metadata;
    if !metadata
        .code_challenge_methods_supported
        .as_ref()
        .is_some_and(|methods| methods.iter().any(|method| method == "S256"))
    {
        return Err(
            "the authorization server does not advertise required PKCE S256 support".to_string(),
        );
    }
    if let Some(response_types) = &metadata.response_types_supported
        && !response_types
            .iter()
            .any(|response_type| response_type == "code")
    {
        return Err(
            "the authorization server does not advertise authorization-code support".to_string(),
        );
    }
    let base = Url::parse(server_url).map_err(|error| error.to_string())?;
    validate_endpoint(&base, &metadata.authorization_endpoint, "authorization")?;
    validate_endpoint(&base, &metadata.token_endpoint, "token")?;
    if let Some(endpoint) = &metadata.registration_endpoint {
        validate_endpoint(&base, endpoint, "registration")?;
    }
    if let Some(issuer) = &metadata.issuer {
        validate_endpoint(&base, issuer, "issuer")?;
    }
    Ok(())
}

fn validate_server_url(url: &str, label: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|error| format!("{label} is invalid: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("{label} must be an HTTP(S) URL with a host"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("{label} cannot contain embedded credentials"));
    }
    if parsed.scheme() == "http" && !is_loopback_host(parsed.host_str()) {
        return Err(format!("{label} must use HTTPS outside a loopback host"));
    }
    Ok(parsed)
}

fn validate_endpoint(base: &Url, value: &str, label: &str) -> Result<(), String> {
    let endpoint = validate_server_url(value, &format!("{label} endpoint"))?;
    if endpoint.scheme() == "http"
        && !(is_loopback_host(base.host_str()) && is_loopback_host(endpoint.host_str()))
    {
        return Err(format!(
            "{label} endpoint uses HTTP outside loopback; OAuth endpoints must use HTTPS"
        ));
    }
    Ok(())
}

fn is_loopback_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_redirect_uri(value: &str) -> Result<(), String> {
    let redirect = validate_server_url(value, "OAuth redirect URI")?;
    if redirect.scheme() == "http" && !is_loopback_host(redirect.host_str()) {
        return Err("OAuth native redirect URI must use HTTPS or a loopback host".to_string());
    }
    if !redirect.username().is_empty()
        || redirect.password().is_some()
        || redirect.query().is_some()
        || redirect.fragment().is_some()
    {
        return Err(
            "OAuth native redirect URI cannot contain credentials, query or fragment".to_string(),
        );
    }
    Ok(())
}

fn validate_resource(value: &str) -> Result<(), String> {
    let resource = validate_server_url(value, "OAuth resource")?;
    if resource.username() != ""
        || resource.password().is_some()
        || resource.query().is_some()
        || resource.fragment().is_some()
    {
        return Err(
            "OAuth resource must be a URL without credentials, query or fragment".to_string(),
        );
    }
    Ok(())
}

/// An OAuth resource indicator may name a parent resource for a more specific
/// MCP endpoint, but it cannot move credentials to another origin or sibling
/// path. This mirrors rmcp's protected-resource binding while also checking
/// the value that Hotline persists beside the registration.
fn validate_resource_binding(server_url: &str, resource: &str) -> Result<(), String> {
    let server = Url::parse(server_url).map_err(|error| error.to_string())?;
    let resource_url = Url::parse(resource).map_err(|error| error.to_string())?;
    if server.scheme() != resource_url.scheme()
        || server
            .host_str()
            .zip(resource_url.host_str())
            .is_none_or(|(server, resource)| !server.eq_ignore_ascii_case(resource))
        || server.port_or_known_default() != resource_url.port_or_known_default()
    {
        return Err(
            "OAuth resource must use the MCP server's origin; sign in again with matching metadata"
                .to_string(),
        );
    }
    let server_path = server.path();
    let resource_path = resource_url.path();
    if server_path != resource_path
        && !(server_path.starts_with(resource_path)
            && (resource_path.ends_with('/')
                || server_path.as_bytes().get(resource_path.len()) == Some(&b'/')))
    {
        return Err(
            "OAuth resource must identify the configured MCP server; sign in again with matching metadata"
                .to_string(),
        );
    }
    Ok(())
}

fn loopback_redirect() -> String {
    "http://127.0.0.1/callback".to_string()
}

/// Keep status polling local for healthy credentials. An expired token is
/// checked through rmcp so a rejected refresh is visible as a reconnectable
/// failure instead of a misleading "signed in" badge.
fn token_needs_refresh(credentials: &rmcp::transport::auth::StoredCredentials) -> bool {
    let Some(received_at) = credentials.token_received_at else {
        return false;
    };
    let Some(expires_in) = credentials
        .token_response
        .as_ref()
        .and_then(|response| serde_json::to_value(response).ok())
        .and_then(|response| response.get("expires_in").and_then(Value::as_u64))
    else {
        return false;
    };
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    expires_in.saturating_sub(now.saturating_sub(received_at)) < 30
}

fn auth_error_message(error: AuthError) -> String {
    match error {
        AuthError::AuthorizationRequired => {
            "MCP OAuth sign-in is required in Settings → Tools.".to_string()
        }
        AuthError::TokenRefreshRejected(_) => {
            "MCP OAuth refresh was rejected; sign in again in Settings → Tools.".to_string()
        }
        AuthError::TokenRefreshFailed(_) => {
            "MCP OAuth refresh failed; reconnect in Settings → Tools.".to_string()
        }
        AuthError::CredentialStoreError(_) => {
            "MCP OAuth credentials could not be read or saved.".to_string()
        }
        AuthError::PkceUnsupported => {
            "the authorization server does not advertise required PKCE S256 support".to_string()
        }
        AuthError::TokenExchangeFailed(_) => {
            "MCP OAuth token exchange failed; try signing in again.".to_string()
        }
        AuthError::RegistrationFailed(_) => {
            "MCP OAuth client registration failed; check that this server supports DCR.".to_string()
        }
        AuthError::MetadataError(_) => "MCP OAuth metadata discovery failed.".to_string(),
        AuthError::HttpError(_) | AuthError::OAuthError(_) => {
            "MCP OAuth authorization request failed.".to_string()
        }
        AuthError::InternalError(message) if message.to_ascii_lowercase().contains("state") => {
            "MCP OAuth callback state did not match this sign-in.".to_string()
        }
        AuthError::AuthorizationFailed(_)
        | AuthError::UrlError(_)
        | AuthError::NoAuthorizationSupport
        | AuthError::InvalidTokenType(_)
        | AuthError::TokenExpired
        | AuthError::InvalidScope(_)
        | AuthError::InsufficientScope { .. }
        | AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. }
        | AuthError::ClientCredentialsError(_)
        | AuthError::InternalError(_) => "MCP OAuth operation failed.".to_string(),
        _ => "MCP OAuth operation failed.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_page_escapes_what_it_is_told() {
        let page = callback_page(
            "Sign-in failed",
            "<script>alert(1)</script> & \"more\"",
            true,
        );
        assert!(!page.contains("<script"), "{page}");
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;more&quot;"));
        assert!(page.contains("<main class=\"failed\">"), "{page}");
    }

    #[test]
    fn callback_validation_rejects_a_different_origin_or_userinfo() {
        let expected = "http://127.0.0.1:4567/oauth/callback";
        assert!(
            validate_callback_url(
                expected,
                "http://127.0.0.1:4567/oauth/callback?code=x&state=y"
            )
            .is_ok()
        );
        assert!(
            validate_callback_url(
                expected,
                "http://127.0.0.1:4568/oauth/callback?code=x&state=y"
            )
            .is_err()
        );
        assert!(
            validate_callback_url(
                expected,
                "http://user:pass@127.0.0.1:4567/oauth/callback?code=x&state=y"
            )
            .is_err()
        );
    }

    #[test]
    fn callback_validation_requires_the_exact_path_scheme_host_and_no_fragment() {
        let expected = "http://127.0.0.1:4567/oauth/callback/linear";
        let good = "http://127.0.0.1:4567/oauth/callback/linear?code=x&state=y";
        assert!(validate_callback_url(expected, good).is_ok());
        for wrong in [
            "http://127.0.0.1:4567/callback?code=x&state=y",
            "http://127.0.0.1:4567/oauth/callback/other?code=x&state=y",
            "http://127.0.0.1:4567/oauth/callback/linear/?code=x&state=y",
            "http://127.0.0.1:4567/oauth/callback/linear/extra?code=x&state=y",
            "http://localhost:4567/oauth/callback/linear?code=x&state=y",
            "https://127.0.0.1:4567/oauth/callback/linear?code=x&state=y",
            "http://127.0.0.1:4567/oauth/callback/linear?code=x&state=y#fragment",
            "http://[::1]:4567/oauth/callback/linear?code=x&state=y",
        ] {
            assert!(validate_callback_url(expected, wrong).is_err(), "{wrong}");
        }
        assert!(validate_callback_url(expected, "not a url").is_err());
    }

    #[test]
    fn remote_http_oauth_urls_are_refused_but_loopback_http_is_allowed() {
        assert!(validate_server_url("http://mcp.example.test/mcp", "MCP server URL").is_err());
        assert!(validate_server_url("http://127.0.0.1:4321/mcp", "MCP server URL").is_ok());
    }
}
