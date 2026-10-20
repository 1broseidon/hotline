//! URL identity for merging results: ketch's `canonical.go`, plus the host
//! helpers the ranking rules share.

use url::Url;

/// Query parameters that are pure click or campaign attribution. Anything not
/// here survives, because a query parameter often selects the page
/// (`?v=`, `?id=`, `?page=2`): under-merging costs one duplicate row,
/// over-merging silently deletes a distinct result.
const TRACKING_PARAMS: [&str; 14] = [
    "fbclid", "gclid", "gbraid", "wbraid", "dclid", "msclkid", "yclid", "twclid", "igshid",
    "mc_cid", "mc_eid", "_hsenc", "_hsmi", "mkt_tok",
];

fn is_tracking(name: &str) -> bool {
    name.starts_with("utm_") || TRACKING_PARAMS.contains(&name)
}

/// A stable merge key: lowercase scheme and host, http folded to https,
/// default port, leading `www.`, fragment and tracking parameters dropped,
/// parameters sorted, one trailing slash stripped. Unparseable or hostless
/// input comes back as it was. Used only for deduplication: the URL shown is
/// always one a provider returned.
pub fn canonical_url(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return raw.to_string();
    };
    let Some(host) = url.host_str() else {
        return raw.to_string();
    };
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let port = match url.port() {
        Some(port) if port != 80 && port != 443 => format!(":{port}"),
        _ => String::new(),
    };
    let path = match url.path() {
        "" | "/" => "/".to_string(),
        path => {
            let trimmed = path.strip_suffix('/').unwrap_or(path);
            if trimmed.is_empty() {
                "/".into()
            } else {
                trimmed.to_string()
            }
        }
    };
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| !is_tracking(name))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    pairs.sort();
    let mut key = format!("https://{host}{port}{path}");
    if !pairs.is_empty() {
        let query: Vec<String> = pairs
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        key.push('?');
        key.push_str(&query.join("&"));
    }
    key
}

/// The host of a URL, lowercase, without a port.
pub fn host_of(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(str::to_lowercase))
}

/// The registrable domain of a host, without a public suffix list: the last
/// two labels, or three when the second to last is a generic second level
/// under a two-letter country code (`bbc.co.uk`).
pub fn registrable(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return host.to_string();
    }
    let generic = ["co", "com", "org", "net", "gov", "ac", "edu"];
    let take = if labels[n - 1].len() == 2 && generic.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    labels[n - take.min(n)..].join(".")
}

/// The registrable domain's first label: `apple` for `www.apple.com`.
pub fn brand_label(host: &str) -> String {
    registrable(host)
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_spellings_of_one_page_share_a_key_and_two_pages_do_not() {
        let key =
            canonical_url("http://WWW.Example.com:80/a/b/?utm_source=x&b=2&a=1&fbclid=z#frag");
        assert_eq!(key, "https://example.com/a/b?a=1&b=2");
        assert_eq!(canonical_url("https://example.com"), "https://example.com/");
        assert_eq!(
            canonical_url("https://example.com/watch?v=1"),
            "https://example.com/watch?v=1"
        );
        assert_ne!(
            canonical_url("https://example.com/watch?v=1"),
            canonical_url("https://example.com/watch?v=2")
        );
        assert_eq!(
            canonical_url("https://example.com:8443/x"),
            "https://example.com:8443/x"
        );
        assert_eq!(canonical_url("not a url"), "not a url");
    }

    #[test]
    fn registrable_domains_and_brand_labels() {
        assert_eq!(registrable("en.wikipedia.org"), "wikipedia.org");
        assert_eq!(registrable("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(registrable("apple.com"), "apple.com");
        assert_eq!(brand_label("www.apple.com"), "apple");
        assert_eq!(brand_label("news.bbc.co.uk"), "bbc");
    }
}
