//! A pairing invitation belongs to the live CLI connection, not its process id.
#![cfg(unix)]
use crate::common;

use common::{free_address, hotline, until};
use std::{
    path::Path,
    process::{Child, Stdio},
    time::Duration,
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// A wire command with no params, which must succeed, and its reply.
fn wire(root: &Path, cmd: &str) -> serde_json::Value {
    let output = common::wire(root, cmd, "{}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn the_pair_cli_opens_a_window_only_while_its_socket_lives() {
    let root = tempfile::tempdir().unwrap();
    let address = free_address();
    let endpoint = format!("https://{address}");
    let mut desk = OwnedChild(
        hotline(
            root.path(),
            &[
                "serve",
                "--store",
                "file",
                "--listen",
                &address.to_string(),
                "--public-url",
                &endpoint,
            ],
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    until("the desk writes door.json", || {
        assert!(
            desk.0.try_wait().unwrap().is_none(),
            "desk exited before startup"
        );
        root.path().join("door.json").exists()
    });
    until("the desk has an endpoint", || {
        !wire(root.path(), "remote.status")["endpoints"]
            .as_array()
            .unwrap()
            .is_empty()
    });

    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Only this isolated, freshly generated test certificate is untrusted.
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let status = |path: &str| {
        runtime.block_on(async {
            client
                .get(format!("{endpoint}{path}"))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        })
    };
    assert_eq!(status("/v2/pair"), 404);
    assert_eq!(status("/pair"), 404);
    assert_eq!(status("/pair/manual/start"), 404);
    let devices = hotline(root.path(), &["devices"]).output().unwrap();
    assert!(devices.status.success());
    assert!(String::from_utf8_lossy(&devices.stdout).contains("No paired devices"));

    // SIGINT cancels explicitly; SIGKILL proves socket teardown alone is enough.
    for signal in [libc::SIGINT, libc::SIGKILL] {
        let mut pair = OwnedChild(
            hotline(root.path(), &["pair", "--companion"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        until("the pairing window opens", || {
            assert!(
                pair.0.try_wait().unwrap().is_none(),
                "pair command exited early"
            );
            status("/v2/pair") == 400 // Open route refuses a request without WS upgrade.
        });
        // SAFETY: only the pid captured when this test spawned the CLI.
        assert_eq!(unsafe { libc::kill(pair.0.id() as libc::pid_t, signal) }, 0);
        until("the pair command exits", || {
            pair.0.try_wait().unwrap().is_some()
        });
        until("the pairing window closes", || status("/v2/pair") == 404);
    }
    assert_eq!(wire(root.path(), "remote.devices"), serde_json::json!([]));
    for flag in ["--json", "--link"] {
        let mut pair = OwnedChild(
            hotline(root.path(), &["pair", flag])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let stdout = pair.0.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let line = std::io::BufReader::new(stdout)
                .lines()
                .next()
                .unwrap()
                .unwrap();
            send.send(line).unwrap();
        });
        let line = receive.recv_timeout(common::PATIENCE).unwrap();
        let payload: hotline_core::remote::PairingPayload = if flag == "--json" {
            serde_json::from_str(&line).unwrap()
        } else {
            hotline_core::remote::PairingPayload::from_link(&line).unwrap()
        };
        assert_eq!(payload.version, 2);
        assert_eq!(payload.url, endpoint);
        assert!(
            pair.0.try_wait().unwrap().is_none(),
            "invitation must remain alive after stdout flush"
        );
        let client_root = tempfile::tempdir().unwrap();
        let client = hotline_core::remote::client::Client::new(std::sync::Arc::new(
            hotline_core::credentials::FileStore::open(client_root.path()).unwrap(),
        ));
        let desk = runtime
            .block_on(client.pair(&payload, "CLI test laptop"))
            .unwrap();
        assert_eq!(desk.url, endpoint);
        until("the pair command exits", || {
            pair.0.try_wait().unwrap().is_some()
        });
        assert!(pair.0.wait().unwrap().success());
        assert!(
            runtime
                .block_on(client.pair(&payload, "Duplicate"))
                .is_err()
        );
    }

    // A relay invitation says so, and makes a grant `devices` names relay.
    let both = hotline(root.path(), &["pair", "--companion", "--relay"])
        .output()
        .unwrap();
    assert!(!both.status.success());
    let mut pair = OwnedChild(
        hotline(root.path(), &["pair", "--relay", "--link"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let line = {
        use std::io::BufRead;
        let stdout = pair.0.stdout.take().unwrap();
        std::io::BufReader::new(stdout)
            .lines()
            .next()
            .unwrap()
            .unwrap()
    };
    let payload = hotline_core::remote::PairingPayload::from_link(&line).unwrap();
    assert_eq!(payload.role, hotline_core::contract::DeviceRole::Relay);
    let client_root = tempfile::tempdir().unwrap();
    let client = hotline_core::remote::client::Client::new(std::sync::Arc::new(
        hotline_core::credentials::FileStore::open(client_root.path()).unwrap(),
    ));
    runtime
        .block_on(client.pair_relay(&payload, "Relay laptop"))
        .unwrap();
    until("the pair command exits", || {
        pair.0.try_wait().unwrap().is_some()
    });
    let devices = hotline(root.path(), &["devices"]).output().unwrap();
    let devices = String::from_utf8_lossy(&devices.stdout);
    assert!(
        devices
            .lines()
            .any(|line| line.contains("  relay  Relay laptop")),
        "{devices}"
    );
}

/// A phone never pairs with a relay-only server. Its QR says it is a relay,
/// and a phone that claims it anyway, with no `purpose` as every phone
/// before ADR 0006 does, is refused as a relay and spends nothing.
#[test]
fn a_relay_only_server_refuses_a_phones_claim_and_keeps_its_window() {
    let root = tempfile::tempdir().unwrap();
    let address = free_address();
    let endpoint = format!("https://{address}");
    let mut server = OwnedChild(
        hotline(
            root.path(),
            &[
                "serve",
                "--store",
                "file",
                "--capabilities",
                "relay",
                "--listen",
                &address.to_string(),
                "--public-url",
                &endpoint,
            ],
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap(),
    );
    until("the server writes door.json", || {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited before startup"
        );
        root.path().join("door.json").exists()
    });

    // What a phone's camera reads: the QR's own text.
    let qr = wire(root.path(), "remote.pairing");
    assert!(qr["url"].as_str().unwrap().contains("r=relay"), "{qr}");

    let mut pair = OwnedChild(
        hotline(root.path(), &["pair", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let line = {
        use std::io::BufRead;
        let stdout = pair.0.stdout.take().unwrap();
        std::io::BufReader::new(stdout)
            .lines()
            .next()
            .unwrap()
            .unwrap()
    };
    let invitation: hotline_core::remote::PairingPayload = serde_json::from_str(&line).unwrap();
    assert_eq!(invitation.role, hotline_core::contract::DeviceRole::Relay);

    // An old phone ignores `r` and claims as a device would: read as an
    // owner invitation, the client sends no `purpose`, so only the server's
    // `{"refused":"relay"}` can make this refusal.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let phone_root = tempfile::tempdir().unwrap();
    let phone = hotline_core::remote::client::Client::new(std::sync::Arc::new(
        hotline_core::credentials::FileStore::open(phone_root.path()).unwrap(),
    ));
    let mut old_phone_reading = invitation.clone();
    old_phone_reading.role = hotline_core::contract::DeviceRole::Owner;
    let refused = runtime
        .block_on(phone.pair(&old_phone_reading, "Old phone"))
        .unwrap_err();
    assert_eq!(
        refused,
        "This invitation is for using the server as a relay."
    );
    assert_eq!(wire(root.path(), "remote.devices"), serde_json::json!([]));
    assert!(
        pair.0.try_wait().unwrap().is_none(),
        "a refused claim closed the window"
    );

    // The window is still open: the desktop it was for claims it.
    let desktop_root = tempfile::tempdir().unwrap();
    let desktop = hotline_core::remote::client::Client::new(std::sync::Arc::new(
        hotline_core::credentials::FileStore::open(desktop_root.path()).unwrap(),
    ));
    runtime
        .block_on(desktop.pair_relay(&invitation, "Laptop"))
        .unwrap();
    until("the pair command exits", || {
        pair.0.try_wait().unwrap().is_some()
    });
    let devices = wire(root.path(), "remote.devices");
    let roles: Vec<&str> = devices
        .as_array()
        .unwrap()
        .iter()
        .map(|device| device["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["relay"], "{devices}");
}
