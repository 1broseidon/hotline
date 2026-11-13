//! A step said in a few words, for a glance at a turn: the work card and the
//! peek on the desk, and their twins on the phone. Worked out once, here, as
//! the step is written, so every window says the same thing; the window only
//! groups a run of one kind into one line and counts it.
//!
//! The agent's own description of a step wins when it gave one (Claude's
//! shell tool writes one beside every command). Otherwise Hotline's own tools
//! are said from [`HOTLINE_TOOLS`], the one manifest of their labels, and any
//! other step from the kinds every ACP agent reports, a connected service's
//! name, or the program a command runs.

use crate::contract::StepSaid;

mod sources;
pub use sources::sources_of;

/// How one of Hotline's own tools is said: once, `{n}` times, and while it
/// runs. Steps in a row with the same group are one line of a glance.
pub struct Label {
    pub group: &'static str,
    pub one: &'static str,
    pub many: &'static str,
    pub doing: &'static str,
}

const fn label(
    group: &'static str,
    one: &'static str,
    many: &'static str,
    doing: &'static str,
) -> Label {
    Label {
        group,
        one,
        many,
        doing,
    }
}

/// Every tool Hotline gives a teammate, by the name it is called by, and how
/// a glance says it. A tool added to the room's MCP server, Hotline Agent's
/// workspace or jobs, or the computer without a line here fails
/// `every_hotline_tool_has_a_label`.
pub const HOTLINE_TOOLS: &[(&str, Label)] = &[
    // The room's own MCP server (`mcp::server::TOOL_NAMES`, and the lead's).
    (
        "search_thread",
        label(
            "history",
            "Searched the conversation",
            "Searched the conversation {n} times",
            "Searching the conversation",
        ),
    ),
    (
        "read_thread",
        label(
            "history",
            "Read back the conversation",
            "Read back the conversation",
            "Reading back",
        ),
    ),
    (
        "list_chapters",
        label(
            "history",
            "Looked through chapters",
            "Looked through chapters",
            "Looking through chapters",
        ),
    ),
    (
        "resume_chapter",
        label(
            "chapter",
            "Reopened a chapter",
            "Reopened a chapter",
            "Reopening a chapter",
        ),
    ),
    (
        "new_chapter",
        label(
            "chapter",
            "Started a new chapter",
            "Started a new chapter",
            "Starting a new chapter",
        ),
    ),
    (
        "request_credential",
        label(
            "ask",
            "Asked you to sign in",
            "Asked you {n} times",
            "Asking you to sign in",
        ),
    ),
    (
        "ask_cookies",
        label(
            "ask",
            "Asked you to sign it in",
            "Asked you {n} times",
            "Asking you to sign it in",
        ),
    ),
    (
        "request_human",
        label("ask", "Asked you", "Asked you {n} times", "Asking you"),
    ),
    (
        "ask_question",
        label("ask", "Asked you", "Asked you {n} times", "Asking you"),
    ),
    (
        "withdraw_request",
        label(
            "ask",
            "Withdrew a request",
            "Withdrew {n} requests",
            "Withdrawing a request",
        ),
    ),
    (
        "set_person_name",
        label(
            "name",
            "Saved your name",
            "Saved your name",
            "Saving your name",
        ),
    ),
    (
        "react",
        label("react", "Reacted", "Reacted {n} times", "Reacting"),
    ),
    // What the teammate says it is on and what it left behind head and mark
    // the path in the pane; the window draws them there rather than as
    // steps, and knows their calls by these two groups.
    (
        "activity",
        label(
            "activity",
            "Said what it is on",
            "Said what it is on",
            "Saying what it is on",
        ),
    ),
    (
        "outcome",
        label(
            "outcome",
            "Said what came of it",
            "Said what came of it",
            "Saying what came of it",
        ),
    ),
    (
        "post_to_chat",
        label(
            "post",
            "Posted to the chat",
            "Posted to the chat {n} times",
            "Posting to the chat",
        ),
    ),
    (
        "send_file",
        label("send", "Sent a file", "Sent {n} files", "Sending a file"),
    ),
    (
        "open_link",
        label(
            "link",
            "Opened a link",
            "Opened {n} links",
            "Opening a link",
        ),
    ),
    (
        "generate_image",
        label(
            "image",
            "Made an image",
            "Made {n} images",
            "Making an image",
        ),
    ),
    (
        "set_avatar",
        label(
            "avatar",
            "Changed its picture",
            "Changed its picture",
            "Changing its picture",
        ),
    ),
    (
        "list_teammates",
        label(
            "team",
            "Checked the team",
            "Checked the team",
            "Checking the team",
        ),
    ),
    (
        "message_teammate",
        label(
            "colleague",
            "Messaged a teammate",
            "Messaged teammates {n} times",
            "Messaging a teammate",
        ),
    ),
    (
        "aside",
        label(
            "colleague",
            "Left a teammate an aside",
            "Left teammates {n} asides",
            "Leaving a teammate an aside",
        ),
    ),
    (
        "propose_inbound",
        label(
            "ask",
            "Proposed an inbound source",
            "Proposed {n} inbound sources",
            "Proposing an inbound source",
        ),
    ),
    (
        "add_teammate",
        label(
            "team-add",
            "Added a teammate",
            "Added {n} teammates",
            "Adding a teammate",
        ),
    ),
    (
        "schedule",
        label(
            "schedule",
            "Set a schedule",
            "Changed schedules",
            "Setting a schedule",
        ),
    ),
    (
        "loop",
        label(
            "schedule",
            "Set a schedule",
            "Changed schedules",
            "Setting a schedule",
        ),
    ),
    (
        "list_schedules",
        label(
            "schedule",
            "Checked its schedules",
            "Changed schedules",
            "Checking its schedules",
        ),
    ),
    (
        "cancel_schedule",
        label(
            "schedule",
            "Cancelled a schedule",
            "Changed schedules",
            "Cancelling a schedule",
        ),
    ),
    (
        "computer_status",
        label(
            "computer",
            "Checked its computer",
            "Used the computer {n} times",
            "Checking its computer",
        ),
    ),
    (
        "web_search",
        label(
            "web",
            "Searched the web",
            "Searched the web {n} times",
            "Searching the web",
        ),
    ),
    (
        "archive_thread",
        label(
            "thread",
            "Closed the thread",
            "Closed the thread",
            "Closing the thread",
        ),
    ),
    (
        "call_tool",
        label(
            "tools",
            "Used a tool",
            "Used tools {n} times",
            "Using a tool",
        ),
    ),
    (
        "tool_schema",
        label(
            "tools",
            "Looked up its tools",
            "Looked up its tools",
            "Looking up its tools",
        ),
    ),
    // Hotline Agent's workspace and jobs.
    (
        "ls",
        label("read", "Listed a folder", "Read {n} files", "Reading"),
    ),
    (
        "read",
        label("read", "Read a file", "Read {n} files", "Reading"),
    ),
    (
        "grep",
        label(
            "search",
            "Searched the files",
            "Searched {n} times",
            "Searching",
        ),
    ),
    (
        "glob",
        label(
            "search",
            "Searched the files",
            "Searched {n} times",
            "Searching",
        ),
    ),
    (
        "write",
        label("edit", "Edited a file", "Edited {n} files", "Editing"),
    ),
    (
        "edit",
        label("edit", "Edited a file", "Edited {n} files", "Editing"),
    ),
    (
        "shell",
        label(
            "execute",
            "Ran a command",
            "Ran {n} commands",
            "Running a command",
        ),
    ),
    ("subagent", STARTED_A_HELPER),
    (
        "inspect_job",
        label(
            "jobs",
            "Checked on a job",
            "Checked on jobs",
            "Checking on a job",
        ),
    ),
    (
        "wait_jobs",
        label(
            "jobs",
            "Waited on a job",
            "Waited on jobs",
            "Waiting on a job",
        ),
    ),
    (
        "cancel_job",
        label(
            "jobs",
            "Cancelled a job",
            "Cancelled {n} jobs",
            "Cancelling a job",
        ),
    ),
    (
        "work_result",
        label("result", "Reported back", "Reported back", "Reporting back"),
    ),
    // The computer's tools, called `computer__<name>`.
    (
        "computer__browser",
        label(
            "browser",
            "Used the browser",
            "Used the browser {n} times",
            "Browsing",
        ),
    ),
    (
        "computer__capture",
        label(
            "screen",
            "Looked at the screen",
            "Looked at the screen {n} times",
            "Looking at the screen",
        ),
    ),
    (
        "computer__input",
        label(
            "computer",
            "Used the computer",
            "Used the computer {n} times",
            "Using the computer",
        ),
    ),
    (
        "computer__windows",
        label(
            "computer",
            "Used the computer",
            "Used the computer {n} times",
            "Using the computer",
        ),
    ),
    (
        "computer__wait",
        label(
            "computer",
            "Waited on the computer",
            "Used the computer {n} times",
            "Waiting on the computer",
        ),
    ),
    (
        "computer__state",
        label(
            "computer",
            "Checked its computer",
            "Used the computer {n} times",
            "Checking its computer",
        ),
    ),
    (
        "computer__files",
        label(
            "computer",
            "Moved files on its computer",
            "Used the computer {n} times",
            "Moving files",
        ),
    ),
    (
        "computer__shell",
        label(
            "execute",
            "Ran a command on its computer",
            "Ran {n} commands",
            "Running a command",
        ),
    ),
];

/// Services named in tools as a person knows them.
const SERVICES: &[(&str, &str)] = &[
    ("github", "GitHub"),
    ("gmail", "Gmail"),
    ("linear", "Linear"),
    ("calendar", "Calendar"),
    ("drive", "Drive"),
    ("brainfile", "the board"),
    ("hotline", "Hotline"),
    ("runprism", "Prism"),
    ("slack", "Slack"),
    ("notion", "Notion"),
    ("figma", "Figma"),
    ("asana", "Asana"),
    ("atlassian", "Atlassian"),
    ("jira", "Jira"),
    ("cloudflare", "Cloudflare"),
];

/// The kinds ACP agents give their steps: a kind, never one of Hotline's tools.
const GENERIC_KINDS: &[&str] = &[
    "read",
    "edit",
    "write",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "other",
    "switch_mode",
    "",
];

/// The longest description of its own an agent's step is said by.
const DESCRIPTION_CHARS: usize = 60;

/// The kind of a call that left a command running in the background: an
/// agent's harness says so of its own shell call (see the ACP driver), and the
/// step is then the start of the command's run.
pub const BACKGROUND_COMMAND: &str = "background";

/// How the start of a helper (a subagent) is said, by Hotline Agent's
/// `subagent` tool or by any agent's own launch of one. Its group is the one
/// the window knows such a start by: the helper's own line in the path stands
/// for it.
const STARTED_A_HELPER: Label = label(
    "subagent",
    "Started a helper",
    "Started {n} helpers",
    "Starting a helper",
);

/// How the start of a command left running is said, its group known the same
/// way by the window.
const STARTED_A_COMMAND: Label = label(
    "command",
    "Started a command",
    "Started {n} commands",
    "Starting a command",
);

fn said(
    key: impl Into<String>,
    one: impl Into<String>,
    many: impl Into<String>,
    doing: impl Into<String>,
) -> StepSaid {
    StepSaid {
        key: key.into(),
        one: one.into(),
        many: many.into(),
        doing: doing.into(),
    }
}

fn from_label(label: &Label) -> StepSaid {
    said(label.group, label.one, label.many, label.doing)
}

/// A step in a few words, from its title, its kind and the agent's own
/// description of it when it gave one.
pub fn step_said(title: &str, kind: &str, description: Option<&str>) -> StepSaid {
    let title = title.trim();
    let kind = kind.trim();
    // A step that started a helper or left a command running is said in the
    // desk's words whatever the agent titled or described it, so every agent's
    // start reads alike and the window knows it by its group: the helper's or
    // the command's own line in the path stands for it.
    match kind {
        crate::session::jobs::SUBAGENT => return from_label(&STARTED_A_HELPER),
        BACKGROUND_COMMAND => return from_label(&STARTED_A_COMMAND),
        _ => {}
    }
    // A Hotline tool by its name, written as the title, through the room's MCP
    // server (`mcp__hotline__post_to_chat`), or as the kind of a step whose
    // title says nothing more. A title that carries what was read, edited or
    // run ("read docs/design.md") says more than the label, and an ACP kind
    // ("read", "execute") is a kind, not one of these tools.
    // A title that names the tool before what it was given
    // ("mcp__hotline__web_search: goldfish care", "web_search goldfish care")
    // is that tool.
    let first = title.split([':', ' ']).next().unwrap_or_default();
    let named = first
        .strip_prefix("mcp__hotline__")
        .or_else(|| kind.strip_prefix("mcp__hotline__"))
        .or_else(|| (first == kind && !GENERIC_KINDS.contains(&kind)).then_some(kind));
    let bare = named.unwrap_or(
        if title.contains(char::is_whitespace) || GENERIC_KINDS.contains(&kind) {
            title
        } else {
            kind
        },
    );
    if let Some((_, label)) = HOTLINE_TOOLS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(bare))
    {
        return from_label(label);
    }
    if let Some(words) = description
        .map(str::trim)
        .filter(|words| !words.is_empty() && words.chars().count() <= DESCRIPTION_CHARS)
    {
        let words = words.trim_end_matches('.');
        let group = format!("own:{}", group_of(kind, title));
        // Its own words for one step; a run of one kind is still counted.
        return said(group, words, many_of(kind, title), words);
    }
    if let Some(rest) = title
        .strip_prefix("mcp__")
        .or_else(|| kind.strip_prefix("mcp__"))
    {
        let service = service_of(rest);
        return said(
            format!("mcp:{service}"),
            format!("Used {service}"),
            format!("Used {service} {{n}} times"),
            format!("Using {service}"),
        );
    }
    match group_of(kind, title) {
        "read" => match file_of(title) {
            Some(file) => said("read", format!("Read {file}"), "Read {n} files", "Reading"),
            None => said("read", "Read a file", "Read {n} files", "Reading"),
        },
        "edit" => match file_of(title) {
            Some(file) => said(
                "edit",
                format!("Edited {file}"),
                "Edited {n} files",
                "Editing",
            ),
            None => said("edit", "Edited a file", "Edited {n} files", "Editing"),
        },
        "search" => said(
            "search",
            "Searched the files",
            "Searched {n} times",
            "Searching",
        ),
        "fetch" => match url_in(title).as_deref().and_then(site_of) {
            Some(site) => said(
                "fetch",
                format!("Read {site}"),
                "Read {n} web pages",
                format!("Reading {site}"),
            ),
            None => said(
                "fetch",
                "Read a web page",
                "Read {n} web pages",
                "Reading a web page",
            ),
        },
        "web" => said(
            "web",
            "Searched the web",
            "Searched the web {n} times",
            "Searching the web",
        ),
        "think" => said(
            "think",
            "Thought it through",
            "Thought it through",
            "Thinking",
        ),
        "execute" => {
            let program = program_of(title);
            said(
                "execute",
                format!("Ran {program}"),
                "Ran {n} commands",
                format!("Running {program}"),
            )
        }
        _ => {
            if title.eq_ignore_ascii_case("toolsearch") {
                return said(
                    "tools",
                    "Looked up its tools",
                    "Looked up its tools",
                    "Looking up its tools",
                );
            }
            let words = tidy(title)
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            let words = if words.is_empty() {
                "Worked".to_string()
            } else {
                words
            };
            said(
                format!("other:{words}"),
                words.clone(),
                format!("{words} ({{n}})"),
                words,
            )
        }
    }
}

/// The first web address in a step's title: the page a fetch reads.
fn url_in(title: &str) -> Option<String> {
    title
        .split_whitespace()
        .map(|word| word.trim_matches(['`', '"', '\'', '(', ')', '<', '>']))
        .find(|word| word.starts_with("https://") || word.starts_with("http://"))
        .filter(|word| url::Url::parse(word).is_ok_and(|url| url.host_str().is_some()))
        .map(str::to_string)
}

/// A site as a person names it: "nextjs.org", without the "www.".
pub fn site_of(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}

/// The kind of step, from the ACP kind or what the title gives away.
fn group_of(kind: &str, title: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "read" => "read",
        "edit" | "write" | "delete" | "move" => "edit",
        "search" | "grep" | "glob" => "search",
        // Claude's web search is a fetch whose title is the query.
        "fetch" if title.starts_with("Search ") && url_in(title).is_none() => "web",
        "fetch" => "fetch",
        "think" => "think",
        "execute" | "exec" | "bash" | "terminal" => "execute",
        _ if title.starts_with("web_fetch") => "fetch",
        _ if is_shell(title) => "execute",
        _ => "other",
    }
}

fn many_of(kind: &str, title: &str) -> &'static str {
    match group_of(kind, title) {
        "read" => "Read {n} files",
        "edit" => "Edited {n} files",
        "search" => "Searched {n} times",
        "fetch" => "Read {n} web pages",
        "web" => "Searched the web {n} times",
        "execute" => "Ran {n} commands",
        _ => "{n} steps",
    }
}

fn is_shell(title: &str) -> bool {
    // Some harnesses title a shell call "bash: grep -n …".
    let first = title
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches(':');
    matches!(
        first.rsplit('/').next().unwrap_or_default(),
        "sh" | "bash" | "zsh" | "shell"
    )
}

/// "claude_ai_Prism__linear__list_issues" → "Linear"; "brainfile__get_task" → "the board".
fn service_of(rest: &str) -> String {
    let parts: Vec<&str> = rest.split("__").filter(|part| !part.is_empty()).collect();
    let mut server = parts.first().copied().unwrap_or_default();
    server = server.strip_prefix("claude_ai_").unwrap_or(server);
    // A gateway's tools are named for the service behind it.
    if server.eq_ignore_ascii_case("prism") && parts.len() > 2 {
        server = parts[1];
    }
    let plain: String = server
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some((_, name)) = SERVICES.iter().find(|(key, _)| *key == plain) {
        return (*name).to_string();
    }
    let spaced = server.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "a service".to_string(),
    }
}

/// The last part of the path a read or an edit names: "Read design.md".
fn file_of(title: &str) -> Option<String> {
    let mut words = title.split_whitespace();
    let first = words.next()?;
    if !matches!(
        first.to_ascii_lowercase().as_str(),
        "read" | "write" | "edit"
    ) {
        return None;
    }
    let path = words.next()?;
    if words.next().is_some() {
        return None;
    }
    // A Windows path names its file after a backslash.
    path.trim_matches(['`', '"', '\''])
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .map(str::to_string)
}

/// A command without the shell around it or the `cd` in front.
fn command_of(title: &str) -> String {
    let mut command = tidy(title);
    for shell in ["shell ", "Run "] {
        if let Some(rest) = command.strip_prefix(shell) {
            command = rest.to_string();
        }
    }
    let mut words = command.split_whitespace().peekable();
    if words.peek().is_some_and(|first| is_shell(first)) {
        words.next();
        if words.peek().is_some_and(|flag| flag.starts_with('-')) {
            words.next();
        }
    }
    let mut command: String = words.collect::<Vec<_>>().join(" ");
    command = command.trim_matches(['"', '\'']).to_string();
    if let Some(rest) = command.strip_prefix("cd ")
        && let Some(at) = rest.find("&&").or_else(|| rest.find(';'))
    {
        command = rest[at..].trim_start_matches(['&', ';', ' ']).to_string();
    }
    command
}

/// The program a command runs, said as a person would: "git status", "a Python script".
fn program_of(title: &str) -> String {
    let command = command_of(title);
    // A harness titles a shell call "Terminal" until it knows the command.
    if command.trim().eq_ignore_ascii_case("terminal") {
        return "a command".to_string();
    }
    // A loop or a condition is the shell's own: what it runs is inside.
    if matches!(
        command.split_whitespace().next().unwrap_or_default(),
        "for" | "while" | "until" | "if" | "case" | "{" | "("
    ) {
        return "a shell script".to_string();
    }
    let mut words = command
        .split_whitespace()
        .skip_while(|word| *word == "sudo" || (word.contains('=') && !word.starts_with('-')));
    let Some(first) = words.next() else {
        return "a command".to_string();
    };
    let program = first.rsplit('/').next().unwrap_or(first);
    let next = words
        .clone()
        .find(|word| word.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !word.is_empty());
    if program.starts_with("python") {
        return "a Python script".to_string();
    }
    if matches!(program, "node" | "deno" | "tsx" | "ts-node") {
        return "a script".to_string();
    }
    if matches!(
        program,
        "git"
            | "gh"
            | "npm"
            | "npx"
            | "bun"
            | "cargo"
            | "make"
            | "docker"
            | "brainfile"
            | "pnpm"
            | "yarn"
            | "go"
            | "uv"
            | "pip"
            | "brew"
            | "kubectl"
    ) {
        return match next {
            Some(sub) => format!("{program} {sub}"),
            None => program.to_string(),
        };
    }
    program.to_string()
}

/// A title as a person reads it: markdown ticks gone, a tool's id as words.
fn tidy(title: &str) -> String {
    let title = title.replace('`', "");
    if let Some((server, tool)) = title.split_once("__")
        && !title.contains(' ')
    {
        return format!("{server} · {}", tool.replace('_', " "));
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(title: &str, kind: &str) -> String {
        step_said(title, kind, None).one
    }

    #[test]
    fn a_file_is_named_by_its_name_on_a_windows_path_as_on_a_unix_one() {
        assert_eq!(
            file_of("Read C:\\Users\\ada\\harbour\\notes.txt").as_deref(),
            Some("notes.txt")
        );
        assert_eq!(
            file_of("Read /home/ada/harbour/notes.txt").as_deref(),
            Some("notes.txt")
        );
    }

    #[test]
    fn a_command_titled_with_its_shell_and_a_colon_is_named_by_its_program() {
        let said = step_said(
            "bash: grep -n \"Median time\" page.txt | head",
            "execute",
            None,
        );
        assert_eq!(said.one, "Ran grep");
        assert_eq!(said.doing, "Running grep");
    }

    #[test]
    fn a_placeholder_title_or_a_loop_is_not_named_as_a_program() {
        assert_eq!(one("Terminal", "execute"), "Ran a command");
        assert_eq!(
            one("for r in a b; do echo $r; done", "execute"),
            "Ran a shell script"
        );
        assert_eq!(
            one("while true; do sleep 1; done", "execute"),
            "Ran a shell script"
        );
    }

    #[test]
    fn every_hotline_tool_has_a_label() {
        let mut names: Vec<&str> = crate::mcp::server::TOOL_NAMES.to_vec();
        names.extend(["add_teammate", "archive_thread", "call_tool", "tool_schema"]);
        names.extend(crate::session::jobs::CONTROL_TOOLS);
        names.push(crate::session::jobs::SUBAGENT);
        names.extend([
            "ls",
            "read",
            "grep",
            "glob",
            "write",
            "edit",
            "shell",
            "work_result",
        ]);
        for name in names {
            assert!(
                HOTLINE_TOOLS.iter().any(|(known, _)| *known == name),
                "{name} has no label in glance::HOTLINE_TOOLS"
            );
        }
        for (name, label) in HOTLINE_TOOLS {
            assert!(
                !label.one.is_empty() && !label.doing.is_empty(),
                "{name} says nothing"
            );
            assert!(
                label.many.contains("{n}") || !label.many.is_empty(),
                "{name} has no count"
            );
        }
    }

    /// A start of a helper or of a command left running reads in the desk's
    /// words and under the group the window knows it by, whatever the agent
    /// titled or described the call: Hotline Agent's `subagent` tool, Claude's
    /// failed `Agent` launch titled and described by its task (as the
    /// adapter shows one), Codex's `spawnAgent`, and Claude's background
    /// `Bash` with its own description.
    #[test]
    fn a_start_is_said_in_the_desks_words_whatever_the_agent_wrote() {
        let helper = ("subagent", "Started a helper");
        for (title, words) in [
            ("subagent Open-choice research 1", None),
            (
                "Profile agent tools group E",
                Some("Profile agent tools group E"),
            ),
            ("spawnAgent", None),
        ] {
            let said = step_said(title, crate::session::jobs::SUBAGENT, words);
            assert_eq!((said.key.as_str(), said.one.as_str()), helper, "{title}");
            assert_eq!(said.many, "Started {n} helpers");
        }
        let said = step_said(
            "python3 -m http.server 8766",
            BACKGROUND_COMMAND,
            Some("Serve octopus.md on port 8766"),
        );
        assert_eq!(
            (said.key.as_str(), said.one.as_str(), said.many.as_str()),
            ("command", "Started a command", "Started {n} commands")
        );
    }

    #[test]
    fn hotline_tools_are_said_from_the_manifest_however_they_were_called() {
        assert_eq!(one("post_to_chat", "post_to_chat"), "Posted to the chat");
        assert_eq!(
            one("mcp__hotline__list_teammates", "other"),
            "Checked the team"
        );
        assert_eq!(
            one("computer__browser", "computer__browser"),
            "Used the browser"
        );
        assert_eq!(
            step_said("wait_jobs", "wait_jobs", None).doing,
            "Waiting on a job"
        );
    }

    #[test]
    fn the_agents_own_description_wins_over_the_command() {
        let said = step_said(
            "cargo test -p hotline-core --lib",
            "execute",
            Some("Run the core tests."),
        );
        assert_eq!(said.one, "Run the core tests");
        assert_eq!(said.many, "Ran {n} commands");
        // A description that is a paragraph is not a few words.
        let long = "x".repeat(DESCRIPTION_CHARS + 1);
        assert_eq!(step_said("make", "execute", Some(&long)).one, "Ran make");
    }

    #[test]
    fn other_steps_are_said_by_their_kind_service_or_program() {
        assert_eq!(
            one("python3 -I - <<'E' import json", "execute"),
            "Ran a Python script"
        );
        assert_eq!(one("git status --short", "execute"), "Ran git status");
        assert_eq!(
            one("zsh -lc 'cd /tmp && cargo test -p core'", "execute"),
            "Ran cargo test"
        );
        assert_eq!(
            one("read .agents/skills/hotline-room/SKILL.md", "read"),
            "Read SKILL.md"
        );
        assert_eq!(
            one("web_fetch: https://github.com/kyutai-labs/moshi/", "fetch"),
            "Read github.com"
        );
        assert_eq!(one("Fetch a page", "fetch"), "Read a web page");
        assert_eq!(
            one("mcp__claude_ai_Prism__linear__list_issues", "other"),
            "Used Linear"
        );
        assert_eq!(one("mcp__brainfile__get_task", "other"), "Used the board");
        assert_eq!(
            step_said("mcp__brainfile__get_task", "other", None).many,
            "Used the board {n} times"
        );
        assert_eq!(one("ToolSearch", "other"), "Looked up its tools");
        // A web search however it was called, and a fetch by the site it read.
        assert_eq!(
            one("Search \"Safari 18 file input upload\"", "fetch"),
            "Searched the web"
        );
        assert_eq!(
            one("mcp__hotline__web_search: goldfish care", "search"),
            "Searched the web"
        );
        assert_eq!(
            one("web_search goldfish memory lifespan", "web_search"),
            "Searched the web"
        );
        let fetch = step_said("Fetch https://www.nextjs.org/blog/next-15-4", "fetch", None);
        assert_eq!(fetch.one, "Read nextjs.org");
        assert_eq!(fetch.doing, "Reading nextjs.org");
        assert_eq!(fetch.many, "Read {n} web pages");
        assert_eq!(
            step_said("cargo test", "execute", None).doing,
            "Running cargo test"
        );
    }
}

#[cfg(test)]
mod written {
    use crate::contract::{ToolStatus, TranscriptEvent};
    use crate::driver::Update;

    fn said_of(events: &[TranscriptEvent]) -> Option<String> {
        events.iter().rev().find_map(|event| match event {
            TranscriptEvent::Tool {
                said: Some(said), ..
            } => Some(said.one.clone()),
            _ => None,
        })
    }

    #[test]
    fn the_agents_own_words_outlast_the_call_being_written_again_while_it_runs() {
        // Claude's adapter announces a shell call as "Terminal", sends its
        // description, then writes the call again with the command as title.
        let written = crate::session::events_of_updates(vec![
            Update::ToolCall {
                call_id: "c1".into(),
                title: "Terminal".into(),
                kind: "execute".into(),
            },
            Update::ToolSaid {
                call_id: "c1".into(),
                words: "Fetch the latest release notes".into(),
            },
            Update::ToolCall {
                call_id: "c1".into(),
                title: "for r in a b; do gh api repos/$r/releases/latest; done".into(),
                kind: "execute".into(),
            },
            Update::ToolResult {
                call_id: "c1".into(),
                ok: true,
                output: "ok".into(),
                images: Vec::new(),
            },
        ]);
        assert_eq!(said_of(&written[0]).as_deref(), Some("Ran a command"));
        assert_eq!(
            said_of(&written[2]).as_deref(),
            Some("Fetch the latest release notes")
        );
        assert_eq!(
            said_of(&written[3]).as_deref(),
            Some("Fetch the latest release notes")
        );
    }

    #[test]
    fn a_page_fetched_with_curl_is_a_source_and_a_failed_one_is_not() {
        let fetch = |ok| {
            crate::session::events_of_updates(vec![
                Update::ToolCall {
                    call_id: "c1".into(),
                    title: "curl -sL -m 30 https://bun.com/blog/bun-v1.4.4 | python3 -I -c 'x'"
                        .into(),
                    kind: "execute".into(),
                },
                Update::ToolResult {
                    call_id: "c1".into(),
                    ok,
                    output: "Bun v1.4.4".into(),
                    images: Vec::new(),
                },
            ])
        };
        let sources = |events: &[Vec<TranscriptEvent>]| match &events[1][0] {
            TranscriptEvent::Tool { sources, .. } => sources.clone(),
            _ => None,
        };
        let read = sources(&fetch(true)).expect("a source");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].url, "https://bun.com/blog/bun-v1.4.4");
        assert_eq!(sources(&fetch(false)), None);
        // An API is not a page.
        assert_eq!(
            super::sources_of("gh api repos/oven-sh/bun/releases/latest", "execute", "{}"),
            Vec::new()
        );
    }

    /// Goldie's first step in its computer's browser (tape 79d12f56, line
    /// 3048), as Hotline Agent reports it: the page it went to is on the
    /// step it writes, and a navigation that failed went nowhere.
    #[test]
    fn a_page_the_browser_went_to_is_written_on_its_step_and_a_failed_one_is_not() {
        let navigate = |ok| {
            crate::session::events_of_updates(vec![
                Update::ToolCall {
                    call_id: "b1".into(),
                    title: "computer__browser".into(),
                    kind: "computer__browser".into(),
                },
                Update::ToolResult {
                    call_id: "b1".into(),
                    ok,
                    output: "navigated to https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition"
                        .into(),
                    images: Vec::new(),
                },
            ])
        };
        let sources = |events: &[Vec<TranscriptEvent>]| match &events[1][0] {
            TranscriptEvent::Tool { sources, .. } => sources.clone(),
            _ => None,
        };
        let opened = sources(&navigate(true)).expect("a source");
        assert_eq!(
            opened[0].url,
            "https://nvidianews.nvidia.com/news/nvidia-announces-ceo-transition"
        );
        assert!(!opened[0].found);
        assert_eq!(sources(&navigate(false)), None);
    }

    #[test]
    fn a_step_is_written_with_its_glance_and_takes_the_agents_own_words_when_they_come() {
        let written = crate::session::events_of_updates(vec![
            Update::ToolCall {
                call_id: "c1".into(),
                title: "cargo test -p hotline-core".into(),
                kind: "execute".into(),
            },
            Update::ToolSaid {
                call_id: "c1".into(),
                words: "Run the core tests".into(),
            },
            Update::ToolResult {
                call_id: "c1".into(),
                ok: true,
                output: "ok".into(),
                images: Vec::new(),
            },
        ]);
        assert_eq!(said_of(&written[0]).as_deref(), Some("Ran cargo test"));
        assert_eq!(said_of(&written[1]).as_deref(), Some("Run the core tests"));
        assert!(matches!(
            &written[2][0],
            TranscriptEvent::Tool {
                status: ToolStatus::Completed,
                ..
            }
        ));
        assert_eq!(said_of(&written[2]).as_deref(), Some("Run the core tests"));
    }

    #[test]
    fn a_finished_step_records_when_it_ended_and_the_page_it_read() {
        let written = crate::session::events_of_updates(vec![
            Update::ToolCall {
                call_id: "f1".into(),
                title: "web_fetch: https://nextjs.org/blog/next-15-4".into(),
                kind: "fetch".into(),
            },
            Update::ToolResult {
                call_id: "f1".into(),
                ok: true,
                output: "Next.js 15.4 | Next.js\n\nTurbopack builds are stable.".into(),
                images: Vec::new(),
            },
        ]);
        let TranscriptEvent::Tool {
            ended_ts: None,
            sources: None,
            ..
        } = &written[0][0]
        else {
            panic!(
                "a running step has no end and no pages yet: {:?}",
                written[0]
            );
        };
        let TranscriptEvent::Tool {
            ts,
            ended_ts: Some(ended),
            sources: Some(sources),
            ..
        } = &written[1][0]
        else {
            panic!(
                "a finished fetch records its end and its page: {:?}",
                written[1]
            );
        };
        assert!(ended >= ts);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].url, "https://nextjs.org/blog/next-15-4");
        assert_eq!(sources[0].title.as_deref(), Some("Next.js 15.4 | Next.js"));
        assert!(!sources[0].found);
    }
}
