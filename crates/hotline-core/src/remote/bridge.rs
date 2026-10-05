//! Each bridge is a loopback capability, never the remote device's private key.
use super::client::{Client, Discovery, PairedDesk, State};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
    },
};
use tokio_util::sync::CancellationToken;

const VIEWER_PROTOCOL: &str = "hotline-viewer.";
const VIEWER_TOKEN_TTL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct ViewerTokens(HashMap<String, ViewerToken>);
struct ViewerToken {
    persona: String,
    expires: Instant,
}
impl ViewerTokens {
    fn mint(&mut self, persona: &str) -> String {
        let now = Instant::now();
        self.0.retain(|_, token| token.expires > now);
        let token = super::secret();
        self.0.insert(
            token.clone(),
            ViewerToken {
                persona: persona.to_owned(),
                expires: now + VIEWER_TOKEN_TTL,
            },
        );
        token
    }

    fn redeem(&mut self, token: &str, persona: &str) -> Result<(), ()> {
        let now = Instant::now();
        self.0.retain(|_, token| token.expires > now);
        if self.0.get(token).ok_or(())?.persona != persona {
            return Err(());
        }
        // Validation and removal share the mutex, so simultaneous upgrades
        // cannot both redeem the same capability.
        self.0.remove(token);
        Ok(())
    }
}

pub struct Bridge {
    pub origin: String,
    pub token: String,
    pub state: tokio::sync::watch::Receiver<State>,
    /// The latest relay learned through an authenticated remote session.
    pub relay: tokio::sync::watch::Receiver<Option<String>>,
    cancel: CancellationToken,
    viewer_tokens: Arc<Mutex<ViewerTokens>>,
}

struct Discoveries {
    relay: tokio::sync::watch::Sender<Option<String>>,
    latest: Mutex<u64>,
}
impl Discoveries {
    fn publish(&self, discovery: &Discovery) {
        let mut latest = self.latest.lock().unwrap();
        if discovery.sequence <= *latest {
            return;
        }
        // Even an unchanged relay advances the stamp: an older session must
        // not overwrite a more recent hello that confirmed this address.
        *latest = discovery.sequence;
        self.relay.send_if_modified(|known| {
            if *known == discovery.relay {
                false
            } else {
                *known = discovery.relay.clone();
                true
            }
        });
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Bridge {
    /// Mint a single-use, 30-second capability for one persona's viewer upgrade.
    /// Present it as the `hotline-viewer.<token>` WebSocket subprotocol. A
    /// reconnect needs a fresh token, even if the remote computer was unavailable.
    pub fn viewer_token(&self, persona_id: &str) -> String {
        self.viewer_tokens.lock().unwrap().mint(persona_id)
    }

    pub async fn start(desk: &PairedDesk) -> Result<Self, String> {
        Self::with_client(desk, Client::default()).await
    }
    pub async fn with_client(desk: &PairedDesk, client: Client) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Could not open the local desk bridge.")?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let origin = format!("http://{address}");
        let token = super::secret();
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let (state, receive) = tokio::sync::watch::channel(State::Connecting);
        let (relay, endpoints) = tokio::sync::watch::channel(desk.relay.clone());
        let discoveries = Arc::new(Discoveries {
            relay,
            latest: Mutex::new(0),
        });
        let desk = desk.clone();
        let key = token.clone();
        let viewer_tokens = Arc::new(Mutex::new(ViewerTokens::default()));
        let viewers = viewer_tokens.clone();
        tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = stopped.cancelled() => break,
                    _ = tasks.join_next(), if !tasks.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((socket, peer)) = accepted else { break; };
                        if !peer.ip().is_loopback() || tasks.len() >= 32 { continue; }
                        let client = client.clone(); let mut desk = desk.clone(); let key = key.clone(); let state = state.clone(); let viewers = viewers.clone(); let discoveries = discoveries.clone();
                        // New command and viewer sessions use what an earlier
                        // authenticated hello taught this bridge.
                        desk.relay = discoveries.relay.borrow().clone();
                        tasks.spawn(async move { serve(socket, client, desk, key, viewers, state, discoveries).await; });
                    }
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        Ok(Self {
            origin,
            token,
            state: receive,
            relay: endpoints,
            cancel,
            viewer_tokens,
        })
    }
}

struct Authorization {
    persona: Option<String>,
    protocol: Option<http::HeaderValue>,
}

fn authorize(
    request: &Request,
    owner_token: &str,
    viewers: &Mutex<ViewerTokens>,
) -> Result<Authorization, ()> {
    // Use the literal path: URL normalization must not turn a different route
    // (such as a dot-segment path) into the persona named by this capability.
    let path = request.uri().path();
    let persona = if path == "/ws" {
        None
    } else {
        let id = path
            .strip_prefix("/computer/")
            .and_then(|path| path.strip_suffix("/ws"))
            .ok_or(())?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(());
        }
        Some(id.to_owned())
    };
    let query_tokens: Vec<_> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
            .filter(|(key, _)| key == "token")
            .collect();
    let mut viewer_protocol = None;
    for value in request
        .headers()
        .get_all(http::header::SEC_WEBSOCKET_PROTOCOL)
    {
        for protocol in value.to_str().map_err(|_| ())?.split(',').map(str::trim) {
            if let Some(token) = protocol.strip_prefix(VIEWER_PROTOCOL) {
                if viewer_protocol.is_some()
                    || token.len() != 64
                    || !token.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(());
                }
                viewer_protocol = Some((protocol, token));
            }
        }
    }
    if let Some((protocol, token)) = viewer_protocol {
        let id = persona.as_deref().ok_or(())?;
        // Never fall back to owner authority for a malformed or spent viewer
        // credential, or accept a viewer capability from the query string.
        if !query_tokens.is_empty() {
            return Err(());
        }
        let protocol = protocol.parse().map_err(|_| ())?;
        viewers.lock().unwrap().redeem(token, id)?;
        return Ok(Authorization {
            persona,
            protocol: Some(protocol),
        });
    }
    // The owner credential opens only the wire, never a computer viewer.
    if persona.is_some()
        || query_tokens.len() != 1
        || !crate::wire::same_secret(&query_tokens[0].1, owner_token)
    {
        return Err(());
    }
    Ok(Authorization {
        persona,
        protocol: None,
    })
}

#[allow(clippy::result_large_err)] // tungstenite fixes the callback error type.
async fn serve(
    stream: TcpStream,
    client: Client,
    desk: PairedDesk,
    token: String,
    viewers: Arc<Mutex<ViewerTokens>>,
    state: tokio::sync::watch::Sender<State>,
    discoveries: Arc<Discoveries>,
) {
    let target = Arc::new(Mutex::new(None));
    let selected = target.clone();
    let callback =
        move |request: &Request, mut response: Response| -> Result<Response, ErrorResponse> {
            match authorize(request, &token, &viewers) {
                Ok(authorization) => {
                    *selected.lock().unwrap() = authorization.persona;
                    if let Some(protocol) = authorization.protocol {
                        response
                            .headers_mut()
                            .insert(http::header::SEC_WEBSOCKET_PROTOCOL, protocol);
                    }
                    Ok(response)
                }
                Err(()) => Err(http::Response::builder()
                    .status(403)
                    .body(Some("Forbidden".into()))
                    .unwrap()),
            }
        };
    let config = WebSocketConfig::default()
        .max_message_size(Some(32 * 1024 * 1024))
        .max_frame_size(Some(32 * 1024 * 1024));
    let Ok(Ok(mut local)) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)),
    )
    .await
    else {
        return;
    };
    let persona = target.lock().unwrap().clone();
    if let Some(persona) = persona {
        viewer(local, client, desk, &persona).await;
        return;
    }
    let mut remote = client.connect(&desk);
    loop {
        tokio::select! {
            changed = remote.state.changed() => {
                if changed.is_err() { break; }
                state.send_replace(*remote.state.borrow_and_update());
            }
            changed = remote.discovery.changed() => {
                if changed.is_err() { break; }
                if let Some(discovery) = remote.discovery.borrow_and_update().as_ref() {
                    discoveries.publish(discovery);
                }
            }
            frame = local.next() => match frame {
                Some(Ok(Message::Text(text))) => if remote.outgoing.send(text.to_string()).await.is_err() { break; },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            frame = remote.incoming.recv() => {
                let Some(frame) = frame else { break; };
                if local.send(Message::text(frame)).await.is_err() { break; }
            }
        }
    }
    // The incoming queue and state watch can close together on revocation.
    // Publish the terminal state even if select! observed the queue first.
    state.send_replace(*remote.state.borrow());
    // The state and queue can close before the watch is selected. Its final
    // authenticated snapshot remains safe; the shared stamp rejects stale ones.
    if let Some(discovery) = remote.discovery.borrow().as_ref() {
        discoveries.publish(discovery);
    }
}

async fn viewer(
    mut local: WebSocketStream<TcpStream>,
    client: Client,
    desk: PairedDesk,
    persona: &str,
) {
    let Ok(mut remote) = client.open(&desk, Some(persona)).await else {
        return;
    };
    loop {
        tokio::select! {
            frame = local.next() => match frame {
                Some(Ok(Message::Text(text))) if text.len() <= super::viewer_files::MAX_REQUEST => {
                    if remote.send(Message::text(json!({"type":"text","data":text.as_str()}).to_string())).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                _ => break,
            },
            frame = remote.next() => {
                let Some(Ok(Message::Text(text))) = frame else { break; };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { break; };
                let Some(data) = value["data"].as_str() else { break; };
                let frame = match value["type"].as_str() {
                    Some("text") => Message::text(data),
                    Some("binary") => { let Ok(bytes) = STANDARD.decode(data) else { break; }; Message::Binary(bytes.into()) }
                    _ => break,
                };
                if local.send(frame).await.is_err() { break; }
            }
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn a_delayed_session_cannot_overwrite_a_newer_authenticated_discovery() {
        let (relay, mut watch) = tokio::sync::watch::channel(None);
        let discoveries = Discoveries {
            relay,
            latest: Mutex::new(0),
        };
        let old = Discovery {
            sequence: 1,
            relay: Some("https://old.example/relay/desk".into()),
        };
        let new = Discovery {
            sequence: 2,
            relay: Some("https://new.example/relay/desk".into()),
        };
        discoveries.publish(&new);
        assert_eq!(*watch.borrow_and_update(), new.relay);
        // An older hello, delayed behind a local writer or in the final
        // snapshot of a closing session, must not roll the bridge back.
        discoveries.publish(&old);
        assert_eq!(*watch.borrow(), new.relay);
        assert!(!watch.has_changed().unwrap());

        // Confirming the same relay still supersedes intervening discovery.
        discoveries.publish(&Discovery {
            sequence: 4,
            relay: new.relay.clone(),
        });
        discoveries.publish(&Discovery {
            sequence: 3,
            relay: None,
        });
        assert_eq!(*watch.borrow(), new.relay);
        assert!(!watch.has_changed().unwrap());
        discoveries.publish(&Discovery {
            sequence: 5,
            relay: None,
        });
        assert_eq!(*watch.borrow(), None);
        assert!(watch.has_changed().unwrap());
    }
}
