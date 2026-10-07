//! The optional API key for each web search provider.
//!
//! A key is kept the way every other secret here is: the value in the OS
//! credential store, a reference to it in `vault/websearch/<provider>.json`.
//! It is not a room setting, so it is never in the room stream, never in a
//! settings snapshot and never on the wire; the window learns only whether
//! one is saved.

use super::*;
use crate::contract::WebSearchProvider;
use crate::websearch;

const MAX_KEY_BYTES: usize = 4096;

impl Vault {
    pub(super) fn web_search_dir(&self) -> PathBuf {
        self.directory().join("websearch")
    }

    fn web_search_path(&self, provider: WebSearchProvider) -> PathBuf {
        self.web_search_dir()
            .join(format!("{}.json", websearch::id(provider)))
    }

    /// Whether a key is saved for `provider`. Never a look into the OS store,
    /// and a link planted where the record goes is not a key.
    pub fn has_web_search_key(&self, provider: WebSearchProvider) -> bool {
        fs::symlink_metadata(self.web_search_path(provider)).is_ok_and(|entry| entry.is_file())
    }

    /// The key saved for `provider`, for the one call that sends it.
    pub fn web_search_key(&self, provider: WebSearchProvider) -> io::Result<Option<String>> {
        self.check_layout()?;
        if !self.has_web_search_key(provider) {
            return Ok(None);
        }
        Ok(self
            .files
            .file(self.web_search_path(provider))
            .read()?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty()))
    }

    /// Every saved key, by provider. One that cannot be read is left out, so a
    /// locked keychain makes web search keyless rather than broken.
    pub fn web_search_keys(&self) -> HashMap<WebSearchProvider, String> {
        websearch::ORDER
            .into_iter()
            .filter_map(|provider| {
                self.web_search_key(provider)
                    .ok()
                    .flatten()
                    .map(|key| (provider, key))
            })
            .collect()
    }

    /// Saves `key` for `provider`, replacing what was there, or forgets it when
    /// `key` is absent or blank.
    pub fn set_web_search_key(
        &self,
        provider: WebSearchProvider,
        key: Option<&str>,
    ) -> io::Result<()> {
        let key = key.map(str::trim).filter(|key| !key.is_empty());
        if let Some(key) = key {
            if key.len() > MAX_KEY_BYTES {
                return Err(io::Error::other("That API key is too long."));
            }
            if key.chars().any(char::is_control) {
                return Err(io::Error::other(
                    "The API key cannot contain control characters.",
                ));
            }
        }
        let _one_writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_layout()?;
        let path = self.web_search_path(provider);
        match key {
            Some(key) => {
                make_private_directory(&self.web_search_dir())?;
                self.files.file(path).write(key.as_bytes())
            }
            None => self.files.file(path).delete(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::tests::MemoryStore;

    fn vault(root: &Path) -> Vault {
        Vault::open_with_store(root, Log::open(root), Arc::new(MemoryStore::default())).unwrap()
    }

    #[test]
    fn a_key_is_saved_read_replaced_and_forgotten() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        assert!(!vault.has_web_search_key(WebSearchProvider::Exa));
        assert_eq!(vault.web_search_key(WebSearchProvider::Exa).unwrap(), None);

        vault
            .set_web_search_key(WebSearchProvider::Exa, Some("  exa-key-1  "))
            .unwrap();
        assert!(vault.has_web_search_key(WebSearchProvider::Exa));
        assert!(!vault.has_web_search_key(WebSearchProvider::Parallel));
        assert_eq!(
            vault.web_search_key(WebSearchProvider::Exa).unwrap(),
            Some("exa-key-1".to_string())
        );

        vault
            .set_web_search_key(WebSearchProvider::Exa, Some("exa-key-2"))
            .unwrap();
        assert_eq!(
            vault.web_search_keys(),
            HashMap::from([(WebSearchProvider::Exa, "exa-key-2".to_string())])
        );

        vault
            .set_web_search_key(WebSearchProvider::Exa, None)
            .unwrap();
        assert!(!vault.has_web_search_key(WebSearchProvider::Exa));
        // Forgetting what is not there is not an error.
        vault
            .set_web_search_key(WebSearchProvider::Exa, Some("  "))
            .unwrap();
    }

    #[test]
    fn a_key_is_never_in_a_room_event_or_a_file_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        vault
            .set_web_search_key(WebSearchProvider::Keenable, Some("sekrit-keenable-value"))
            .unwrap();
        fn walk(path: &Path, found: &mut bool) {
            for entry in fs::read_dir(path).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else if fs::read(&path)
                    .map(|bytes| String::from_utf8_lossy(&bytes).contains("sekrit-keenable-value"))
                    .unwrap_or(false)
                {
                    *found = true;
                }
            }
        }
        let mut found = false;
        walk(root.path(), &mut found);
        assert!(!found, "the key reached a plain file under the data root");
    }

    #[test]
    fn a_key_with_a_control_character_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let vault = vault(root.path());
        assert!(
            vault
                .set_web_search_key(WebSearchProvider::Parallel, Some("a\nb"))
                .is_err()
        );
        assert!(!vault.has_web_search_key(WebSearchProvider::Parallel));
    }
}
