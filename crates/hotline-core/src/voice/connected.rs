//! The vault's side of the voice engines, which are `hotline-voice`'s: what
//! the vault keeps that voice reads, as the [`Connected`] the engines take,
//! and the two logins whose tokens the core's providers refresh. The readers
//! here take the vault, as the calls and the settings ask with it.

use crate::providers::{self, chatgpt, xai};
use crate::session::ProviderAuth;
use crate::vault::Vault;
use futures_util::future::BoxFuture;
use hotline_room::voice::settings::VoiceSettings;
use hotline_support::credentials::CredentialFile;
use hotline_voice::realtime::Realtime;
use hotline_voice::realtime::live::Login;
use hotline_voice::speech::{self, Connected, Options, SignIns, SpeechOutput, SpeechSet};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// Every connected provider's models for each speech job ([`speech::options`]).
pub async fn options(vault: &Vault) -> Options {
    speech::options(&connected(vault)).await
}

/// The live models the connected providers can run ([`speech::live_models`]).
pub fn live_models(vault: &Vault) -> Vec<crate::contract::CapabilityLiveModel> {
    speech::live_models(&connected(vault))
}

/// The realtime model that fronts a live call ([`speech::resolve_front`]).
pub fn resolve_front(vault: &Vault, settings: &VoiceSettings) -> Option<Arc<dyn Realtime>> {
    speech::resolve_front(&connected(vault), settings)
}

/// STT, TTS and a fallback TTS from what the owner has connected ([`speech::resolve`]).
pub fn resolve(vault: &Vault, settings: &VoiceSettings) -> Result<SpeechSet, String> {
    speech::resolve(&connected(vault), settings)
}

/// What the desk's own model listens for ([`speech::listens_for`]).
pub fn listens_for(vault: &Vault, settings: &VoiceSettings) -> Vec<String> {
    speech::listens_for(&connected(vault), settings)
}

/// A text call's voice ([`speech::resolve_output`]).
pub fn resolve_output(vault: &Vault, settings: &VoiceSettings) -> Result<SpeechOutput, String> {
    speech::resolve_output(&connected(vault), settings)
}

/// The connected providers in the order the owner connected them, one each:
/// a credential's place is the first one not revoked, and its auth is the
/// vault's for that provider.
pub(crate) fn connected(vault: &Vault) -> Connected {
    let auth = vault.provider_auth();
    let mut seen = HashSet::new();
    let providers = vault
        .list()
        .into_iter()
        .filter(|credential| !credential.revoked)
        .filter_map(|credential| {
            let auth = auth.get(&credential.provider_id)?;
            seen.insert(credential.provider_id.clone()).then(|| {
                (
                    credential.provider_id.clone(),
                    read_key(&credential.provider_id, auth),
                )
            })
        })
        .collect();
    Connected {
        root: vault.root().to_path_buf(),
        providers,
        sign_ins: SignIns {
            xai: xai_bearer,
            chatgpt: chatgpt_login,
        },
    }
}

/// OpenRouter's sign-in is a PKCE exchange that leaves a plain key, which
/// voice uses as a key. A sign-in whose key cannot be read is left as it is,
/// and voice does not count it.
fn read_key(provider_id: &str, auth: &ProviderAuth) -> ProviderAuth {
    match auth {
        ProviderAuth::StoredLogin { tokens } if provider_id == "openrouter" => {
            providers::openrouter_key(tokens).map_or_else(|_| auth.clone(), ProviderAuth::ApiKey)
        }
        _ => auth.clone(),
    }
}

fn xai_bearer<'a>(
    tokens: &'a CredentialFile,
    rejected: Option<&'a str>,
) -> BoxFuture<'a, Result<String, String>> {
    Box::pin(xai::bearer(tokens, rejected))
}

fn chatgpt_login(token_dir: &Path) -> BoxFuture<'_, Result<Login, String>> {
    Box::pin(async move {
        let record = chatgpt::refreshed_auth(token_dir).await?;
        Ok(Login {
            access_token: record.access_token,
            account_id: record.account_id,
        })
    })
}

// Which credentials count, and in what order: the engines' own tests take the
// connections as given, so these read them out of a real vault.
#[cfg(test)]
mod tests {
    use super::*;

    fn vault_in(root: &std::path::Path) -> Vault {
        let log = crate::log::Log::open(root);
        Vault::open_with_store(
            root,
            log,
            Arc::new(hotline_support::credentials::MemoryStore::default()),
        )
        .unwrap()
    }

    #[test]
    fn the_vault_gives_providers_in_the_order_they_were_connected() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("anthropic", "Claude", "sk-ant").unwrap();
        vault.create("groq", "Groq", "gsk-first").unwrap();
        let openai = vault.create("openai", "OpenAI", "sk-second").unwrap();
        vault.create("google", "Gemini", "AIza-third").unwrap();

        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert_eq!(set.stt.id().provider_id, "groq");
        assert_eq!(set.tts.id().provider_id, "groq");
        assert_eq!(
            set.fallback_tts.as_ref().unwrap().id().provider_id,
            "openai"
        );

        // A revoked key is not a connection.
        vault.revoke(&openai.id).unwrap();
        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert_eq!(
            set.fallback_tts.as_ref().unwrap().id().provider_id,
            "google"
        );
    }

    #[test]
    fn a_vault_with_only_chat_providers_cannot_speak() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("anthropic", "Claude", "sk-ant").unwrap();
        assert!(resolve(&vault, &VoiceSettings::default()).is_err());
    }

    #[test]
    fn a_vault_with_native_xai_can_hear_live_and_speak() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault_in(root.path());
        vault.create("xai", "Grok", "xai-key").unwrap();
        let set = resolve(&vault, &VoiceSettings::default()).unwrap();
        assert!(set.stt.supports_live_input());
        assert_eq!(set.tts.id().provider_id, "xai");
    }
}
