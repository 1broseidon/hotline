//! A served desk retries exactly the operator's named socket, never a fallback.
use super::*;
use std::{net::SocketAddr, time::Duration};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        self,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
};

#[derive(Clone)]
pub struct ServeOptions {
    pub listen: SocketAddr,
    pub public_url: String,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}
impl ServeOptions {
    pub fn validate(&self) -> Result<(), String> {
        let ip = self.listen.ip();
        if ip.is_unspecified() || ip.is_multicast() || self.listen.port() == 0 {
            return Err("Remote needs one explicit IP address and a nonzero port; wildcard binds are refused.".into());
        }
        if let SocketAddr::V6(address) = self.listen
            && (address.scope_id() != 0 || address.ip().to_ipv4_mapped().is_some())
        {
            return Err("Scoped and IPv4-mapped listen addresses are not supported.".into());
        }
        let url = url::Url::parse(&self.public_url).map_err(message)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "The public URL must be HTTPS, without credentials, query or fragment.".into(),
            );
        }
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            return Err(
                "Supply both a TLS certificate and its key, or neither for self-signed TLS.".into(),
            );
        }
        Ok(())
    }
}
impl Remote {
    pub fn open_served_with_store(
        root: &Path,
        log: Log,
        room: Arc<dyn RoomHandle>,
        store: Arc<dyn SecretStore>,
        mut options: ServeOptions,
    ) -> io::Result<Arc<Self>> {
        options.validate().map_err(io::Error::other)?;
        options.public_url = options.public_url.trim_end_matches('/').into();
        Self::open_options(root, log, room, store, Some(options))
    }

    pub(super) async fn restore_served(self: &Arc<Self>) {
        let host = self.served.as_ref().unwrap().listen.to_string();
        if let Err(error) = self.configure_served(true, &host).await {
            self.state.lock().unwrap().error = Some(error);
        }
    }

    fn served_tls(&self, options: &ServeOptions) -> Result<TlsAcceptor, String> {
        if let (Some(cert), Some(key)) = (&options.tls_cert, &options.tls_key) {
            let certificates = CertificateDer::pem_slice_iter(&fs::read(cert).map_err(message)?)
                .collect::<Result<Vec<_>, _>>()
                .map_err(message)?;
            let key =
                PrivateKeyDer::from_pem_slice(&fs::read(key).map_err(message)?).map_err(message)?;
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(message)?
            .with_no_client_auth()
            .with_single_cert(certificates, key)
            .map_err(message)?;
            return Ok(TlsAcceptor::from(Arc::new(config)));
        }
        let url = url::Url::parse(&options.public_url).map_err(message)?;
        let host = url
            .host_str()
            .ok_or("The public URL needs a host.")?
            .trim_matches(['[', ']'])
            .to_string();
        let hosts = vec![host, options.listen.ip().to_string()];
        let old = self
            .identity
            .read()
            .map_err(message)?
            .and_then(|bytes| serde_json::from_slice::<Identity>(&bytes).ok());
        let identity = match old {
            Some(identity) if hosts.iter().all(|h| identity.hosts.contains(h)) => identity,
            _ => {
                let key = rcgen::KeyPair::generate().map_err(message)?;
                let mut params = rcgen::CertificateParams::new(hosts.clone()).map_err(message)?;
                params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
                params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
                let certificate = params.self_signed(&key).map_err(message)?;
                let identity = Identity {
                    hosts,
                    certificate: certificate.der().to_vec(),
                    key: key.serialize_der(),
                };
                self.identity
                    .write(&serde_json::to_vec(&identity).map_err(message)?)
                    .map_err(message)?;
                identity
            }
        };
        server::tls(&identity)
    }

    pub(super) async fn configure_served(
        self: &Arc<Self>,
        enabled: bool,
        host: &str,
    ) -> Result<RemoteStatus, String> {
        let options = self.served.as_ref().unwrap().clone();
        if host != options.listen.to_string() {
            return Err("The served listen address is fixed by its startup options.".into());
        }
        let _held = self.lifecycle.lock().await;
        {
            let mut s = self.state.lock().unwrap();
            if enabled && s.saved.enabled && !s.cancel.is_cancelled() {
                drop(s);
                return Ok(self.status());
            }
            s.cancel.cancel();
            s.endpoints.clear();
            s.devices.clear();
            s.invitation = None;
            s.manual = None;
            s.sealed_pairing = None;
            s.saved.enabled = false;
            s.error = None;
            self.save(&s.saved)?;
        }
        if let Some(task) = self.server.lock().await.take() {
            let _ = task.await;
        }
        if !enabled {
            return Ok(self.status());
        }
        self.noise_keys()?;
        let tls = self.served_tls(&options)?;
        let cancel = CancellationToken::new();
        {
            let mut s = self.state.lock().unwrap();
            s.saved.enabled = true;
            self.save(&s.saved)?;
            s.cancel = cancel.clone();
            s.error = Some(format!("Waiting for {}.", options.listen));
        }
        let remote = self.clone();
        *self.server.lock().await = Some(tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    result = tokio::net::TcpListener::bind(options.listen) => result,
                };
                match result {
                    Ok(listener) => {
                        {
                            let mut s = remote.state.lock().unwrap();
                            if cancel.is_cancelled() {
                                return;
                            }
                            s.endpoints = vec![options.public_url.clone()];
                            s.error = None;
                            for grant in s.saved.grants.clone() {
                                s.devices.insert(grant.device.id, cancel.child_token());
                            }
                        }
                        server::run(remote, listener, tls, cancel).await;
                        return;
                    }
                    Err(error) => {
                        remote.state.lock().unwrap().error =
                            Some(format!("Waiting for {}: {error}", options.listen))
                    }
                }
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
            }
        }));
        Ok(self.status())
    }

    pub(super) fn route_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        match &self.served {
            Some(options) => {
                let url = url::Url::parse(&options.public_url).ok()?;
                path.strip_prefix(url.path().trim_end_matches('/'))
            }
            None => Some(path),
        }
    }
}
