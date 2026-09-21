//! The running computer's guide, fetched from the computer itself.
//!
//! Hotline Computer bundles a skill for the release it is and serves it from
//! `state {"action":"guide"}` with its version and a checksum. Hotline never
//! bundles a copy: the desk pins one image tag, but an existing container
//! keeps the release it was made on (see docs/computer.md), so the only
//! guide guaranteed to describe the tools the agent actually has is the one
//! the running computer hands over. It is asked for once, when the computer
//! is granted at session start, and written into the workspace as the
//! `hotline-computer` skill. A computer that cannot answer — an older image
//! without the action, an endpoint that will not open — leaves no skill, and
//! the preamble tells the agent to ask the computer itself.

use super::Ready;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientInfo, Implementation};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};
use std::time::Duration;

/// A computer that is healthy answers its own guide in well under this;
/// one that does not is not holding up the teammate's start for longer.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// What a running computer says its guide is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guide {
    pub version: String,
    pub sha256: String,
    pub skill: String,
}

/// Asks the computer at `ready` for its guide.
pub async fn fetch(ready: &Ready) -> Result<Guide, String> {
    tokio::time::timeout(FETCH_TIMEOUT, ask(ready))
        .await
        .map_err(|_| "The computer did not answer for its guide in time.".to_string())?
}

async fn ask(ready: &Ready) -> Result<Guide, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let transport = StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(ready.url.clone())
            .auth_header(ready.token.clone()),
    );
    let service = ClientInfo::new(
        Default::default(),
        Implementation::new("hotline", env!("CARGO_PKG_VERSION")),
    )
    .serve(transport)
    .await
    .map_err(|error| format!("The computer's endpoint did not open: {error}"))?;
    let answered = service
        .call_tool(
            CallToolRequestParams::new("state").with_arguments(
                json!({"action": "guide"})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
        )
        .await;
    service.cancel().await.ok();
    let result = answered.map_err(|error| format!("The computer refused its guide: {error}"))?;
    let text = result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .unwrap_or_default();
    if result.is_error == Some(true) {
        return Err(format!("The computer refused its guide: {text}"));
    }
    parse(&text)
}

fn parse(text: &str) -> Result<Guide, String> {
    let manifest: Value = serde_json::from_str(text)
        .map_err(|_| "The computer's guide was not the manifest Hotline expects.".to_string())?;
    let field = |name: &str| {
        manifest
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("The computer's guide has no {name}."))
    };
    Ok(Guide {
        version: field("version")?,
        sha256: field("sha256")?,
        skill: field("skill")?,
    })
}

/// A computer a test can grant: `/health` open and `state guide` answering
/// the manifest a real one does, on a loopback port the fake runtime reports.
#[cfg(test)]
pub(crate) mod fake {
    use rmcp::ErrorData;
    use rmcp::handler::server::ServerHandler;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    };
    use rmcp::service::RequestContext;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    };
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// What the desk asked the browser to drop: the saved login's name and
    /// the domains named, in order.
    type Forgotten = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    /// Every set of secrets the fake was handed through `PUT /secrets`, in
    /// order, so a test can prove what reached the machine and what did
    /// not. A release too old for a guide has no such route.
    /// What the fake computer was handed and where its passkey arming
    /// stands: the record a test reads and drives.
    #[derive(Clone, Default)]
    pub(crate) struct Taken {
        sets: Arc<Mutex<Vec<BTreeMap<String, Value>>>>,
        /// The site armed for a passkey, and whether the browser has since
        /// "minted" one.
        arming: Arc<Mutex<Option<(String, bool)>>>,
        /// Every forget the desk asked for: the saved login's name and the
        /// domains, in order.
        forgotten: Forgotten,
    }

    /// A PKCS#8 P-256 key, as the virtual authenticator answers one.
    pub(crate) const KEY_BASE64: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeE7T2PDJPCRfPvTUFvVcQI7KFSDmnKbrRfEjRGbtV9WhRANCAATfeasaWHkMKJ4oCcdDzVX9c2xUUkC7Uiuqu8tS0LXtRJ8pCk+gNSvvqWaB3WgFNn4rvQ8wS1bH+dOjfgZoq2gz";

    impl Taken {
        // Read by the session tests, which drive a scripted runtime and so
        // exist on unix alone.
        #[cfg(unix)]
        pub(crate) fn sets(&self) -> Vec<BTreeMap<String, Value>> {
            self.sets.lock().unwrap().clone()
        }

        pub(crate) fn armed(&self) -> Option<String> {
            self.arming
                .lock()
                .unwrap()
                .as_ref()
                .map(|(rp_id, _)| rp_id.clone())
        }

        /// What the desk asked the browser to drop, in order.
        #[cfg(unix)]
        pub(crate) fn forgotten(&self) -> Vec<(String, Vec<String>)> {
            self.forgotten.lock().unwrap().clone()
        }

        /// The person added a passkey in the browser: the next poll answers
        /// the credential the authenticator minted.
        pub(crate) fn mint(&self) {
            if let Some(arming) = self.arming.lock().unwrap().as_mut() {
                arming.1 = true;
            }
        }

        /// The credential the fake authenticator mints for `rp_id`, as the
        /// computer answers it: a whole record, kind first.
        pub(crate) fn credential(rp_id: &str) -> Value {
            json!({
                "kind": "passkey", "rpId": rp_id, "credentialId": "AQID",
                "privateKey": KEY_BASE64, "userHandle": "dGVhbW1hdGUtMQ==", "userName": "teammate",
            })
        }
    }

    /// A JSON answer, built by hand: this axum is without its `json` feature.
    fn json_answer(status: axum::http::StatusCode, body: Value) -> axum::response::Response {
        use axum::response::IntoResponse;
        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response()
    }

    fn bearer_present(headers: &axum::http::HeaderMap) -> bool {
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|bearer| !bearer.is_empty())
    }

    async fn take_secrets(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
        body: String,
    ) -> axum::http::StatusCode {
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED;
        }
        match serde_json::from_str::<BTreeMap<String, Value>>(&body) {
            Ok(set) => {
                taken.sets.lock().unwrap().push(set);
                axum::http::StatusCode::NO_CONTENT
            }
            Err(_) => axum::http::StatusCode::BAD_REQUEST,
        }
    }

    async fn arm_passkey(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
        body: String,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        let rp_id = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|body| body["rpId"].as_str().map(str::to_owned))
            .unwrap_or_default();
        if rp_id.is_empty()
            || !rp_id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        {
            return json_answer(
                axum::http::StatusCode::BAD_REQUEST,
                json!({"error": format!("{rp_id:?} is not a site for a passkey")}),
            );
        }
        *taken.arming.lock().unwrap() = Some((rp_id.clone(), false));
        json_answer(
            axum::http::StatusCode::OK,
            json!({"state": "armed", "rpId": rp_id, "expiresAt": 1_700_000_600_000_i64}),
        )
    }

    async fn passkey_registration(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        let answer = match taken.arming.lock().unwrap().clone() {
            None => json!({"state": "idle"}),
            Some((rp_id, false)) => {
                json!({"state": "armed", "rpId": rp_id, "expiresAt": 1_700_000_600_000_i64})
            }
            Some((rp_id, true)) => json!({
                "state": "registered", "rpId": rp_id, "expiresAt": 1_700_000_600_000_i64,
                "credential": Taken::credential(&rp_id),
            }),
        };
        json_answer(axum::http::StatusCode::OK, answer)
    }

    async fn disarm_passkey(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
    ) -> axum::http::StatusCode {
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED;
        }
        *taken.arming.lock().unwrap() = None;
        axum::http::StatusCode::NO_CONTENT
    }

    async fn forget_login(
        axum::extract::State(taken): axum::extract::State<Taken>,
        axum::extract::Path(name): axum::extract::Path<String>,
        headers: axum::http::HeaderMap,
        body: String,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        let domains: Vec<String> = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|body| serde_json::from_value(body["domains"].clone()).ok())
            .unwrap_or_default();
        let count = domains.len();
        taken
            .forgotten
            .lock()
            .unwrap()
            .push((name.clone(), domains.clone()));
        json_answer(
            axum::http::StatusCode::OK,
            json!({"name": name, "domains": domains, "forgotten": count, "kept": 0}),
        )
    }

    /// Whether a release has the door that takes a login back: 0.8.1 and
    /// later do.
    fn has_logins_door(version: &str) -> bool {
        let mut parts = version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0));
        let (major, minor, patch) = (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        );
        (major, minor, patch) >= (0, 8, 1)
    }

    /// Whether a release has the passkey door: 0.8 and later do.
    fn has_passkeys(version: &str) -> bool {
        let mut parts = version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0));
        let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        (major, minor) >= (0, 8)
    }

    /// The skill text a release `version` would serve, with the placeholder
    /// filled the way Hotline Computer's own build fills it.
    pub(crate) fn skill_of(version: &str) -> String {
        format!(
            "---\nname: hotline-computer\ndescription: Operate a Hotline Computer through its MCP tools.\n---\n\nThis guide ships with Hotline Computer {version}.\n"
        )
    }

    #[derive(Clone)]
    struct FakeComputer {
        /// The release this fake is; `None` is an older image whose `state`
        /// has no `guide` action.
        version: Option<String>,
    }

    impl ServerHandler for FakeComputer {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<rmcp::RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            let schema = json!({"type":"object","properties":{"action":{"type":"string"}}});
            Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "state",
                "Computer state.",
                Arc::new(schema.as_object().cloned().unwrap()),
            )]))
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            context: RequestContext<rmcp::RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            let bearer = context
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.headers.get(axum::http::header::AUTHORIZATION))
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or("");
            let action = request
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.get("action"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let answered = match (&self.version, action) {
                _ if bearer.is_empty() => {
                    CallToolResult::error(vec![ContentBlock::text("no bearer token")])
                }
                (Some(version), "guide") => {
                    let skill = skill_of(version);
                    let manifest = json!({
                        "version": version,
                        "build": {"channel": "release", "revision": "fixture"},
                        "sha256": format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(skill.as_bytes())),
                        "skill": skill,
                        "catalog": {},
                    });
                    CallToolResult::success(vec![ContentBlock::text(manifest.to_string())])
                }
                _ => CallToolResult::error(vec![ContentBlock::text(format!(
                    "state: unknown action {action}"
                ))]),
            };
            Ok(answered.into())
        }
    }

    /// Serves a computer on a loopback port and answers the port. `version`
    /// is the release it claims; `None` is an image too old to have a guide.
    pub(crate) async fn serve(version: Option<&str>) -> u16 {
        serve_taking(version).await.0
    }

    /// The same, with the record of every set of secrets handed to it. An
    /// image too old for a guide is too old for `/secrets` as well, and
    /// answers 404 there the way a real one does.
    pub(crate) async fn serve_taking(version: Option<&str>) -> (u16, Taken) {
        let taken = Taken::default();
        let computer = FakeComputer {
            version: version.map(str::to_owned),
        };
        let service: StreamableHttpService<FakeComputer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(computer.clone()),
                Arc::new(LocalSessionManager::default()),
                Default::default(),
            );
        let mut app = axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .nest_service("/mcp", service);
        if version.is_some() {
            app = app.route("/secrets", axum::routing::put(take_secrets));
        }
        if version.is_some_and(has_passkeys) {
            app = app.route(
                "/passkeys/registration",
                axum::routing::put(arm_passkey)
                    .get(passkey_registration)
                    .delete(disarm_passkey),
            );
        }
        if version.is_some_and(has_logins_door) {
            app = app.route("/logins/{name}", axum::routing::delete(forget_login));
        }
        let app = app.with_state(taken.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (port, taken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_guide_is_the_release_the_computer_says_it_is() {
        let port = fake::serve(Some("0.9.1")).await;
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "fixture-token".into(),
        };
        let guide = fetch(&ready).await.expect("the fake answers");
        assert_eq!(guide.version, "0.9.1");
        assert_eq!(guide.skill, fake::skill_of("0.9.1"));
        assert_eq!(guide.sha256.len(), 64);
    }

    #[tokio::test]
    async fn an_older_image_without_the_action_is_a_sentence_not_a_skill() {
        let port = fake::serve(None).await;
        let ready = Ready {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: "fixture-token".into(),
        };
        let refused = fetch(&ready).await.unwrap_err();
        assert!(refused.contains("unknown action guide"), "{refused}");
    }

    #[test]
    fn a_manifest_missing_its_skill_is_refused_by_name() {
        let refused = parse(r#"{"version":"0.9.1","sha256":"abc"}"#).unwrap_err();
        assert_eq!(refused, "The computer's guide has no skill.");
        assert!(parse("not json").is_err());
    }
}
