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
        self.stub("dpkg", "exit 99")
        self.stub("dpkg-query", "echo 'install ok installed 0.26.0'")
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

    def test_pipe_to_sh_help(self):
        result = subprocess.run(["sh", "-s", "--", "--help"], input=INSTALL.read_text(),
                                text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        self.assertIn("--force", result.stdout)
        self.assertIn("--yes", result.stdout)


if __name__ == "__main__":
    unittest.main()
