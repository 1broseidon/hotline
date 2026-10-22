//! Discovery reads host columns on a private copy. Only an accepted offer
//! calls `Source::read`, which may launch a browser and unlock its keychain.
use super::*;
use std::collections::HashSet;

#[derive(Clone)]
pub(crate) struct Source {
    candidate: Candidate,
    profile: BrowserProfile,
}

impl Source {
    pub(crate) fn browser_name(&self) -> &str {
        self.candidate.name
    }
    pub(crate) fn profile_name(&self) -> &str {
        &self.profile.name
    }
    pub(crate) fn login(&self, id: &str) -> String {
        format!(
            "lent-{}-{id}",
            super::super::login::name_for(self.candidate.id, &self.profile.id)
                .trim_start_matches("import-")
        )
    }

    pub(crate) fn sites(&self) -> Result<HashSet<String>, String> {
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
        // WAL contains recent cookies too. SQLite reads the copy, never takes
        // a lock on the live profile, and never selects a value column.
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

    pub(crate) async fn read(&self, site: &str) -> Result<Vec<Value>, String> {
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

pub(crate) fn inventory(candidates: Vec<Candidate>) -> Inventory {
    let discovery = discover(candidates);
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

pub(crate) fn discover(mut candidates: Vec<Candidate>) -> Discovery {
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
            discovery
                .sources
                .extend(browser.profiles.into_iter().map(move |profile| Source {
                    candidate: candidate.clone(),
                    profile,
                }));
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
