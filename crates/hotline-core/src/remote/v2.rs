//! Pairing is an explicit, single-use capability, never an empty-room bootstrap.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

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
    /// `"relay"` from a desktop pairing only to use this server's relay.
    /// A relay window admits only that, and no other window admits it.
    #[serde(default)]
    purpose: Option<String>,
}
/// What a claim with the right secret comes to.
pub(super) enum Claimed {
    Granted(DeviceRole),
    /// The claim's purpose does not match the window's role: no grant, and
    /// the window stays open. The word is what this invitation is for.
    Refused(&'static str),
}

impl Remote {
    pub(super) fn noise_keys(&self) -> Result<([u8; 32], [u8; 32]), String> {
        // Serialize first-use creation with grant changes. Never replace an
        // unreadable identity: that could make an existing pairing trust a new desk.
        let s = lock(&self.state);
        if let Some(bytes) = self.noise_identity.read().map_err(message)? {
            let keys: Keys = serde_json::from_slice(&bytes).map_err(message)?;
            return Ok((keys.private, keys.public));
        }
        // A desk this one linked with trusts the key too, and would refuse a
        // new one as the link ending.
        let linked = self
            .agents()
            .is_ok_and(|agents| !crate::linked::records(&agents.log).is_empty());
        if linked || s.saved.all_grants().any(|g| g.device.public_key.is_some()) {
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
        lock(&self.state)
            .saved
            .all_grants()
            .map(|g| g.device.clone())
            .collect()
    }

    /// `remote.pairing`, from the wire or a relay-only server's door: start
    /// an invitation, or poll or cancel one by its id. With no role named it
    /// is for the role this server's capabilities make the usual one: owner
    /// where it runs agents, relay where it runs only a relay.
    pub fn pairing(
        &self,
        role: Option<DeviceRole>,
        id: Option<String>,
        cancel: bool,
    ) -> Result<Value, String> {
        let Some(id) = id else {
            if cancel {
                return Err("Name the pairing invitation to cancel.".into());
            }
            let usual = if self.agents.is_some() {
                DeviceRole::Owner
            } else {
                DeviceRole::Relay
            };
            return Ok(json!(self.pairing_v2(role.unwrap_or(usual))?));
        };
        if role.is_some() {
            return Err("A pairing role is chosen when the invitation is created.".into());
        }
        if cancel {
            self.cancel_pairing(&id)?;
            return Ok(Value::Null);
        }
        Ok(json!(self.pairing_result(&id)?))
    }

    pub fn pairing_v2(&self, role: DeviceRole) -> Result<SealedPairing, String> {
        // A desktop's own Remote never hosts a relay, so a relay grant there
        // could do nothing; nor could one on a server that does not relay.
        if role == DeviceRole::Relay && self.served.is_none() {
            return Err("Only a served desk carries a relay.".into());
        }
        if role == DeviceRole::Relay && !self.relays() {
            return Err("This server does not run a relay. Serve it with --capabilities agents,relay to pair one.".into());
        }
        // A relay-only server has no wire, so an owner or companion grant
        // there could do nothing but stand in, which a relay grant does.
        if role != DeviceRole::Relay && self.agents.is_none() {
            return Err("This server runs only a relay, so it pairs a desktop to relay through it and nothing else: phones pair with a server that runs agents.".into());
        }
        let (_, public) = self.noise_keys()?;
        let mut s = lock(&self.state);
        let endpoint = s.endpoints.first().ok_or("Enable remote access first.")?;
        let relay = s.relay.url.clone();
        // A served desk's only address is already its public one.
        let public_address = self
            .public_endpoint_of(&s)
            .filter(|address| address != endpoint);
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
            public: public_address.clone(),
        };
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("v", "2")
            .append_pair("k", &URL_SAFE_NO_PAD.encode(public))
            .append_pair("u", endpoint)
            .append_pair("s", &secret)
            .append_pair(
                "r",
                match role {
                    DeviceRole::Owner => "owner",
                    DeviceRole::Companion => "companion",
                    DeviceRole::Relay => "relay",
                },
            )
            .append_pair("e", &payload.expires_at.to_string())
            .append_pair("n", &payload.name);
        if let Some(address) = &public_address {
            query.append_pair("x", address);
        }
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
        Ok(pairing)
    }

    pub fn pairing_result(&self, id: &str) -> Result<Option<RemoteDevice>, String> {
        let s = lock(&self.state);
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
        let mut s = lock(&self.state);
        if s.sealed_pairing.as_ref().is_some_and(|w| w.id == id) {
            s.sealed_pairing = None;
        }
        Ok(())
    }

    pub(super) fn pairing_open(&self) -> bool {
        let s = lock(&self.state);
        !s.endpoints.is_empty()
            && s.sealed_pairing
                .as_ref()
                .is_some_and(|w| w.expires_at > now() && w.result.is_none())
    }

    pub(super) fn claim_v2(&self, public: &[u8], payload: &[u8]) -> Result<Claimed, String> {
        let claim: Claim = serde_json::from_slice(payload).map_err(|_| "Invalid pairing claim.")?;
        if public.len() != 32 || claim.name.trim().is_empty() || claim.name.len() > 80 {
            return Err("Invalid pairing claim.".into());
        }
        let mut s = lock(&self.state);
        Self::throttle(&mut s)?;
        let window = s.sealed_pairing.as_ref().ok_or("Pairing is closed.")?;
        if s.endpoints.is_empty()
            || window.expires_at <= now()
            || window.result.is_some()
            || !crate::wire::same_secret(&window.secret, &claim.secret)
        {
            return Err("Pairing is closed.".into());
        }
        let role = window.role;
        // The throttle above has already counted this claim.
        let relay_claim = claim.purpose.as_deref() == Some("relay");
        if claim.purpose.is_some() && !relay_claim {
            return Err("Invalid pairing claim.".into());
        }
        match (role, relay_claim) {
            (DeviceRole::Relay, false) => return Ok(Claimed::Refused("relay")),
            (DeviceRole::Owner | DeviceRole::Companion, true) => {
                return Ok(Claimed::Refused("device"));
            }
            _ => {}
        }
        let public_key = URL_SAFE_NO_PAD.encode(public);
        // Re-pairing the same key explicitly replaces its grant, in either
        // list, and revokes its old sockets. A name alone never identifies or
        // replaces a device.
        let old_id = s
            .saved
            .all_grants()
            .find(|g| g.device.public_key.as_deref() == Some(&public_key))
            .map(|g| g.device.id.clone());
        // Inert bearer records remain revocable but must not block their
        // phones from migrating to a sealed grant.
        let keyed_devices = s
            .saved
            .all_grants()
            .filter(|grant| grant.device.public_key.is_some())
            .count();
        if old_id.is_none() && keyed_devices >= 16 {
            return Err("Device limit reached.".into());
        }
        let device = RemoteDevice {
            id: Uuid::new_v4().to_string(),
            name: claim.name.trim().into(),
            paired_at: now(),
            role,
            public_key: Some(public_key),
            current: None,
        };
        let mut saved = s.saved.clone();
        saved
            .grants
            .retain(|g| Some(&g.device.id) != old_id.as_ref());
        saved
            .relay_grants
            .retain(|g| Some(&g.device.id) != old_id.as_ref());
        let grant = Grant {
            device: device.clone(),
            push: None,
        };
        if role == DeviceRole::Relay {
            saved.relay_grants.push(grant);
        } else {
            saved.grants.push(grant);
        }
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
        let window = s
            .sealed_pairing
            .as_mut()
            .expect("the window was checked open above, under this same lock");
        window.result = Some(device);
        window.secret.clear();
        Ok(Claimed::Granted(role))
    }

    pub(super) fn authenticate_v2(self: &Arc<Self>, public: &[u8]) -> Option<Phone> {
        let s = lock(&self.state);
        if s.endpoints.is_empty() || public.len() != 32 {
            return None;
        }
        let public = URL_SAFE_NO_PAD.encode(public);
        let grant = s
            .saved
            .all_grants()
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
