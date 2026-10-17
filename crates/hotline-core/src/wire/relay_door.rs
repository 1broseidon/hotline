//! The loopback door of a server that runs only a relay (ADR 0006).
//!
//! The same frames as the wire, `{id, cmd, params}` answered by id, but only
//! the commands that administer the relay from the server's own terminal:
//! `remote.status`, `remote.pairing`, `remote.devices` and `remote.revoke`.
//! Everything else needs a room, which this server never opens, and is
//! refused with a sentence saying so. As on the wire, an invitation this
//! socket opened closes when the socket does.

use super::read_command;
use crate::contract::Command;
use crate::remote::{RELAY_ONLY, Remote};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error, Message};

pub(super) async fn serve<S>(
    mut socket: WebSocketStream<S>,
    remote: Arc<Remote>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut invitation = None;
    let ended = loop {
        let text = match socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(Message::Close(_))) | None => break Ok(()),
            Some(Ok(_)) => continue,
            Some(Err(error)) => break Err(error),
        };
        let Ok(frame) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let id = frame["id"].clone();
        let answer = match answer(&remote, &frame, &mut invitation) {
            Ok(result) => json!({"id": id, "ok": true, "result": result}),
            Err(error) => json!({"id": id, "ok": false, "error": error}),
        };
        if let Err(error) = socket.send(Message::text(answer.to_string())).await {
            break Err(error);
        }
    };
    if let Some(id) = invitation {
        let _ = remote.cancel_pairing(&id);
    }
    ended
}

fn answer(
    remote: &Remote,
    frame: &Value,
    invitation: &mut Option<String>,
) -> Result<Value, String> {
    match read_command(frame)? {
        Command::RemoteStatus {} => Ok(json!(remote.status())),
        Command::RemoteDevices {} => Ok(json!(remote.devices())),
        Command::RemoteRevoke { device_id } => Ok(json!(remote.revoke(&device_id)?)),
        Command::RemotePairing { role, id, cancel } => {
            let opening = id.is_none() && !cancel;
            let result = remote.pairing(role, id, cancel)?;
            if opening {
                *invitation = result["id"].as_str().map(str::to_owned);
            }
            Ok(result)
        }
        _ => Err(RELAY_ONLY.into()),
    }
}
