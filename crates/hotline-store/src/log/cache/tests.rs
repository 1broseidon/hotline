use super::*;
use crate::log::{Log, fold, parse_lines};
use serde_json::json;
use std::{fs, io::Write, time::Instant};

#[test]
fn warm_revisions_keep_fold_order_and_chapter_activity_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let stream = StreamId::Tape("ada".into());
    assert_eq!(log.load(&stream), [] as [Value; 0]);
    let mut written = Vec::new();
    for event in [
        json!({"id":"c1", "kind":"chapter", "ts":1}),
        json!({"id":"u1", "kind":"user", "ts":2}),
        json!({"id":"a1", "kind":"agent", "ts":3}),
        json!({"id":"u1", "kind":"user", "ts":20}),
        json!({"id":"tool", "kind":"tool", "ts":40}),
        json!({"id":"c1", "kind":"chapter", "ts":1, "endedAt":50}),
        json!({"id":"c2", "kind":"chapter", "ts":60}),
        json!({"id":"a2", "kind":"agent", "ts":70}),
        json!({"id":"a2", "kind":"notice", "ts":80}),
        json!({"id":"c2", "kind":"notice", "ts":90}),
        json!({"id":"c1", "kind":"chapter", "ts":1}),
    ] {
        log.clone().append(&stream, &event).unwrap();
        written.push(event);
        let expected = fold(written.clone().into_iter());
        assert_eq!(log.load(&stream), expected);
        let chapter = crate::chapters::open_chapter(&expected);
        let activity = chapter
            .and_then(|c| crate::chapters::last_activity(crate::chapters::slice_of(&expected, c)));
        assert_eq!(log.chapter_activity(&stream), (chapter.cloned(), activity));
        assert_eq!(Log::open(root.path()).load(&stream), expected);
    }
    let expected = log.load(&stream);
    assert_eq!(log.compact(&stream).unwrap(), Some(1));
    assert_eq!(log.load(&stream), expected);
    assert_eq!(Log::open(root.path()).load(&stream), expected);
}

#[test]
fn cache_is_warm_bounded_and_keeps_the_room() {
    let mut cache = Cache::default();
    cache.insert(&StreamId::Room, Fold::new(vec![json!({"id":"room"})]));
    for i in 0..MAX_STREAMS {
        cache.insert(&StreamId::Tape(i.to_string()), Fold::new(vec![]));
    }
    cache.get(&StreamId::Tape("0".into())).unwrap();
    cache.insert(&StreamId::Side("new".into()), Fold::new(vec![]));
    assert!(cache.get(&StreamId::Tape("1".into())).is_none());
    assert!(cache.get(&StreamId::Tape("0".into())).is_some());
    let large = Fold::new(vec![json!({"id":"large", "text":"x".repeat(MAX_BYTES)})]);
    cache.insert(&StreamId::Tape("large".into()), large);
    assert!(cache.get(&StreamId::Tape("large".into())).is_none());
    assert!(cache.get(&StreamId::Room).is_some());
}

#[test]
fn migrations_replace_warm_folds_and_legacy_segments_remain_compatible() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("transcripts")).unwrap();
    fs::write(
        root.path().join("transcripts/ada.jsonl"),
        "{\"id\":\"c\",\"kind\":\"chapter\",\"backendId\":\"toad\"}\n",
    )
    .unwrap();
    let log = Log::open(root.path());
    let stream = StreamId::Tape("ada".into());
    assert_eq!(log.load(&stream)[0]["backendId"], "toad");
    log.migrate_backend_id().unwrap();
    assert_eq!(log.load(&stream)[0]["backendId"], "hotline");
    log.append(&stream, &json!({"id":"u", "kind":"user", "ts":2}))
        .unwrap();
    assert!(!root.path().join("transcripts/ada.jsonl").exists());
    assert_eq!(log.load(&stream), Log::open(root.path()).load(&stream));
    log.append(
        &StreamId::Room,
        &json!({"id":"mcpServers", "kind":"setting", "value":"old"}),
    )
    .unwrap();
    log.load(&StreamId::Room);
    log.migrate_mcp_settings(|_| Ok(json!("protected")))
        .unwrap();
    assert_eq!(log.load(&StreamId::Room)[0]["value"], "protected");
}

#[test]
fn recovery_and_failed_writes_never_leave_a_cached_partial_fold() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("room.jsonl");
    fs::write(&file, "{\"id\":\"old\",\"kind\":\"setting\"}").unwrap();
    let log = Log::open(root.path());
    assert_eq!(log.load(&StreamId::Room).len(), 1);
    assert!(log.writer.lock().unwrap().get(&StreamId::Room).is_none());
    log.append(&StreamId::Room, &json!({"id":"new", "kind":"setting"}))
        .unwrap();
    assert_eq!(
        log.load(&StreamId::Room),
        Log::open(root.path()).load(&StreamId::Room)
    );
    assert!(log.try_load_strict(&StreamId::Room).is_err());
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    assert!(log.try_load(&StreamId::Room).is_err());
    assert!(
        log.append(&StreamId::Room, &json!({"id":"failed"}))
            .is_err()
    );
    assert!(log.writer.lock().unwrap().get(&StreamId::Room).is_none());
}

#[test]
#[ignore = "needs an explicit 29 MB tape performance run"]
fn measure_29mb_tape() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("transcripts/ada");
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("1.jsonl");
    let mut output = fs::File::create(&file).unwrap();
    writeln!(
        output,
        "{}",
        json!({"id":"chapter", "kind":"chapter", "ts":1})
    )
    .unwrap();
    let body = "representative tool output and conversation text ".repeat(85);
    for i in 0..7100 {
        writeln!(
            output,
            "{}",
            json!({"id":format!("message-{i}"), "kind":"agent", "ts":i+2, "text":body})
        )
        .unwrap();
    }
    drop(output);
    let start = Instant::now();
    let text = fs::read_to_string(&file).unwrap();
    let expected = fold(parse_lines(&text));
    let uncached = start.elapsed();
    let log = Log::open(root.path());
    let stream = StreamId::Tape("ada".into());
    let start = Instant::now();
    assert_eq!(log.load(&stream), expected);
    let cold = start.elapsed();
    assert!(log.writer.lock().unwrap().get(&stream).is_some());
    let start = Instant::now();
    for _ in 0..10 {
        std::hint::black_box(log.load(&stream));
    }
    let warm = start.elapsed() / 10;
    let start = Instant::now();
    for _ in 0..10000 {
        std::hint::black_box(log.chapter_activity(&stream));
    }
    let activity = start.elapsed() / 10000;
    eprintln!(
        "tape={} bytes uncached={uncached:?} cold={cold:?} warm={warm:?} chapter={activity:?}",
        fs::metadata(file).unwrap().len()
    );
}

#[test]
fn compaction_preserves_revisions_across_segments() {
    let root = tempfile::tempdir().unwrap();
    let segments = root.path().join("transcripts/ada");
    fs::create_dir_all(&segments).unwrap();
    let old = json!({"id":"tool", "kind":"tool", "status":"pending"});
    let other = json!({"id":"other", "kind":"notice"});
    let revised = json!({"id":"tool", "kind":"tool", "status":"completed"});
    fs::write(segments.join("1.jsonl"), format!("{old}\n{other}\n")).unwrap();
    fs::write(segments.join("2.jsonl"), format!("{old}\n{revised}\n")).unwrap();
    let log = Log::open(root.path());
    let stream = StreamId::Tape("ada".into());
    let expected = vec![revised, other];
    assert_eq!(log.load(&stream), expected);
    assert_eq!(log.compact(&stream).unwrap(), Some(2));
    assert_eq!(log.load(&stream), expected);
    assert_eq!(Log::open(root.path()).load(&stream), expected);
    assert_eq!(
        fs::read_to_string(segments.join("1.jsonl")).unwrap(),
        format!("{old}\n{}\n", expected[1])
    );
}

#[test]
fn clones_serialize_cold_reads_and_appends_without_losing_a_revision() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let stream = StreamId::Room;
    std::thread::scope(|scope| {
        for writer in 0..4 {
            let log = log.clone();
            let stream = &stream;
            scope.spawn(move || {
                let writer = writer.to_string();
                for revision in 0..50 {
                    log.append(
                        stream,
                        &json!({"id":writer, "kind":"setting", "value":revision}),
                    )
                    .unwrap();
                    let snapshot = log.load(stream);
                    assert_eq!(
                        snapshot.iter().find(|e| e["id"] == writer).unwrap()["value"],
                        revision
                    );
                }
            });
        }
    });
    let expected = log.load(&stream);
    assert_eq!(expected.len(), 4);
    assert!(expected.iter().all(|e| e["value"] == 49));
    assert_eq!(Log::open(root.path()).load(&stream), expected);
}

#[test]
fn a_poisoned_writer_discards_derived_state() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let event = json!({"id":"kept", "kind":"setting", "value":1});
    log.append(&StreamId::Room, &event).unwrap();
    log.load(&StreamId::Room);
    let _ = std::panic::catch_unwind(|| {
        let mut cache = log.writer.lock().unwrap();
        cache.append(&StreamId::Room, &json!({"id":"unsaved", "kind":"setting"}));
        panic!("interrupted cache update");
    });
    assert_eq!(log.load(&StreamId::Room), vec![event]);
    assert!(!log.writer.is_poisoned());
}

#[test]
fn warm_reads_need_no_disk_but_authoritative_reads_still_report_io_errors() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let event = json!({"id":"kept", "kind":"setting", "value":1});
    log.append(&StreamId::Room, &event).unwrap();
    assert_eq!(log.load(&StreamId::Room), vec![event.clone()]);
    // Fault injection: make disk unreadable after the successful cold read.
    let file = root.path().join("room.jsonl");
    let saved = root.path().join("saved");
    fs::rename(&file, &saved).unwrap();
    fs::create_dir(&file).unwrap();
    assert_eq!(log.load(&StreamId::Room), vec![event.clone()]);
    assert!(log.try_load(&StreamId::Room).is_err());
    assert!(log.writer.lock().unwrap().get(&StreamId::Room).is_none());
    fs::remove_dir(&file).unwrap();
    fs::rename(&saved, &file).unwrap();
    assert_eq!(log.load(&StreamId::Room), vec![event]);
}

#[test]
fn an_id_is_found_on_its_stream_across_revisions_compaction_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let stream = StreamId::Tape("ada".into());
    assert!(!log.contains(&stream, "u1"));
    log.append(&stream, &json!({"id":"u1", "kind":"user", "ts":1}))
        .unwrap();
    log.append(&stream, &json!({"id":"u1", "kind":"user", "ts":2}))
        .unwrap();
    assert!(log.contains(&stream, "u1"));
    assert!(!log.contains(&stream, "u2"));
    assert!(!log.contains(&StreamId::Tape("bob".into()), "u1"));
    log.compact(&stream).unwrap();
    assert!(log.contains(&stream, "u1"));
    assert!(Log::open(root.path()).contains(&stream, "u1"));
}

#[test]
fn thread_links_follow_revisions_compaction_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let log = Log::open(root.path());
    let thread = hotline_contract::ThreadId::side("work");
    let stream = StreamId::Side("work".into());
    assert!(log.thread_link(&thread).is_none());
    let mut link = json!({"kind":"link", "id":"link:side:work", "ts":1,
        "thread":"work", "threadKind":"side", "personaId":"ada",
        "title":"First", "state":"parked"});
    log.append(&stream, &link).unwrap();
    assert_eq!(log.thread_link(&thread).unwrap().title, "First");
    log.append(
        &stream,
        &json!({"kind":"agent", "id":"answer", "ts":2, "text":"Done"}),
    )
    .unwrap();
    link["title"] = json!("Revised");
    log.append(&stream, &link).unwrap();
    assert_eq!(log.thread_link(&thread).unwrap().title, "Revised");
    log.compact(&stream).unwrap();
    let expected = crate::link::Link::find(&log.load(&stream), &thread);
    assert_eq!(log.thread_link(&thread), expected);
    assert_eq!(Log::open(root.path()).thread_link(&thread), expected);
}
