//! Credential-scoped origins for real-provider harnesses. This module is
//! absent from shipping builds; no setting or wire command can redirect a
//! built-in provider. Unique fixture credentials keep parallel tests apart.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

static ORIGINS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

pub struct Origin(String);

impl Origin {
    /// `credential` is an Anthropic fixture key or a ChatGPT login directory.
    /// The guard must outlive every request using it.
    pub fn new(credential: impl Into<String>, origin: &str) -> Self {
        let url = url::Url::parse(origin).expect("fixture origin");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        let credential = credential.into();
        assert!(
            ORIGINS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(credential.clone(), origin.into())
                .is_none(),
            "each fixture owns its credential"
        );
        Self(credential)
    }
}

impl Drop for Origin {
    fn drop(&mut self) {
        ORIGINS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

pub(crate) fn origin(credential: &str) -> Option<String> {
    ORIGINS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(credential)
        .cloned()
}
