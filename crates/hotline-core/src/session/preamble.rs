//! The preamble a teammate's agent starts on: who it is, what it may reach,
//! its folders and skills, and how the clock reads.

use super::secrets::describe_secret;
use super::*;

pub(crate) fn preamble(
    persona: &Persona,
    reach: Option<Reach>,
    wake: Option<String>,
    stored: &[SharedSecret],
) -> String {
    let reach_sentence = format!(
        "{}{}",
        reach_sentence(reach),
        folders_sentence(persona, reach)
    );
    // A computer is granted at start, outside the policy, so the agent is told
    // here rather than by a tool listing: what the desktop is, that the person
    // can watch it and take it over, and what to do when a page wants
    // something only the person has. What they type on the desktop goes
    // straight to the desktop and never through the agent.
    let computer_sentence = if persona
        .computer
        .as_ref()
        .is_some_and(|computer| computer.enabled)
    {
        if crate::skills::computer_entry(Path::new(&persona.cwd))
            .is_some_and(|entry| entry.invalid.is_none())
        {
            "\n\nYou have a computer: a Linux desktop of your own, with a browser, driven with the `computer__` tools. Its guide for the release it is running is your `hotline-computer` skill; read it before your first call on the computer. Use its managed shell jobs and prepared workspaces for installs, builds, and app launches. The person can see its screen and take it over at any time. When a page wants a login or passkey, use `request_credential` with its full origin: the secure card on the desk and phone sends values only to the vault and your computer. Never ask for credentials by voice or in chat. For a 2FA tap or a CAPTCHA, get that page on screen first, then call `request_human` and say exactly what to do; they act on your desktop directly, and whatever they type never passes through you. When the call returns, look at the screen again before going on."
        } else {
            "\n\nYou have a computer: a Linux desktop of your own, with a browser, driven with the `computer__` tools. On connection, read `computer__state` with action `info` and action `guide` to load the skill and environment catalog from that running computer. Use its managed shell jobs and prepared workspaces for installs, builds, and app launches. If an older image does not offer those actions, use its advertised tool schemas. The person can see its screen and take it over at any time. When a page wants a login or passkey, use `request_credential` with its full origin: the secure card on the desk and phone sends values only to the vault and your computer. Never ask for credentials by voice or in chat. For a 2FA tap or a CAPTCHA, get that page on screen first, then call `request_human` and say exactly what to do; they act on your desktop directly, and whatever they type never passes through you. When the call returns, look at the screen again before going on."
        }
    } else {
        ""
    };
    // What the person put in the computer for it, by name and kind. The
    // values it is told it will never see, and told why, because a model
    // that goes looking for one is a model that was not told the computer
    // redacts them.
    let secrets_sentence = match persona
        .computer
        .as_ref()
        .filter(|computer| computer.enabled)
        .and_then(|computer| computer.secrets.as_deref())
        .filter(|names| !names.is_empty())
    {
        Some(names) => {
            let described: Vec<String> = names
                .iter()
                .map(
                    |name| match stored.iter().find(|secret| &secret.name == name) {
                        Some(secret) => format!("{name} ({})", describe_secret(secret)),
                        None => format!("{name} (not stored right now)"),
                    },
                )
                .collect();
            format!(
                "\n\nThe person has put these secrets in your computer, by name: {}. A variable is an environment variable in every shell job you run there, so use it as $NAME in a command, or leave it for a tool that reads that variable. A login is typed for you: on a sign-in form of one of its sites, use `browser fill` with `secret` set to NAME.username, NAME.password, or NAME.code for the six-digit code, instead of `text`; the computer types it only on that login's own sites and refuses any other page, and that refusal is right. A passkey signs in by itself when the site asks the browser for one; nothing types it. You never see a value: the computer redacts every one from what it answers you, and no tool returns one. If a task needs a secret you were not given, ask the person for it rather than working around it.",
                described.join(", ")
            )
        }
        None => String::new(),
    };
    let goal = persona.goal.trim();
    let identity = if goal.is_empty() {
        format!("You are {}.", persona.name)
    } else {
        format!(
            "You are {}. You were created for this:\n\n{goal}",
            persona.name
        )
    };
    // Every teammate has Hotline's own tools — over its conversation and over
    // the room — on either driver, so the sentence about them is
    // unconditional: a tool an agent was never told about is a tool it does
    // not have.
    // Skills are the one thing here the agent is told about rather than told:
    // a line each, and the body is read when a task calls for it. The index
    // is what is in the workspace — the built-ins are always there, so it is
    // never empty — and a skill the teammate wrote over a built-in's name is
    // the one listed, because it is the one on disk.
    // The computer's guide is listed only for a teammate that has the
    // computer: it is written by that teammate's grant, and a colleague
    // sharing the working directory has no use for a line about a machine it
    // cannot drive.
    let skills = skills_index(persona);
    let standing = format!(
        "{identity}\n\nYour working directory is {}.{reach_sentence}{computer_sentence}{secrets_sentence}\n\n{CLOCK}\n\n{}\n\n{skills}\n\n{}",
        persona.cwd,
        crate::mcp::server::HOW_TO_USE,
        pacing::HOUSE_STYLE,
    );
    match wake {
        Some(wake) => format!("{standing}\n\n{wake}"),
        None => standing,
    }
}

/// How far a teammate's tools reach, said the way its preamble says it.
///
/// An ACP harness manages its own tools; only file operations it delegates
/// to Hotline share Hotline's workspace boundary.
pub(super) fn reach_sentence(reach: Option<Reach>) -> &'static str {
    match reach {
        Some(Reach::Workspace) => {
            if cfg!(target_os = "linux") {
                " Your built-in file tools stay in that directory: a path that leaves it is refused, except for your saved tool output. The shell also has read-only installed tools and a private home at .hotline-home inside the workspace; other host files are hidden. Granted integrations have their own permissions."
            } else {
                " Your built-in file tools stay in that directory: a path that leaves it is refused, except for your saved tool output. Shell restrictions depend on the operating system; consult its tool description. Granted integrations have their own permissions."
            }
        }
        Some(Reach::Machine) => " Your tools reach the whole machine, not only that directory.",
        None => {
            " Your harness manages the permissions of its own tools. File operations delegated to Hotline through ACP stay inside your working directory."
        }
    }
}

/// The folders the person granted besides the working directory, said the
/// way the code holds the teammate to them: enforced for Hotline Agent's
/// tools under workspace reach and for an ACP harness's delegated file
/// operations, not under whole-machine reach, and mounted on a computer.
pub(super) fn folders_sentence(persona: &Persona, reach: Option<Reach>) -> String {
    let Some(folders) = persona
        .folders
        .as_deref()
        .filter(|folders| !folders.is_empty())
    else {
        return String::new();
    };
    let mode = |writable: bool| {
        if writable {
            "you may change it"
        } else {
            "read only"
        }
    };
    let listed: Vec<String> = folders
        .iter()
        .map(|folder| format!("{} ({})", folder.path, mode(folder.writable)))
        .collect();
    let listed = listed.join(", ");
    let mut sentence = match reach {
        Some(Reach::Workspace) => format!(
            " The person also granted you these folders, which your file tools and shell reach by absolute path: {listed}. A change inside a read-only one is refused."
        ),
        Some(Reach::Machine) => format!(
            " The person also pointed you at these folders: {listed}. Your tools already reach the whole machine, so this is not enforced; change only the ones you may."
        ),
        None => format!(
            " The person also granted you these folders: {listed}. File operations delegated to Hotline through ACP may use them, and a change inside a read-only one is refused; your harness's own tools keep their own permissions."
        ),
    };
    if persona
        .computer
        .as_ref()
        .is_some_and(|computer| computer.enabled)
    {
        let mounted: Vec<String> = crate::computer::folder_mounts(persona)
            .into_iter()
            .map(|(folder, path)| format!("{} at {path}", folder.path))
            .collect();
        if !mounted.is_empty() {
            let _ = write!(
                sentence,
                " Your computer has them too, the same way: {}.",
                mounted.join(", ")
            );
        }
    }
    sentence
}

/// The skills in a teammate's workspace, a line each. The computer's guide
/// is left out for a teammate this view has no computer for.
pub(super) fn skills_index(persona: &Persona) -> String {
    let has_computer = persona
        .computer
        .as_ref()
        .is_some_and(|computer| computer.enabled);
    let listed: Vec<_> = crate::skills::visible(Path::new(&persona.cwd))
        .into_iter()
        .filter(|skill| has_computer || skill.name != crate::skills::COMPUTER)
        .collect();
    crate::skills::index(&listed)
}

/// What the preamble says in place of a date: the clock is on the lines.
pub(crate) const CLOCK: &str = "Every message you get opens with the local day and time it was sent, in brackets, like [Thu 24 Sep 2026, 15:12]: that is how you know the date and the hour. Do not open your own messages with one. The brackets may also say where the person wrote from. `desktop` means they are at this computer in Hotline's desktop app: their browser, screen and apps are this computer's, so 'open it in my browser' means opening it here with `open_link`. `phone` means they are away from this computer, on their phone or another of their devices, and cannot see its screen: do browser work on your own computer if you have one, and bring the result back here as a link, a screenshot or a file with `send_file`. Act on this without mentioning it.";
