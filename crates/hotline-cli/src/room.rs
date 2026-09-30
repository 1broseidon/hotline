//! Which room a client command knocks on.
//!
//! `--data` and `HOTLINE_DATA_DIR` are the operator's own word and are taken
//! as given. With neither, the caller's own data folder is tried, and if no
//! desk runs there, the room the hotline service keeps: the
//! `HOTLINE_DATA_DIR` in the unit's environment, then `/var/lib/hotline/room`.
//! On a served box that is the room every `pair` and `status` means, and the
//! caller's own folder is one that nothing ever wrote to.
//!
//! What systemd says about the unit is read through one function, so the
//! order here is tested without systemd.

use crate::door::{self, Presence};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The room the packaged unit sets, and the account it creates.
const SERVED_ROOM: &str = "/var/lib/hotline/room";
const SERVICE_ACCOUNT: &str = "hotline";

/// What the resolver asks the machine.
pub(crate) struct Host {
    /// The caller's own data folder.
    pub default_dir: PathBuf,
    /// Where the packaged unit keeps the room.
    pub served_dir: PathBuf,
    /// The output of `systemctl show hotline -p LoadState -p Environment -p
    /// User`, or `None` where there is no systemd to ask.
    pub systemd: Box<dyn Fn() -> Option<String>>,
}

impl Host {
    pub fn real(default_dir: PathBuf) -> Host {
        Host {
            default_dir,
            served_dir: PathBuf::from(SERVED_ROOM),
            systemd: Box::new(systemctl_show),
        }
    }
}

fn systemctl_show() -> Option<String> {
    let output = Command::new("systemctl")
        .env("LC_ALL", "C")
        .args([
            "--no-pager",
            "show",
            "hotline",
            "-p",
            "LoadState",
            "-p",
            "Environment",
            "-p",
            "User",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

/// The hotline service, when this machine has one.
struct Unit {
    /// `HOTLINE_DATA_DIR` in the unit's environment.
    room: Option<PathBuf>,
    /// `User=`; empty when the unit runs as root.
    user: String,
}

fn parse_unit(text: &str) -> Option<Unit> {
    let (mut load, mut environment, mut user) = ("", "", "");
    for line in text.lines() {
        match line.split_once('=') {
            Some(("LoadState", value)) => load = value,
            Some(("Environment", value)) => environment = value,
            Some(("User", value)) => user = value,
            _ => {}
        }
    }
    (load == "loaded").then(|| Unit {
        // A later assignment overrides an earlier one, as it does in the unit.
        room: words(environment)
            .into_iter()
            .rev()
            .find_map(|word| word.strip_prefix("HOTLINE_DATA_DIR=").map(PathBuf::from))
            .filter(|room| !room.as_os_str().is_empty()),
        user: user.to_string(),
    })
}

/// The words of a line as a shell reads them: systemd quotes an assignment
/// that has a space in it.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match (quote, character) {
            (None, ' ' | '\t') => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, '"' | '\'') => {
                quote = Some(character);
                started = true;
            }
            (Some(open), close) if open == close => quote = None,
            (Some('\''), other) => word.push(other),
            (_, '\\') => {
                if let Some(escaped) = characters.next() {
                    word.push(escaped);
                }
                started = true;
            }
            (_, other) => {
                word.push(other);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// A word as it can be pasted into a shell.
fn shell_word(word: &str) -> String {
    let plain =
        |character: char| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The command line as typed, minus `--data`, for an error to say again.
pub(crate) fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_word(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The room a client command knocks on, and what it takes to explain a
/// failure to reach it.
pub(crate) struct Room {
    pub root: PathBuf,
    /// What was typed after `hotline`, without `--data`.
    command: String,
    /// `None` for a room known exactly, which has no service to consult.
    host: Option<Host>,
}

/// Picks the room: `chosen` if the operator named one, else the first of the
/// caller's folder and the service's rooms with a desk running in it.
///
/// Failing that, the room to report is the first one that has a door this
/// account cannot read (the actionable one), then the first with a stale
/// door, then the caller's own folder.
pub(crate) fn locate(chosen: Option<PathBuf>, command: String, host: Host) -> Room {
    let root = chosen.unwrap_or_else(|| pick(&host));
    Room {
        root,
        command,
        host: Some(host),
    }
}

fn pick(host: &Host) -> PathBuf {
    if door::presence(&host.default_dir) == Presence::Live {
        return host.default_dir.clone();
    }
    let mut candidates = vec![host.default_dir.clone()];
    for room in host
        .unit()
        .and_then(|unit| unit.room)
        .into_iter()
        .chain([host.served_dir.clone()])
    {
        if !candidates.contains(&room) {
            candidates.push(room);
        }
    }
    let seen: Vec<_> = candidates
        .iter()
        .map(|room| (room, door::presence(room)))
        .collect();
    [Presence::Live, Presence::Unreadable, Presence::Stale]
        .into_iter()
        .find_map(|wanted| seen.iter().find(|(_, presence)| *presence == wanted))
        .map_or_else(|| host.default_dir.clone(), |(room, _)| (*room).clone())
}

impl Host {
    fn unit(&self) -> Option<Unit> {
        (self.systemd)().and_then(|text| parse_unit(&text))
    }
}

impl Room {
    /// A room already known exactly, such as the one a running service was
    /// started on, which has no other to be mistaken for.
    pub fn exact(root: PathBuf) -> Room {
        Room {
            root,
            command: String::new(),
            host: None,
        }
    }

    /// This command, run as the account the service runs as, on its room.
    fn as_service(&self, room: &Path, user: Option<&str>) -> String {
        let who = match user {
            Some("") => "sudo ".to_string(),
            Some(user) => format!("sudo -u {user} "),
            None => format!("sudo -u {SERVICE_ACCOUNT} "),
        };
        format!(
            "{who}HOTLINE_DATA_DIR={} hotline {}",
            shell_word(&room.to_string_lossy()),
            self.command
        )
    }

    /// The tail of a "no door.json" error: where the hotline service keeps its
    /// room, when this machine has a unit for it and that is not this room.
    pub fn served_hint(&self) -> String {
        let Some(host) = &self.host else {
            return String::new();
        };
        let Some(unit) = host.unit() else {
            return String::new();
        };
        let room = unit.room.clone().unwrap_or_else(|| host.served_dir.clone());
        if room == self.root {
            return String::new();
        }
        format!(
            " This machine's hotline service keeps its room at {}; to use it, run `{}`.",
            room.display(),
            self.as_service(&room, Some(unit.user.as_str()))
        )
    }

    /// The error for a `door.json` this account may not read. On the
    /// service's room it names the room and the command to run as its user.
    pub fn refused(&self, error: &io::Error) -> String {
        let unreadable = || {
            format!(
                "{} could not be read: {error}",
                door::path(&self.root).display()
            )
        };
        let Some(host) = &self.host else {
            return unreadable();
        };
        let unit = host.unit();
        let served = unit
            .as_ref()
            .and_then(|unit| unit.room.clone())
            .into_iter()
            .chain([host.served_dir.clone()])
            .any(|room| room == self.root);
        if !served {
            return unreadable();
        }
        format!(
            "The hotline service's room, {}, has a door.json this account cannot read. Run this as the service's user: `{}`",
            self.root.display(),
            self.as_service(&self.root, unit.as_ref().map(|unit| unit.user.as_str()))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::door::DoorFile;

    /// What `systemctl show hotline -p LoadState -p Environment -p User`
    /// prints for the unit on the development box.
    const UNIT: &str = "Environment=HOME=/home/agent HOTLINE_DATA_DIR=/srv/room PATH=/usr/bin:/bin\nUser=agent\nLoadState=loaded\n";
    const NO_UNIT: &str = "Environment=\nUser=\nLoadState=not-found\n";

    struct Machine {
        home: tempfile::TempDir,
        service: tempfile::TempDir,
        fallback: tempfile::TempDir,
    }

    impl Machine {
        fn new() -> Machine {
            Machine {
                home: tempfile::tempdir().unwrap(),
                service: tempfile::tempdir().unwrap(),
                fallback: tempfile::tempdir().unwrap(),
            }
        }

        /// A machine whose unit names `service` as its room.
        fn host(&self, systemd: Option<&str>) -> Host {
            let text = systemd
                .map(|text| text.replace("/srv/room", &self.service.path().to_string_lossy()));
            Host {
                default_dir: self.home.path().into(),
                served_dir: self.fallback.path().into(),
                systemd: Box::new(move || text.clone()),
            }
        }
    }

    fn door_in(root: &Path, pid: u32) {
        door::write(
            root,
            &DoorFile {
                pid,
                port: 1,
                token: "token".into(),
                version: "0".into(),
                started_at: "2026-09-30T00:00:00Z".into(),
                data_dir: root.to_string_lossy().into(),
                store: "file".into(),
            },
        )
        .unwrap();
    }

    fn running_in(root: &Path) {
        door_in(root, std::process::id());
    }

    fn stale_in(root: &Path) {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        door_in(root, pid);
    }

    fn room(machine: &Machine, systemd: Option<&str>, chosen: Option<&Path>) -> Room {
        locate(
            chosen.map(Path::to_path_buf),
            "pair --link".into(),
            machine.host(systemd),
        )
    }

    fn as_root() -> bool {
        // SAFETY: geteuid takes no arguments and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn a_room_the_operator_named_is_used_as_given_even_with_a_service_running() {
        let machine = Machine::new();
        running_in(machine.service.path());
        let named = tempfile::tempdir().unwrap();
        let picked = room(&machine, Some(UNIT), Some(named.path()));
        assert_eq!(picked.root, named.path());
    }

    #[test]
    fn a_desk_running_in_the_callers_own_folder_comes_first() {
        let machine = Machine::new();
        running_in(machine.home.path());
        running_in(machine.service.path());
        running_in(machine.fallback.path());
        assert_eq!(room(&machine, Some(UNIT), None).root, machine.home.path());
    }

    #[test]
    fn the_callers_own_folder_is_not_left_to_ask_systemd_when_it_has_a_desk() {
        let machine = Machine::new();
        running_in(machine.home.path());
        let host = Host {
            systemd: Box::new(|| panic!("asked systemd when the caller's own desk was running")),
            ..machine.host(None)
        };
        assert_eq!(pick(&host), machine.home.path());
    }

    #[test]
    fn without_one_the_room_in_the_units_environment_comes_next() {
        let machine = Machine::new();
        running_in(machine.service.path());
        running_in(machine.fallback.path());
        assert_eq!(
            room(&machine, Some(UNIT), None).root,
            machine.service.path()
        );
    }

    #[test]
    fn then_the_documented_room_when_the_unit_names_none_or_there_is_no_systemd() {
        let machine = Machine::new();
        running_in(machine.fallback.path());
        for systemd in [None, Some(NO_UNIT), Some(UNIT)] {
            assert_eq!(room(&machine, systemd, None).root, machine.fallback.path());
        }
        let bare = "Environment=HOME=/home/agent\nUser=agent\nLoadState=loaded\n";
        assert_eq!(
            room(&machine, Some(bare), None).root,
            machine.fallback.path()
        );
    }

    #[test]
    fn a_stale_door_in_the_callers_folder_does_not_hide_the_service() {
        let machine = Machine::new();
        stale_in(machine.home.path());
        running_in(machine.service.path());
        assert_eq!(
            room(&machine, Some(UNIT), None).root,
            machine.service.path()
        );
    }

    #[test]
    fn with_no_desk_anywhere_a_stale_door_is_the_one_reported_and_else_the_callers_folder() {
        let machine = Machine::new();
        assert_eq!(room(&machine, Some(UNIT), None).root, machine.home.path());
        stale_in(machine.fallback.path());
        assert_eq!(
            room(&machine, Some(UNIT), None).root,
            machine.fallback.path()
        );
    }

    #[test]
    fn a_room_whose_door_this_account_cannot_read_is_the_one_reported() {
        if as_root() {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let machine = Machine::new();
        stale_in(machine.home.path());
        running_in(machine.service.path());
        std::fs::set_permissions(
            machine.service.path(),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let picked = room(&machine, Some(UNIT), None);
        std::fs::set_permissions(
            machine.service.path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert_eq!(picked.root, machine.service.path());
    }

    #[test]
    fn the_unit_is_read_for_its_room_and_its_account() {
        let unit = parse_unit(UNIT).unwrap();
        assert_eq!(unit.room, Some(PathBuf::from("/srv/room")));
        assert_eq!(unit.user, "agent");
        assert!(
            parse_unit(NO_UNIT).is_none(),
            "a unit that is not loaded is not one"
        );
        assert!(parse_unit("").is_none());
    }

    #[test]
    fn a_later_assignment_wins_and_a_quoted_one_is_read_whole() {
        let unit = parse_unit(
            "Environment=HOTLINE_DATA_DIR=/old \"HOTLINE_DATA_DIR=/srv/the room\" PATH=/bin\nUser=\nLoadState=loaded\n",
        )
        .unwrap();
        assert_eq!(unit.room, Some(PathBuf::from("/srv/the room")));
        assert_eq!(unit.user, "");
        assert_eq!(
            words(r#"A=1 "B=two words" 'C=$x' D=e\ f "" G="h i""#),
            ["A=1", "B=two words", "C=$x", "D=e f", "", "G=h i"]
        );
        assert!(words("   ").is_empty());
    }

    #[test]
    fn an_empty_room_in_the_environment_is_no_room() {
        let unit = parse_unit("Environment=HOTLINE_DATA_DIR=\nUser=x\nLoadState=loaded\n").unwrap();
        assert_eq!(unit.room, None);
    }

    #[test]
    fn a_word_is_quoted_for_a_shell_only_when_it_needs_it() {
        assert_eq!(shell_word("--link"), "--link");
        assert_eq!(shell_word("/var/lib/hotline/room"), "/var/lib/hotline/room");
        assert_eq!(shell_word("/srv/the room"), "'/srv/the room'");
        assert_eq!(shell_word("it's"), r"'it'\''s'");
        assert_eq!(shell_word(""), "''");
        assert_eq!(
            command_line(&["revoke".into(), "id with space".into()]),
            "revoke 'id with space'"
        );
    }

    // What an error says.

    #[test]
    fn no_door_json_suggests_the_services_room_whenever_there_is_a_unit() {
        let machine = Machine::new();
        let service = machine.service.path().display().to_string();
        let here = room(&machine, Some(UNIT), Some(machine.home.path()));
        assert_eq!(
            here.served_hint(),
            format!(
                " This machine's hotline service keeps its room at {service}; to use it, run `sudo -u agent HOTLINE_DATA_DIR={service} hotline pair --link`."
            )
        );
        // A unit that names no room is the documented one; one that runs as root needs no -u.
        let bare = room(
            &machine,
            Some("Environment=\nUser=\nLoadState=loaded\n"),
            Some(machine.home.path()),
        );
        assert_eq!(
            bare.served_hint(),
            format!(
                " This machine's hotline service keeps its room at {0}; to use it, run `sudo HOTLINE_DATA_DIR={0} hotline pair --link`.",
                machine.fallback.path().display()
            )
        );
    }

    #[test]
    fn no_suggestion_without_a_unit_or_when_this_already_is_the_services_room() {
        let machine = Machine::new();
        assert_eq!(
            room(&machine, None, Some(machine.home.path())).served_hint(),
            ""
        );
        assert_eq!(
            room(&machine, Some(NO_UNIT), Some(machine.home.path())).served_hint(),
            ""
        );
        assert_eq!(
            room(&machine, Some(UNIT), Some(machine.service.path())).served_hint(),
            ""
        );
    }

    #[test]
    fn an_unreadable_door_on_the_services_room_names_the_room_and_the_exact_command() {
        let machine = Machine::new();
        let picked = room(&machine, Some(UNIT), Some(machine.service.path()));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(
            picked.refused(&denied),
            format!(
                "The hotline service's room, {0}, has a door.json this account cannot read. Run this as the service's user: `sudo -u agent HOTLINE_DATA_DIR={0} hotline pair --link`",
                machine.service.path().display()
            )
        );
    }

    #[test]
    fn with_no_systemd_the_documented_account_is_named() {
        let machine = Machine::new();
        let picked = room(&machine, None, Some(machine.fallback.path()));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(
            picked
                .refused(&denied)
                .contains("`sudo -u hotline HOTLINE_DATA_DIR=")
        );
    }

    #[test]
    fn an_unreadable_door_anywhere_else_is_only_that() {
        let machine = Machine::new();
        let picked = room(&machine, Some(UNIT), Some(machine.home.path()));
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let said = picked.refused(&denied);
        assert!(
            said.ends_with("could not be read: permission denied"),
            "{said}"
        );
        assert!(!said.contains("sudo"));
    }
}
