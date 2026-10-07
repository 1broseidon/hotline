//! What a result says of itself, made fit to read: error and interstitial
//! pages are dropped, and a snippet loses its repeats and its page chrome.

use regex::Regex;
use std::sync::LazyLock;

/// An HTTP status line standing where a title or snippet should: "301 Moved
/// Permanently", "403 Forbidden", "404 Not Found".
static STATUS_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\W*(30[1-8]|4\d\d|5\d\d)\b\W*(moved|found|see other|not modified|temporary redirect|permanent redirect|bad request|unauthorized|payment required|forbidden|not found|method not allowed|not acceptable|request timeout|gone|too many requests|internal server error|not implemented|bad gateway|service unavailable|gateway time-?out)",
    )
    .expect("a fixed pattern")
});

static CLOUDFLARE_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)error\s*(code)?:?\s*\d{3,4}|\b[345]\d\d\b|ray id").expect("a fixed pattern")
});

/// Words an interstitial or block page says instead of its content.
const INTERSTITIALS: [&str; 12] = [
    "just a moment",
    "attention required",
    "access denied",
    "enable javascript",
    "javascript is disabled",
    "captcha",
    "checking your browser",
    "verify you are human",
    "are you a robot",
    "unusual traffic",
    "request blocked",
    "page not found",
];

/// Whether a result is an error or interstitial page rather than a page.
pub fn is_junk(title: &str, snippet: &str) -> bool {
    let title = title.trim();
    let snippet = snippet.trim();
    if STATUS_LINE.is_match(title) || STATUS_LINE.is_match(snippet) {
        return true;
    }
    let title_lower = title.to_lowercase();
    if INTERSTITIALS
        .iter()
        .any(|phrase| title_lower.contains(phrase))
    {
        return true;
    }
    // A snippet is a block page when it says so up front or is nothing else.
    let lower = snippet.to_lowercase();
    let head: String = lower.chars().take(120).collect();
    let short = lower.chars().count() < 300;
    if INTERSTITIALS
        .iter()
        .any(|phrase| head.contains(phrase) || (short && lower.contains(phrase)))
    {
        return true;
    }
    lower.contains("cloudflare") && short && CLOUDFLARE_ERROR.is_match(&lower)
}

/// Words that mark a line as site furniture.
const NAV_WORDS: [&str; 30] = [
    "home",
    "menu",
    "subscribe",
    "sign",
    "login",
    "log",
    "register",
    "search",
    "contact",
    "about",
    "skip",
    "content",
    "navigation",
    "toggle",
    "cart",
    "account",
    "privacy",
    "terms",
    "cookies",
    "cookie",
    "newsletter",
    "follow",
    "share",
    "facebook",
    "twitter",
    "instagram",
    "youtube",
    "shop",
    "donate",
    "language",
];

const SEPARATORS: [char; 6] = ['|', '·', '•', '»', '›', '/'];

fn words(line: &str) -> Vec<&str> {
    line.split(|c: char| c.is_whitespace() || SEPARATORS.contains(&c))
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_title_case(word: &str) -> bool {
    let mut chars = word.chars().filter(|c| c.is_alphabetic());
    match chars.next() {
        Some(first) => first.is_uppercase() || word.chars().all(|c| !c.is_alphabetic()),
        None => true,
    }
}

/// A line made of menu items and links rather than sentences.
fn is_chrome(line: &str) -> bool {
    let tokens = words(line);
    if tokens.is_empty() {
        return true;
    }
    let nav = tokens
        .iter()
        .filter(|token| {
            let lower = token
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            NAV_WORDS.contains(&lower.as_str())
        })
        .count();
    let title_case = tokens.iter().filter(|token| is_title_case(token)).count();
    let separators = line.chars().filter(|c| SEPARATORS.contains(c)).count();
    let ratio = title_case as f64 / tokens.len() as f64;
    let sentence = line.trim_end().ends_with(['.', '!', '?', '"', '”']) && tokens.len() >= 6;
    if sentence {
        return false;
    }
    // Several nav words in a short line, or a run of capitalised fragments cut
    // by separators, is a menu. Pure capitalisation is not enough alone:
    // "Mount Everest Facts" is a heading, not a menu.
    (nav >= 2 && nav * 4 >= tokens.len())
        || (nav >= 1 && separators >= 2 && ratio >= 0.6)
        || (separators >= 3 && tokens.len() <= 18 && ratio >= 0.7)
        || (tokens.len() >= 6 && ratio >= 0.85 && separators >= 1)
}

fn normalize(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Markdown reduced to its words: links to their text, images and the marks
/// of headings, emphasis and code gone.
fn strip_markdown(text: &str) -> String {
    static IMAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("a fixed pattern"));
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("a fixed pattern"));
    static BARE_URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"https?://\S+").expect("a fixed pattern"));
    // A wiki infobox comes through as a table: footnote marks such as
    // `[[note 2]](` (often cut off before their target), `<br>` inside cells,
    // and runs of `|` between them. The cells read fine with a dot between.
    static FOOTNOTE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[\[[^\]]*\]\](\([^)\s]*\)?)?").expect("a fixed pattern"));
    static BREAK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("a fixed pattern"));
    static CELLS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s*\|(\s*\|)*\s*").expect("a fixed pattern"));
    // An extractor's placeholder for a field it could not find says nothing.
    static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?im)^\s*(published|updated|date|author)\s*:\s*(n/?a|none|unknown)\s*$")
            .expect("a fixed pattern")
    });
    let text = IMAGE.replace_all(text, "");
    let text = FOOTNOTE.replace_all(&text, "");
    let text = LINK.replace_all(&text, "$1");
    let text = BARE_URL.replace_all(&text, "");
    let text = BREAK.replace_all(&text, " ");
    let text = PLACEHOLDER.replace_all(&text, "");
    text.lines()
        .map(|line| CELLS.replace_all(line, " · ").to_string())
        .map(|line| line.trim().trim_matches(['·', ' ']).to_string())
        .map(|line| {
            line.trim()
                .trim_start_matches(['#', '>', '-', '*'])
                .trim()
                .to_string()
        })
        .map(|line| line.replace(['*', '`'], ""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A snippet made readable: markup stripped, chrome lines dropped, and any
/// sentence or paragraph said twice kept once. The flag says nothing
/// meaningful was left, so the result ranks down.
pub fn clean_snippet(raw: &str) -> (String, bool) {
    let stripped = strip_markdown(raw);
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for line in stripped
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        // A line can still carry several sentences, each of which may repeat.
        for piece in split_sentences(line) {
            if is_chrome(&piece) {
                continue;
            }
            let key = normalize(&piece);
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            kept.push(piece);
        }
    }
    let mut text = kept.join(" ");
    text = collapse_doubled(&text);
    let meaningful = text.split_whitespace().count() >= 3;
    (text, !meaningful)
}

/// Sentences of a line, each keeping its end mark.
fn split_sentences(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = line.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        current.push(*c);
        let ends = matches!(c, '.' | '!' | '?')
            && chars.get(i + 1).is_none_or(|next| next.is_whitespace());
        // "8.8 km" and "e.g." do not end a sentence: the next word must open
        // with a capital.
        let capital_next = chars
            .iter()
            .skip(i + 1)
            .find(|c| !c.is_whitespace())
            .is_none_or(|next| next.is_uppercase() || !next.is_alphabetic());
        if ends && capital_next && current.trim().chars().count() > 3 {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// "X X" with no mark between the copies: one copy.
fn collapse_doubled(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() >= 6 && words.len().is_multiple_of(2) {
        let (a, b) = words.split_at(words.len() / 2);
        if normalize(&a.join(" ")) == normalize(&b.join(" ")) {
            return a.join(" ");
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_infobox_reads_as_cells_and_placeholders_say_nothing() {
        let (text, thin) = clean_snippet(
            "| Mount Everest | | Elevation | 8,848.86m (29,031.7ft)[[note 2]]( 1st | Location<br>Nepal |",
        );
        assert!(!thin);
        assert!(
            !text.contains('|') && !text.contains("[[") && !text.contains("<br>"),
            "{text}"
        );
        assert!(text.contains("Elevation · 8,848.86m"), "{text}");
        let (text, thin) = clean_snippet("Published: N/A");
        assert!(text.is_empty() && thin, "{text}");
    }

    #[test]
    fn error_and_interstitial_pages_are_junk() {
        for (title, snippet) in [
            ("301 Moved Permanently", ""),
            ("RFC 9110", "301 Moved Permanently cloudflare"),
            ("Fine title", "403 Forbidden"),
            ("404 Not Found", "x"),
            ("Just a moment...", ""),
            ("Attention Required! | Cloudflare", "x"),
            ("Access denied", ""),
            ("Page", "Please enable JavaScript to continue"),
            ("Page", "Complete the captcha to prove you are human"),
            ("Page", "Cloudflare Ray ID: 8f2a error 1020"),
        ] {
            assert!(is_junk(title, snippet), "{title:?} / {snippet:?}");
        }
    }

    #[test]
    fn real_pages_that_mention_such_words_are_not_junk() {
        for (title, snippet) in [
            (
                "Cloudflare | Connectivity cloud",
                "Cloudflare protects and accelerates websites, apps and teams.",
            ),
            (
                "HTTP 404 explained",
                &"The 404 status code means the server cannot find the page. ".repeat(8),
            ),
            (
                "Mount Everest - Wikipedia",
                "Mount Everest is Earth's highest mountain above sea level.",
            ),
        ] {
            assert!(!is_junk(title, snippet), "{title:?}");
        }
    }

    #[test]
    fn a_paragraph_pasted_twice_is_kept_once() {
        let text =
            "Everest is the highest mountain on Earth. Everest is the highest mountain on Earth.";
        assert_eq!(
            clean_snippet(text).0,
            "Everest is the highest mountain on Earth."
        );
        let doubled = "Everest rises to 8849 metres Everest rises to 8849 metres";
        assert_eq!(clean_snippet(doubled).0, "Everest rises to 8849 metres");
        let paragraphs =
            "The sky is blue because of scattering.\nThe sky is blue because of scattering.";
        assert_eq!(
            clean_snippet(paragraphs).0,
            "The sky is blue because of scattering."
        );
    }

    #[test]
    fn menus_are_dropped_and_the_sentence_beside_them_stays() {
        let text = "Home | News | Sport | Subscribe | Sign in\nThe committee approved the measure on Tuesday after a long debate.\nMenu Search Log in";
        let (cleaned, thin) = clean_snippet(text);
        assert_eq!(
            cleaned,
            "The committee approved the measure on Tuesday after a long debate."
        );
        assert!(!thin);
    }

    #[test]
    fn a_snippet_that_is_only_chrome_is_thin() {
        let (cleaned, thin) = clean_snippet("Home · About · Contact · Subscribe · Log in");
        assert_eq!(cleaned, "");
        assert!(thin);
        assert!(clean_snippet("").1);
    }

    #[test]
    fn markdown_is_reduced_to_words_and_headings_are_not_chrome() {
        let (cleaned, thin) = clean_snippet(
            "## [Spain approves housing measures](https://example.com/a)\n![pic](https://x/y.png)\nMount Everest Facts\nMinisters said the plan would take effect next month.",
        );
        assert!(
            cleaned.starts_with("Spain approves housing measures"),
            "{cleaned}"
        );
        assert!(cleaned.contains("Mount Everest Facts"));
        assert!(!cleaned.contains("http"));
        assert!(!thin);
    }

    #[test]
    fn a_decimal_does_not_end_a_sentence() {
        let (cleaned, _) = clean_snippet("The peak is 8.8 km high. The peak is 8.8 km high.");
        assert_eq!(cleaned, "The peak is 8.8 km high.");
    }
}
