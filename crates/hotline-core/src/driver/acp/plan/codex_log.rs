//! Codex's plan limits, as it writes them into its own session logs.
//!
//! Codex keeps each session as `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`
//! (`~/.codex` by default), and after each model call writes a `token_count`
//! event whose `rate_limits` say how much of each window is used:
//!
//! ```json
//! {"timestamp":"2026-10-29T04:32:05.037Z","type":"event_msg","payload":{"type":"token_count",
//!  "rate_limits":{"limit_id":"codex","primary":{"used_percent":2.0,"window_minutes":10080,
//!  "resets_at":1793774976},"secondary":null,"plan_type":"pro"}}}
//! ```
//!
//! The newest such event, in the newest log that has one, is the account's
//! reading as of when Codex wrote it. Reading it costs nothing but the file.

use super::{codex_plan, kind_of_minutes, order, status_of};
use crate::contract::{PlanUsage, PlanWindow};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How many of the latest days' folders are looked in.
const DAYS: usize = 3;
/// How many of the latest logs are read for an event, newest first.
const LOGS: usize = 8;
/// How much of the end of one log is read at most. A long session writes
/// one every model call, so the last is near the end.
const TAIL: u64 = 8 * 1024 * 1024;

/// Where Codex keeps its state: `CODEX_HOME` as the child is launched with
/// it, else as this process has it, else `~/.codex`.
pub(crate) fn codex_home(launch_env: &[(String, String)]) -> Option<PathBuf> {
    launch_env
        .iter()
        .find(|(name, _)| name == "CODEX_HOME")
        .map(|(_, value)| PathBuf::from(value))
        .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
        .filter(|home| !home.as_os_str().is_empty())
        .or_else(|| super::super::home().map(|home| home.join(".codex")))
}

/// The account's latest reading from Codex's logs under `home`, `now` being
/// when it is read (milliseconds), for an event with no time of its own.
pub(crate) fn read(home: &Path, now: i64) -> Option<PlanUsage> {
    latest_logs(&home.join("sessions"))
        .iter()
        .find_map(|log| last_reading(log, now))
}

/// The latest logs, newest written first.
fn latest_logs(sessions: &Path) -> Vec<PathBuf> {
    let mut days = Vec::new();
    'years: for year in numbered(sessions) {
        for month in numbered(&year) {
            for day in numbered(&month) {
                days.push(day);
                if days.len() == DAYS {
                    break 'years;
                }
            }
        }
    }
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = days
        .iter()
        .filter_map(|day| std::fs::read_dir(day).ok())
        .flat_map(Iterator::flatten)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("rollout-") && name.ends_with(".jsonl")
        })
        .filter_map(|entry| {
            let written = entry.metadata().ok()?.modified().ok()?;
            Some((written, entry.path()))
        })
        .collect();
    logs.sort_by(|a, b| b.cmp(a));
    logs.into_iter().take(LOGS).map(|(_, path)| path).collect()
}

/// A folder's numbered folders (`2026`, `10`, `29`), the highest first.
fn numbered(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let number = entry.file_name().to_str()?.parse().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    found.sort_by_key(|(number, _)| std::cmp::Reverse(*number));
    found.into_iter().map(|(_, path)| path).collect()
}

/// The last `token_count` event in a log that says the limits, read from the
/// end so a long log is not read whole.
fn last_reading(log: &Path, now: i64) -> Option<PlanUsage> {
    let mut file = std::fs::File::open(log).ok()?;
    let length = file.metadata().ok()?.len();
    let mut window = 64 * 1024_u64;
    loop {
        let start = length.saturating_sub(window.min(TAIL));
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(length - start)
            .read_to_end(&mut bytes)
            .ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        // The first line of a window that starts mid-file is a piece of one.
        if start > 0 && !lines.is_empty() {
            lines.remove(0);
        }
        if let Some(reading) = lines.iter().rev().find_map(|line| reading_of(line, now)) {
            return Some(reading);
        }
        if start == 0 || window >= TAIL {
            return None;
        }
        window *= 4;
    }
}

/// One log line as a reading, when it is a `token_count` with limits.
fn reading_of(line: &str, now: i64) -> Option<PlanUsage> {
    if !line.contains("\"token_count\"") || !line.contains("\"rate_limits\"") {
        return None;
    }
    let event: Value = serde_json::from_str(line).ok()?;
    let payload = event.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let limits = payload.get("rate_limits")?.as_object()?;
    let mut windows: Vec<PlanWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|which| window_of(limits.get(*which)?))
        .collect();
    if windows.is_empty() {
        return None;
    }
    windows.sort_by_key(|one| order(&one.kind));
    windows.dedup_by(|a, b| a.kind == b.kind);
    let plan = limits
        .get("plan_type")
        .and_then(Value::as_str)
        .and_then(codex_plan);
    let at = event
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map_or(now, |at| at.timestamp_millis());
    Some(PlanUsage {
        windows,
        at,
        plan,
        banked: None,
        banked_expires_at: None,
    })
}

fn window_of(limit: &Value) -> Option<PlanWindow> {
    let used = limit.get("used_percent")?.as_f64()?.clamp(0.0, 100.0);
    let minutes = limit.get("window_minutes").and_then(Value::as_i64)?;
    Some(PlanWindow {
        kind: kind_of_minutes(minutes),
        used_percent: Some(used),
        // Unix seconds, as Codex writes it.
        resets_at: limit
            .get("resets_at")
            .and_then(Value::as_f64)
            .map(|seconds| (seconds * 1000.0) as i64),
        window_minutes: Some(minutes),
        label: None,
        status: status_of(used),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::PlanStatus;

    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "hotline-codex-log-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn token_count(at: &str, limits: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":12505,"output_tokens":140,"total_tokens":12645}},"model_context_window":258400}},"rate_limits":{limits}}}}}"#
        )
    }

    fn write_log(home: &Path, day: &str, name: &str, lines: &[String]) -> PathBuf {
        let folder = home.join("sessions").join(day);
        std::fs::create_dir_all(&folder).unwrap();
        let log = folder.join(name);
        std::fs::write(&log, lines.join("\n") + "\n").unwrap();
        log
    }

    #[test]
    fn the_last_token_count_with_limits_in_the_newest_log_is_the_reading() {
        let home = home("newest");
        // An older day's log, whose reading has been overtaken.
        write_log(
            &home,
            "2026/10/27",
            "rollout-2026-10-27T22-46-13-a.jsonl",
            &[token_count(
                "2026-10-27T22:50:00Z",
                r#"{"limit_id":"codex","primary":{"used_percent":90.0,"window_minutes":10080,"resets_at":1793000000},"secondary":null}"#,
            )],
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_log(
            &home,
            "2026/10/28",
            "rollout-2026-10-28T23-32-02-b.jsonl",
            &[
                r#"{"timestamp":"2026-10-29T04:32:00.000Z","type":"session_meta","payload":{"id":"b"}}"#.to_string(),
                token_count(
                    "2026-10-29T04:32:01.000Z",
                    r#"{"limit_id":"codex","primary":{"used_percent":1.0,"window_minutes":10080,"resets_at":1793774976},"secondary":null}"#,
                ),
                token_count(
                    "2026-10-29T04:32:05.037Z",
                    r#"{"limit_id":"codex","limit_name":null,"primary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1793774976},"secondary":{"used_percent":81.0,"window_minutes":300,"resets_at":1793700000},"credits":{"has_credits":true,"unlimited":false,"balance":"62497.43"},"plan_type":"pro","rate_limit_reached_type":null}"#,
                ),
                // A count with no limits, as Codex writes one for an API key.
                token_count("2026-10-29T04:33:00.000Z", "null"),
                r#"{"timestamp":"2026-10-29T04:33:01.000Z","type":"response_item","payload":{"type":"message","role":"assistant"}}"#.to_string(),
            ],
        );
        let plan = read(&home, 0).unwrap();
        assert_eq!(
            plan.at,
            chrono::DateTime::parse_from_rfc3339("2026-10-29T04:32:05.037Z")
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            plan.windows,
            [
                PlanWindow {
                    kind: "five_hour".into(),
                    used_percent: Some(81.0),
                    resets_at: Some(1_793_700_000_000),
                    window_minutes: Some(300),
                    label: None,
                    status: PlanStatus::Warning,
                },
                PlanWindow {
                    kind: "seven_day".into(),
                    used_percent: Some(2.0),
                    resets_at: Some(1_793_774_976_000),
                    window_minutes: Some(10_080),
                    label: None,
                    status: PlanStatus::Allowed,
                },
            ]
        );
        assert_eq!(plan.plan.as_deref(), Some("ChatGPT Pro"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn no_logs_or_no_limits_in_them_is_no_reading() {
        let home = home("none");
        assert_eq!(read(&home, 0), None);
        write_log(
            &home,
            "2026/10/29",
            "rollout-2026-10-29T08-00-00-c.jsonl",
            &[token_count("2026-10-29T08:00:01.000Z", "null")],
        );
        assert_eq!(read(&home, 0), None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_reading_far_back_in_a_long_log_is_still_found_from_the_end() {
        let home = home("long");
        let mut lines = vec![token_count(
            "2026-10-29T08:00:01.000Z",
            r#"{"primary":{"used_percent":12.5,"window_minutes":120,"resets_at":1793700000}}"#,
        )];
        let filler = format!(
            r#"{{"type":"response_item","payload":{{"text":"{}"}}}}"#,
            "x".repeat(1000)
        );
        lines.extend(std::iter::repeat_n(filler, 400));
        write_log(&home, "2026/10/29", "rollout-long.jsonl", &lines);
        let plan = read(&home, 0).unwrap();
        assert_eq!(plan.windows[0].kind, "2h");
        assert_eq!(plan.windows[0].used_percent, Some(12.5));
        let _ = std::fs::remove_dir_all(home);
    }
}
