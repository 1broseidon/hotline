//! A link's card, its picture and its site's icon, read by the desk.
//!
//! The window never asks another site for anything, so the desk does, under
//! the same rules the phone keeps for its own cards: https only, never a
//! machine on this network, a short wait and a bounded read. Each page,
//! picture and icon is read once and kept in memory, bounded by count: a
//! picture is shrunk to a thumbnail before it is kept, and an icon is at
//! most 64 KB, so the cache is a few megabytes in practice and a few tens
//! at worst, cheaper to read again after a restart than to manage on disk. A
//! page or icon with nothing to show is remembered as nothing for a while,
//! so scrolling past it again does not ask again. Nothing is read at all
//! while the room's `linkPreviews` setting is off.

use crate::contract::LinkPreview;
use crate::images::{self, DECODERS};
use crate::log::Log;
use crate::room;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::StreamExt;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use regex::Regex;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use url::Url;

const PAGE_TIMEOUT: Duration = Duration::from_secs(6);
/// A page is read only as far as this for its head.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// A picture or an icon is a small thing beside its page, and a card that
/// waits on one is a card not shown.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(4);
/// A picture heavier than this is no picture: a card's picture is a
/// thumbnail, and a social card that needs more is not worth the wait.
const MAX_PICTURE_BYTES: usize = 3 * 1024 * 1024;
/// The card draws the picture 92 px wide; this is that at twice the
/// density, with room to crop.
const PICTURE_EDGE: u32 = 320;
const MAX_ICON_BYTES: usize = 64 * 1024;
const RETRY_EMPTY: Duration = Duration::from_hours(6);
/// Cards (with their pictures) and icons each kept, the oldest forgotten first.
const KEEP: usize = 300;
/// A name that will not resolve is no card, and nobody waits on a resolver
/// for longer than the page itself is given.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

static CARDS: LazyLock<Remembered<LinkPreview>> = LazyLock::new(|| Remembered::new(KEEP));
static ICONS: LazyLock<Remembered<String>> = LazyLock::new(|| Remembered::new(KEEP));

/// The card for `url`, read once and then remembered; `None` when the page
/// has nothing to show, may not be read, or link previews are off.
pub async fn preview(log: &Log, url: &str) -> Option<LinkPreview> {
    if !room::link_previews(log) {
        return None;
    }
    let owned = url.to_string();
    CARDS
        .get(url, move || async move { read_card(&owned).await }.boxed())
        .await
}

/// The icon of `site` (a bare host, as `github.com`) as a `data:` URL, read
/// once and then remembered; `None` when it has none, may not be read, or
/// link previews are off.
pub async fn icon(log: &Log, site: &str) -> Option<String> {
    if !room::link_previews(log) {
        return None;
    }
    let site = site.to_ascii_lowercase();
    let owned = site.clone();
    ICONS
        .get(&site, move || {
            async move { read_icon(&owned).await }.boxed()
        })
        .await
}

type Reading<T> = Shared<BoxFuture<'static, Option<T>>>;

/// What each key gave when it was read, each key read once: two asks while
/// it is being read share the one read, a found value is kept until it is
/// the oldest of `keep`, and nothing found is asked again after
/// `RETRY_EMPTY`.
struct Remembered<T: Clone> {
    keep: usize,
    known: Mutex<HashMap<String, (Instant, Option<T>)>>,
    reading: Mutex<HashMap<String, Reading<T>>>,
}

impl<T: Clone + Send + Sync + 'static> Remembered<T> {
    fn new(keep: usize) -> Self {
        Self {
            keep,
            known: Mutex::new(HashMap::new()),
            reading: Mutex::new(HashMap::new()),
        }
    }

    async fn get(
        &self,
        key: &str,
        read: impl FnOnce() -> BoxFuture<'static, Option<T>>,
    ) -> Option<T> {
        if let Some(kept) = self.kept(key) {
            return kept;
        }
        let reading = {
            let mut reading = self.reading.lock().ok()?;
            // A read that finished between the look above and this lock is
            // known by now, since it is known before it stops being read.
            if let Some(kept) = self.kept(key) {
                return kept;
            }
            reading
                .entry(key.to_string())
                .or_insert_with(|| read().shared())
                .clone()
        };
        let found = reading.await;
        self.remember(key, found.clone());
        if let Ok(mut reading) = self.reading.lock() {
            reading.remove(key);
        }
        found
    }

    /// `Some` when `key` has an answer that still stands.
    fn kept(&self, key: &str) -> Option<Option<T>> {
        let known = self.known.lock().ok()?;
        let (at, kept) = known.get(key)?;
        (kept.is_some() || at.elapsed() < RETRY_EMPTY).then(|| kept.clone())
    }

    fn remember(&self, key: &str, found: Option<T>) {
        let Ok(mut known) = self.known.lock() else {
            return;
        };
        if known.len() >= self.keep && !known.contains_key(key) {
            let oldest = known
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                known.remove(&oldest);
            }
        }
        known.insert(key.to_string(), (Instant::now(), found));
    }
}

/// The page's card, with its picture fetched and shrunk when it has one.
async fn read_card(url: &str) -> Option<LinkPreview> {
    let parsed = Url::parse(url).ok()?;
    let response = fetch(&parsed, "text/html,application/xhtml+xml", PAGE_TIMEOUT).await?;
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
        if body.len() >= MAX_PAGE_BYTES || ends_head(&body) {
            break;
        }
    }
    body.truncate(MAX_PAGE_BYTES);
    let (mut card, picture_at) = parse(&String::from_utf8_lossy(&body), url, &base)?;
    if let Some(picture_at) = picture_at {
        card.image = picture(&picture_at).await;
    }
    Some(card)
}

/// The picture at `url` as a small JPEG `data:` URL, or nothing: a picture
/// that cannot be read, is too heavy, or is not an image is no picture.
async fn picture(url: &Url) -> Option<String> {
    let response = fetch(url, "image/*", PICTURE_TIMEOUT).await?;
    let bytes = whole(response, MAX_PICTURE_BYTES).await?;
    let permit = DECODERS.acquire().await.ok()?;
    let shrunk = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        images::shrink(&bytes, PICTURE_EDGE, 80)
    })
    .await
    .ok()?
    .ok()?;
    Some(data_url("image/jpeg", &shrunk))
}

/// `https://<site>/favicon.ico`, kept as the site sent it when it is an
/// image a page can draw.
async fn read_icon(site: &str) -> Option<String> {
    let bare = !site.is_empty()
        && site
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !bare {
        return None;
    }
    let url = Url::parse(&format!("https://{site}/favicon.ico")).ok()?;
    let response = fetch(&url, "image/*", PICTURE_TIMEOUT).await?;
    let bytes = whole(response, MAX_ICON_BYTES).await?;
    Some(data_url(icon_type(&bytes)?, &bytes))
}

/// What kind of picture an icon's bytes are, by their first bytes rather
/// than what the site called them.
fn icon_type(bytes: &[u8]) -> Option<&'static str> {
    use image::ImageFormat;
    match image::guess_format(bytes).ok()? {
        ImageFormat::Ico => Some("image/x-icon"),
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", BASE64.encode(bytes))
}

/// The one way the desk asks a site for anything here: only a fetchable
/// address, every hop of a redirect fetchable too, every name resolved to
/// addresses out on the internet, and the whole exchange bounded by `wait`.
/// `None` for anything but a success.
async fn fetch(url: &Url, accept: &str, wait: Duration) -> Option<reqwest::Response> {
    if !fetchable(url) || offline(url) {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(wait)
        // A proxy would resolve the name itself, past the resolver below.
        .no_proxy()
        .dns_resolver(Outside)
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
        .get(url.clone())
        .header("Accept", accept)
        .send()
        .await
        .ok()?;
    response.status().is_success().then_some(response)
}

/// The whole body, or nothing when it is over `cap`: a cut-off picture is
/// no picture.
async fn whole(response: reqwest::Response, cap: usize) -> Option<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > cap as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.ok()?);
        if body.len() > cap {
            return None;
        }
    }
    Some(body)
}

fn ends_head(body: &[u8]) -> bool {
    body.windows(7).any(|w| w.eq_ignore_ascii_case(b"</head>"))
}

/// In tests the desk contacts no site: each address it would have asked is
/// written down instead, so a test can say what was and was not asked.
#[cfg(test)]
static CONTACTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(test)]
fn offline(url: &Url) -> bool {
    CONTACTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(url.to_string());
    true
}

/// Whether the desk, in a test, would have asked a site for `url`.
#[cfg(test)]
pub(crate) fn contacted(url: &str) -> bool {
    CONTACTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|asked| asked == url)
}

#[cfg(not(test))]
fn offline(_: &Url) -> bool {
    false
}

/// Only https, and never a name that means this machine or this network.
pub fn fetchable(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            let private = [".localhost", ".local", ".internal", ".lan", ".home.arpa"];
            host.contains('.')
                && host != "localhost"
                && !private.iter().any(|suffix| host.ends_with(suffix))
        }
        Some(url::Host::Ipv4(ip)) => public(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => public(IpAddr::V6(ip)),
        None => false,
    }
}

/// The resolver every fetch here connects through. A public name can
/// still point inward, so every address it resolves to has to be out on the
/// internet; checking at connection rather than before it means a redirect,
/// or a name that answers differently the second time, is checked too.
struct Outside;

impl reqwest::dns::Resolve for Outside {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses = outside(&host).await.ok_or_else(|| {
                std::io::Error::other(format!("{host} is not an address on the internet"))
            })?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Every address `host` resolves to, when there is at least one and all of
/// them are out on the internet.
async fn outside(host: &str) -> Option<Vec<SocketAddr>> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host, 0)))
            .await
            .ok()?
            .ok()?
            .collect();
    (!addresses.is_empty() && addresses.iter().all(|address| public(address.ip())))
        .then_some(addresses)
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

static META: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").expect("a fixed pattern"));
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("a fixed pattern"));
static ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\b([a-z:_-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
        .expect("a fixed pattern")
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
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+\d*);").expect("a fixed pattern")
    });
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

/// A card from a page's head, and where its picture is: its Open Graph
/// title and picture, then its Twitter card, then its `<title>`. No title,
/// no card. `base` is where the page was finally read from, for a picture
/// given relative to it. The picture is only an https one.
pub fn parse(html: &str, url: &str, base: &Url) -> Option<(LinkPreview, Option<Url>)> {
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
    // The first line is where the page lives, and the site's own name
    // after it only when that says something the address and the title
    // have not: `github.com · GitHub` would say one thing twice.
    let squashed = |text: &str| {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase()
    };
    let says_more = !named.is_empty()
        && !host.contains(&squashed(&named))
        && !named.eq_ignore_ascii_case(&title);
    let site = if says_more {
        format!("{host} · {named}")
    } else {
        host
    };
    let description = (!description.is_empty() && !description.eq_ignore_ascii_case(&title))
        .then(|| clip(&description, 300));
    let picture = pick(&["og:image:secure_url", "og:image", "twitter:image"])
        .and_then(|raw| base.join(&decode(&raw)).ok())
        .filter(|picture| picture.scheme() == "https");
    let card = LinkPreview {
        url: url.to_string(),
        title: clip(&title, 200),
        site: clip(&site, 160),
        description,
        image: None,
    };
    Some((card, picture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    /// The resolver every fetch connects through refuses a name that
    /// points at this machine or this network, whatever the name looks like.
    #[tokio::test]
    async fn a_name_that_resolves_inward_is_never_connected_to() {
        assert!(outside("localhost").await.is_none());
        assert!(outside("127.0.0.1").await.is_none());
        assert!(outside("[::1]").await.is_none());
        assert!(outside("10.1.2.3").await.is_none());
        assert!(outside("169.254.169.254").await.is_none());
        assert!(outside("[::ffff:192.168.1.4]").await.is_none());
        assert!(outside("1.1.1.1").await.is_some());
    }

    /// A page's picture and a site's icon pass the gate the page does: an
    /// http, local or private address is never asked for.
    #[tokio::test]
    async fn a_picture_or_icon_over_http_or_on_this_network_is_never_asked_for() {
        for refused in [
            "http://1.2.3.4/card.png",
            "https://localhost/card.png",
            "https://nas.local/card.png",
            "https://192.168.1.4/card.png",
            "https://[fd00::1]/card.png",
        ] {
            assert!(picture(&url(refused)).await.is_none());
            assert!(!contacted(refused), "{refused} was asked for");
        }
        for refused in ["localhost", "nas.local", "10.0.0.1", "printer"] {
            assert!(read_icon(refused).await.is_none());
            let at = format!("https://{refused}/favicon.ico");
            assert!(!contacted(&at), "{at} was asked for");
        }
        // Nor anything that is not a bare host: a path, a port or a user.
        for odd in ["1.2.3.9/x?", "1.2.3.9:8443", "me@1.2.3.9"] {
            assert!(read_icon(odd).await.is_none());
        }
        assert!(!contacted("https://1.2.3.9/favicon.ico"));
        // The same gate lets an address on the internet through.
        assert!(picture(&url("https://1.2.3.7/card.png")).await.is_none());
        assert!(contacted("https://1.2.3.7/card.png"));
        assert!(read_icon("1.2.3.8").await.is_none());
        assert!(contacted("https://1.2.3.8/favicon.ico"));
    }

    fn counted(
        reads: &Arc<AtomicUsize>,
        found: Option<&'static str>,
    ) -> impl FnOnce() -> BoxFuture<'static, Option<String>> {
        let reads = reads.clone();
        move || {
            async move {
                reads.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                found.map(String::from)
            }
            .boxed()
        }
    }

    /// The window asks for a card's picture, or a site's icon, as often as
    /// it draws one; the desk fetches it once, whether the asks come one
    /// after the other or together.
    #[tokio::test]
    async fn a_picture_asked_twice_is_fetched_once() {
        let kept = Remembered::new(KEEP);
        let reads = Arc::new(AtomicUsize::new(0));
        let first = kept.get("a", counted(&reads, Some("data:a"))).await;
        let second = kept.get("a", counted(&reads, Some("data:b"))).await;
        assert_eq!(first.as_deref(), Some("data:a"));
        assert_eq!(second.as_deref(), Some("data:a"));
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        let (one, two) = tokio::join!(
            kept.get("b", counted(&reads, Some("data:b"))),
            kept.get("b", counted(&reads, Some("data:b"))),
        );
        assert_eq!(
            (one.as_deref(), two.as_deref()),
            (Some("data:b"), Some("data:b"))
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);

        // Nothing found is remembered as nothing, and not asked again soon.
        assert!(kept.get("c", counted(&reads, None)).await.is_none());
        assert!(
            kept.get("c", counted(&reads, Some("data:c")))
                .await
                .is_none()
        );
        assert_eq!(reads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn the_cache_forgets_its_oldest_past_its_bound() {
        let kept = Remembered::new(2);
        let reads = Arc::new(AtomicUsize::new(0));
        for key in ["a", "b", "c"] {
            kept.get(key, counted(&reads, Some("data:x"))).await;
        }
        assert_eq!(kept.known.lock().unwrap().len(), 2);
        assert!(kept.kept("a").is_none(), "the oldest is forgotten");
        kept.get("a", counted(&reads, Some("data:x"))).await;
        assert_eq!(reads.load(Ordering::SeqCst), 4);
    }

    /// A picture or icon over its cap is no picture, never a cut-off one.
    #[tokio::test]
    async fn a_body_over_its_cap_is_nothing() {
        let response =
            |bytes: usize| reqwest::Response::from(http::Response::new(vec![7u8; bytes]));
        assert_eq!(
            whole(response(10), 10).await.map(|body| body.len()),
            Some(10)
        );
        assert!(whole(response(11), 10).await.is_none());
    }

    #[test]
    fn an_icon_is_kept_only_when_its_bytes_are_a_picture() {
        assert_eq!(icon_type(&[0, 0, 1, 0, 1, 0]), Some("image/x-icon"));
        assert_eq!(icon_type(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(icon_type(b"<!doctype html><html>"), None);
        assert_eq!(
            icon_type(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
    }

    #[test]
    fn a_card_reads_open_graph_first_and_resolves_the_picture() {
        let html = r#"<html><head><title>Fallback</title>
            <meta property="og:title" content="Hotline &amp; friends">
            <meta content='The desk app' property='og:site_name'>
            <meta property="og:description" content="A room for a team of agents.">
            <meta property="og:image" content="/card.png">
            </head><body><meta property="og:title" content="Not this"></body>"#;
        let (card, picture) = parse(
            html,
            "https://example.com/a",
            &url("https://www.example.com/a"),
        )
        .unwrap();
        assert_eq!(card.title, "Hotline & friends");
        assert_eq!(card.site, "example.com · The desk app");
        assert_eq!(
            card.description.as_deref(),
            Some("A room for a team of agents.")
        );
        assert_eq!(
            picture.as_ref().map(Url::as_str),
            Some("https://www.example.com/card.png")
        );
        assert_eq!(card.image, None, "the picture is fetched, not linked");
        assert_eq!(card.url, "https://example.com/a");
    }

    #[test]
    fn the_first_line_is_the_address_and_a_name_only_when_it_adds_one() {
        let base = url("https://www.github.com/");
        let named = r#"<head><title>Pull request</title><meta property="og:site_name" content="GitHub"></head>"#;
        let (card, _) = parse(named, "https://github.com/", &base).unwrap();
        assert_eq!(card.site, "github.com");
        assert_eq!(card.description, None);
        let described =
            r#"<head><title>Example</title><meta name="description" content="A page"></head>"#;
        let (card, _) = parse(described, "https://github.com/", &base).unwrap();
        assert_eq!(card.site, "github.com");
        assert_eq!(card.description.as_deref(), Some("A page"));
        assert!(parse("<head></head>", "https://github.com/", &base).is_none());
    }

    #[test]
    fn a_picture_over_http_is_no_picture() {
        let html = r#"<head><meta property="og:title" content="T"><meta property="og:image" content="http://x.com/a.png"></head>"#;
        let (_, picture) = parse(html, "https://x.com/", &url("https://x.com/")).unwrap();
        assert!(picture.is_none());
    }
}
