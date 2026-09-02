//! Jobs from the previous Toad's `schedules.json`, written onto the room
//! stream the way `schedule.create` writes them.
//!
//! The file's job is the contract's `ScheduledJob` under different names
//! for the interval (`everyMs` there, `every` here) and without `when`
//! on a one-shot — that tree fired from `nextAt` alone. A loop is still
//! the job that carries an interval. `name` and `lastFiredAt` stay in
//! the old file; this stream never learned them.

use super::{Report, Skipped};
use crate::contract::{ScheduleKind, ScheduledJob};
use crate::log::Log;
use crate::room;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

/// Appends each job for a teammate in this room, skipping one already
/// present by id and one whose teammate is not here.
pub(super) fn import(from: &Path, log: &Log, report: &mut Report) -> io::Result<()> {
    let Some(jobs) = source_jobs(from) else {
        return Ok(());
    };
    let known: HashSet<String> = room::roster(log)
        .into_iter()
        .map(|persona| persona.id)
        .collect();
    let present: HashSet<String> = room::schedules(log).into_iter().map(|job| job.id).collect();
    for value in jobs {
        let Some(job) = job_from_legacy(&value) else {
            let id = value.get("id").and_then(Value::as_str).unwrap_or("?");
            report.skipped.push(Skipped {
                item: format!("schedule {id}"),
                reason: "its record does not read as a job".into(),
            });
            continue;
        };
        if !known.contains(&job.persona_id) {
            report.skipped.push(Skipped {
                item: format!("schedule {}", job.id),
                reason: format!("teammate {} is not in this room", job.persona_id),
            });
            continue;
        }
        if present.contains(&job.id) {
            report.skipped.push(Skipped {
                item: format!("schedule {}", job.id),
                reason: "already scheduled".into(),
            });
            continue;
        }
        room::append_schedule(log, &job).map_err(io::Error::other)?;
        report.schedules += 1;
    }
    Ok(())
}

fn source_jobs(from: &Path) -> Option<Vec<Value>> {
    let text = fs::read_to_string(from.join("schedules.json")).ok()?;
    let stored: Value = serde_json::from_str(&text).ok()?;
    stored.get("jobs").and_then(Value::as_array).cloned()
}

/// The previous Toad's job, as this stream folds a schedule event.
///
/// `everyMs` is `every`. A one-shot that never carried `when` uses
/// `nextAt` for both, which is what `schedule.create` writes today.
fn job_from_legacy(value: &Value) -> Option<ScheduledJob> {
    let id = value.get("id")?.as_str()?.to_string();
    let persona_id = value.get("personaId")?.as_str()?.to_string();
    let prompt = value.get("prompt")?.as_str()?.to_string();
    let next_at = value.get("nextAt")?.as_i64()?;
    let created_at = value.get("createdAt")?.as_i64()?;
    if id.is_empty() || persona_id.is_empty() || prompt.trim().is_empty() {
        return None;
    }
    let every = value
        .get("every")
        .or_else(|| value.get("everyMs"))
        .and_then(Value::as_i64);
    let kind = if every.is_some() {
        ScheduleKind::Loop
    } else if value.get("kind").and_then(Value::as_str) == Some("loop") {
        return None;
    } else {
        ScheduleKind::Schedule
    };
    let when = match kind {
        ScheduleKind::Loop => None,
        ScheduleKind::Schedule => {
            Some(value.get("when").and_then(Value::as_i64).unwrap_or(next_at))
        }
    };
    Some(ScheduledJob {
        id,
        persona_id,
        kind,
        when,
        every,
        prompt,
        quiet: value
            .get("quiet")
            .and_then(Value::as_bool)
            .filter(|quiet| *quiet),
        next_at,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{McpPolicy, Persona, PolicyMode};
    use crate::log::{Log, StreamId};
    use serde_json::json;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "toad-core-import-schedules-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn teammate(id: &str) -> Persona {
        Persona {
            node: None,
            id: id.to_string(),
            name: id.to_string(),
            goal: String::new(),
            face: None,
            team: None,
            backend_id: "pi".to_string(),
            cwd: format!("/tmp/{id}"),
            reach: None,
            model_id: None,
            mode_id: None,
            effort_id: None,
            harness_override: None,
            hop_notice: None,
            mcp_policy: McpPolicy {
                mode: PolicyMode::All,
                server_ids: Vec::new(),
            },
            web_search_policy: None,
            computer: None,
            subagents: None,
            session_checkpoints: Vec::new(),
            last_session_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn a_job_for_a_teammate_here_is_appended_and_folded() {
        let from = scratch("from");
        fs::write(
            from.join("schedules.json"),
            json!({
                "version": 1,
                "jobs": [
                    {
                        "id": "job-once",
                        "personaId": "ada",
                        "kind": "schedule",
                        "prompt": "Check the order",
                        "quiet": true,
                        "nextAt": 1_700_000_001_000i64,
                        "createdAt": 1,
                    },
                    {
                        "id": "job-loop",
                        "personaId": "ada",
                        "kind": "loop",
                        "prompt": "Sweep the inbox",
                        "everyMs": 15_000,
                        "nextAt": 1_700_000_015_000i64,
                        "createdAt": 2,
                    },
                    {
                        "id": "job-stranger",
                        "personaId": "nobody",
                        "kind": "schedule",
                        "prompt": "A job for someone who is not here",
                        "nextAt": 1_700_000_002_000i64,
                        "createdAt": 3,
                    },
                ],
            })
            .to_string(),
        )
        .unwrap();

        let dest = scratch("dest");
        let log = Log::open(&dest);
        crate::room::append_persona(&log, &teammate("ada")).unwrap();

        let mut report = Report::default();
        import(&from, &log, &mut report).unwrap();

        assert_eq!(report.schedules, 2, "{report:?}");
        assert!(
            report
                .skipped
                .iter()
                .any(|skipped| skipped.item == "schedule job-stranger"
                    && skipped.reason == "teammate nobody is not in this room"),
            "{report:?}"
        );

        let jobs = room::schedules(&log);
        assert_eq!(jobs.len(), 2, "{jobs:?}");
        assert_eq!(jobs[0].id, "job-once");
        assert_eq!(jobs[0].kind, ScheduleKind::Schedule);
        assert_eq!(jobs[0].when, Some(1_700_000_001_000));
        assert_eq!(jobs[0].quiet, Some(true));
        assert_eq!(jobs[1].id, "job-loop");
        assert_eq!(jobs[1].kind, ScheduleKind::Loop);
        assert_eq!(jobs[1].every, Some(15_000));
        assert_eq!(jobs[1].when, None);

        let events = log.load(&StreamId::Room);
        let schedule_events: Vec<&Value> = events
            .iter()
            .filter(|event| event.get("kind").and_then(Value::as_str) == Some("schedule"))
            .collect();
        assert_eq!(schedule_events.len(), 2);
        assert_eq!(schedule_events[0]["id"], "job-once");
        assert!(schedule_events[0].get("every").is_none());
        assert_eq!(schedule_events[1]["every"], 15_000);

        let second = {
            let mut report = Report::default();
            import(&from, &log, &mut report).unwrap();
            report
        };
        assert_eq!(second.schedules, 0, "{second:?}");
        assert!(
            second
                .skipped
                .iter()
                .any(|skipped| skipped.item == "schedule job-once"
                    && skipped.reason == "already scheduled"),
            "{second:?}"
        );
        assert_eq!(room::schedules(&log).len(), 2);
    }
}
