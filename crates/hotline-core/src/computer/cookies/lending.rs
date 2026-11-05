//! Discovery reads host columns on a private copy. Only an accepted offer
//! or a lend calls `Source::read`, which may launch a browser and unlock its
//! keychain. A browser profile the person switched off in Settings › Secrets
//! is neither discovered nor read: the [`Gate`] is checked here, where the
//! stores are opened, so no caller can read past it.
use super::*;
use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

/// The browser profiles the person switched off as cookie sources, by
/// [`source_key`]. One live set per room, shared by every [`Source`] it
/// discovers, so a profile switched off after a card was raised or a lend
/// was made is not read for it either.
#[derive(Clone, Default)]
pub(crate) struct Gate(Arc<RwLock<HashSet<String>>>);

impl Gate {
    pub(crate) fn allows(&self, browser_id: &str, profile_id: &str) -> bool {
        !self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&source_key(browser_id, profile_id))
    }

    pub(crate) fn replace(&self, off: HashSet<String>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = off;
    }
}

/// How the gate and the room's record name one browser profile.
pub(crate) fn source_key(browser_id: &str, profile_id: &str) -> String {
    format!("{browser_id}/{profile_id}")
}

/// What reading a switched-off source answers, whoever asked.
pub(crate) const SWITCHED_OFF: &str =
    "This browser is switched off as a cookie source in Settings › Secrets.";

#[derive(Clone)]
pub(crate) struct Source {
    candidate: Candidate,
    profile: BrowserProfile,
    gate: Gate,
}

impl Source {
    pub(crate) fn browser_id(&self) -> &str {
        self.candidate.id
    }
    pub(crate) fn browser_name(&self) -> &str {
        self.candidate.name
    }
    pub(crate) fn profile_id(&self) -> &str {
        &self.profile.id
    }
    pub(crate) fn profile_name(&self) -> &str {
        &self.profile.name
    }

    fn open(&self) -> Result<(), String> {
        if self.gate.allows(self.candidate.id, &self.profile.id) {
            Ok(())
        } else {
            Err(SWITCHED_OFF.into())
        }
    }
    pub(crate) fn login(&self, id: &str) -> String {
        format!(
            "lent-{}-{id}",
            super::super::login::name_for(self.candidate.id, &self.profile.id)
                .trim_start_matches("import-")
        )
    }

    pub(crate) fn sites(&self) -> Result<HashSet<String>, String> {
        self.open()?;
        let (_copy, connection) = self.store_copy()?;
        let query = match self.candidate.family {
            Family::Chromium => "SELECT DISTINCT host_key FROM cookies",
            Family::Firefox => "SELECT DISTINCT host FROM moz_cookies",
        };
        let mut query = connection
            .prepare(query)
            .map_err(|_| "Could not read cookie sites.")?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| "Could not read cookie sites.")?;
        let mut sites = HashSet::new();
        for host in rows {
            let host = host.map_err(|_| "Could not read cookie sites.")?;
            if let Some(site) = registrable(&host) {
                sites.insert(site);
            }
        }
        Ok(sites)
    }

    /// What one site's cookies in this profile are now, as a hash that
    /// changes when any of them is set, refreshed, expires or goes: their
    /// hosts, names, paths, expiry and stored value (Chromium's encrypted,
    /// so it is never decrypted to tell). Keep fresh compares it to decide
    /// whether to copy the site again, which launching a browser to read
    /// every minute would make too dear. Only that site's rows are selected.
    pub(crate) fn fingerprint(&self, site: &str) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        self.open()?;
        let (_copy, connection) = self.store_copy()?;
        let query = match self.candidate.family {
            Family::Chromium => {
                "SELECT host_key, name, path, CAST(expires_utc AS TEXT), hex(value), hex(encrypted_value) FROM cookies WHERE host_key = ?1 OR host_key LIKE '%.' || ?1"
            }
            Family::Firefox => {
                "SELECT host, name, path, CAST(expiry AS TEXT), hex(value), '' FROM moz_cookies WHERE host = ?1 OR host LIKE '%.' || ?1"
            }
        };
        let mut query = connection
            .prepare(query)
            .map_err(|_| "Could not read the site's cookies.")?;
        let rows = query
            .query_map([site], |row| {
                (0..6)
                    .map(|column| row.get::<_, Option<String>>(column))
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|_| "Could not read the site's cookies.")?;
        let mut rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Could not read the site's cookies.")?;
        rows.sort();
        let mut hash = Sha256::new();
        for row in rows {
            for column in row {
                hash.update(column.unwrap_or_default().as_bytes());
                hash.update([0]);
            }
            hash.update([1]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    /// The profile's cookie store copied aside, with its write-ahead log,
    /// and opened read-only: SQLite never takes a lock on the live profile.
    fn store_copy(&self) -> Result<(tempfile::TempDir, rusqlite::Connection), String> {
        let store = match self.candidate.family {
            Family::Chromium => chromium_cookie_store(&self.candidate.root, &self.profile.id),
            Family::Firefox => Some(
                self.candidate
                    .root
                    .join(&self.profile.id)
                    .join("cookies.sqlite"),
            ),
        }
        .ok_or("No cookie store.")?;
        let copy = tempfile::tempdir().map_err(|_| "Could not stage cookie sites.")?;
        let dest = copy.path().join("Cookies");
        std::fs::copy(&store, &dest).map_err(|_| "Could not copy cookie sites.")?;
        // WAL contains recent cookies too.
        let wal = store.with_file_name(format!(
            "{}-wal",
            store.file_name().unwrap_or_default().to_string_lossy()
        ));
        if wal.exists() {
            std::fs::copy(wal, copy.path().join("Cookies-wal"))
                .map_err(|_| "Could not copy cookie sites.")?;
        }
        let connection = rusqlite::Connection::open_with_flags(
            &dest,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| "Could not read cookie sites.")?;
        Ok((copy, connection))
    }

    pub(crate) async fn read(&self, site: &str) -> Result<Vec<Value>, String> {
        self.open()?;
        let all = match self.candidate.family {
            Family::Chromium => read_chromium(&self.candidate, &self.profile.id).await,
            Family::Firefox => read_firefox(&self.candidate, &self.profile.id),
        }.map_err(|_| "Could not read this browser's cookies. Check its access on the desk, then try again.")?;
        Ok(for_site(unexpired(all, unix_now()), site))
    }
}

pub(crate) struct Discovery {
    pub(crate) sources: Vec<Source>,
    pub(crate) refused: Vec<HostBrowser>,
}

pub(crate) struct Inventory {
    pub(crate) sources: Vec<(Source, HashSet<String>)>,
    pub(crate) refused: Vec<HostBrowser>,
    pub(crate) unavailable: Vec<String>,
}

pub(crate) fn inventory(candidates: Vec<Candidate>, gate: &Gate) -> Inventory {
    let discovery = discover(candidates, gate);
    let mut inventory = Inventory {
        sources: Vec::new(),
        refused: discovery.refused,
        unavailable: Vec::new(),
    };
    for source in discovery.sources {
        match source.sites() {
            Ok(sites) => inventory.sources.push((source, sites)),
            Err(_) => inventory.unavailable.push(format!(
                "Could not read cookie sites from {} · {}.",
                source.browser_name(),
                source.profile_name()
            )),
        }
    }
    inventory
}

/// The readable profiles of the browsers found, but for those switched off,
/// and the browsers the host refuses the desk.
pub(crate) fn discover(mut candidates: Vec<Candidate>, gate: &Gate) -> Discovery {
    // Deterministic first choice; one card per site even with several profiles.
    candidates.sort_by_key(|c| c.id);
    let mut discovery = Discovery {
        sources: Vec::new(),
        refused: Vec::new(),
    };
    for candidate in candidates {
        let Some(browser) = candidate.listed() else {
            continue;
        };
        if browser.blocked && candidate.first_binary().is_some() {
            discovery.refused.push(browser);
        } else {
            discovery.sources.extend(
                browser
                    .profiles
                    .into_iter()
                    .filter(|profile| gate.allows(candidate.id, &profile.id))
                    .map(|profile| Source {
                        candidate: candidate.clone(),
                        profile,
                        gate: gate.clone(),
                    }),
            );
        }
    }
    discovery
}

pub(crate) fn registrable(host: &str) -> Option<String> {
    let host = host.trim_start_matches('.').trim_end_matches('.');
    let host = url::Host::parse(host).ok()?;
    let url::Host::Domain(host) = host else {
        return None;
    };
    psl::domain_str(&host).map(str::to_owned)
}

pub(crate) fn site_for_origin(origin: &str) -> Option<String> {
    let url = url::Url::parse(origin).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    registrable(url.host_str()?)
}

fn for_site(cookies: Vec<Value>, site: &str) -> Vec<Value> {
    cookies
        .into_iter()
        .filter(|c| c["domain"].as_str().and_then(registrable).as_deref() == Some(site))
        .collect()
}

/// Older computers have no pages door and simply produce no offers.
pub(crate) async fn pages(ready: &crate::computer::Ready) -> Result<Vec<String>, ()> {
    #[derive(serde::Deserialize)]
    struct Pages {
        pages: Vec<Page>,
    }
    #[derive(serde::Deserialize)]
    struct Page {
        origin: String,
    }
    let base = ready.url.strip_suffix("/mcp").unwrap_or(&ready.url);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| ())?;
    let response = client
        .get(format!("{base}/pages"))
        .bearer_auth(&ready.token)
        .send()
        .await
        .map_err(|_| ())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let pages = response
        .error_for_status()
        .map_err(|_| ())?
        .json::<Pages>()
        .await
        .map_err(|_| ())?;
    Ok(pages
        .pages
        .into_iter()
        .take(256)
        .filter_map(|p| site_for_origin(&p.origin))
        .collect())
}

#[cfg(test)]
pub(crate) mod tests;
