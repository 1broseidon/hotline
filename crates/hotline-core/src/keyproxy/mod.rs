//! The desk shell's key proxy: how a confined teammate's commands use a
//! protected key without ever holding it.
//!
//! A key granted to a teammate's desk shell protected is not put in a
//! command's environment. Each run gets a *stand-in* for it instead, and a
//! proxy user and password of its own in `HTTPS_PROXY`. The proxy:
//!
//! - answers an authenticated `CONNECT` only, and tunnels every host byte
//!   for byte unless the target is an origin of one of the run's teammate's
//!   keys;
//! - for such an origin, terminates TLS with a leaf from a CA made when the
//!   proxy started, whose key lives in this process alone;
//! - puts the real value in place of the run's own stand-in only in the
//!   key's header and only under one of its schemes (`Basic` is decoded,
//!   swapped and re-encoded), never in a body, a path, a query or another
//!   header; another run's stand-in, another teammate's included, is only a
//!   string;
//! - opens the upstream itself and adds the value only after the upstream
//!   proves the origin's name against the roots compiled into this binary;
//! - never follows a redirect: the command gets the 3xx and decides;
//! - writes the stand-in back over the literal value in every reply it
//!   carries from a terminated origin, headers and body;
//! - refuses, before anything is sent, a request carrying a value to one of
//!   the key's key-minting routes (`policy`).
//!
//! `deliver` replaces a teammate's keys, and a stand-in of a key it no
//! longer holds dies at once, in runs still going too. `end` kills a run's
//! user and stand-ins.
//!
//! This is the computer's proxy (hotline-computer `src/keyproxy`) with
//! teammates and run users added; the two copies are to become one crate.

mod ca;
mod conn;
mod policy;
mod scrub;
mod table;
#[cfg(test)]
pub(crate) mod tests;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};

pub use table::{Binding, RunAccess};

/// How a protected variable is used, as the desk sends it inside `use`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
    /// `https://host[:port]`, the only places the value is sent.
    pub origins: Vec<String>,
    /// The one request header the value goes in.
    #[serde(default = "authorization")]
    pub header: String,
    /// The schemes the header may carry it under (`Bearer`, `token`,
    /// `Basic`). None means the header's whole value is the key.
    #[serde(default)]
    pub schemes: Vec<String>,
    /// The stand-in's shape, so a CLI that checks a token's prefix still runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stand_in: Option<StandIn>,
    /// Key-minting routes on these origins the value is not sent to, from
    /// the desk's `services.json`, refused on top of the built-in list for
    /// the services Hotline names (`policy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuse: Option<Vec<String>>,
}

fn authorization() -> String {
    "Authorization".to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StandIn {
    pub prefix: String,
    /// The whole stand-in's length, prefix included (`ghp_` + 36 is 40).
    pub length: usize,
}

/// The proxy, its CA and its table. Cloning shares them.
#[derive(Clone)]
pub struct KeyProxy {
    inner: Arc<Inner>,
}

struct Inner {
    ca: ca::Ca,
    table: Mutex<table::Table>,
    upstream: Upstream,
}

/// Where the proxy goes on a job's behalf, and what it trusts there.
struct Upstream {
    /// Verifies the upstream's name against `roots` and nothing else.
    tls: Arc<ClientConfig>,
    /// Tests send every host to a local fake upstream. Release builds have
    /// no such field: a bound origin is always the real name on the network.
    #[cfg(test)]
    route: Option<SocketAddr>,
}

impl Upstream {
    fn address(&self, host: &str, port: u16) -> Address {
        #[cfg(test)]
        if let Some(route) = self.route {
            return Address::Fixed(route);
        }
        Address::Name(host.to_owned(), port)
    }
}

enum Address {
    Name(String, u16),
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "only a test routes every host to its fake service"
        )
    )]
    Fixed(SocketAddr),
}

impl KeyProxy {
    /// A proxy with a fresh CA, trusting only the compiled-in web roots
    /// upstream.
    pub fn new() -> Result<Self, String> {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        Self::with_roots(roots)
    }

    fn with_roots(roots: RootCertStore) -> Result<Self, String> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut tls = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            inner: Arc::new(Inner {
                ca: ca::Ca::new(provider)?,
                table: Mutex::new(table::Table::default()),
                upstream: Upstream {
                    tls: Arc::new(tls),
                    #[cfg(test)]
                    route: None,
                },
            }),
        })
    }

    /// A proxy that trusts `roots` upstream and sends every host to
    /// `route`, a test's fake service.
    #[cfg(test)]
    pub(crate) fn for_test(roots: RootCertStore, route: SocketAddr) -> Self {
        let mut proxy = Self::with_roots(roots).expect("proxy");
        Arc::get_mut(&mut proxy.inner)
            .expect("unshared")
            .upstream
            .route = Some(route);
        proxy
    }

    /// This proxy's CA certificate, for the bundle a run trusts.
    pub fn ca_pem(&self) -> &str {
        self.inner.ca.pem()
    }

    /// Replaces `teammate`'s protected keys; none is how they are revoked.
    /// All or nothing. A stand-in of a key it no longer holds dies here.
    pub fn deliver(&self, teammate: &str, bindings: Vec<Binding>) -> Result<(), String> {
        self.table().deliver(teammate, bindings)
    }

    /// Whether `teammate` holds a protected key here.
    pub fn holds(&self, teammate: &str) -> bool {
        self.table().holds(teammate)
    }

    /// A new run of `teammate`'s: its proxy user and password, and a
    /// stand-in for each of its keys.
    pub fn begin(&self, teammate: &str) -> RunAccess {
        self.table().begin(teammate)
    }

    /// Ends the run whose proxy user is `user`.
    pub fn end(&self, user: &str) {
        self.table().end(user);
    }

    /// Serves runs on `listener` until the process ends.
    pub async fn serve(self, listener: std::net::TcpListener) -> std::io::Result<()> {
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;
        conn::serve(self.inner, listener).await
    }

    fn table(&self) -> std::sync::MutexGuard<'_, table::Table> {
        self.inner
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
