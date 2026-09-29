//! Operator-only Remote commands through the running desk's loopback Door.
use crate::door;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{path::Path, process::ExitCode, time::Duration};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn outcome(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hotline: {error}");
            ExitCode::from(1)
        }
    }
}

pub fn devices(root: &Path) -> ExitCode {
    outcome(door::runtime().block_on(async {
        let desk = door::running(root)?;
        let devices = door::ask(&desk, "remote.devices", json!({})).await?;
        let devices = devices
            .as_array()
            .ok_or("The desk returned an invalid device list")?;
        if devices.is_empty() {
            println!("No paired devices. Run `hotline pair` to link a phone.");
        }
        for device in devices {
            println!(
                "{}  {}  {}",
                field(device, "id")?,
                field(device, "role")?,
                field(device, "name")?.escape_default()
            );
        }
        Ok(())
    }))
}

pub fn revoke(root: &Path, id: &str) -> ExitCode {
    outcome(door::runtime().block_on(async {
        let desk = door::running(root)?;
        door::ask(&desk, "remote.revoke", json!({"deviceId":id})).await?;
        println!("Device revoked.");
        Ok(())
    }))
}

pub fn pair(root: &Path, companion: bool, json: bool, link: bool) -> ExitCode {
    outcome(door::runtime().block_on(async {
        let desk = door::running(root)?;
        let url = format!("ws://127.0.0.1:{}/ws?token={}", desk.port, desk.token);
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(url),
        )
        .await
        .map_err(|_| "The desk did not answer")?
        .map_err(|_| "The desk refused the connection")?;
        // One socket owns the invitation until this command exits. Disconnect
        // also closes the window, including when SSH or this process dies.
        let mut serial = 0;
        let role = if companion { "companion" } else { "owner" };
        let invitation = ask(&mut socket, &mut serial, json!({"role":role})).await?;
        let id = field(&invitation, "id")?.to_owned();
        if json {
            println!(
                "{}",
                invitation
                    .get("payload")
                    .ok_or("The desk does not support JSON pairing; update it first.")?
            );
        } else if link {
            println!("{}", field(&invitation, "link")?);
        } else {
            print_invitation(&invitation, role)?;
        }
        use std::io::Write;
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let result = wait_for_phone(&mut socket, &mut serial, &invitation, &id).await;
        let _ = ask(&mut socket, &mut serial, json!({"id":id,"cancel":true})).await;
        let _ = socket.close(None).await;
        result
    }))
}

fn print_invitation(invitation: &Value, role: &str) -> Result<(), String> {
    let url = field(invitation, "url")?;
    let code =
        qrcode::QrCode::new(url.as_bytes()).map_err(|_| "The pairing QR could not be drawn")?;
    println!(
        "Scan this QR in Hotline on your phone to pair as {role}. It expires in two minutes.\n"
    );
    println!(
        "{}",
        code.render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build()
    );
    // The link is not printed beside it: scrollback is still sensitive, but
    // a casual copy never carries the secret. `--link` asks for it by name.
    println!("Pairing a desktop instead? Run `hotline pair --link` and paste the link.");
    Ok(())
}

async fn wait_for_phone(
    socket: &mut Socket,
    serial: &mut u64,
    _invitation: &Value,
    id: &str,
) -> Result<(), String> {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|e| e.to_string())?;
    let deadline = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = term.recv() => return Err("Pairing cancelled.".into()),
            _ = interrupt.recv() => return Err("Pairing cancelled.".into()),
            _ = &mut deadline => return Err("Pairing expired. Run `hotline pair` to try again.".into()),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let device = tokio::select! {
            _ = term.recv() => return Err("Pairing cancelled.".into()),
            _ = interrupt.recv() => return Err("Pairing cancelled.".into()),
            _ = &mut deadline => return Err("Pairing expired.".into()),
            result = ask(socket, serial, json!({"id":id})) => result?,
        };
        if !device.is_null() {
            eprintln!(
                "Paired {} as {}.",
                field(&device, "name")?.escape_default(),
                field(&device, "role")?
            );
            return Ok(());
        }
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("The desk's response has no {name}"))
}

async fn ask(socket: &mut Socket, serial: &mut u64, params: Value) -> Result<Value, String> {
    *serial += 1;
    let id = *serial;
    tokio::time::timeout(Duration::from_secs(5), async {
        socket
            .send(Message::text(
                json!({"id":id,"cmd":"remote.pairing","params":params}).to_string(),
            ))
            .await
            .map_err(|_| "The desk disconnected".to_string())?;
        while let Some(frame) = socket.next().await {
            let frame = frame.map_err(|_| "The desk disconnected".to_string())?;
            let Message::Text(text) = frame else { continue };
            let value: Value = serde_json::from_str(&text)
                .map_err(|_| "Invalid reply from the desk".to_string())?;
            if value["id"].as_u64() != Some(id) {
                continue;
            }
            return if value["ok"] == true {
                Ok(value["result"].clone())
            } else {
                Err(value["error"]
                    .as_str()
                    .unwrap_or("The desk refused pairing")
                    .to_owned())
            };
        }
        Err("The desk disconnected".into())
    })
    .await
    .map_err(|_| "The desk did not answer within five seconds".to_string())?
}
