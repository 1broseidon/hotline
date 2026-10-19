//! Remote: the paired devices and the desk's status, a pairing invitation and
//! its link, and the relay a desk stands in on.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::DeviceRole;

#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    #[serde(default)]
    pub role: DeviceRole,
    // Older bearer grants remain visible for revocation, but only a public
    // key can authorize a sealed connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public_key: Option<String>,
    /// Set only on the grant a phone's own `remote.devices` came in on, so it
    /// can name itself and warn before revoking itself. Never saved true:
    /// the flag is put on a copy as the list is answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current: Option<bool>,
}

#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteStatus {
    pub enabled: bool,
    pub host: String,
    pub endpoint: Option<String>,
    pub endpoints: Vec<String>,
    pub addresses: Vec<String>,
    pub devices: Vec<RemoteDevice>,
    pub error: Option<String>,
    /// The paired desk carrying visitors to this one, when one is chosen.
    pub relay: Option<RemoteRelay>,
    /// The person's own public address for this desk, kept whether or not
    /// Remote is on, and advertised to phones after this computer's own.
    pub public_url: Option<String>,
}

/// What the desk shows about the relay it stands in on.
#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct RemoteRelay {
    /// The paired desk carrying this one's visitors.
    pub desk_id: String,
    pub name: String,
    /// Where a visitor reaches this desk, once the relay has said.
    pub url: Option<String>,
    pub error: Option<String>,
}

/// A server this computer pairs with only to stand in on its relay (ADR
/// 0006). It is not a desk: its grant opens no wire, so it has no bridge and
/// is never in `desks.json`, where an older window would list it and dial
/// its `/v2`. The shell keeps these in `relays.json` beside that file.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PairedRelay {
    pub desk_id: String,
    pub name: String,
    pub url: String,
    pub desk_key: String,
    /// The person's public address for the server, tried after its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public: Option<String>,
}

#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct SealedPairing {
    pub payload: PairingPayload,
    pub link: String,
    pub id: String,
    pub url: String,
    pub qr_svg: String,
    pub expires_at: i64,
}

/// The link and SSH formats carry the same short-lived capability as the QR.
#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "contract.ts")]
pub struct PairingPayload {
    pub version: u8,
    pub url: String,
    pub desk_key: String,
    pub secret: String,
    pub role: DeviceRole,
    pub expires_at: i64,
    pub name: String,
    /// The desk's address on its relay, when it stands in on one: the same
    /// desk and key, reached through a desk that carries sealed records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay: Option<String>,
    /// The person's public address for the desk, tried after `url` and
    /// before the relay when the device is not on the desk's own network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub public: Option<String>,
}
impl PairingPayload {
    pub fn link(&self) -> Result<String, String> {
        Ok(format!(
            "hotline://pair?p={}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).map_err(|error| error.to_string())?)
        ))
    }

    pub fn from_link(link: &str) -> Result<Self, String> {
        let url = url::Url::parse(link).map_err(|_| "Invalid pairing link.")?;
        if url.scheme() != "hotline" || url.host_str() != Some("pair") {
            return Err("Invalid pairing link.".into());
        }
        let encoded = url
            .query_pairs()
            .find(|(key, _)| key == "p")
            .ok_or("This pairing link has no payload.")?
            .1;
        if encoded.len() > 16384 {
            return Err("Pairing link is too large.".into());
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded.as_bytes())
            .map_err(|_| "Invalid pairing payload.")?;
        serde_json::from_slice(&bytes).map_err(|_| "Invalid pairing payload.".into())
    }
}
