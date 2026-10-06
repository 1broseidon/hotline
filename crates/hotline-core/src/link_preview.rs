//! A link's card: what a page says about itself, read from its head.
//!
//! The window cannot read another site's page, so the desk does, under the
//! same rules the phone keeps for its own cards: https only, never a machine
//! on this network, a short wait and a bounded read. A page with nothing to
//! show is remembered as nothing for a while, so scrolling past it again does
//! not ask again.

use crate::contract::LinkPreview;
use futures_util::StreamExt;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use regex::Regex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(6);
/// A page is read only as far as this for its head.
const MAX_BYTES: usize = 2 * 1024 * 1024;
const RETRY_EMPTY: Duration = Duration::from_secs(6 * 60 * 60);
const KEEP: usize = 300;
/// A name that will not resolve is no card, and nobody waits on a resolver
/// for longer than the page itself is given.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

/// When each link was read, and what it gave.
type Known = HashMap<String, (Instant, Option<LinkPreview>)>;

static KNOWN: LazyLock<Mutex<Known>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type Reading = Shared<BoxFuture<'static, Option<LinkPreview>>>;

/// The links being read right now, so a message that names one link twice, or
/// two windows that ask at once, share one fetch.
static READING: LazyLock<Mutex<HashMap<String, Reading>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The card for `url`, read once and then remembered; `None` when the page
/// has nothing to show or may not be read.
pub async fn preview(url: &str) -> Option<LinkPreview> {
    if let Some((at, kept)) = KNOWN.lock().ok()?.get(url).cloned()
        && (kept.is_some() || at.elapsed() < RETRY_EMPTY)
    {
        return kept;
    }
    let reading = {
        let mut reading = READING.lock().ok()?;
        reading
            .entry(url.to_string())
            .or_insert_with(|| {
                let url = url.to_string();
                async move { remember(&url).await }.boxed().shared()
            })
            .clone()
    };
    let found = reading.await;
    if let Ok(mut reading) = READING.lock() {
        reading.remove(url);
    }
    found
}

async fn remember(url: &str) -> Option<LinkPreview> {
    let found = read(url).await;
    if let Ok(mut known) = KNOWN.lock() {
        if known.len() >= KEEP {
            let oldest = known
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                known.remove(&oldest);
            }
        }
        known.insert(url.to_string(), (Instant::now(), found.clone()));
    }
    found
}

async fn read(url: &str) -> Option<LinkPreview> {
    let parsed = Url::parse(url).ok()?;
    if !fetchable(&parsed) || !resolves_outside(&parsed).await {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && fetchable(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent("Mozilla/5.0 (compatible; Hotline link preview)")
        .build()
        .ok()?;
    let response = client
        .get(parsed)
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("html"));
    if !html {
        return None;
    }
    let base = response.url().clone();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.ok()?);
        if body.len() >= MAX_BYTES || ends_head(&body) {
            break;
        }
    }
    body.truncate(MAX_BYTES);
    parse(&String::from_utf8_lossy(&body), url, &base)
}

fn ends_head(body: &[u8]) -> bool {
    body.windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>"))
}

/// Only https, and never a name that means this machine or this network.
pub fn fetchable(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            host.contains('.')
                && !(host == "localhost"
                    || host.ends_with(".localhost")
                    || host.ends_with(".local")
                    || host.ends_with(".internal")
                    || host.ends_with(".lan")
                    || host.ends_with(".home.arpa"))
        }
        Some(url::Host::Ipv4(ip)) => public(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => public(IpAddr::V6(ip)),
        None => false,
    }
}

/// A public name can still point inward; every address it resolves to has
/// to be out on the internet.
async fn resolves_outside(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port_or_known_default().unwrap_or(443);
    match tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, port))).await {
        Ok(Ok(addresses)) => {
            let addresses: Vec<_> = addresses.collect();
            !addresses.is_empty() && addresses.iter().all(|address| public(address.ip()))
        }
        Ok(Err(_)) | Err(_) => false,
    }
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || (a == 100 && (64..=127).contains(&b)))
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
                && ip.to_ipv4_mapped().is_none_or(|v4| public(IpAddr::V4(v4)))
        }
    }
}

static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").unwrap());
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
static ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\b([a-z:_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap()
});

/// The value of each `<meta>` in the head, by property or name, first one winning.
fn metas(head: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    for tag in META.find_iter(head) {
        let mut key = None;
        let mut content = None;
        for attr in ATTR.captures_iter(tag.as_str()) {
            let value = attr
                .get(2)
                .or_else(|| attr.get(3))
                .or_else(|| attr.get(4))
                .map_or("", |m| m.as_str());
            match attr[1].to_ascii_lowercase().as_str() {
                "property" | "name" if key.is_none() => key = Some(value.to_ascii_lowercase()),
                "content" => content = Some(value.to_string()),
                _ => {}
            }
        }
        if let (Some(key), Some(content)) = (key, content)
            && !key.is_empty()
        {
            found.entry(key).or_insert(content);
        }
    }
    found
}

fn decode(value: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+\d*);").unwrap());
    let decoded = ENTITY.replace_all(value, |caps: &regex::Captures| {
        let name = caps[1].to_ascii_lowercase();
        let named = match name.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => None,
        };
        let code = name
            .strip_prefix("#x")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()));
        named
            .or_else(|| code.and_then(char::from_u32))
            .map_or_else(|| caps[0].to_string(), String::from)
    });
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// A card from a page's head: its Open Graph title and picture, then its
/// Twitter card, then its `<title>`. No title, no card. `base` is where the
/// page was finally read from, for a picture given relative to it.
pub fn parse(html: &str, url: &str, base: &Url) -> Option<LinkPreview> {
    let lower = html.to_ascii_lowercase();
    let end = lower
        .find("</head>")
        .unwrap_or_else(|| html.len().min(200_000));
    let head = html.get(..end).unwrap_or(html);
    let meta = metas(head);
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| meta.get(*key).filter(|value| !value.trim().is_empty()))
            .cloned()
    };
    let title_tag = TITLE.captures(head).map(|caps| caps[1].to_string());
    let title = decode(&pick(&["og:title", "twitter:title"]).or(title_tag)?);
    if title.is_empty() {
        return None;
    }
    let host = base
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string();
    let named = decode(&pick(&["og:site_name"]).unwrap_or_default());
    let description = decode(
        &pick(&["og:description", "twitter:description", "description"]).unwrap_or_default(),
    );
    // The second line says something the title has not: the site's name,
    // or, when that is the title again, what the page is about, or where.
    let site = if !named.is_empty() && !named.eq_ignore_ascii_case(&title) {
        named
    } else if !description.is_empty() && !description.eq_ignore_ascii_case(&title) {
        description
    } else {
        host
    };
    let image = pick(&["og:image:secure_url", "og:image", "twitter:image"])
        .and_then(|raw| base.join(&decode(&raw)).ok())
        .filter(|image| image.scheme() == "https")
        .map(String::from);
    Some(LinkPreview {
        url: url.to_string(),
        title: clip(&title, 200),
        site: clip(&site, 160),
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn only_https_on_the_internet_is_read() {
        assert!(fetchable(&url("https://github.com/x")));
        assert!(!fetchable(&url("http://github.com/x")));
        assert!(!fetchable(&url("https://localhost/x")));
        assert!(!fetchable(&url("https://nas.local/x")));
        assert!(!fetchable(&url("https://printer/x")));
        assert!(!fetchable(&url("https://192.168.1.4/x")));
        assert!(!fetchable(&url("https://10.0.0.1/x")));
        assert!(!fetchable(&url("https://100.100.1.1/x")));
        assert!(!fetchable(&url("https://[::1]/x")));
        assert!(!fetchable(&url("https://[fd00::1]/x")));
        assert!(fetchable(&url("https://1.1.1.1/x")));
    }

    #[test]
    fn a_card_reads_open_graph_first_and_resolves_the_picture() {
        let html = r#"<html><head><title>Fallback</title>
            <meta property="og:title" content="Hotline &amp; friends">
            <meta content='The desk app' property='og:site_name'>
            <meta property="og:image" content="/card.png">
            </head><body><meta property="og:title" content="Not this"></body>"#;
        let card = parse(
            html,
            "https://example.com/a",
            &url("https://www.example.com/a"),
        )
        .unwrap();
        assert_eq!(card.title, "Hotline & friends");
        assert_eq!(card.site, "The desk app");
        assert_eq!(
            card.image.as_deref(),
            Some("https://www.example.com/card.png")
        );
        assert_eq!(card.url, "https://example.com/a");
    }

    #[test]
    fn the_second_line_falls_back_to_the_description_then_the_host() {
        let base = url("https://www.example.com/");
        let described =
            r#"<head><title>Example</title><meta name="description" content="A page"></head>"#;
        assert_eq!(
            parse(described, "https://example.com/", &base)
                .unwrap()
                .site,
            "A page"
        );
        let bare = "<head><title>Example</title></head>";
        assert_eq!(
            parse(bare, "https://example.com/", &base).unwrap().site,
            "example.com"
        );
        assert!(parse("<head></head>", "https://example.com/", &base).is_none());
    }

    #[test]
    fn a_picture_over_http_is_no_picture() {
        let html = r#"<head><meta property="og:title" content="T"><meta property="og:image" content="http://x.com/a.png"></head>"#;
        assert!(
            parse(html, "https://x.com/", &url("https://x.com/"))
                .unwrap()
                .image
                .is_none()
        );
    }
}
