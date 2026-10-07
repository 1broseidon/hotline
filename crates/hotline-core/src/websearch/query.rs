//! What a query says beyond its words: its language, a domain it names,
//! whether it compares two things or asks for the news, and which of its
//! distinctive words no result carried.

use regex::Regex;
use std::sync::LazyLock;

/// A language as the providers and the ranking know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    /// ISO 639-1, `fr`.
    pub code: &'static str,
    /// For a sentence a provider reads, `French`.
    pub name: &'static str,
}

pub const ENGLISH: Language = Language {
    code: "en",
    name: "English",
};

/// whatlang's ISO 639-3 codes for the languages with a 639-1 code a provider
/// takes. A language outside this list is detected but not passed on.
const LANGUAGES: [(&str, &str, &str); 27] = [
    ("eng", "en", "English"),
    ("fra", "fr", "French"),
    ("deu", "de", "German"),
    ("spa", "es", "Spanish"),
    ("ita", "it", "Italian"),
    ("por", "pt", "Portuguese"),
    ("nld", "nl", "Dutch"),
    ("rus", "ru", "Russian"),
    ("jpn", "ja", "Japanese"),
    ("kor", "ko", "Korean"),
    ("cmn", "zh", "Chinese"),
    ("ara", "ar", "Arabic"),
    ("tur", "tr", "Turkish"),
    ("pol", "pl", "Polish"),
    ("swe", "sv", "Swedish"),
    ("ukr", "uk", "Ukrainian"),
    ("hin", "hi", "Hindi"),
    ("ind", "id", "Indonesian"),
    ("vie", "vi", "Vietnamese"),
    ("ces", "cs", "Czech"),
    ("ell", "el", "Greek"),
    ("heb", "he", "Hebrew"),
    ("dan", "da", "Danish"),
    ("fin", "fi", "Finnish"),
    ("nob", "no", "Norwegian"),
    ("hun", "hu", "Hungarian"),
    ("ron", "ro", "Romanian"),
];

/// A language by its two-letter code.
pub fn language_by_code(code: &str) -> Option<Language> {
    LANGUAGES
        .iter()
        .find(|(_, two, _)| *two == code)
        .map(|(_, code, name)| Language { code, name })
}

fn language_of(info: &whatlang::Info) -> Option<Language> {
    let code = info.lang().code();
    LANGUAGES
        .iter()
        .find(|(three, _, _)| *three == code)
        .map(|(_, code, name)| Language { code, name })
}

/// Words that are plainly one language and not English, for the short queries
/// a trigram detector cannot tell: "Mont Everest altitude" is five words of
/// which one is French, and that one is enough. Words shared with English or
/// between these languages are left out.
const MARKERS: [(&str, &[&str]); 6] = [
    (
        "fr",
        &[
            "le",
            "les",
            "du",
            "des",
            "une",
            "est",
            "sont",
            "où",
            "qui",
            "quel",
            "quelle",
            "quels",
            "comment",
            "pourquoi",
            "mont",
            "montagne",
            "hauteur",
            "dans",
            "pour",
            "avec",
            "sur",
            "été",
            "très",
            "chez",
            "aujourd'hui",
        ],
    ),
    (
        "de",
        &[
            "der", "das", "und", "ist", "eine", "wie", "warum", "berg", "hoch", "höhe", "für",
            "von", "mit", "nicht", "oder", "welche", "wann",
        ],
    ),
    (
        "es",
        &[
            "los", "las", "del", "una", "cómo", "dónde", "qué", "porque", "monte", "montaña",
            "altura", "para", "con", "sin", "cuál", "cuándo",
        ],
    ),
    (
        "it",
        &[
            "il", "gli", "della", "delle", "è", "sono", "che", "come", "dove", "perché",
            "montagna", "altezza", "quale", "quando",
        ],
    ),
    (
        "pt",
        &[
            "os", "uma", "são", "onde", "porquê", "montanha", "altura", "para", "com", "qual",
            "quando", "você",
        ],
    ),
    (
        "nl",
        &[
            "het", "een", "zijn", "wat", "hoe", "waar", "waarom", "berg", "hoogte", "voor", "met",
            "van", "niet",
        ],
    ),
];

fn marker_language(text: &str) -> Option<Language> {
    let words = tokens(text);
    let mut scored: Vec<(usize, &str)> = MARKERS
        .iter()
        .map(|(code, markers)| {
            (
                words
                    .iter()
                    .filter(|w| markers.contains(&w.as_str()))
                    .count(),
                *code,
            )
        })
        .filter(|(hits, _)| *hits > 0)
        .collect();
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    match scored.as_slice() {
        [(_, code)] => language_by_code(code),
        [(first, code), (second, _), ..] if first > second => language_by_code(code),
        _ => None,
    }
}

/// A query's language, when it is known with confidence. A trigram detector
/// is sure of a sentence and guessing at a few words, so a confident detection
/// stands, and otherwise a word that only one language has decides.
pub fn query_language(text: &str) -> Option<Language> {
    if let Some(info) = whatlang::detect(text)
        && (info.is_reliable() || info.confidence() >= 0.6)
    {
        return language_of(&info);
    }
    marker_language(text)
}

/// The language a result's own words are in, if clear enough to say.
pub fn text_language(text: &str) -> Option<Language> {
    if text.chars().filter(|c| c.is_alphabetic()).count() < 25 {
        return None;
    }
    let info = whatlang::detect(text)?;
    (info.confidence() >= 0.5)
        .then(|| language_of(&info))
        .flatten()
}

/// Words too ordinary to say a query is distinctive, or to name its subject.
const COMMON: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "for",
    "with",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "at",
    "by",
    "from",
    "as",
    "how",
    "what",
    "why",
    "when",
    "where",
    "which",
    "who",
    "whom",
    "do",
    "does",
    "did",
    "can",
    "could",
    "should",
    "would",
    "will",
    "about",
    "into",
    "than",
    "then",
    "there",
    "their",
    "they",
    "them",
    "you",
    "your",
    "our",
    "not",
    "no",
    "yes",
    "vs",
    "versus",
    "app",
    "best",
    "top",
    "new",
    "latest",
    "news",
    "today",
    "recent",
    "world",
    "official",
    "free",
    "online",
    "download",
    "guide",
    "tutorial",
    "review",
    "reviews",
    "example",
    "examples",
    "compare",
    "comparison",
    "difference",
    "between",
    "which",
    "using",
    "after",
    "before",
    "first",
    "other",
    "years",
    "year",
    "still",
    "while",
    "these",
    "desktop",
    "application",
    "website",
    "page",
    "price",
    "prices",
    "cheap",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "week",
    "month",
    "breaking",
    "update",
    "updates",
    "list",
    "work",
    "works",
    "make",
    "makes",
    "need",
    "needs",
    "good",
    "better",
    "great",
    "simple",
    "using",
    "used",
    "uses",
    "use",
    "get",
    "gets",
    "many",
    "much",
    "some",
    "more",
    "most",
    "every",
    "each",
    // French and a few other very common function words, so a French query's
    // "pourquoi" is not a rare word.
    "le",
    "la",
    "les",
    "un",
    "une",
    "des",
    "du",
    "de",
    "et",
    "ou",
    "est",
    "sont",
    "pour",
    "dans",
    "avec",
    "sur",
    "que",
    "qui",
    "quoi",
    "pourquoi",
    "comment",
    "quel",
    "quelle",
    "mont",
    "der",
    "die",
    "das",
    "und",
    "ist",
    "ein",
    "eine",
    "el",
    "los",
    "las",
    "por",
    "qué",
    "como",
    "il",
    "che",
    "per",
    "con",
];

/// The words of a query, lowercase.
pub fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-' || c == '\''))
        .map(|word| word.trim_matches(|c: char| c == '.' || c == '-' || c == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Words that tell the query wants what is new.
const RECENCY_WORDS: [&str; 7] = [
    "latest",
    "news",
    "today",
    "breaking",
    "recent",
    "yesterday",
    "tonight",
];

static DOMAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[a-z0-9-]+\.)+[a-z]{2,24}$").expect("a fixed pattern"));

static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(20\d\d-\d\d-\d\d|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+\d{1,2}(?:st|nd|rd|th)?\b|\d{1,2}(?:st|nd|rd|th)?\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*)",
    )
    .expect("a fixed pattern")
});

static COMPARISON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)\s+(?:vs\.?|versus|or)\s+(.+)$").expect("a fixed pattern")
});

/// A query, read.
#[derive(Clone, Debug)]
pub struct Query {
    pub text: String,
    pub tokens: Vec<String>,
    /// A confidently detected language, English included.
    pub language: Option<Language>,
    /// A hostname the query names, `hotline.dev`.
    pub domains: Vec<String>,
    /// The two sides of "X vs Y", each a lowercase phrase.
    pub comparison: Option<(String, String)>,
    /// Asks for the news, or for now.
    pub recency: bool,
}

impl Query {
    pub fn parse(text: &str) -> Self {
        let text = text.trim().to_string();
        let tokens = tokens(&text);
        let domains: Vec<String> = tokens
            .iter()
            .filter(|token| {
                DOMAIN.is_match(token)
                    && token
                        .rsplit('.')
                        .next()
                        .is_some_and(|tld| !tld.chars().all(|c| c.is_ascii_digit()))
            })
            .cloned()
            .collect();
        let lower = text.to_lowercase();
        let comparison = COMPARISON
            .captures(&lower)
            .map(|c| (c[1].trim().to_string(), c[2].trim().to_string()));
        let comparison = comparison.or_else(|| {
            // "compare X and Y"
            let rest = lower
                .strip_prefix("compare ")
                .or_else(|| lower.strip_suffix(" comparison"))?;
            let (a, b) = rest
                .split_once(" and ")
                .or_else(|| rest.split_once(" with "))?;
            Some((a.trim().to_string(), b.trim().to_string()))
        });
        let recency =
            tokens.iter().any(|t| RECENCY_WORDS.contains(&t.as_str())) || DATE.is_match(&text);
        Self {
            language: query_language(&text),
            text,
            tokens,
            domains,
            comparison,
            recency,
        }
    }

    /// The language the query is in for ranking: what was detected, English
    /// when nothing was.
    pub fn effective_language(&self) -> Language {
        self.language.unwrap_or(ENGLISH)
    }

    /// Whether a language other than English was detected: the only case the
    /// results are steered toward a language.
    pub fn non_english(&self) -> Option<Language> {
        self.language.filter(|language| language.code != "en")
    }

    /// Words that say what the query is about, without the ones any query has.
    pub fn content_tokens(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .map(String::as_str)
            .filter(|t| t.chars().count() >= 2 && !COMMON.contains(t))
            .collect()
    }

    /// The words a result is expected to carry, and which are rare enough that
    /// their absence means something: those with a digit, or of five letters
    /// or more that are not ordinary words. A bare year is not one.
    pub fn distinctive(&self) -> Vec<&str> {
        self.content_tokens()
            .into_iter()
            .filter(|t| !is_year(t))
            .filter(|t| {
                t.chars().any(|c| c.is_ascii_digit()) && t.chars().count() >= 3
                    || t.chars().count() >= 5
            })
            .filter(|t| !t.contains('.'))
            .collect()
    }
}

fn is_year(token: &str) -> bool {
    token.len() == 4
        && token.chars().all(|c| c.is_ascii_digit())
        && (token.starts_with("19") || token.starts_with("20"))
}

/// A line to say when distinctive words of the query matched nothing.
/// `haystack` is every result's title, snippet and URL, lowercase.
pub fn missed_line(query: &Query, haystack: &str) -> Option<String> {
    let distinctive = query.distinctive();
    if distinctive.is_empty() {
        return None;
    }
    let missed: Vec<&str> = distinctive
        .iter()
        .copied()
        .filter(|token| {
            let stem = token
                .strip_suffix('s')
                .filter(|s| s.len() >= 4)
                .unwrap_or(token);
            !haystack.contains(stem)
        })
        .collect();
    if missed.is_empty() {
        return None;
    }
    let quoted: Vec<String> = missed.iter().map(|token| format!("\"{token}\"")).collect();
    let listed = match quoted.as_slice() {
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
        [] => return None,
    };
    let all_missed = query.content_tokens().iter().all(|token| {
        let stem = token
            .strip_suffix('s')
            .filter(|s| s.len() >= 4)
            .unwrap_or(token);
        !haystack.contains(stem)
    });
    Some(if all_missed {
        format!("No real matches: no result mentions {listed}.")
    } else {
        format!("No result mentions {listed}; these are matches for the other words only.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_domain_in_the_query_is_found_and_a_version_number_is_not() {
        assert_eq!(
            Query::parse("hotline.dev desktop app").domains,
            vec!["hotline.dev"]
        );
        assert_eq!(Query::parse("what is docs.rs").domains, vec!["docs.rs"]);
        assert!(Query::parse("rust 1.75 release").domains.is_empty());
        assert!(Query::parse("Mount Everest.").domains.is_empty());
    }

    #[test]
    fn comparisons_are_read_in_their_common_shapes() {
        for (q, a, b) in [
            ("tokio vs async-std", "tokio", "async-std"),
            ("Postgres versus MySQL", "postgres", "mysql"),
            ("tea or coffee", "tea", "coffee"),
            ("compare rust and go", "rust", "go"),
        ] {
            assert_eq!(
                Query::parse(q).comparison,
                Some((a.to_string(), b.to_string())),
                "{q}"
            );
        }
        assert!(Query::parse("tokio runtime").comparison.is_none());
    }

    #[test]
    fn recency_is_a_word_or_a_date() {
        for q in [
            "latest world news October 6 2026",
            "news today",
            "weather 2026-10-06",
            "events 6 october",
        ] {
            assert!(Query::parse(q).recency, "{q}");
        }
        assert!(!Query::parse("why is the sky blue").recency);
        assert!(!Query::parse("mount everest").recency);
    }

    #[test]
    fn a_french_query_is_detected_and_a_one_word_query_is_not() {
        let french = Query::parse("Pourquoi le ciel est-il bleu pendant la journée");
        assert_eq!(french.language.map(|l| l.code), Some("fr"));
        assert_eq!(french.non_english().map(|l| l.name), Some("French"));
        assert!(Query::parse("apple").language.is_none());
        assert!(Query::parse("tokio vs async-std").language.is_none());
        assert!(Query::parse("Mount Everest").language.is_none());
        // Short, and one word that is French and not English is enough.
        assert_eq!(
            Query::parse("Mont Everest altitude")
                .language
                .map(|l| l.code),
            Some("fr")
        );
        assert_eq!(
            Query::parse("Wie hoch ist der Mount Everest")
                .language
                .map(|l| l.code),
            Some("de")
        );
        assert_eq!(Query::parse("apple").effective_language().code, "en");
    }

    #[test]
    fn distinctive_words_have_a_digit_or_are_long_and_not_ordinary() {
        let q = Query::parse("xqzflarnib 98421 protocol");
        assert_eq!(q.distinctive(), vec!["xqzflarnib", "98421", "protocol"]);
        let q = Query::parse("latest world news October 6 2026");
        assert!(q.distinctive().is_empty(), "{:?}", q.distinctive());
        assert!(Query::parse("why is the sky blue").distinctive().is_empty());
    }

    #[test]
    fn the_missed_line_names_what_no_result_carried() {
        let q = Query::parse("xqzflarnib 98421 protocol");
        let haystack = "clinical trial protocol for adults https://trials.example/protocol";
        assert_eq!(
            missed_line(&q, haystack).unwrap(),
            "No result mentions \"xqzflarnib\" or \"98421\"; these are matches for the other words only."
        );
        let none = "something unrelated";
        assert_eq!(
            missed_line(&q, none).unwrap(),
            "No real matches: no result mentions \"xqzflarnib\", \"98421\" or \"protocol\"."
        );
        let all = "xqzflarnib 98421 protocol";
        assert_eq!(missed_line(&q, all), None);
        assert_eq!(missed_line(&Query::parse("why is the sky blue"), "x"), None);
    }
}
