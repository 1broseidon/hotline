//! The capability-checked loopback proxy through which a child agent reaches a
//! granted OAuth MCP server, so the token stays in the vault and out of the
//! child. It has a file of its own so it can be reviewed on its own.

use super::*;

pub(super) struct OAuthProxy {
    pub(super) url: String,
    pub(super) token: String,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for OAuthProxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

pub(super) struct OAuthProxyState {
    pub(super) token: String,
    pub(super) upstream: reqwest::Url,
    pub(super) upstream_path: String,
    pub(super) client: reqwest::Client,
    pub(super) credential: ProxyCredential,
    pub(super) capability: Option<CapabilityLease>,
}

/// What the proxy puts on each upstream request. OAuth asks the manager for
/// a current token every time, so expiry and refresh stay in the gateway; a
/// pasted token is fixed for the session and read from the vault once.
pub(super) enum ProxyCredential {
    Oauth {
        manager: Box<tokio::sync::Mutex<rmcp::transport::auth::AuthorizationManager>>,
        token_lock: Arc<tokio::sync::Mutex<()>>,
    },
    Fixed(mcp::Presented),
}

/// A server whose credential lives in the vault is reached through the
/// proxy, so the child never holds it: OAuth tokens and pasted tokens alike.
pub(super) fn is_oauth_server(server: &McpServer) -> bool {
    matches!(
        &server.transport,
        McpTransport::Http {
            auth: HttpAuth::Oauth | HttpAuth::OauthConfigured { .. } | HttpAuth::Secret { .. },
            ..
        }
    )
}

pub(super) async fn start_oauth_proxy(
    server: &McpServer,
    vault: Arc<Vault>,
    capability: Option<CapabilityLease>,
) -> Result<OAuthProxy, String> {
    let McpTransport::Http { url, auth } = &server.transport else {
        return Err("MCP OAuth proxy requires an HTTP server".to_string());
    };
    let upstream = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
    let credential = if let HttpAuth::Secret { header } = auth {
        let token = vault
            .mcp_secret(&server.id, url)
            .map_err(|error| format!("could not read the saved MCP token: {error}"))?
            .ok_or_else(|| mcp::NO_SAVED_TOKEN.to_string())?;
        ProxyCredential::Fixed(mcp::Presented::new(header.as_deref(), &token)?)
    } else {
        let token_lock = vault.mcp_token_lock(&server.id);
        let manager = mcp::manager_for_server(server, vault, None).await?;
        {
            let _token_guard = token_lock.lock().await;
            manager
                .get_access_token()
                .await
                .map_err(|_| "MCP OAuth sign-in is required in Settings → Tools.".to_string())?;
        }
        ProxyCredential::Oauth {
            manager: Box::new(tokio::sync::Mutex::new(manager)),
            token_lock,
        }
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("could not open MCP OAuth proxy: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("could not read MCP OAuth proxy address: {error}"))?
        .port();
    let proxy_url = format!(
        "http://127.0.0.1:{port}{}{}",
        upstream.path(),
        upstream
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("could not build MCP OAuth proxy client: {error}"))?;
    // Loopback is shared with every network-enabled teammate. This separate
    // session capability authorizes the child without exposing OAuth secrets.
    let token = uuid::Uuid::new_v4().to_string();
    let state = Arc::new(OAuthProxyState {
        token: token.clone(),
        upstream_path: upstream.path().to_string(),
        upstream,
        client,
        credential,
        capability,
    });
    let shutdown = CancellationToken::new();
    let child_shutdown = shutdown.clone();
    let router = Router::new()
        .fallback(any(oauth_proxy_request))
        .with_state(state);
    let task = tokio::spawn(async move {
        let shutdown_signal = async move {
            child_shutdown.cancelled().await;
        };
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal)
            .await;
    });
    Ok(OAuthProxy {
        url: proxy_url,
        token,
        shutdown,
        task,
    })
}

pub(super) async fn oauth_proxy_request(
    State(state): State<Arc<OAuthProxyState>>,
    request: Request<Body>,
) -> Response {
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if !crate::wire::same_secret(presented, &state.token) {
        return proxy_error(
            StatusCode::UNAUTHORIZED,
            "MCP proxy authentication required",
        );
    }
    if let Some(capability) = &state.capability
        && !capability.is_current()
    {
        return proxy_error(StatusCode::FORBIDDEN, "MCP access was revoked");
    }
    if request.uri().path() != state.upstream_path {
        return proxy_error(StatusCode::NOT_FOUND, "MCP endpoint not found");
    }
    let method = request_method(&request);
    let headers = request_headers(&request);
    let query = request.uri().query().map(str::to_string);
    let body = match axum::body::to_bytes(request.into_body(), 16 * 1024 * 1024).await {
        Ok(body) => body,
        Err(_) => return proxy_error(StatusCode::BAD_REQUEST, "MCP request body was too large"),
    };
    let (presented, oauth_token) = match &state.credential {
        ProxyCredential::Fixed(presented) => (presented.clone(), None),
        ProxyCredential::Oauth {
            manager,
            token_lock,
        } => {
            let _token_guard = token_lock.lock().await;
            let manager = manager.lock().await;
            match manager.get_access_token().await {
                Ok(token) => match mcp::Presented::new(None, &token) {
                    Ok(presented) => (presented, Some(token)),
                    Err(_) => {
                        return proxy_error(
                            StatusCode::BAD_GATEWAY,
                            "MCP OAuth token is malformed",
                        );
                    }
                },
                Err(_) => {
                    return proxy_error(StatusCode::UNAUTHORIZED, "MCP OAuth sign-in required");
                }
            }
        }
    };
    if let Some(capability) = &state.capability
        && !capability.is_current()
    {
        return proxy_error(StatusCode::FORBIDDEN, "MCP access was revoked");
    }
    let mut response = match send_proxy_request(
        &state,
        method.clone(),
        headers.clone(),
        body.clone(),
        query.as_deref(),
        &presented,
    )
    .await
    {
        Ok(response) => response,
        Err(_) => return proxy_error(StatusCode::BAD_GATEWAY, "MCP OAuth proxy request failed"),
    };
    // A pasted token has nothing to refresh; a 401 on it is the answer.
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        && let (
            ProxyCredential::Oauth {
                manager,
                token_lock,
            },
            Some(token),
        ) = (&state.credential, oauth_token)
    {
        let refreshed = {
            let _token_guard = token_lock.lock().await;
            if state
                .capability
                .as_ref()
                .is_some_and(|capability| !capability.is_current())
            {
                Err(rmcp::transport::auth::AuthError::AuthorizationRequired)
            } else {
                let manager = manager.lock().await;
                match manager.get_access_token().await {
                    Ok(current) if current != token => Ok(current),
                    Ok(_) => {
                        if manager.refresh_token().await.is_ok() {
                            manager.get_access_token().await
                        } else {
                            Err(rmcp::transport::auth::AuthError::AuthorizationRequired)
                        }
                    }
                    Err(error) => Err(error),
                }
            }
        };
        if let Ok(token) = refreshed
            && let Ok(presented) = mcp::Presented::new(None, &token)
            && state
                .capability
                .as_ref()
                .is_none_or(CapabilityLease::is_current)
            && let Ok(retried) =
                send_proxy_request(&state, method, headers, body, query.as_deref(), &presented)
                    .await
        {
            response = retried;
        }
    }
    proxy_response(response)
}

fn request_method(request: &Request<Body>) -> reqwest::Method {
    reqwest::Method::from_bytes(request.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET)
}

fn request_headers(request: &Request<Body>) -> Vec<(HeaderName, HeaderValue)> {
    request
        .headers()
        .iter()
        .filter(|(name, _)| *name != AUTHORIZATION && *name != HOST && *name != CONTENT_LENGTH)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

async fn send_proxy_request(
    state: &OAuthProxyState,
    method: reqwest::Method,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: axum::body::Bytes,
    query: Option<&str>,
    presented: &mcp::Presented,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut upstream = state.upstream.clone();
    upstream.set_query(query);
    let mut request = state
        .client
        .request(method, upstream)
        .header(presented.name.clone(), presented.value.clone())
        .body(body);
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_bytes());
    }
    request.send().await
}

fn proxy_response(response: reqwest::Response) -> Response {
    let status = response.status();
    let headers = response.headers().clone();
    let stream = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    let mut builder = Response::builder().status(status);
    for (name, value) in &headers {
        if *name != SET_COOKIE {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| proxy_error(StatusCode::BAD_GATEWAY, "MCP proxy response failed"))
}

fn proxy_error(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
