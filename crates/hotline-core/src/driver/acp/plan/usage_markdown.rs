//! Claude Code's `/usage`, as its ACP adapter answers it.
//!
//! The adapter (`@agentclientprotocol/claude-agent-acp`) takes a prompt that
//! is exactly `/usage` (its `isUsageCommandText`: the text, trimmed, and
//! nothing else in the prompt) as a local command: Claude Code reads the
//! account's limits with a control request and no model call, and the
//! adapter says them as one agent message in Markdown (`formatUsageResponse`
//! in its `usage-markdown.js`). The limits are under `### Limits`, a line
//! each:
//!
//! ```text
//! **5-hour limit** — **38%** · Resets Oct 29, 9:00 PM CDT
//! **Weekly · all models** — **61%** · Resets Nov 2, 9:00 AM CST
//! **Weekly · Opus** — **40%** · Resets Nov 2, 9:00 AM CST
//! ```
//!
//! The use is already a percentage, and the reset is said in the adapter's
//! own time zone with no year, so the year is the one that puts the reset
//! nearest now.

use super::{order, status_of};
use crate::contract::{PlanUsage, PlanWindow};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, TimeZone, Utc};

/// The prompt the adapter answers locally.
pub(crate) const USAGE_COMMAND: &str = "/usage";

/// The windows the answer to `/usage` lists, as of `now` (milliseconds).
/// `None` when it lists none: an account with no plan limits, an API key, or
/// an answer in some other shape. That is no reading, not an empty one.
pub(crate) fn parse(text: &str, now: i64) -> Option<PlanUsage> {
    let mut lines = text.lines().map(str::trim);
    lines.by_ref().find(|line| *line == "### Limits")?;
    let mut windows: Vec<PlanWindow> = Vec::new();
    for line in lines {
        if line.starts_with('#') || line == "---" {
            break;
        }
        let Some(window) = limit_line(line, now) else {
            continue;
        };
        // Two model names that come to one window: the first said stands.
        if !windows.iter().any(|one| one.kind == window.kind) {
            windows.push(window);
        }
    }
    if windows.is_empty() {
        return None;
    }
    windows.sort_by_key(|one| order(&one.kind));
    Some(PlanUsage { windows, at: now })
}

/// One `**label** — **38%** · Resets …` line.
fn limit_line(line: &str, now: i64) -> Option<PlanWindow> {
    let rest = line.strip_prefix("**")?;
    let (label, rest) = rest.split_once("** — **")?;
    let (used, rest) = rest.split_once("%**")?;
    let used: f64 = used.trim().parse().ok()?;
    let used = used.clamp(0.0, 100.0);
    let resets_at = rest
        .trim()
        .strip_prefix("· Resets ")
        .and_then(|when| reset_at(when, now));
    Some(PlanWindow {
        kind: kind_of(&unescape(label)),
        used_percent: Some(used),
        resets_at,
        status: status_of(used),
    })
}

/// The window a line's label names, in the words the per-reply report uses.
fn kind_of(label: &str) -> String {
    match label {
        "5-hour limit" => return "five_hour".into(),
        "Weekly · all models" => return "seven_day".into(),
        _ => {}
    }
    if let Some(model) = label.strip_prefix("Weekly · ") {
        // Claude Code names a model's own window by its display name, which
        // may carry a version ("Opus 4.5"): the family is the window.
        let family = model.split_whitespace().next().unwrap_or_default();
        return match family.to_ascii_lowercase().as_str() {
            "opus" => "seven_day_opus".into(),
            "sonnet" => "seven_day_sonnet".into(),
            _ => format!("seven_day_{}", slug(model)),
        };
    }
    slug(label)
}

fn slug(words: &str) -> String {
    let mut slug = String::new();
    for c in words.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    slug.trim_end_matches('_').to_string()
}

/// The adapter escapes Markdown in a label with a backslash.
fn unescape(label: &str) -> String {
    let mut plain = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => plain.extend(chars.next()),
            c => plain.push(c),
        }
    }
    plain
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `Oct 29, 9:00 PM CDT` as milliseconds: the next such moment on or after
/// now, give or take a day for a reading taken just as a window reset.
fn reset_at(when: &str, now: i64) -> Option<i64> {
    // Intl puts a narrow no-break space before the AM/PM in some versions.
    let when: String = when
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let mut words = when.split_whitespace();
    let month = words.next()?;
    let month = MONTHS.iter().position(|name| *name == month)? as u32 + 1;
    let day: u32 = words.next()?.trim_end_matches(',').parse().ok()?;
    let (hour, minute) = words.next()?.split_once(':')?;
    let (mut hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    let mut zone = words.next();
    match zone {
        Some("AM") => {
            hour %= 12;
            zone = words.next();
        }
        Some("PM") => {
            hour = hour % 12 + 12;
            zone = words.next();
        }
        _ => {}
    }
    let offset = zone.and_then(offset_of);
    let year = DateTime::<Utc>::from_timestamp_millis(now)?.year();
    let at = |year: i32| -> Option<i64> {
        let local = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)?;
        let at = match offset {
            Some(offset) => offset
                .from_local_datetime(&local)
                .single()?
                .timestamp_millis(),
            // A zone this does not know is the adapter's, which runs on this
            // machine: the desk's own.
            None => Local
                .from_local_datetime(&local)
                .earliest()?
                .timestamp_millis(),
        };
        Some(at)
    };
    const DAY: i64 = 86_400_000;
    [year - 1, year, year + 1]
        .into_iter()
        .filter_map(at)
        .find(|at| *at >= now - DAY)
}

/// The offset of a zone as `Intl` names it in English: an abbreviation for
/// the zones it has one for, otherwise `GMT+5:30`.
fn offset_of(zone: &str) -> Option<FixedOffset> {
    let hours = match zone {
        "UTC" | "GMT" => 0,
        "EST" => -5,
        "EDT" | "AST" => -4,
        "CST" => -6,
        "CDT" => -5,
        "MST" => -7,
        "MDT" => -6,
        "PST" => -8,
        "PDT" => -7,
        "AKST" => -9,
        "AKDT" => -8,
        "HST" => -10,
        "HDT" => -9,
        _ => {
            let signed = zone
                .strip_prefix("GMT")
                .or_else(|| zone.strip_prefix("UTC"))?;
            let sign = match signed.chars().next()? {
                '+' => 1,
                '-' | '−' => -1,
                _ => return None,
            };
            let digits = &signed[signed.chars().next()?.len_utf8()..];
            let (hours, minutes) = digits.split_once(':').unwrap_or((digits, "0"));
            let seconds = hours.parse::<i32>().ok()? * 3600 + minutes.parse::<i32>().ok()? * 60;
            return FixedOffset::east_opt(sign * seconds);
        }
    };
    FixedOffset::east_opt(hours * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::PlanStatus;

    // Each of these is what `formatUsageResponse` of claude-agent-acp
    // 0.99.0 wrote under TZ=America/Chicago, for the response named.

    /// The 5-hour and weekly windows.
    const BOTH: &str = "## Usage\n\n> Claude max subscription usage\n\n### Limits\n\n**5-hour limit** — **38%** · Resets Oct 29, 9:00 PM CDT\n\n`████████░░░░░░░░░░░░`\n\n**Weekly · all models** — **61%** · Resets Nov 2, 9:00 AM CST\n\n`████████████░░░░░░░░`\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |\n\n| Breakdown | Tokens |\n|:--|--:|\n| Input | 0 |\n| Output | 0 |\n| Cache read | 0 |\n| Cache write | 0 |";

    /// A model's own window, by its display name, the 5-hour used up and
    /// the weekly with no reset.
    const SCOPED: &str = "## Usage\n\n> Claude max subscription usage\n\n### Limits\n\n**5-hour limit** — **100%** · Resets Oct 29, 9:00 PM CDT\n\n`████████████████████`\n\n**Weekly · all models** — **82.5%**\n\n`█████████████████░░░`\n\n**Weekly · Opus 4.5** — **12%** · Resets Nov 2, 9:00 AM CST\n\n`██░░░░░░░░░░░░░░░░░░`\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |\n\n| Breakdown | Tokens |\n|:--|--:|\n| Input | 0 |\n| Output | 0 |\n| Cache read | 0 |\n| Cache write | 0 |";

    /// The older per-family weekly windows.
    const FAMILIES: &str = "## Usage\n\n> Claude pro subscription usage\n\n### Limits\n\n**Weekly · all models** — **5%** · Resets Nov 2, 9:00 AM CST\n\n`█░░░░░░░░░░░░░░░░░░░`\n\n**Weekly · Opus** — **40%** · Resets Nov 2, 9:00 AM CST\n\n`████████░░░░░░░░░░░░`\n\n**Weekly · Sonnet** — **3%** · Resets Nov 2, 9:00 AM CST\n\n`█░░░░░░░░░░░░░░░░░░░`\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |\n\n| Breakdown | Tokens |\n|:--|--:|\n| Input | 0 |\n| Output | 0 |\n| Cache read | 0 |\n| Cache write | 0 |";

    /// An account with no plan limits to report.
    const NO_LIMITS: &str = "## Usage\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |\n\n| Breakdown | Tokens |\n|:--|--:|\n| Input | 0 |\n| Output | 0 |\n| Cache read | 0 |\n| Cache write | 0 |";

    /// A reset in the new year, read at the end of the old one.
    const NEW_YEAR: &str = "## Usage\n\n> Claude max subscription usage\n\n### Limits\n\n**5-hour limit** — **7%** · Resets Jan 1, 9:30 PM CST\n\n`█░░░░░░░░░░░░░░░░░░░`\n\n---\n\n### This session\n\n| Cost | API time | Active |\n|:--|:--|:--|\n| $0.12 | 4s | 9s |\n\n| Breakdown | Tokens |\n|:--|--:|\n| Input | 0 |\n| Output | 0 |\n| Cache read | 0 |\n| Cache write | 0 |";

    fn ms(rfc3339: &str) -> i64 {
        DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .timestamp_millis()
    }

    fn lines(plan: &PlanUsage) -> Vec<(&str, Option<f64>, Option<i64>, PlanStatus)> {
        plan.windows
            .iter()
            .map(|one| {
                (
                    one.kind.as_str(),
                    one.used_percent,
                    one.resets_at,
                    one.status,
                )
            })
            .collect()
    }

    #[test]
    fn the_five_hour_and_weekly_lines_read_as_their_windows_with_resets_this_year() {
        let now = ms("2026-10-29T15:00:00Z");
        let plan = parse(BOTH, now).unwrap();
        assert_eq!(plan.at, now);
        assert_eq!(
            lines(&plan),
            [
                (
                    "five_hour",
                    Some(38.0),
                    Some(ms("2026-10-30T02:00:00Z")),
                    PlanStatus::Allowed
                ),
                (
                    "seven_day",
                    Some(61.0),
                    Some(ms("2026-11-02T15:00:00Z")),
                    PlanStatus::Allowed
                ),
            ]
        );
    }

    #[test]
    fn a_models_own_window_reads_as_its_family_and_a_missing_reset_is_no_reset() {
        let now = ms("2026-10-29T15:00:00Z");
        let plan = parse(SCOPED, now).unwrap();
        assert_eq!(
            lines(&plan),
            [
                (
                    "five_hour",
                    Some(100.0),
                    Some(ms("2026-10-30T02:00:00Z")),
                    PlanStatus::Limited
                ),
                ("seven_day", Some(82.5), None, PlanStatus::Warning),
                (
                    "seven_day_opus",
                    Some(12.0),
                    Some(ms("2026-11-02T15:00:00Z")),
                    PlanStatus::Allowed
                ),
            ]
        );
        let plan = parse(FAMILIES, now).unwrap();
        assert_eq!(
            plan.windows
                .iter()
                .map(|one| one.kind.as_str())
                .collect::<Vec<_>>(),
            ["seven_day", "seven_day_opus", "seven_day_sonnet"]
        );
    }

    #[test]
    fn an_answer_with_no_limits_is_no_reading() {
        assert_eq!(parse(NO_LIMITS, ms("2026-10-29T15:00:00Z")), None);
        assert_eq!(parse("", 0), None);
        // Claude Code's own text, when the adapter could not structure it.
        assert_eq!(parse("Current session: 12% used", 0), None);
    }

    #[test]
    fn a_reset_that_would_be_past_this_year_is_next_years() {
        let now = ms("2026-12-30T12:00:00Z");
        let plan = parse(NEW_YEAR, now).unwrap();
        assert_eq!(plan.windows[0].resets_at, Some(ms("2027-01-02T03:30:00Z")));
        // Read just after New Year, a reset late on 31 December is last year's.
        let late = "### Limits\n\n**5-hour limit** — **7%** · Resets Dec 31, 11:30 PM GMT+5:30";
        let plan = parse(late, ms("2027-01-01T00:30:00Z")).unwrap();
        assert_eq!(plan.windows[0].resets_at, Some(ms("2026-12-31T18:00:00Z")));
    }

    #[test]
    fn other_windows_are_named_by_their_label_and_escapes_are_read_through() {
        let text = "### Limits\n\n**Weekly · Haiku\\_x** — **3%**\n\n**Monthly credits** — **9%** · Resets Nov 1, 12:00 AM UTC";
        let plan = parse(text, ms("2026-10-29T15:00:00Z")).unwrap();
        assert_eq!(
            plan.windows
                .iter()
                .map(|one| (one.kind.as_str(), one.resets_at))
                .collect::<Vec<_>>(),
            [
                ("seven_day_haiku_x", None),
                ("monthly_credits", Some(ms("2026-11-01T00:00:00Z")))
            ]
        );
    }
}
