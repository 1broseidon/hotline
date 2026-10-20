//! The process's TLS default, which a WebSocket to a provider needs.

/// Both rustls backends are compiled in (reqwest asks for aws-lc-rs, other
/// dependencies bring ring), so rustls cannot choose a process default on its
/// own. reqwest names its provider, but WebSocket TLS such as Grok's live
/// transcription asks for the default and panics without one. The desk picks
/// aws-lc-rs, the one its HTTP already uses, unless something already chose.
pub fn install_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
}
