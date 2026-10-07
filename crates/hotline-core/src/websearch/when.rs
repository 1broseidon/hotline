//! Time in a query and in a result: the days a query asks about, and the date a
//! page carries, so a news search can tell today's story from a 2023 one.

use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;
use std::sync::LazyLock;

/// The days a query is about, first and last, both included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl Window {
    /// Whether a date falls in the window, a day either side allowed: a story
    /// is dated in its own timezone and a query is not.
    pub fn near(&self, date: NaiveDate) -> bool {
        date >= self.from - Duration::days(1) && date <= self.to + Duration::days(1)
    }

    /// Whether a date is clearly before the window: a month or more earlier.
    pub fn long_before(&self, date: NaiveDate) -> bool {
        date < self.from - Duration::days(30)
    }
}

const MONTH: &str = r"(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)";

static ISO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{4})-(\d{2})-(\d{2})").expect("a fixed pattern"));
static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b{MONTH}\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?\b(?:,?\s+(\d{{4}})\b)?"
    ))
    .expect("a fixed pattern")
});
static DAY_MONTH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(\d{{1,2}})(?:st|nd|rd|th)?\s+{MONTH}\b\.?(?:,?\s+(\d{{4}})\b)?"
    ))
    .expect("a fixed pattern")
});
static SLASHED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{1,2})/(\d{1,2})/(\d{4})\b").expect("a fixed pattern"));
static AGO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(\d{1,3})\s*(min|mins|minutes?|hrs?|hours?|h|days?|d|weeks?|wks?|w)\s+ago\b")
        .expect("a fixed pattern")
});
static RELATIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(today|tonight|yesterday|this week|past week|this morning)\b")
        .expect("a fixed pattern")
});
static LATEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(latest|breaking|news|recent|headlines|current)\b").expect("a fixed pattern")
});

fn month_number(name: &str) -> Option<u32> {
    const NAMES: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower = name.to_lowercase();
    NAMES
        .iter()
        .position(|short| lower.starts_with(short))
        .map(|at| at as u32 + 1)
}

/// A date without a year is the latest such day that is not in the future.
fn assume_year(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year > today + Duration::days(2) {
        NaiveDate::from_ymd_opt(today.year() - 1, month, day)
    } else {
        Some(this_year)
    }
}

/// Whether what follows a month and day makes it the name of an event.
fn names_an_event(rest: &str) -> bool {
    let next = rest
        .trim_start()
        .split(|c: char| !c.is_alphabetic())
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(
        next.as_str(),
        "attack" | "attacks" | "massacre" | "anniversary" | "war" | "bombing" | "bombings"
    )
}

/// Every explicit calendar date in `text`, with the text that wrote it.
fn written_dates(text: &str, today: NaiveDate) -> Vec<(NaiveDate, String)> {
    let mut found: Vec<(usize, NaiveDate, String)> = Vec::new();
    for caps in ISO.captures_iter(text) {
        let date = NaiveDate::from_ymd_opt(
            caps[1].parse().unwrap_or(0),
            caps[2].parse().unwrap_or(0),
            caps[3].parse().unwrap_or(0),
        );
        if let (Some(date), Some(whole)) = (date, caps.get(0)) {
            found.push((whole.start(), date, whole.as_str().to_string()));
        }
    }
    for caps in MONTH_DAY.captures_iter(text) {
        let (Some(month), Ok(day)) = (month_number(&caps[1]), caps[2].parse::<u32>()) else {
            continue;
        };
        let whole = caps.get(0);
        // "October 7 attacks" is an event's name, not a day.
        if caps.get(3).is_none() && whole.is_some_and(|w| names_an_event(&text[w.end()..])) {
            continue;
        }
        let date = match caps.get(3) {
            Some(year) => NaiveDate::from_ymd_opt(year.as_str().parse().unwrap_or(0), month, day),
            None => assume_year(month, day, today),
        };
        if let (Some(date), Some(whole)) = (date, whole) {
            found.push((whole.start(), date, whole.as_str().to_string()));
        }
    }
    for caps in DAY_MONTH.captures_iter(text) {
        let (Ok(day), Some(month)) = (caps[1].parse::<u32>(), month_number(&caps[2])) else {
            continue;
        };
        let date = match caps.get(3) {
            Some(year) => NaiveDate::from_ymd_opt(year.as_str().parse().unwrap_or(0), month, day),
            None => assume_year(month, day, today),
        };
        if let (Some(date), Some(whole)) = (date, caps.get(0)) {
            found.push((whole.start(), date, whole.as_str().to_string()));
        }
    }
    for caps in SLASHED.captures_iter(text) {
        let date = NaiveDate::from_ymd_opt(
            caps[3].parse().unwrap_or(0),
            caps[1].parse().unwrap_or(0),
            caps[2].parse().unwrap_or(0),
        );
        if let (Some(date), Some(whole)) = (date, caps.get(0)) {
            found.push((whole.start(), date, whole.as_str().to_string()));
        }
    }
    found.sort_by_key(|entry| entry.0);
    found
        .into_iter()
        .map(|(_, date, written)| (date, written))
        .collect()
}

/// "34 min ago", "2 hours ago", "3 days ago", as the day it names.
fn ago(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let caps = AGO.captures(text)?;
    let count: i64 = caps[1].parse().ok()?;
    let unit = caps[2].to_lowercase();
    let days = match unit.chars().next()? {
        'm' => 0,
        'h' => count / 24,
        'd' => count,
        'w' => count * 7,
        _ => 0,
    };
    Some(today - Duration::days(days))
}

/// What a query asks for in time: the window, and the words that asked, which
/// are not about the subject.
pub fn query_window(text: &str, today: NaiveDate) -> (Option<Window>, Vec<String>) {
    let mut consumed: Vec<String> = Vec::new();
    let mut eat = |phrase: &str| {
        consumed.extend(
            phrase
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_string),
        );
    };
    let dates = written_dates(text, today);
    for (_, written) in &dates {
        eat(written);
    }
    let relative = RELATIVE.find(text).map(|m| m.as_str().to_lowercase());
    if let Some(relative) = &relative {
        eat(relative);
    }
    let latest = LATEST.is_match(text);
    for word in LATEST.find_iter(text) {
        eat(word.as_str());
    }
    let window = if let (Some(first), Some(last)) = (dates.first(), dates.last()) {
        let (a, b) = (first.0.min(last.0), first.0.max(last.0));
        Some(Window { from: a, to: b })
    } else if let Some(relative) = relative {
        let day = match relative.as_str() {
            "yesterday" => Window {
                from: today - Duration::days(1),
                to: today - Duration::days(1),
            },
            "this week" | "past week" => Window {
                from: today - Duration::days(6),
                to: today,
            },
            _ => Window {
                from: today,
                to: today,
            },
        };
        Some(day)
    } else if latest {
        Some(Window {
            from: today - Duration::days(7),
            to: today,
        })
    } else {
        None
    };
    (window, consumed)
}

/// The day a result says it was published: its provider's date when it gave
/// one, else the first date or relative time its title or snippet carries.
pub fn result_date(
    published: Option<&str>,
    title: &str,
    snippet: &str,
    today: NaiveDate,
) -> Option<NaiveDate> {
    if let Some(published) = published {
        if let Some(date) = written_dates(published, today).first() {
            return Some(date.0);
        }
        if let Some(date) = ago(published, today) {
            return Some(date);
        }
        if published.trim().eq_ignore_ascii_case("yesterday") {
            return Some(today - Duration::days(1));
        }
    }
    let head: String = format!("{title} {snippet}").chars().take(400).collect();
    if let Some(date) = ago(&head, today) {
        return Some(date);
    }
    written_dates(&head, today).first().map(|entry| entry.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn dates_in_every_written_form_are_found() {
        let today = day(2026, 10, 7);
        for (text, want) in [
            ("latest world news October 7 2026", day(2026, 10, 7)),
            ("news Oct 6, 2026", day(2026, 10, 6)),
            ("news 7 Oct", day(2026, 10, 7)),
            ("news 2026-10-07", day(2026, 10, 7)),
            ("news 3 March 2024", day(2024, 3, 3)),
            ("news Dec 25", day(2025, 12, 25)),
        ] {
            let (window, _) = query_window(text, today);
            assert_eq!(window.map(|w| w.from), Some(want), "{text}");
        }
    }

    #[test]
    fn relative_words_make_windows_and_the_date_words_are_consumed() {
        let today = day(2026, 10, 7);
        let (window, words) = query_window("what happened yesterday", today);
        assert_eq!(window.unwrap().from, day(2026, 10, 6));
        assert!(words.contains(&"yesterday".to_string()));
        let (window, _) = query_window("news this week", today);
        assert_eq!(window.unwrap().from, day(2026, 10, 1));
        let (window, words) = query_window("latest world news October 7 2026", today);
        assert_eq!(
            window,
            Some(Window {
                from: day(2026, 10, 7),
                to: day(2026, 10, 7)
            })
        );
        for word in ["october", "7", "2026", "latest", "news"] {
            assert!(words.contains(&word.to_string()), "{word} in {words:?}");
        }
        assert!(!words.contains(&"world".to_string()));
        assert_eq!(query_window("why is the sky blue", today).0, None);
        assert_eq!(query_window("mount everest", today).0, None);
        let (window, _) = query_window("latest world news", today);
        assert_eq!(window.unwrap().from, day(2026, 9, 30));
    }

    #[test]
    fn a_result_is_dated_by_its_provider_then_its_own_words() {
        let today = day(2026, 10, 7);
        let date = |p: Option<&str>, t: &str, s: &str| result_date(p, t, s, today);
        assert_eq!(
            date(Some("2026-10-05T12:00:24Z"), "", ""),
            Some(day(2026, 10, 5))
        );
        assert_eq!(date(Some("2 hours ago"), "", ""), Some(today));
        assert_eq!(date(None, "", "34 min ago Iran war"), Some(today));
        assert_eq!(
            date(None, "", "On October 7, 2023 Hamas"),
            Some(day(2023, 10, 7))
        );
        assert_eq!(
            date(None, "", "House passes it 09/16/2026 06:54 PM"),
            Some(day(2026, 9, 16))
        );
        assert_eq!(date(None, "Plain page", "no date here"), None);
        assert_eq!(date(Some("N/A"), "", ""), None);
    }

    #[test]
    fn the_window_allows_a_day_either_side_and_calls_a_month_before_old() {
        let w = Window {
            from: day(2026, 10, 7),
            to: day(2026, 10, 7),
        };
        assert!(w.near(day(2026, 10, 6)) && w.near(day(2026, 10, 8)));
        assert!(!w.near(day(2026, 10, 1)));
        assert!(w.long_before(day(2026, 9, 1)) && !w.long_before(day(2026, 9, 20)));
    }
}
