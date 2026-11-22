use super::*;

#[test]
fn each_row_has_one_opening_line() {
    let person = WorkFor::Person;
    let colleague = WorkFor::Colleague {
        name: "Pork Chop".into(),
        item: "6d1b005a-4e90-47a2-a9e6-9efed6db0581".into(),
    };
    for (starter, owner, expected) in [
        (Starter::Person, &person, "Your reply is the chat."),
        (
            Starter::Helpers,
            &person,
            "Hotline: your helpers came back. This work is for the person. Your reply does not reach their chat. Post once if their work is done, a decision is theirs, or a failure matters to them.",
        ),
        (
            Starter::Answers("Fritz"),
            &person,
            "Hotline: Fritz answered. This work is for the person. They read this turn: tell them the result in your own words.",
        ),
        (
            Starter::Quick,
            &colleague,
            "Hotline: a colleague sent you a quick message. This work is for Pork Chop (item 6d1b005a), not the person. Your result goes to Pork Chop: answer the item. Nothing goes to the person's chat.",
        ),
        (
            Starter::Helpers,
            &colleague,
            "Hotline: your helpers came back. This work is for Pork Chop (item 6d1b005a), not the person. Your result goes to Pork Chop: when it is done, give it as the item's `answer` in `update_inbox`, which closes it. Post to the person only for a decision that is theirs or a problem only they can fix.",
        ),
        (
            Starter::Job {
                quiet: true,
                final_run: false,
            },
            &person,
            "Hotline: this job is for the person. Post once only if there is something worth their attention.",
        ),
        (
            Starter::Desk,
            &person,
            "Hotline: this is a desk notice. There is nothing to answer or post.",
        ),
    ] {
        assert_eq!(opening(starter, owner), expected);
    }
}

#[test]
fn posting_guidance_lives_in_the_opening_or_tool_guide() {
    fn check(path: &std::path::Path) {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests" || name == "tests.rs" || name == "returns.rs" {
            return;
        }
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                check(&entry.unwrap().path());
            }
        } else if name.ends_with(".rs") || name == "SKILL.md" {
            let text = std::fs::read_to_string(path).unwrap();
            for line in text
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("//"))
            {
                let lower = line.to_ascii_lowercase();
                for duplicate in [
                    "the job they gave you is done",
                    "nobody reads what you say",
                    "this turn is quiet: what you write",
                    "the person reads this turn",
                ] {
                    assert!(!lower.contains(duplicate), "{}: {line}", path.display());
                }
                if name != "server.rs"
                    && name != "post.rs"
                    && line.starts_with('"')
                    && line.contains("post_to_chat")
                {
                    assert!(line == "\"post_to_chat\",", "{}: {line}", path.display());
                }
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    check(&root.join("src/session"));
    check(&root.join("src/driver"));
    check(&root.join("skills"));
}
