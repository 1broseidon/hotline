//! Its own process: no other test can have chosen a TLS backend first.

use tokio_rustls::rustls::crypto::CryptoProvider;

/// The desktop build compiles both rustls backends in, so WebSocket TLS (Grok
/// live transcription) panics unless a default was chosen. This crate's own
/// test build may compile only one, where rustls would choose by itself, so
/// the desk's choice is checked before anything could make it implicitly.
#[tokio::test]
async fn an_open_desk_has_chosen_the_tls_backend_for_websockets() {
    assert!(CryptoProvider::get_default().is_none());
    let root = tempfile::tempdir().unwrap();
    let _desk = hotline_core::desk::Desk::open(root.path()).unwrap();
    assert!(CryptoProvider::get_default().is_some());
}
