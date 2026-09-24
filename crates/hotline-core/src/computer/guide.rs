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

    /// Where the fake's passkey arming stands: the site, the request a
    /// page parked under it with the person's answer so far, and whether
    /// the browser has since "minted" one.
    #[derive(Clone, Debug)]
    struct FakeArming {
        rp_id: String,
        ask: Option<(Value, Option<bool>)>,
        minted: bool,
    }

    /// Every set of secrets the fake was handed through `PUT /secrets`, in
    /// order, so a test can prove what reached the machine and what did
    /// not. A release too old for a guide has no such route.
    /// What the fake computer was handed and where its passkey arming
    /// stands: the record a test reads and drives.
    #[derive(Clone, Default)]
    pub(crate) struct Taken {
        sets: Arc<Mutex<Vec<BTreeMap<String, Value>>>>,
        arming: Arc<Mutex<Option<FakeArming>>>,
        /// How many requests pages have parked, so each gets its own id.
        asks: Arc<std::sync::atomic::AtomicUsize>,
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
                .map(|arming| arming.rp_id.clone())
        }

        /// The site asked for a passkey: a request for the armed site waits
        /// for the person from the next poll on. Answers its id, `ask-1`
        /// for the first, so a test can answer it.
        pub(crate) fn ask(&self) -> String {
            let id = format!(
                "ask-{}",
                self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
            );
            if let Some(arming) = self.arming.lock().unwrap().as_mut() {
                let rp_id = arming.rp_id.clone();
                arming.ask = Some((
                    json!({
                        "id": id, "rpId": rp_id, "origin": format!("https://{rp_id}"),
                        "rpName": "The site", "userName": "teammate", "userDisplayName": "The teammate",
                        "askedAt": 1_700_000_000_000_i64,
                    }),
                    None,
                ));
            }
            id
        }

        /// The person's answer to the request, as the desk carried it in.
        pub(crate) fn answered(&self) -> Option<bool> {
            self.arming
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|arming| arming.ask.as_ref())
                .and_then(|(_, answer)| *answer)
        }

        /// The page the request was parked on went away, and the request
        /// with it.
        #[cfg(unix)]
        pub(crate) fn page_left(&self) {
            if let Some(arming) = self.arming.lock().unwrap().as_mut() {
                arming.ask = None;
            }
        }

        /// What the desk asked the browser to drop, in order.
        #[cfg(unix)]
        pub(crate) fn forgotten(&self) -> Vec<(String, Vec<String>)> {
            self.forgotten.lock().unwrap().clone()
        }

        /// The browser minted a passkey: the next poll answers the
        /// credential. (A real computer mints only under an approved
        /// request; the test decides the order here.)
        pub(crate) fn mint(&self) {
            if let Some(arming) = self.arming.lock().unwrap().as_mut() {
                arming.minted = true;
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
        *taken.arming.lock().unwrap() = Some(FakeArming {
            rp_id: rp_id.clone(),
            ask: None,
            minted: false,
        });
        json_answer(
            axum::http::StatusCode::OK,
            json!({"state": "armed", "rpId": rp_id, "expiresAt": 1_700_000_600_000_i64}),
        )
    }

    /// Where the arming stands, as a real computer answers it.
    fn registration_status(arming: Option<&FakeArming>) -> Value {
        let Some(arming) = arming else {
            return json!({"state": "idle"});
        };
        let state = match (arming.minted, &arming.ask) {
            (true, _) => "registered",
            (false, Some((_, None))) => "asked",
            (false, Some((_, Some(true)))) => "approved",
            (false, Some((_, Some(false)))) => "denied",
            (false, None) => "armed",
        };
        let mut status =
            json!({"state": state, "rpId": arming.rp_id, "expiresAt": 1_700_000_600_000_i64});
        if let Some((ask, _)) = &arming.ask {
            status["ask"] = ask.clone();
        }
        if arming.minted {
            status["credential"] = Taken::credential(&arming.rp_id);
        }
        status
    }

    async fn passkey_registration(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        let answer = registration_status(taken.arming.lock().unwrap().as_ref());
        json_answer(axum::http::StatusCode::OK, answer)
    }

    async fn answer_passkey(
        axum::extract::State(taken): axum::extract::State<Taken>,
        headers: axum::http::HeaderMap,
        body: String,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        let body = serde_json::from_str::<Value>(&body).unwrap_or_default();
        let id = body["id"].as_str().unwrap_or_default().to_owned();
        let approved = body["approved"].as_bool().unwrap_or(false);
        let mut guard = taken.arming.lock().unwrap();
        let waiting = guard
            .as_mut()
            .and_then(|arming| arming.ask.as_mut())
            .filter(|(ask, _)| ask["id"] == id);
        let Some((_, answer)) = waiting else {
            return json_answer(
                axum::http::StatusCode::CONFLICT,
                json!({"error": "no passkey request with that id is waiting for an answer"}),
            );
        };
        *answer = Some(approved);
        if !approved {
            *guard = None;
        }
        json_answer(
            axum::http::StatusCode::OK,
            registration_status(guard.as_ref()),
        )
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

    /// The one file the fake's home holds, at `/home/agent/report.txt`.
    pub(crate) const FILE_BODY: &[u8] = b"quarterly numbers\n";

    /// `GET /files/download`, as a real computer answers it: the file
    /// itself, or a refusal in JSON.
    async fn download(
        headers: axum::http::HeaderMap,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if !bearer_present(&headers) {
            return json_answer(
                axum::http::StatusCode::UNAUTHORIZED,
                json!({"error": "unauthorized"}),
            );
        }
        let path = reqwest::Url::parse(&format!("http://computer{uri}"))
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "path")
                    .map(|(_, value)| value.into_owned())
            });
        if path.as_deref() != Some("/home/agent/report.txt") {
            // What a real one says of a path it cannot resolve.
            return json_answer(
                axum::http::StatusCode::BAD_REQUEST,
                json!({"error": "path must be under /home/agent/"}),
            );
        }
        (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            FILE_BODY,
        )
            .into_response()
    }

    /// A small picture, as `capture` returns one: a PNG.
    fn screen_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbImage::from_pixel(8, 6, image::Rgb([40, 90, 200]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
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

    /// Whether a release asks the person before a passkey is made, and so
    /// has the answer door: 0.9 and later do.
    fn has_answer_door(version: &str) -> bool {
        let mut parts = version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0));
        let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        (major, minor) >= (0, 9)
    }

    /// Whether a release has the download door: 0.10 and later do.
    fn has_download_door(version: &str) -> bool {
        let mut parts = version
            .split('.')
            .map(|part| part.parse::<u32>().unwrap_or(0));
        let (major, minor) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        (major, minor) >= (0, 10)
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
            let schema = Arc::new(schema.as_object().cloned().unwrap());
            Ok(ListToolsResult::with_all_items(vec![
                Tool::new("state", "Computer state.", schema.clone()),
                Tool::new("capture", "See the screen.", schema),
            ]))
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
            let window = request
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.get("window"))
                .and_then(Value::as_str);
            let answered = match (&self.version, action) {
                _ if bearer.is_empty() => {
                    CallToolResult::error(vec![ContentBlock::text("no bearer token")])
                }
                _ if request.name == "capture" => match window {
                    Some(wanted) if wanted != "Editor" => {
                        CallToolResult::error(vec![ContentBlock::text(format!(
                            "no window is {wanted:?}; the windows are 1 \"Editor\""
                        ))])
                    }
                    _ => CallToolResult::success(vec![
                        ContentBlock::image(
                            base64::Engine::encode(&base64::prelude::BASE64_STANDARD, screen_png()),
                            "image/png",
                        ),
                        ContentBlock::text("8x6 image; 1 px = 1 screen px"),
                    ]),
                },
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
        if version.is_some_and(has_download_door) {
            app = app.route("/files/download", axum::routing::get(download));
        }
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
        if version.is_some_and(has_answer_door) {
            app = app.route(
                "/passkeys/registration/answer",
                axum::routing::post(answer_passkey),
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
