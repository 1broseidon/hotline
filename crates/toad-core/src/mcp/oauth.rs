//! OAuth integration for HTTP MCP servers.
//!
//! rmcp owns the OAuth protocol state machine: protected-resource and
//! authorization-server discovery, DCR, PKCE, token exchange and refresh.
//! This module supplies Toad's policy around that state machine. In particular,
//! it refuses rmcp's legacy endpoint fallback, requires advertised S256 PKCE,
//! binds registrations and credentials to the configured URL and issuer, and
//! never lets a missing token turn a configured connection into an anonymous
//! one.

use super::{HttpAuth, McpServer, McpTransport};
use crate::vault::{McpOAuthRegistration, Vault};
use axum::http;
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
use serde_json::{Value, json};
use std::collections::HashMap;
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

/// Connect one already signed-in OAuth server through rmcp's auth-aware
/// Streamable HTTP transport.
pub(crate) async fn connect_oauth(
    server: &McpServer,
    vault: Arc<Vault>,
) -> Result<OAuthRunningService, String> {
    let token_lock = vault.mcp_token_lock(&server.id);
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
    super::handshake(super::toad_client().serve(transport)).await
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
pub(crate) async fn manager_for_server(
    server: &McpServer,
    vault: Arc<Vault>,
    redirect_uri: Option<&str>,
) -> Result<AuthorizationManager, String> {
    let McpTransport::Http { url, auth } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    validate_server_url(url, "MCP server URL")?;

    let store = vault.mcp_credential_store(&server.id, url);
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
    let saved = vault
        .mcp_oauth_registration(&server.id, url)
        .map_err(|error| error.to_string())?;
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
                redirect_uri: redirect_uri
                    .map(str::to_string)
                    .unwrap_or_else(loopback_redirect),
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
/// handled.
pub(crate) async fn authorization_session(
    server: &McpServer,
    vault: Arc<Vault>,
    redirect_uri: &str,
) -> Result<AuthorizationSession, String> {
    let mut manager = manager_for_server(server, vault.clone(), Some(redirect_uri)).await?;
    let McpTransport::Http { url, auth } = &server.transport else {
        return Err("OAuth authentication is only available for HTTP MCP servers".to_string());
    };
    let metadata = discover_metadata(server).await?;
    let registration = vault
        .mcp_oauth_registration(&server.id, url)
        .map_err(|error| error.to_string())?;
    let configured = oauth_config(auth);
    let mut request = AuthorizationRequest::new(redirect_uri)
        .with_client_name("Toad MCP Gateway")
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
        vault
            .save_mcp_oauth_registration(&server.id, url, configured_registration)
            .map_err(|error| error.to_string())?;
        request = request.with_preregistered_client(client_id);
    } else {
        // Register here rather than hiding the response inside
        // AuthorizationSession::new: rmcp intentionally exposes only the
        // resulting token, while Toad must retain a returned client secret in
        // its protected boundary for future refreshes.
        let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        let config = manager
            .register_client("Toad MCP Gateway", redirect_uri, &scope_refs)
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
        vault
            .save_mcp_oauth_registration(&server.id, url, registration)
            .map_err(|error| error.to_string())?;
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
/// memory; only the resulting registration and token are persisted.
pub(crate) struct McpOAuthService {
    vault: Arc<Vault>,
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
    pub(crate) fn new(vault: Arc<Vault>) -> Self {
        Self {
            vault,
            pending: Arc::new(StdMutex::new(HashMap::new())),
            start_locks: Arc::new(StdMutex::new(HashMap::new())),
            on_complete: Arc::new(StdMutex::new(None)),
        }
    }

    pub(crate) fn set_on_complete(&self, callback: OAuthCompleteCallback) {
        *self
            .on_complete
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(callback);
    }

    pub(crate) async fn start(&self, server: &McpServer) -> Result<Value, String> {
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

        let saved = self
            .vault
            .mcp_oauth_registration(&server.id, url)
            .map_err(|error| error.to_string())?;
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
        let session = authorization_session(server, self.vault.clone(), &redirect_uri).await?;
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

    pub(crate) async fn complete(
        &self,
        login_id: &str,
        callback_url: &str,
    ) -> Result<Value, String> {
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

    pub(crate) async fn status(&self, server: &McpServer) -> Result<Value, String> {
        let McpTransport::Http { url, auth } = &server.transport else {
            return Err("MCP OAuth status is only available for HTTP servers".to_string());
        };
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
        let store = self.vault.mcp_credential_store(&server.id, url);
        let Some(credentials) = store.load().await.map_err(auth_error_message)? else {
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

        match manager_for_server(server, self.vault.clone(), None).await {
            Ok(manager) => {
                let token_lock = self.vault.mcp_token_lock(&server.id);
                let result = {
                    let _token_guard = token_lock.lock().await;
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

    pub(crate) async fn sign_out(&self, server_id: &str) -> Result<(), String> {
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
        let token_lock = self.vault.mcp_token_lock(server_id);
        let _token_guard = token_lock.lock().await;
        self.vault
            .clear_mcp_oauth(server_id)
            .map_err(|error| format!("could not sign out of MCP OAuth: {error}"))
    }

    fn clone_for_task(&self) -> Self {
        Self {
            vault: self.vault.clone(),
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
                _ = tokio::time::sleep_until(deadline_at) => {
                    let mut state = login.state.lock().await;
                    if *state == OAuthLoginState::Pending {
                        *state = OAuthLoginState::Failed("MCP OAuth sign-in timed out.".to_string());
                    }
                    break;
                }
                _ = login.shutdown.cancelled() => break,
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
                                let body = "<html><body>Toad sign-in complete. You may close this tab.</body></html>";
                                write_callback_response(&mut socket, "200 OK", body).await;
                                break;
                            }
                            Err(error) => {
                                let body = format!("<html><body>Toad could not finish sign-in: {}</body></html>", html_escape(&error));
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
                            let body = format!("<html><body>Toad could not read the sign-in callback: {}</body></html>", html_escape(&error));
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
        let token_lock = self.vault.mcp_token_lock(&login.server_id);
        let _token_guard = token_lock.lock().await;
        match session.handle_callback_url(callback_url).await {
            Ok(_) => {
                *state = OAuthLoginState::Complete;
                login.shutdown.cancel();
                if let Some(callback) = self
                    .on_complete
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
                {
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
        _ = shutdown.cancelled() => Err("MCP OAuth callback listener was closed".to_string()),
        result = tokio::time::timeout_at(deadline_at, read) => result
            .map_err(|_| "MCP OAuth callback request timed out".to_string())?,
    }
}

async fn write_callback_response(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
            "this MCP server did not publish OAuth metadata; Toad requires discovery and will not guess /authorize or /token endpoints"
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
            .map(|address| address.is_loopback())
            .unwrap_or(false)
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
/// the value that Toad persists beside the registration.
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
    use crate::log::{Log, StreamId};
    use crate::vault::Vault;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, WWW_AUTHENTICATE};
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use axum::routing::any;
    use axum::{Router, body::to_bytes};
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct MockRequest {
        path: String,
        method: String,
        authorization: Option<String>,
        body: String,
    }

    struct MockOAuthState {
        base: String,
        supports_registration: bool,
        current_access: Mutex<String>,
        registrations: AtomicUsize,
        code_exchanges: AtomicUsize,
        refreshes: AtomicUsize,
        requests: Mutex<Vec<MockRequest>>,
    }

    struct MockOAuthServer {
        base: String,
        state: Arc<MockOAuthState>,
        shutdown: CancellationToken,
        task: JoinHandle<()>,
    }

    impl MockOAuthServer {
        async fn start() -> Self {
            Self::start_with_registration(true).await
        }

        async fn start_without_registration() -> Self {
            Self::start_with_registration(false).await
        }

        async fn start_with_registration(supports_registration: bool) -> Self {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("mock OAuth listener");
            let address = listener.local_addr().expect("mock OAuth address");
            let base = format!("http://{address}");
            let state = Arc::new(MockOAuthState {
                base: base.clone(),
                supports_registration,
                current_access: Mutex::new("access-1".to_string()),
                registrations: AtomicUsize::new(0),
                code_exchanges: AtomicUsize::new(0),
                refreshes: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
            });
            let router = Router::new()
                .fallback(any(mock_request))
                .with_state(state.clone());
            let shutdown = CancellationToken::new();
            let wait = shutdown.clone();
            let task = tokio::spawn(async move {
                let signal = async move { wait.cancelled().await };
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(signal)
                    .await;
            });
            Self {
                base,
                state,
                shutdown,
                task,
            }
        }

        fn server_with_auth(&self, auth: HttpAuth) -> McpServer {
            McpServer {
                id: "mock-oauth".to_string(),
                name: "Mock OAuth MCP".to_string(),
                transport: McpTransport::Http {
                    url: format!("{}/mcp", self.base),
                    auth,
                },
                refuse: None,
            }
        }

        fn server(&self) -> McpServer {
            self.server_with_auth(HttpAuth::Oauth)
        }
    }

    impl Drop for MockOAuthServer {
        fn drop(&mut self) {
            self.shutdown.cancel();
            self.task.abort();
        }
    }

    async fn mock_request(
        State(state): State<Arc<MockOAuthState>>,
        request: Request<Body>,
    ) -> Response {
        let path = request.uri().path().to_string();
        let method = request.method().to_string();
        let authorization = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = to_bytes(request.into_body(), 16 * 1024 * 1024)
            .await
            .expect("mock request body");
        state
            .requests
            .lock()
            .expect("mock request log")
            .push(MockRequest {
                path: path.clone(),
                method: method.clone(),
                authorization: authorization.clone(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });

        match (method.as_str(), path.as_str()) {
            ("GET", "/.well-known/oauth-protected-resource") => json_response(
                StatusCode::OK,
                json!({
                    "resource": format!("{}/mcp", state.base),
                    "authorization_servers": [format!("{}/as", state.base)],
                    "scopes_supported": ["mcp"],
                }),
            ),
            ("GET", "/.well-known/oauth-authorization-server/as") => {
                let mut metadata = json!({
                    "issuer": format!("{}/as", state.base),
                    "authorization_endpoint": format!("{}/as/authorize", state.base),
                    "token_endpoint": format!("{}/as/token", state.base),
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                    "scopes_supported": ["mcp"],
                    "token_endpoint_auth_methods_supported": ["client_secret_post"],
                });
                if state.supports_registration {
                    metadata["registration_endpoint"] =
                        json!(format!("{}/as/register", state.base));
                }
                json_response(StatusCode::OK, metadata)
            }
            ("POST", "/as/register") => {
                state.registrations.fetch_add(1, Ordering::SeqCst);
                let registration: Value =
                    serde_json::from_slice(&body).expect("mock registration request");
                json_response(
                    StatusCode::CREATED,
                    json!({
                        "client_id": "mock-client",
                        "client_secret": "mock-client-secret",
                        "redirect_uris": registration["redirect_uris"].clone(),
                    }),
                )
            }
            ("POST", "/as/token") => {
                let form: HashMap<String, String> =
                    url::form_urlencoded::parse(&body).into_owned().collect();
                match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code")
                        if form.get("code").map(String::as_str) == Some("good-code") =>
                    {
                        state.code_exchanges.fetch_add(1, Ordering::SeqCst);
                        json_response(
                            StatusCode::OK,
                            json!({
                                "access_token": "access-1",
                                "token_type": "Bearer",
                                "expires_in": 0,
                                "refresh_token": "refresh-1",
                                "scope": "mcp",
                            }),
                        )
                    }
                    Some("refresh_token") => {
                        let refresh = form.get("refresh_token").map(String::as_str);
                        if refresh.is_some_and(|value| value.starts_with("refresh-")) {
                            let number = state.refreshes.fetch_add(1, Ordering::SeqCst) + 2;
                            let access = format!("access-{number}");
                            let refresh = format!("refresh-{number}");
                            *state.current_access.lock().expect("mock token") = access.clone();
                            json_response(
                                StatusCode::OK,
                                json!({
                                    "access_token": access,
                                    "token_type": "Bearer",
                                    "expires_in": 3600,
                                    "refresh_token": refresh,
                                    "scope": "mcp",
                                }),
                            )
                        } else {
                            json_response(
                                StatusCode::BAD_REQUEST,
                                json!({ "error": "invalid_grant" }),
                            )
                        }
                    }
                    _ => json_response(
                        StatusCode::BAD_REQUEST,
                        json!({ "error": "invalid_request" }),
                    ),
                }
            }
            ("GET", "/mcp") => {
                if valid_access(&state, authorization.as_deref()) {
                    response(StatusCode::OK, "application/json", Vec::new())
                } else {
                    mcp_challenge(&state)
                }
            }
            ("DELETE", "/mcp") => {
                if valid_access(&state, authorization.as_deref()) {
                    response(StatusCode::ACCEPTED, "application/json", Vec::new())
                } else {
                    mcp_challenge(&state)
                }
            }
            ("POST", "/mcp") => {
                if !valid_access(&state, authorization.as_deref()) {
                    return mcp_challenge(&state);
                }
                let message: Value = serde_json::from_slice(&body).expect("MCP JSON request");
                let id = message.get("id").cloned().unwrap_or(Value::Null);
                match message.get("method").and_then(Value::as_str) {
                    Some("initialize") => json_response(
                        StatusCode::OK,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": { "tools": {} },
                                "serverInfo": { "name": "mock", "version": "1" },
                            },
                        }),
                    ),
                    Some("tools/list") => json_response(
                        StatusCode::OK,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Returns a deterministic response.",
                                    "inputSchema": { "type": "object" },
                                }],
                            },
                        }),
                    ),
                    Some("tools/call") => json_response(
                        StatusCode::OK,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": "mock-pong" }],
                                "isError": false,
                            },
                        }),
                    ),
                    Some(method) if method.starts_with("notifications/") => {
                        response(StatusCode::ACCEPTED, "application/json", Vec::new())
                    }
                    Some(method) => json_response(
                        StatusCode::NOT_FOUND,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": method },
                        }),
                    ),
                    None => json_response(
                        StatusCode::BAD_REQUEST,
                        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32600 } }),
                    ),
                }
            }
            _ => response(StatusCode::NOT_FOUND, "text/plain", Vec::new()),
        }
    }

    fn valid_access(state: &MockOAuthState, authorization: Option<&str>) -> bool {
        let expected = format!(
            "Bearer {}",
            state.current_access.lock().expect("mock token").as_str()
        );
        authorization == Some(expected.as_str())
    }

    fn mcp_challenge(state: &MockOAuthState) -> Response {
        let metadata = format!("{}/.well-known/oauth-protected-resource", state.base);
        let value = format!(r##"Bearer resource_metadata="{metadata}", scope="mcp""##);
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(WWW_AUTHENTICATE, value)
            .header(CONTENT_LENGTH, "0")
            .body(Body::empty())
            .expect("mock challenge")
    }

    fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .expect("mock response")
    }

    fn json_response(status: StatusCode, value: Value) -> Response {
        response(
            status,
            "application/json",
            serde_json::to_vec(&value).expect("mock JSON"),
        )
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "toad-oauth-{name}-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("oauth scratch");
        path
    }

    fn callback_value(started: &Value, code: &str, state: &str) -> String {
        let mut callback =
            Url::parse(started["redirectUri"].as_str().expect("redirect")).expect("redirect URL");
        callback
            .query_pairs_mut()
            .append_pair("code", code)
            .append_pair("state", state);
        callback.to_string()
    }

    fn state_from(started: &Value) -> String {
        Url::parse(
            started["authorizationUrl"]
                .as_str()
                .expect("authorization URL"),
        )
        .expect("authorization URL")
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("OAuth state")
    }

    async fn send_native_callback(started: &Value, code: &str, state: &str) -> String {
        let callback = callback_value(started, code, state);
        let callback = Url::parse(&callback).expect("callback URL");
        let port = callback.port().expect("callback port");
        let target = format!(
            "{}?{}",
            callback.path(),
            callback.query().expect("callback query")
        );
        let mut socket = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("native callback listener");
        socket
            .write_all(
                format!(
                    "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("native callback request");
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
            .await
            .expect("native callback response timeout")
            .expect("native callback response");
        String::from_utf8(response).expect("native callback response text")
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
    fn remote_http_oauth_urls_are_refused_but_loopback_http_is_allowed() {
        assert!(validate_server_url("http://mcp.example.test/mcp", "MCP server URL").is_err());
        assert!(validate_server_url("http://127.0.0.1:4321/mcp", "MCP server URL").is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oauth_discovery_dcr_native_callback_refresh_restart_and_mcp_call() {
        let mock = MockOAuthServer::start().await;
        let root = scratch("flow");
        let log = Log::open(&root);
        let vault = Arc::new(Vault::open(&root, log.clone()).expect("vault"));
        let server = mock.server();
        let service = McpOAuthService::new(vault.clone());

        let started = service.start(&server).await.expect("start OAuth");
        assert_eq!(started["status"], "pending");
        assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 1);
        let state = state_from(&started);
        let response = send_native_callback(&started, "good-code", &state).await;
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert_eq!(
            service.status(&server).await.expect("completed status")["status"],
            "signed_in"
        );
        assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 1);
        let registration = vault
            .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
            .expect("registration read")
            .expect("saved registration");
        assert_eq!(registration.client_id, "mock-client");
        let isolation = vault
            .mcp_oauth_registration(&server.id, &format!("{}/other", mock.base))
            .expect_err("a registration must be bound to its MCP URL");
        assert!(isolation.to_string().contains("different server URL"));

        let (client, tools) = connect_oauth(&server, vault.clone())
            .await
            .expect("authenticated MCP connection");
        assert_eq!(tools.len(), 1);
        let called = client
            .peer()
            .call_tool(CallToolRequestParams::new("echo"))
            .await
            .expect("authenticated tools/call");
        assert_eq!(called.content[0].as_text().expect("text").text, "mock-pong");
        client.cancel().await.ok();
        assert_eq!(mock.state.refreshes.load(Ordering::SeqCst), 1);

        // A second manager after an application restart reuses the saved
        // registration and credential record instead of registering again.
        let restarted = McpOAuthService::new(vault.clone());
        let status = restarted
            .status(&server)
            .await
            .expect("status after restart");
        assert_eq!(status["status"], "signed_in");
        let (_, restarted_tools) = connect_oauth(&server, vault.clone())
            .await
            .expect("reconnected MCP");
        assert_eq!(restarted_tools.len(), 1);
        assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 1);
        {
            let requests = mock.state.requests.lock().expect("mock request log");
            let mcp_posts: Vec<&MockRequest> = requests
                .iter()
                .filter(|request| request.path == "/mcp" && request.method == "POST")
                .collect();
            assert!(!mcp_posts.is_empty());
            assert!(
                mcp_posts
                    .iter()
                    .all(|request| request.authorization.as_deref() == Some("Bearer access-2"))
            );
            assert!(
                mcp_posts
                    .iter()
                    .all(|request| !request.body.contains("refresh-"))
            );
        }

        // Independent sessions can decide at the same time that an access
        // token is expired. The vault lock covers the decision as well as the
        // SDK refresh, so the second manager observes the rotated token
        // instead of spending the same refresh token again.
        let store = vault.mcp_credential_store(&server.id, &format!("{}/mcp", mock.base));
        let mut expired = store
            .load()
            .await
            .expect("stored credentials")
            .expect("credentials after reconnect");
        let mut token = serde_json::to_value(
            expired
                .token_response
                .as_ref()
                .expect("access and refresh token"),
        )
        .expect("token JSON");
        token["expires_in"] = json!(0);
        token["refresh_token"] = json!("refresh-2");
        expired.token_response = Some(serde_json::from_value(token).expect("expired token"));
        expired.token_received_at = Some(0);
        store.save(expired).await.expect("expire credentials");
        let first_manager = manager_for_server(&server, vault.clone(), None)
            .await
            .expect("first manager");
        let second_manager = manager_for_server(&server, vault.clone(), None)
            .await
            .expect("second manager");
        let first_token_lock = vault.mcp_token_lock(&server.id);
        let second_token_lock = vault.mcp_token_lock(&server.id);
        let (first, second) = tokio::join!(
            async {
                let _token_guard = first_token_lock.lock().await;
                first_manager.get_access_token().await
            },
            async {
                let _token_guard = second_token_lock.lock().await;
                second_manager.get_access_token().await
            }
        );
        assert_eq!(first.expect("first concurrent refresh"), "access-3");
        assert_eq!(second.expect("second concurrent refresh"), "access-3");
        assert_eq!(mock.state.refreshes.load(Ordering::SeqCst), 2);

        // A definitive refresh-token rejection is actionable and secret-free
        // in the status surface; it does not silently fall back to anonymous
        // MCP requests.
        let mut rejected = store
            .load()
            .await
            .expect("stored credentials")
            .expect("rotated credentials");
        let mut token = serde_json::to_value(
            rejected
                .token_response
                .as_ref()
                .expect("access and refresh token"),
        )
        .expect("token JSON");
        token["expires_in"] = json!(0);
        token["refresh_token"] = json!("invalid-refresh");
        rejected.token_response = Some(serde_json::from_value(token).expect("rejected token"));
        rejected.token_received_at = Some(0);
        store.save(rejected).await.expect("reject credentials");
        let status = restarted.status(&server).await.expect("refresh status");
        assert_eq!(status["status"], "failed");
        assert!(
            !status["error"]
                .as_str()
                .unwrap_or_default()
                .contains("invalid-refresh")
        );
        let error = connect_oauth(&server, vault.clone())
            .await
            .expect_err("rejected refresh must stop connection");
        assert!(error.contains("sign-in"), "{error}");

        // The room log never receives bearer, refresh or client-secret material.
        let room_text = serde_json::to_string(&log.load(&StreamId::Room)).expect("room JSON");
        assert!(!room_text.contains("access-"));
        assert!(!room_text.contains("refresh-"));
        assert!(!room_text.contains("mock-client-secret"));

        restarted.sign_out(&server.id).await.expect("sign out");
        assert_eq!(
            restarted.status(&server).await.expect("signed out status")["status"],
            "signed_out"
        );
        let store = vault.mcp_credential_store(&server.id, &format!("{}/mcp", mock.base));
        assert!(store.load().await.expect("empty OAuth store").is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_dcr_is_reported_without_synthesizing_registration() {
        let mock = MockOAuthServer::start_without_registration().await;
        let root = scratch("missing-dcr");
        let log = Log::open(&root);
        let vault = Arc::new(Vault::open(&root, log).expect("vault"));
        let server = mock.server();
        let service = McpOAuthService::new(vault.clone());

        let error = service
            .start(&server)
            .await
            .expect_err("a server without DCR cannot start a new native client");
        assert!(
            error.to_ascii_lowercase().contains("dcr")
                || error.to_ascii_lowercase().contains("registration")
        );
        assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 0);
        assert!(
            vault
                .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
                .expect("registration read")
                .is_none()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn configured_client_reuses_without_dcr() {
        let mock = MockOAuthServer::start_without_registration().await;
        let root = scratch("configured-client");
        let log = Log::open(&root);
        let vault = Arc::new(Vault::open(&root, log).expect("vault"));
        let server = mock.server_with_auth(HttpAuth::OauthConfigured {
            scopes: vec!["mcp".to_string()],
            resource: Some(format!("{}/mcp", mock.base)),
            client_id: Some("configured-client".to_string()),
            token_endpoint_auth_method: Some("none".to_string()),
        });
        let service = McpOAuthService::new(vault.clone());

        let started = service
            .start(&server)
            .await
            .expect("start configured OAuth");
        let callback = callback_value(&started, "good-code", &state_from(&started));
        service
            .complete(started["loginId"].as_str().expect("login id"), &callback)
            .await
            .expect("complete configured OAuth");
        assert_eq!(mock.state.registrations.load(Ordering::SeqCst), 0);
        assert_eq!(mock.state.code_exchanges.load(Ordering::SeqCst), 1);
        let registration = vault
            .mcp_oauth_registration(&server.id, &format!("{}/mcp", mock.base))
            .expect("registration read")
            .expect("configured client registration");
        assert_eq!(registration.client_id, "configured-client");
        assert_eq!(registration.client_secret, None);

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wrong_state_and_denied_consent_do_not_create_credentials() {
        let mock = MockOAuthServer::start().await;
        let root = scratch("denied");
        let log = Log::open(&root);
        let vault = Arc::new(Vault::open(&root, log).expect("vault"));
        let server = mock.server();
        let service = McpOAuthService::new(vault.clone());
        let started = service.start(&server).await.expect("start OAuth");
        let login_id = started["loginId"].as_str().expect("login id");
        let wrong = callback_value(&started, "good-code", "wrong-state");
        let error = service
            .complete(login_id, &wrong)
            .await
            .expect_err("wrong state");
        assert!(error.contains("state did not match"));
        assert_eq!(
            service.status(&server).await.expect("pending status")["status"],
            "pending"
        );

        let state = state_from(&started);
        let mut denied = Url::parse(started["redirectUri"].as_str().expect("redirect")).unwrap();
        denied
            .query_pairs_mut()
            .append_pair("error", "access_denied")
            .append_pair("state", &state);
        let error = service
            .complete(login_id, denied.as_str())
            .await
            .expect_err("denied consent");
        assert!(error.contains("consent was denied"));
        assert_eq!(
            service.status(&server).await.expect("denied status")["status"],
            "failed"
        );
        assert!(
            vault
                .mcp_credential_store(&server.id, &format!("{}/mcp", mock.base))
                .load()
                .await
                .expect("empty OAuth store")
                .is_none()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
