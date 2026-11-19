//! The CA made at boot, and the leaves it issues for bound hosts.
//!
//! The key is made in memory and never written anywhere; a new boot is a
//! new CA. The certificates are what strict verifiers ask for (Python 3.13
//! refuses a leaf without an Authority Key Identifier): a critical
//! `CA:TRUE, pathlen:0` with `keyCertSign` on the CA, and on a leaf the
//! name as its SAN, `serverAuth`, a critical `CA:FALSE` and
//! `digitalSignature`, with subject and authority key identifiers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Datelike, Utc};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, SerialNumber,
};
use ring::rand::{SecureRandom, SystemRandom};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::CryptoProvider;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const CA_DAYS: i64 = 2 * 365;
const LEAF_DAYS: i64 = 90;
/// A leaf is issued again well before it runs out.
const LEAF_REISSUE: Duration = Duration::from_hours(30 * 24);
const MAX_LEAVES: usize = 256;

pub(super) struct Ca {
    issuer: Issuer<'static, KeyPair>,
    pem: String,
    provider: Arc<CryptoProvider>,
    leaves: Mutex<HashMap<String, (Instant, Arc<ServerConfig>)>>,
}

impl Ca {
    pub fn new(provider: Arc<CryptoProvider>) -> Result<Self, String> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::OrganizationName, "Hotline Computer");
        name.push(DnType::CommonName, "Hotline Computer key proxy (this boot)");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.use_authority_key_identifier_extension = true;
        params.serial_number = Some(serial());
        set_validity(&mut params, CA_DAYS);
        let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
        Ok(Self {
            pem: cert.pem(),
            issuer: Issuer::new(params, key),
            provider,
            leaves: Mutex::new(HashMap::new()),
        })
    }

    pub fn pem(&self) -> &str {
        &self.pem
    }

    /// The TLS server side for `host`, with a leaf for it.
    pub fn server_config(&self, host: &str) -> Result<Arc<ServerConfig>, String> {
        let mut leaves = self
            .leaves
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((issued, config)) = leaves.get(host)
            && issued.elapsed() < LEAF_REISSUE
        {
            return Ok(Arc::clone(config));
        }
        let (chain, key) = self.leaf(host)?;
        let mut config = ServerConfig::builder_with_provider(Arc::clone(&self.provider))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(|e| e.to_string())?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        if leaves.len() >= MAX_LEAVES {
            leaves.clear();
        }
        leaves.insert(host.to_owned(), (Instant::now(), Arc::clone(&config)));
        Ok(config)
    }

    fn leaf(
        &self,
        host: &str,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
        let mut params =
            CertificateParams::new(vec![host.to_owned()]).map_err(|e| e.to_string())?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.serial_number = Some(serial());
        set_validity(&mut params, LEAF_DAYS);
        let cert = params
            .signed_by(&key, &self.issuer)
            .map_err(|e| e.to_string())?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        Ok((vec![cert.der().clone()], key))
    }

    #[cfg(test)]
    pub fn leaf_der(&self, host: &str) -> Vec<u8> {
        self.leaf(host).expect("leaf").0[0].to_vec()
    }
}

/// From yesterday, so a clock a little behind still accepts it.
fn set_validity(params: &mut CertificateParams, days: i64) {
    let ymd = |at: chrono::DateTime<Utc>| {
        rcgen::date_time_ymd(at.year(), at.month() as u8, at.day() as u8)
    };
    let now = Utc::now();
    params.not_before = ymd(now - chrono::Duration::days(1));
    params.not_after = ymd(now + chrono::Duration::days(days));
}

fn serial() -> SerialNumber {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system randomness");
    // Positive, and never a leading zero byte to trim.
    bytes[0] = (bytes[0] & 0x7f) | 0x40;
    SerialNumber::from_slice(&bytes)
}
