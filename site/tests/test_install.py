"""Isolated tests; no network, privilege escalation, or live service access.
Run: python3 -m unittest discover -s site/tests -v
"""
import hashlib
import io
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import unittest

INSTALL = Path(__file__).resolve().parents[1] / "public/install"


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.room = self.root / "room"
        self.room.mkdir()
        self.unit = self.root / "hotline.service"
        self.binary = self.root / "hotline"
        self.applications = self.root / "Applications"
        self.applications.mkdir()
        self.log = self.root / "actions"
        # Fail closed if a test reaches installation or networking by mistake.
        for command in ("sudo", "runuser", "systemctl", "install", "curl", "apt-get", "dnf", "zypper", "useradd"):
            self.stub(command, 'echo "UNEXPECTED command" >&2; exit 99')
        self.stub("dpkg-query", "exit 1")
        self.stub("rpm", 'echo "package hotline is not installed"; exit 1')
        self.stub("uname", 'case "$1" in -s) echo Linux;; -m) echo x86_64;; esac')
        source = INSTALL.read_text().removesuffix('main "$@"\n')
        self.assertNotIn('\nmain "$@"', source)
        source = source.replace("$HOME/Applications", "__HOME_APPS__")
        source = source.replace("/Applications", str(self.applications))
        source = source.replace("__HOME_APPS__", "$HOME/Applications")
        source = source.replace("/usr/libexec/PlistBuddy", str(self.bin / "PlistBuddy"))
        source = source.replace("/proc/", str(self.root / "proc") + "/")
        self.source = self.root / "functions.sh"
        self.source.write_text(source)

    def stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(0o755)

    def server_install(self, version="0.25.0", supports_version=True):
        self.unit.touch()
        self.binary.write_text("#!/bin/sh\n" + (f"echo 'hotline {version}'\n" if supports_version else "exit 2\n"))
        self.binary.chmod(0o755)

    def run_shell(self, body, success=True):
        q = shlex.quote
        prelude = f'''set -eu
. {q(str(self.source))}
UNIT={q(str(self.unit))}
BINARY={q(str(self.binary))}
ROOM={q(str(self.room))}
LOG={q(str(self.log))}
has_tty() {{ return 1; }}
ask() {{ echo PROMPT >>"$LOG"; answer=N; }}
server() {{ echo INSTALL >>"$LOG"; }}
desktop_linux() {{ echo INSTALL >>"$LOG"; }}
desktop_macos() {{ echo INSTALL >>"$LOG"; }}
'''
        result = subprocess.run(["sh", "-c", prelude + body], text=True, capture_output=True,
                                env={**os.environ, "HOME": str(self.home), "PATH": f"{self.bin}:/usr/bin:/bin"}, timeout=10)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def actions(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_server_same_version_does_not_prompt_or_install_even_with_yes(self):
        self.server_install("0.26.0")
        out = self.run_shell('has_tty() { return 0; }; main --server --version 0.26.0 --yes')
        self.assertIn("Hotline 0.26.0 is installed; 0.26.0 is available", out)
        self.assertEqual(self.actions(), [])

    def test_server_binary_without_unit_is_detected(self):
        self.server_install("0.26.0")
        self.unit.unlink()
        self.run_shell('main --server --version 0.26.0')
        self.assertEqual(self.actions(), [])

    def test_unit_without_binary_is_unknown(self):
        self.unit.touch()
        out = self.run_shell('main --server --version 0.26.0')
        self.assertIn("Hotline unknown is installed", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_old_server_without_version_and_stale_door_is_unknown(self):
        self.server_install(supports_version=False)
        (self.room / "door.json").write_text('{"version":"0.26.0", "pid":99999999}')
        out = self.run_shell('main --server --version 0.26.0')
        self.assertIn("Hotline unknown is installed", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_old_running_binary_uses_matching_door_metadata(self):
        # Model Linux's /proc executable link without depending on the test host.
        # The installer's -ef check still compares real filesystem identities.
        self.server_install(supports_version=False)
        running = self.root / "proc/12345/exe"
        running.parent.mkdir(parents=True)
        running.symlink_to(self.binary)
        (self.room / "door.json").write_text('{"version":"0.26.0", "pid":12345}')
        out = self.run_shell('main --server --version 0.26.0')
        self.assertIn("Hotline 0.26.0 is installed", out)
        self.assertEqual(self.actions(), [])
        # A replaced binary must not inherit the still-running process's version.
        old_binary = self.root / "hotline.old"
        self.binary.rename(old_binary)
        running.unlink()
        running.symlink_to(old_binary)
        self.server_install(supports_version=False)
        out = self.run_shell('main --server --version 0.26.0')
        self.assertIn("Hotline unknown is installed", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_force_reaches_apt_reinstall(self):
        self.stub("dpkg-query", "echo 'install ok installed 0.26.0'")
        # The host's own arch and dpkg are not a Debian x86_64 box everywhere CI runs (macOS).
        self.stub("dpkg", "exit 0")
        self.stub("arch", "echo x86_64")
        source = INSTALL.read_text()
        desktop = source[source.index("desktop_linux() {"):source.index("# The address this host")]
        self.run_shell(desktop + '''
asset() { touch "$work/$1"; }
as_root() { printf '%s\\n' "$*" >>"$LOG"; }
main --version 0.26.0 --force --yes
''')
        self.assertTrue(any("apt-get install --reinstall -y" in action for action in self.actions()))

    def test_terminal_default_no_declines(self):
        self.server_install()
        self.run_shell('has_tty() { return 0; }; main --server --version 0.26.0')
        self.assertEqual(self.actions(), ["PROMPT"])

    def test_terminal_yes_accepts(self):
        self.server_install()
        self.run_shell('has_tty() { return 0; }; ask() { answer=y; }; main --server --version 0.26.0')
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_yes_bypasses_prompt(self):
        self.server_install()
        self.run_shell('has_tty() { return 0; }; main --server --version 0.26.0 --yes')
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_no_terminal_upgrades_without_prompt(self):
        self.server_install()
        self.run_shell('main --server --version 0.26.0')
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_force_reinstalls_but_does_not_imply_yes(self):
        self.server_install("0.26.0")
        self.run_shell('has_tty() { return 0; }; main --server --version 0.26.0 --force')
        self.assertEqual(self.actions(), ["PROMPT"])
        self.log.unlink()
        self.run_shell('has_tty() { return 0; }; main --server --version 0.26.0 --force --yes')
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_fresh_interactive_install_proceeds_without_prompt(self):
        out = self.run_shell('has_tty() { return 0; }; main --version 0.26.0')
        self.assertIn("Hotline 0.26.0 is available.", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_debian_metadata_normalizes_packaging_version(self):
        self.stub("dpkg-query", "echo 'install ok installed 1:0.26.0-1'")
        out = self.run_shell('main --version v0.26.0')
        self.assertIn("Hotline 0.26.0 is installed", out)
        self.assertEqual(self.actions(), [])

    def test_rpm_detection(self):
        self.stub("rpm", "echo 0.26.0")
        self.run_shell('main --version 0.26.0')
        self.assertEqual(self.actions(), [])

    def test_uninstalled_rpm_is_not_detected(self):
        out = self.run_shell('main --version 0.26.0')
        self.assertNotIn("is installed;", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_appimage_unknown_then_recorded_version(self):
        app = self.home / ".local/bin/Hotline.AppImage"
        app.parent.mkdir(parents=True)
        app.touch()
        out = self.run_shell('main --version 0.26.0')
        self.assertIn("Hotline unknown is installed", out)
        self.log.unlink()
        app.with_suffix(".AppImage.version").write_text(f"0.26.0\n{hashlib.sha256(app.read_bytes()).hexdigest()}\n")
        self.run_shell('main --version 0.26.0')
        self.assertEqual(self.actions(), [])

    def test_appimage_replaced_by_desktop_updater_ignores_stale_marker(self):
        app = self.home / ".local/bin/Hotline.AppImage"
        app.parent.mkdir(parents=True)
        app.write_bytes(b"old app")
        app.with_suffix(".AppImage.version").write_text(f"0.26.0\n{hashlib.sha256(app.read_bytes()).hexdigest()}\n")
        app.write_bytes(b"updated app")
        out = self.run_shell('main --version 0.26.0')
        self.assertIn("Hotline unknown is installed", out)
        self.assertEqual(self.actions(), ["INSTALL"])

    def test_macos_bundle_version(self):
        self.stub("uname", "echo Darwin")
        self.stub("PlistBuddy", "echo 0.26.0")
        (self.applications / "Hotline.app").mkdir()
        self.run_shell('main --version 0.26.0')
        self.assertEqual(self.actions(), [])

    def test_macos_per_user_bundle_version(self):
        self.stub("uname", "echo Darwin")
        self.stub("PlistBuddy", "echo 0.26.0")
        (self.home / "Applications/Hotline.app").mkdir(parents=True)
        self.run_shell('main --version 0.26.0')
        self.assertEqual(self.actions(), [])

    def test_invalid_options_or_version_cannot_install(self):
        for options in ('--wat', '--version ../bad', '--listen localhost:1234 --version 0.26.0'):
            self.run_shell('main ' + options, success=False)
        self.assertEqual(self.actions(), [])

    def test_latest_release_lookup(self):
        self.stub("dpkg-query", "echo 'install ok installed 0.26.0'")
        self.run_shell('fetch() { printf \'  "version": "0.26.0",\\n\' >"$2"; }; main')
        self.assertEqual(self.actions(), [])

    def test_asset_checksum_and_real_server_tar_layout(self):
        self.server_install()
        directory = "hotline-server_0.26.0_linux_x86_64"
        archive = self.root / (directory + ".tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            for name in ("hotline", "hotline.service"):
                data = b"test fixture\n"
                entry = tarfile.TarInfo(f"{directory}/{name}")
                entry.size = len(data)
                tar.addfile(entry, io.BytesIO(data))
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        checksums = self.root / "checksums.txt"
        checksums.write_text(f"{checksum}  {archive.name}\n")
        # Reload the actual server function; privileged calls remain stubs.
        source = INSTALL.read_text()
        server = source[source.index("server() {"):source.index("\nmain() {")]
        body = server + f'''
fetch() {{ cp {shlex.quote(str(self.root))}/"${{1##*/}}" "$2"; }}
id() {{ echo 0; }}
as_root() {{
    if [ "$1" = install ]; then [ -f "$4" ] || exit 95; fi
    printf '%s\\n' "$*" >>"$LOG"
}}
as_desk() {{ return 0; }}
main --server --version 0.26.0 --yes
'''
        out = self.run_shell(body)
        self.assertIn(archive.name, out)
        self.assertIn("sudo hotline update", out)
        self.assertTrue(any(directory + "/hotline" in action for action in self.actions()))
        self.log.unlink()
        checksums.write_text(f"{'0' * 64}  {archive.name}\n")
        out = self.run_shell(body, success=False)
        self.assertIn("does not match its checksum", out)
        self.assertEqual(self.actions(), [])

    def test_desk_commands_run_as_the_units_user(self):
        self.stub("systemctl", '[ "$*" = "show -p User --value hotline" ] && echo agent')
        self.stub("sudo", f'echo "sudo $*" >>{shlex.quote(str(self.log))}')
        self.run_shell('as_desk hotline status')
        self.assertEqual(self.actions(), [f"sudo -u agent env HOTLINE_DATA_DIR={self.room} hotline status"])
        self.log.unlink()
        self.stub("systemctl", "exit 1")
        self.run_shell('as_desk hotline status')
        self.assertEqual(self.actions(), [f"sudo -u hotline env HOTLINE_DATA_DIR={self.room} hotline status"])

    # -- which user runs the desk (BRO-139) ------------------------------------

    def pick(self, run_as="", sudo_user="", me="root", people=(), success=True):
        """pick_user as `me`, on a machine whose passwd lists `people` (name, uid, shell, home)."""
        passwd = "\n".join(f"{name}:x:{uid}:{uid}::{home}:{shell}" for name, uid, shell, home in people)
        return self.run_shell(f'''
id() {{
    case "$*" in
        -u) [ {shlex.quote(me)} = root ] && echo 0 || echo 1001 ;;
        -un) echo {shlex.quote(me)} ;;
        "-u agent" | "-u bob" | "-u carol") echo 1001 ;;
        *) return 1 ;;
    esac
}}
getent() {{ printf '%s\\n' {shlex.quote(passwd)}; }}
run_as={shlex.quote(run_as)}
{f"SUDO_USER={shlex.quote(sudo_user)}" if sudo_user else "unset SUDO_USER"}
pick_user
echo "$run_user"
''', success=success).strip()

    def person(self, name, uid=1001, shell="/bin/bash", home=None):
        if home is None:
            home = self.root / "people" / name
            home.mkdir(parents=True, exist_ok=True)
        return (name, uid, shell, str(home))

    def test_the_desk_runs_as_whoever_installed_it_unless_told_otherwise(self):
        self.assertEqual(self.pick(sudo_user="agent"), "agent")
        self.assertEqual(self.pick(me="agent"), "agent")
        self.assertEqual(self.pick(run_as="bob", sudo_user="agent"), "bob")
        self.assertEqual(self.pick(run_as="bob", me="agent"), "bob")

    def test_root_alone_runs_the_desk_as_the_one_person_here(self):
        self.assertEqual(self.pick(people=[self.person("agent")]), "agent")
        self.assertEqual(self.pick(sudo_user="root", people=[self.person("agent")]), "agent")
        # Accounts nobody logs in to are not people.
        system = [
            ("daemon", 1, "/usr/sbin/nologin", "/usr/sbin"),
            ("nobody", 65534, "/bin/sh", "/nonexistent"),
            self.person("svc", shell="/usr/sbin/nologin"),
            self.person("off", shell="/bin/false"),
            self.person("gone", home=self.root / "no-such-home"),
        ]
        self.assertEqual(self.pick(people=system + [self.person("agent")]), "agent")
        self.assertEqual(self.pick(people=system), "")

    def test_the_service_account_is_there_for_anyone_who_names_it_and_for_root_among_several(self):
        self.assertEqual(self.pick(), "")
        self.assertEqual(self.pick(people=[self.person("agent"), self.person("bob")]), "")
        self.assertEqual(self.pick(sudo_user="hotline"), "")
        self.assertEqual(self.pick(run_as="hotline", sudo_user="agent"), "")
        self.assertEqual(self.pick(run_as="hotline", people=[self.person("agent")]), "")

    def test_a_user_that_cannot_run_the_desk_is_refused(self):
        self.assertIn("does not run as root", self.pick(run_as="root", success=False))
        self.assertIn("no user ghost", self.pick(run_as="ghost", success=False))
        self.assertIn("is not a user name", self.pick(run_as="a;b", success=False))
        self.assertIn("is not a user name", self.pick(run_as="-x", success=False))

    def test_the_unit_is_pointed_at_the_users_account_and_nothing_else_moves(self):
        packaged = (INSTALL.parents[2] / "packaging/hotline.service").read_text()
        self.unit.write_text(packaged)
        self.run_shell(f'''
as_root() {{ "$@"; }}
run_unit_as agent staff /home/agent
''')
        lines = set(self.unit.read_text().splitlines())
        for line in (
            "User=agent",
            "Group=staff",
            "Environment=HOME=/home/agent",
            "Environment=PATH=/home/agent/.local/bin:/home/agent/.npm-global/bin:/usr/local/bin:/usr/bin:/bin",
        ):
            self.assertIn(line, lines)
        for unchanged in ("Environment=HOTLINE_DATA_DIR=/var/lib/hotline/room", "UMask=0077", "Type=exec"):
            self.assertIn(unchanged, lines)
        self.assertEqual([line for line in packaged.splitlines() if line.startswith("ExecStart=")],
                         [line for line in self.unit.read_text().splitlines() if line.startswith("ExecStart=")])
        self.assertNotIn("User=hotline", lines)

    def test_a_unit_that_cannot_be_pointed_at_the_user_is_an_error(self):
        self.unit.write_text("[Service]\nUser=hotline\nEnvironment=HOME=/var/lib/hotline\n")
        out = self.run_shell('as_root() { "$@"; }; run_unit_as agent agent /home/agent', success=False)
        self.assertIn("could not set Group=agent", out)

    def server_run(self, args, sudo_user=None, unit_exists=False, hotline_exists=False, success=True, home=None, alone=False):
        """The real server() over a real archive, with privileged commands logged instead of run."""
        directory = "hotline-server_0.26.0_linux_x86_64"
        packaged = (INSTALL.parents[2] / "packaging/hotline.service").read_bytes()
        archive = self.root / (directory + ".tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            for name, data in (("hotline", b"binary\n"), ("hotline.service", packaged)):
                entry = tarfile.TarInfo(f"{directory}/{name}")
                entry.size = len(data)
                tar.addfile(entry, io.BytesIO(data))
        (self.root / "checksums.txt").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
        if unit_exists:
            self.unit.write_bytes(packaged)
        source = INSTALL.read_text()
        server = source[source.index("server() {"):source.index("\nmain() {")]
        agent_home = self.home if home is None else home
        return self.run_shell(server + f'''
fetch() {{ cp {shlex.quote(str(self.root))}/"${{1##*/}}" "$2"; }}
id() {{
    case "$1" in
        -u) if [ -n "${{2:-}}" ]; then case "$2" in agent) echo 1001 ;; *) return 1 ;; esac; else echo 0; fi ;;
        -gn) echo agentgroup ;;
        hotline) {"return 0" if hotline_exists else "return 1"} ;;
    esac
}}
getent() {{ {"" if alone else '[ -n "${2:-}" ] &&'} echo "agent:x:1001:1001:Agent:{agent_home}:/bin/bash"; }}
as_root() {{
    case "$1" in
        sed) "$@" ;;
        install) case "$*" in *" $UNIT") cp "$4" "$UNIT" ;; *) echo "$*" >>"$LOG" ;; esac ;;
        *) echo "$*" >>"$LOG" ;;
    esac
}}
as_desk() {{ return 0; }}
desk_user() {{ sed -n 's/^User=//p' "$UNIT"; }}
{f"SUDO_USER={shlex.quote(sudo_user)}" if sudo_user else "unset SUDO_USER"}
main --server --version 0.26.0 --yes {args}
''', success=success)

    LISTEN = "--listen 192.0.2.10:9443 --public-url https://desk.example:9443"

    def assert_runs_as_agent(self, out):
        actions = self.actions()
        self.assertIn("install -d -m 0755 /var/lib/hotline", actions)
        self.assertIn(f"install -d -o agent -g agentgroup -m 0700 {self.room}", actions)
        self.assertFalse([a for a in actions if a.startswith("useradd")], actions)
        unit = self.unit.read_text().splitlines()
        self.assertIn("User=agent", unit)
        self.assertIn("Group=agentgroup", unit)
        self.assertIn(f"Environment=HOME={self.home}", unit)
        self.assertIn("The desk will run as agent", out)
        self.assertIn(f"sudo -u agent HOTLINE_DATA_DIR={self.room} hotline pair", out)
        self.assertNotIn("sudo -u hotline -H bash", out)

    def test_a_first_server_install_can_run_the_desk_as_a_named_user(self):
        out = self.server_run(f"{self.LISTEN} --user agent")
        self.assert_runs_as_agent(out)

    def test_a_first_server_install_under_sudo_runs_the_desk_as_the_person_who_ran_it(self):
        out = self.server_run(self.LISTEN, sudo_user="agent")
        self.assert_runs_as_agent(out)

    def test_a_first_server_install_as_root_alone_runs_the_desk_as_the_one_person_here(self):
        out = self.server_run(self.LISTEN, alone=True)
        self.assert_runs_as_agent(out)
        self.assertIn("the only person's account here", out)

    def test_a_first_server_install_with_nobody_behind_it_is_as_it_was(self):
        for args, sudo_user in ((self.LISTEN, None), (self.LISTEN, "root"), (f"{self.LISTEN} --user hotline", "agent")):
            self.log.unlink(missing_ok=True)
            self.unit.unlink(missing_ok=True)
            out = self.server_run(args, sudo_user=sudo_user)
            actions = self.actions()
            self.assertTrue(any(a.startswith("useradd --system --create-home --home-dir /var/lib/hotline") for a in actions), actions)
            self.assertFalse([a for a in actions if a.startswith("install -d")], actions)
            unit = self.unit.read_text().splitlines()
            self.assertIn("User=hotline", unit)
            self.assertIn("Environment=HOME=/var/lib/hotline", unit)
            self.assertNotIn("The desk will run as", out)
            self.assertIn(f"sudo -u hotline HOTLINE_DATA_DIR={self.room} hotline pair", out)
            self.assertIn("sudo -u hotline -H bash", out)
            self.assertIn("Environment=PATH=/var/lib/hotline/.local/bin:/var/lib/hotline/.npm-global/bin:/usr/local/bin:/usr/bin:/bin", unit)

    def test_an_installed_desk_keeps_its_user_however_it_is_upgraded(self):
        # sudo's name is no reason to move a room the desk already has.
        out = self.server_run("", sudo_user="agent", unit_exists=True, hotline_exists=True)
        self.assertFalse([a for a in self.actions() if a.startswith("install -d")], self.actions())
        self.assertIn("User=hotline", self.unit.read_text().splitlines())
        self.assertNotIn("The desk will run as", out)

    def test_naming_a_user_for_an_installed_desk_is_refused_before_anything_changes(self):
        out = self.server_run("--user agent", unit_exists=True, hotline_exists=True, success=False)
        self.assertIn("--user is for a first install", out)
        self.assertEqual(self.actions(), [])

    def test_a_named_user_needs_a_home_the_unit_can_carry(self):
        for home, complaint in (
            (self.root / "no-such-home", "does not exist"),
            ("", "has no home folder"),
            ("relative/home", "has no home folder"),
        ):
            out = self.server_run(f"{self.LISTEN} --user agent", success=False, home=home)
            self.assertIn(complaint, out)
            self.assertEqual(self.actions(), [])
        odd = self.root / "home|with pipe"
        odd.mkdir()
        out = self.server_run(f"{self.LISTEN} --user agent", success=False, home=odd)
        self.assertIn("characters the unit cannot carry", out)
        self.assertEqual(self.actions(), [])

    def test_help_lists_user_and_the_desktop_install_refuses_it(self):
        self.assertIn("--user NAME", self.run_shell("main --help"))
        out = self.run_shell("main --user agent", success=False)
        self.assertIn("are for --server", out)
        self.assertEqual(self.actions(), [])

    def test_pipe_to_sh_help(self):
        result = subprocess.run(["sh", "-s", "--", "--help"], input=INSTALL.read_text(),
                                text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        self.assertIn("--force", result.stdout)
        self.assertIn("--yes", result.stdout)


if __name__ == "__main__":
    unittest.main()
