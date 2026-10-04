//! Pairing is an explicit, single-use capability, never an empty-room bootstrap.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

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
}
impl PairingPayload {
    pub fn link(&self) -> Result<String, String> {
        Ok(format!(
            "hotline://pair?p={}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).map_err(message)?)
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

pub(super) struct Window {
    id: String,
    secret: String,
    role: DeviceRole,
    pub(super) expires_at: i64,
    result: Option<RemoteDevice>,
}
#[derive(Serialize, Deserialize)]
struct Keys {
    private: [u8; 32],
    public: [u8; 32],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Claim {
    secret: String,
    name: String,
}

impl Remote {
    pub(super) fn noise_keys(&self) -> Result<([u8; 32], [u8; 32]), String> {
        // Serialize first-use creation with grant changes. Never replace an
        // unreadable identity: that could make an existing pairing trust a new desk.
        let s = self.state.lock().unwrap();
        if let Some(bytes) = self.noise_identity.read().map_err(message)? {
            let keys: Keys = serde_json::from_slice(&bytes).map_err(message)?;
            return Ok((keys.private, keys.public));
        }
        if s.saved.grants.iter().any(|g| g.device.public_key.is_some()) {
            return Err(
                "The sealed identity is missing; restore its secret store before enabling Remote."
                    .into(),
            );
        }
        let (private, public) = sealed::keypair()?;
        self.noise_identity
            .write(&serde_json::to_vec(&Keys { private, public }).map_err(message)?)
            .map_err(message)?;
        Ok((private, public))
    }

    pub fn devices(&self) -> Vec<RemoteDevice> {
        self.state
            .lock()
            .unwrap()
            .saved
            .grants
            .iter()
            .map(|g| g.device.clone())
            .collect()
    }

    pub fn pairing_v2(&self, role: DeviceRole) -> Result<SealedPairing, String> {
        let (_, public) = self.noise_keys()?;
        let mut s = self.state.lock().unwrap();
        let endpoint = s.endpoints.first().ok_or("Enable remote access first.")?;
        let relay = s.relay.url.clone();
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(message)?;
        let secret = URL_SAFE_NO_PAD.encode(secret);
        let payload = PairingPayload {
            version: 2,
            url: endpoint.clone(),
            desk_key: URL_SAFE_NO_PAD.encode(public),
            secret: secret.clone(),
            role,
            expires_at: now() + 120_000,
            name: desktop_name(),
            relay: relay.clone(),
        };
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("v", "2")
            .append_pair("k", &URL_SAFE_NO_PAD.encode(public))
            .append_pair("u", endpoint)
            .append_pair("s", &secret)
            .append_pair(
                "r",
                if role == DeviceRole::Owner {
                    "owner"
                } else {
                    "companion"
                },
            )
            .append_pair("e", &payload.expires_at.to_string())
            .append_pair("n", &payload.name);
        if let Some(relay) = &relay {
            query.append_pair("a", relay);
        }
        let query = query.finish();
        let url = format!("hotline://pair?{query}");
        let code = qrcode::QrCode::new(url.as_bytes()).map_err(message)?;
        let pairing = SealedPairing {
            link: payload.link()?,
            expires_at: payload.expires_at,
            payload,
            id: Uuid::new_v4().to_string(),
            url,
            qr_svg: code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(280, 280)
                .build(),
        };
        s.sealed_pairing = Some(Window {
            id: pairing.id.clone(),
            secret,
            role,
            expires_at: pairing.expires_at,
            result: None,
        });
        s.invitation = None;
        s.manual = None;
        Ok(pairing)
    }

    pub fn pairing_result(&self, id: &str) -> Result<Option<RemoteDevice>, String> {
        let s = self.state.lock().unwrap();
        let window = s
            .sealed_pairing
            .as_ref()
            .filter(|w| w.id == id)
            .ok_or("That pairing window is closed.")?;
        if let Some(device) = &window.result {
            return Ok(Some(device.clone()));
        }
        if window.expires_at <= now() {
            return Err("That pairing window expired.".into());
        }
        Ok(None)
    }

    pub fn cancel_pairing(&self, id: &str) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if s.sealed_pairing.as_ref().is_some_and(|w| w.id == id) {
            s.sealed_pairing = None;
        }
        Ok(())
    }

    pub(super) fn pairing_open(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.endpoints.is_empty()
            && s.sealed_pairing
                .as_ref()
                .is_some_and(|w| w.expires_at > now() && w.result.is_none())
    }

    pub(super) fn claim_v2(&self, public: &[u8], payload: &[u8]) -> Result<DeviceRole, String> {
        let claim: Claim = serde_json::from_slice(payload).map_err(|_| "Invalid pairing claim.")?;
        if public.len() != 32 || claim.name.trim().is_empty() || claim.name.len() > 80 {
            return Err("Invalid pairing claim.".into());
        }
        let mut s = self.state.lock().unwrap();
        Self::throttle(&mut s)?;
        let window = s.sealed_pairing.as_ref().ok_or("Pairing is closed.")?;
        if s.endpoints.is_empty()
            || window.expires_at <= now()
            || window.result.is_some()
            || !crate::wire::same_secret(&window.secret, &claim.secret)
        {
            return Err("Pairing is closed.".into());
        }
        let public_key = URL_SAFE_NO_PAD.encode(public);
        // Re-pairing the same key explicitly replaces its grant and revokes
        // its old sockets. A name alone never identifies or replaces a device.
        let old_id = s
            .saved
            .grants
            .iter()
            .find(|g| g.device.public_key.as_deref() == Some(&public_key))
            .map(|g| g.device.id.clone());
        if old_id.is_none() && s.saved.grants.len() >= 16 {
            return Err("Device limit reached.".into());
        }
        let role = window.role;
        let device = RemoteDevice {
            id: Uuid::new_v4().to_string(),
            name: claim.name.trim().into(),
            paired_at: now(),
            role,
            public_key: Some(public_key),
        };
        let mut saved = s.saved.clone();
        saved
            .grants
            .retain(|g| Some(&g.device.id) != old_id.as_ref());
        saved.grants.push(Grant {
            device: device.clone(),
            token_hash: String::new(),
            push: None,
        });
        self.save(&saved)?;
        s.saved = saved;
        if let Some(id) = old_id
            && let Some(cancel) = s.devices.remove(&id)
        {
            cancel.cancel();
        }
        let cancel = s.cancel.child_token();
        s.devices.insert(device.id.clone(), cancel);
        // Persistence and consumption share this lock. A concurrent claimant
        // can never observe a successful grant without a consumed invitation.
        let window = s.sealed_pairing.as_mut().unwrap();
        window.result = Some(device);
        window.secret.clear();
        Ok(role)
    }

    pub(super) fn authenticate_v2(self: &Arc<Self>, public: &[u8]) -> Option<Phone> {
        let s = self.state.lock().unwrap();
        if s.endpoints.is_empty() || public.len() != 32 {
            return None;
        }
        let public = URL_SAFE_NO_PAD.encode(public);
        let grant = s
            .saved
            .grants
            .iter()
            .find(|g| g.device.public_key.as_deref() == Some(&public))?;
        let cancel = s.devices.get(&grant.device.id)?.child_token();
        if cancel.is_cancelled() {
            return None;
        }
        Some(Phone {
            remote: self.clone(),
            id: grant.device.id.clone(),
            role: grant.device.role,
            cancel,
        })
    }
}
