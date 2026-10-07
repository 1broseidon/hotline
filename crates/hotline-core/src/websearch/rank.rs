//! From each provider's list to the one a model reads.
//!
//! The lists are cleaned (error pages out, snippets freed of chrome and
//! repeats), fused by Reciprocal Rank Fusion as ketch's `multi.go` does,
//! deduplicated, and then nudged by a few small, named rules about what the
//! query asked for. Everything here is pure, so each rule has a test with a
//! fixture.

use super::Hit;
use super::canonical::{brand_label, canonical_url, host_of, registrable};
use super::clean::{clean_snippet, is_junk};
use super::query::{Haystack, Language, Query, missed, missed_line, text_language};
use super::when::result_date;
use crate::contract::WebSearchProvider;
use std::collections::HashMap;

/// RRF's constant. 60 is the published default (Cormack, Clarke and Büttcher,
/// SIGIR 2009): it keeps one engine's first place from outvoting two engines
/// agreeing in the middle. Not a tuning knob.
const RRF_K: f64 = 60.0;

// The weights. Each is a factor on the fused score, one rank-1 hit being worth
// about 0.016 and ten ranks down about 0.014, so a factor of 1.3 moves a result
// a few places and a factor of 3 puts it first. They are small on purpose, and
// every one is explained where it is used in `adjust`.

/// A keyed provider is one the person set up on purpose, and it is often the
/// better index; its list counts a little more than a keyless one's.
const KEYED_WEIGHT: f64 = 1.25;
/// A result with nothing readable left in its snippet is worth less than one
/// that can say what it is.
const THIN_SNIPPET: f64 = 0.6;
/// The query wrote a domain: those hosts first.
const DOMAIN_IN_QUERY: f64 = 3.0;
/// A one or two word query whose word is a site's name: `apple` and apple.com.
const BRAND_MATCH: f64 = 1.8;
/// Social and video hosts are rarely what a research query wants.
const SOCIAL_HOST: f64 = 0.5;
/// Wikipedia in the query's own language is a good first stop.
const WIKIPEDIA_IN_LANGUAGE: f64 = 1.5;
/// For a non-English query, pages written in it.
const IN_QUERY_LANGUAGE: f64 = 1.4;
/// "X vs Y" matching a package whose name just joins X and Y.
const COMPARISON_PACKAGE: f64 = 0.4;
/// "X vs Y" with a page that talks about both.
const COMPARISON_BOTH: f64 = 1.4;
/// A news query landing on a tag or index page rather than a story.
const INDEX_PAGE: f64 = 0.5;
/// A news query landing on a section front ("/news/sanctions", a site's home):
/// a page that changes by the hour, whose snippet is a menu and a stale story.
const SECTION_PAGE: f64 = 0.45;
/// A news query's result dated inside the days asked about.
const IN_WINDOW: f64 = 1.6;
/// A news query's result dated a month or more before them: not news.
const OLD_DATED: f64 = 0.4;
/// Video hosts for a news query that did not ask for video: a clip's page has
/// a title and a player, not the story.
const VIDEO_FOR_NEWS: f64 = 0.6;
/// A small prior for established news outlets on a news query.
const NEWS_OUTLET: f64 = 1.3;
/// A sitemap page is a list of links, never what was looked for.
const SITEMAP: f64 = 0.3;
/// Nothing readable at all under the title.
const EMPTY_SNIPPET: f64 = 0.8;
/// A result that mentions none of the query's distinctive words, when others do.
const GENERIC_ONLY: f64 = 0.6;

/// Hosts that are social networks or video sites, by registrable label.
const SOCIAL: [&str; 9] = [
    "facebook",
    "fb",
    "tiktok",
    "instagram",
    "x",
    "twitter",
    "pinterest",
    "snapchat",
    "threads",
];

/// Package registries, whose pages are named for the package.
const PACKAGE_HOSTS: [&str; 9] = [
    "docs.rs",
    "crates.io",
    "lib.rs",
    "npmjs.com",
    "pkg.go.dev",
    "pypi.org",
    "rubygems.org",
    "packagist.org",
    "mvnrepository.com",
];

/// Established news outlets, by the first label of their registrable domain.
const NEWS_OUTLETS: [&str; 19] = [
    "reuters",
    "apnews",
    "bbc",
    "theguardian",
    "guardian",
    "nytimes",
    "cnbc",
    "aljazeera",
    "npr",
    "ft",
    "bloomberg",
    "washingtonpost",
    "cnn",
    "abcnews",
    "cbsnews",
    "nbcnews",
    "france24",
    "dw",
    "politico",
];

/// Video hosts.
const VIDEO_HOSTS: [&str; 4] = ["youtube", "youtu", "vimeo", "dailymotion"];

const INDEX_PATHS: [&str; 6] = [
    "/tag/",
    "/tags/",
    "/topic/",
    "/topics/",
    "/category/",
    "/categories/",
];

/// One provider's answer to be fused.
pub struct Source {
    pub provider: WebSearchProvider,
    pub keyed: bool,
    pub hits: Vec<Hit>,
}

/// What came out: the results to show, and a note when the query's rare words
/// matched nothing.
#[derive(Debug)]
pub struct Ranked {
    pub hits: Vec<Hit>,
    pub note: Option<String>,
}

/// A result as the ranking holds it.
#[derive(Clone, Debug)]
struct Item {
    hit: Hit,
    thin: bool,
    language: Option<Language>,
}

impl Item {
    fn new(hit: Hit) -> Option<Self> {
        if hit.url.trim().is_empty() || hit.title.trim().is_empty() && hit.snippet.trim().is_empty()
        {
            return None;
        }
        if is_junk(&hit.title, &hit.snippet) {
            return None;
        }
        // A bare IP address is never a page worth a model's reading: it is
        // spam that has learned to look like a result.
        if host_of(&hit.url).is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok()) {
            return None;
        }
        let (snippet, thin) = clean_snippet(&hit.snippet);
        let mut title = hit.title.split_whitespace().collect::<Vec<_>>().join(" ");
        // A placeholder title reads as the page's address instead.
        if matches!(
            title.to_ascii_lowercase().as_str(),
            "n/a" | "na" | "none" | "untitled"
        ) {
            title = page_name(&hit.url);
        }
        let language =
            host_language(&hit.url).or_else(|| text_language(&format!("{title}. {snippet}")));
        Some(Self {
            hit: Hit {
                title,
                url: hit.url,
                snippet,
                published: hit.published,
            },
            thin,
            language,
        })
    }

    /// How well a snippet serves: readable first, then the one that carries
    /// the most of the query, then the longer, within reason.
    fn snippet_worth(&self, query: &Query) -> (bool, usize, usize) {
        let lower = self.hit.snippet.to_lowercase();
        let carried = query
            .content_tokens()
            .iter()
            .filter(|token| lower.contains(*token))
            .count();
        (!self.thin, carried, lower.chars().count().min(300))
    }
}

/// `fr` from `fr.wikipedia.org`, `de` from `de.example.com`.
fn host_language(url: &str) -> Option<Language> {
    let host = host_of(url)?;
    let first = host.split('.').next()?;
    super::query::language_by_code(first).filter(|_| host.matches('.').count() >= 2)
}

#[derive(Debug)]
struct Doc {
    key: String,
    score: f64,
    best_rank: usize,
    providers: usize,
    item: Item,
    /// Every distinct snippet seen, for choosing the best.
    others: Vec<Item>,
}

/// Fuses the lists and cuts to `limit`.
pub fn rank(query: &Query, sources: Vec<Source>, limit: usize) -> Ranked {
    let mut docs: HashMap<String, Doc> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for source in sources {
        let weight = if source.keyed { KEYED_WEIGHT } else { 1.0 };
        let mut seen: Vec<String> = Vec::new();
        let items = source.hits.into_iter().filter_map(Item::new);
        for (index, item) in items.enumerate() {
            let rank = index + 1;
            let key = canonical_url(&item.hit.url);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key.clone());
            let gain = weight / (RRF_K + rank as f64);
            match docs.get_mut(&key) {
                Some(doc) => {
                    doc.score += gain;
                    doc.providers += 1;
                    if rank < doc.best_rank {
                        doc.best_rank = rank;
                        let earlier = std::mem::replace(&mut doc.item, item);
                        doc.others.push(earlier);
                    } else {
                        doc.others.push(item);
                    }
                }
                None => {
                    order.push(key.clone());
                    docs.insert(
                        key.clone(),
                        Doc {
                            key,
                            score: gain,
                            best_rank: rank,
                            providers: 1,
                            item,
                            others: Vec::new(),
                        },
                    );
                }
            }
        }
    }
    let mut fused: Vec<Doc> = order
        .into_iter()
        .filter_map(|key| docs.remove(&key))
        .collect();
    fused.sort_by(by_score);
    let mut merged = merge_duplicates(query, fused);
    for doc in &mut merged {
        let best = pick_snippet(query, doc);
        doc.item.hit.snippet = best.hit.snippet;
        doc.item.thin = best.thin;
        if doc.item.hit.published.is_none() {
            doc.item.hit.published = doc
                .others
                .iter()
                .find_map(|other| other.hit.published.clone());
        }
    }
    // Which of the query's distinctive words any result carries at all.
    let everything: String = merged
        .iter()
        .map(|doc| {
            format!(
                "{} {} {}\n",
                doc.item.hit.title, doc.item.hit.snippet, doc.item.hit.url
            )
        })
        .collect();
    let distinctive = query.distinctive();
    let found_somewhere = missed(query, &Haystack::new(&everything)).len() < distinctive.len();
    for doc in &mut merged {
        let words = Haystack::new(&format!(
            "{} {} {}",
            doc.item.hit.title, doc.item.hit.snippet, doc.item.hit.url
        ));
        let generic_only = found_somewhere
            && !distinctive.is_empty()
            && !distinctive.iter().any(|token| words.has(token));
        doc.score *= adjust(query, &doc.item, generic_only);
    }
    merged.sort_by(by_score);

    // The note speaks for the results shown: a distinctive word found only on
    // a page cut from the list is still missing from what the reader sees.
    let haystack: String = merged
        .iter()
        .take(limit)
        .map(|doc| {
            format!(
                "{} {} {}\n",
                doc.item.hit.title, doc.item.hit.snippet, doc.item.hit.url
            )
        })
        .collect::<String>()
        .to_lowercase();
    let note = if merged.is_empty() {
        None
    } else {
        missed_line(query, &haystack)
    };
    Ranked {
        hits: merged
            .into_iter()
            .take(limit)
            .map(|doc| doc.item.hit)
            .collect(),
        note,
    }
}

/// Whether a result looks like a story rather than a section front.
#[cfg(test)]
pub(crate) fn is_story(hit: &Hit) -> bool {
    let path = url::Url::parse(&hit.url)
        .map(|u| u.path().to_lowercase())
        .unwrap_or_default();
    is_article_path(&path) && !SECTION_TITLE.is_match(&hit.title)
}

/// Score, then agreement, then best rank, then key: a total order.
fn by_score(a: &Doc, b: &Doc) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(b.providers.cmp(&a.providers))
        .then(a.best_rank.cmp(&b.best_rank))
        .then(a.key.cmp(&b.key))
}

fn pick_snippet(query: &Query, doc: &Doc) -> Item {
    std::iter::once(&doc.item)
        .chain(doc.others.iter())
        .max_by_key(|item| item.snippet_worth(query))
        .cloned()
        .unwrap_or_else(|| doc.item.clone())
}

// Duplicates ------------------------------------------------------------------

/// Language codes that appear as a subdomain or a path segment of a mirror.
fn is_language_label(label: &str) -> bool {
    let plain = label.len() == 2 && label.chars().all(|c| c.is_ascii_lowercase());
    let regional = label.len() == 5
        && label.as_bytes()[2] == b'-'
        && label[..2].chars().all(|c| c.is_ascii_lowercase());
    (plain && super::query::language_by_code(label).is_some())
        || regional
        || matches!(label, "sp" | "jp" | "cn" | "br" | "kr" | "ua")
}

/// A page's identity with its language, mobile and `www` dressing removed, so
/// `fr.example.com/en/x`, `m.example.com/x` and `example.com/es/x` share it.
fn mirror_key(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    let mut labels: Vec<&str> = host.split('.').collect();
    while labels.len() > 2
        && (matches!(labels[0], "www" | "m" | "mobile" | "amp") || is_language_label(labels[0]))
    {
        labels.remove(0);
    }
    let segments: Vec<&str> = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty() && !is_language_label(&segment.to_lowercase()))
        .collect();
    Some(format!(
        "{}/{}",
        labels.join("."),
        segments.join("/").to_lowercase()
    ))
}

fn normalize_title(title: &str) -> String {
    // Wikipedia titles end in the site and language: "Everest - Wikipédia".
    let title = title.split(" - Wiki").next().unwrap_or(title);
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn long_words(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(str::to_string)
        .collect()
}

/// Whether two snippets are the same words, to within a few.
fn near_identical(a: &str, b: &str) -> bool {
    let (a, b) = (long_words(a), long_words(b));
    if a.len() < 8 || b.len() < 8 {
        return false;
    }
    let shared = a.intersection(&b).count() as f64;
    shared / a.union(&b).count() as f64 >= 0.85
}

/// Whether two results are the same page seen twice.
fn same_page(a: &Doc, b: &Doc) -> bool {
    let (ua, ub) = (&a.item.hit.url, &b.item.hit.url);
    if let (Some(ka), Some(kb)) = (mirror_key(ua), mirror_key(ub))
        && ka == kb
    {
        return true;
    }
    let (ta, tb) = (
        normalize_title(&a.item.hit.title),
        normalize_title(&b.item.hit.title),
    );
    if ta == tb && ta.len() >= 8 {
        let (ha, hb) = (
            host_of(ua).unwrap_or_default(),
            host_of(ub).unwrap_or_default(),
        );
        if registrable(&ha) == registrable(&hb) {
            return true;
        }
    }
    // The same text under another site's name: mirrors and syndication.
    ta == tb && ta.len() >= 8 && near_identical(&a.item.hit.snippet, &b.item.hit.snippet)
}

/// Merges results that are one page: the best-ranked stands, unless another
/// copy is in the query's language and the stander is not.
fn merge_duplicates(query: &Query, fused: Vec<Doc>) -> Vec<Doc> {
    let wanted = query.non_english();
    let mut kept: Vec<Doc> = Vec::new();
    for doc in fused {
        match kept.iter().position(|earlier| same_page(earlier, &doc)) {
            None => kept.push(doc),
            Some(at) => {
                let earlier = &mut kept[at];
                let prefer_new = wanted.is_some()
                    && doc.item.language == wanted
                    && earlier.item.language != wanted;
                let incoming = doc.item.clone();
                earlier.others.push(incoming);
                earlier.others.extend(doc.others);
                if prefer_new {
                    let replaced = std::mem::replace(&mut earlier.item, doc.item);
                    earlier.others.push(replaced);
                }
                earlier.providers = earlier.providers.max(doc.providers);
            }
        }
    }
    kept
}

// Intent ------------------------------------------------------------------------

/// The factor a result's score is multiplied by for what the query asked for.
fn adjust(query: &Query, item: &Item, generic_only: bool) -> f64 {
    let mut factor = 1.0;
    let url = &item.hit.url;
    let host = host_of(url).unwrap_or_default();
    let bare = host.strip_prefix("www.").unwrap_or(&host);
    let label = brand_label(&host);
    let path = url::Url::parse(url)
        .map(|u| u.path().to_lowercase())
        .unwrap_or_default();
    let text = format!("{} {}", item.hit.title, item.hit.snippet).to_lowercase();
    let content = query.content_tokens();

    if item.thin {
        factor *= THIN_SNIPPET;
    }
    if item.hit.snippet.trim().is_empty() {
        factor *= EMPTY_SNIPPET;
    }
    // A result that says nothing of the words that made the query particular
    // is a match for the generic ones, and others do better.
    if generic_only {
        factor *= GENERIC_ONLY;
    }
    // A sitemap is a list of links.
    if item
        .hit
        .title
        .trim()
        .to_lowercase()
        .replace(' ', "")
        .starts_with("sitemap")
        || path.contains("sitemap")
    {
        factor *= SITEMAP;
    }
    // A host the query wrote out is what was asked for.
    if query
        .domains
        .iter()
        .any(|d| bare == d.strip_prefix("www.").unwrap_or(d) || bare.ends_with(&format!(".{d}")))
    {
        factor *= DOMAIN_IN_QUERY;
    }
    // A short query that is a site's name wants the site.
    if content.len() <= 2 && !label.is_empty() && content.iter().any(|token| *token == label) {
        factor *= BRAND_MATCH;
    }
    let named = query
        .tokens
        .iter()
        .any(|t| *t == label || (label.len() > 2 && t.contains(&label)));
    if SOCIAL.contains(&label.as_str()) && !named {
        factor *= SOCIAL_HOST;
    }
    if host == format!("{}.wikipedia.org", query.effective_language().code)
        || host == format!("{}.m.wikipedia.org", query.effective_language().code)
    {
        factor *= WIKIPEDIA_IN_LANGUAGE;
    }
    if let Some(wanted) = query.non_english()
        && item.language == Some(wanted)
    {
        factor *= IN_QUERY_LANGUAGE;
    }
    if let Some((a, b)) = &query.comparison {
        if PACKAGE_HOSTS.contains(&bare) && names_package_of(&path, a, b) {
            factor *= COMPARISON_PACKAGE;
        } else if mentions(&text, a) && mentions(&text, b) {
            factor *= COMPARISON_BOTH;
        }
    }
    if query.recency {
        factor *= news_factor(query, item, &host, &path);
    }
    factor
}

/// What a news query makes of a result: where it sits in time and what kind of
/// page it is. Only the days asked about, and what a story page looks like.
fn news_factor(query: &Query, item: &Item, host: &str, path: &str) -> f64 {
    let mut factor = 1.0;
    let bare = host.strip_prefix("www.").unwrap_or(host);
    let first_label = registrable(bare)
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string();
    let hit = &item.hit;

    // Dated in the window is news; dated a month before it is not. The date is
    // the provider's, else one the page's own words give ("34 min ago",
    // "Oct 6, 2026").
    if let Some(window) = query.window
        && let Some(date) = result_date(
            hit.published.as_deref(),
            &hit.title,
            &hit.snippet,
            query.today,
        )
    {
        if window.near(date) {
            factor *= IN_WINDOW;
        } else if window.long_before(date) {
            factor *= OLD_DATED;
        }
    }
    // A tag, topic or section front is a list that changes by the hour, not a
    // story: its snippet is a menu and whatever was top when it was crawled.
    let named_video = query.tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "video" | "videos" | "youtube" | "watch" | "clip"
        )
    });
    if INDEX_PATHS
        .iter()
        .any(|p| path.contains(p) || path.ends_with(p.trim_end_matches('/')))
        || (!is_article_path(path) && shallow(path) && !VIDEO_HOSTS.contains(&first_label.as_str()))
        || SECTION_TITLE.is_match(&hit.title)
    {
        factor *= if INDEX_PATHS.iter().any(|p| path.contains(p)) {
            INDEX_PAGE
        } else {
            SECTION_PAGE
        };
    }
    if VIDEO_HOSTS.contains(&first_label.as_str()) && !named_video {
        factor *= VIDEO_FOR_NEWS;
    }
    if NEWS_OUTLETS.contains(&first_label.as_str()) {
        factor *= NEWS_OUTLET;
    }
    factor
}

/// "Latest X news", "X: Latest News, Top Stories & Analysis", "Topics".
static SECTION_TITLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(latest|top|breaking)\b.{0,30}\b(news|stories|headlines|updates)\b|^\s*(topics?|archive|sections?)\s*([|\-:]|$)|\bnews, analysis\b",
    )
    .expect("a fixed pattern")
});

/// The path's segments, without a trailing page number.
fn segments(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts
        .last()
        .is_some_and(|last| last.len() <= 3 && last.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    parts
}

/// Two segments deep or less: a front, not a story.
fn shallow(path: &str) -> bool {
    segments(path).len() <= 2
}

/// A path that names a story: a slug of three words or more, a long id, or a
/// date in the path.
fn is_article_path(path: &str) -> bool {
    segments(path).iter().any(|segment| {
        let words = segment.split(['-', '_']).filter(|w| !w.is_empty()).count();
        let digits = segment.chars().filter(char::is_ascii_digit).count();
        words >= 3
            || digits >= 6
            || (segment.len() == 4 && segment.starts_with("20") && digits == 4)
    })
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Whether one path segment is a package name that just joins both terms.
fn names_package_of(path: &str, a: &str, b: &str) -> bool {
    let (a, b) = (compact(a), compact(b));
    !a.is_empty()
        && !b.is_empty()
        && path.split('/').take(4).any(|segment| {
            let segment = compact(segment);
            segment.len() > a.len().max(b.len()) && segment.contains(&a) && segment.contains(&b)
        })
}

/// Whether `text` has `term` as its own word or phrase.
fn mentions(text: &str, term: &str) -> bool {
    let term = term.trim().to_lowercase();
    if term.is_empty() {
        return false;
    }
    text.match_indices(&term).any(|(at, found)| {
        let before = text[..at].chars().next_back();
        let after = text[at + found.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}

/// What to call a page whose title is missing: its host and path, as written.
fn page_name(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn mirror_key_for_tests(url: &str) -> Option<String> {
    mirror_key(url)
}
