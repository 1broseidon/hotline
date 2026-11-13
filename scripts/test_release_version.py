from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from release_version import parse, write

ROOT = Path(__file__).resolve().parents[1]


class ReleaseVersionTests(unittest.TestCase):
    def test_a_release_is_a_final_or_a_numbered_beta_of_one(self):
        self.assertEqual(parse("0.49.0"), ("0.49.0", "0.49.0", None))
        self.assertEqual(parse("0.49.0-beta.1"), ("0.49.0-beta.1", "0.49.0", 1))
        self.assertEqual(parse("1.2.3-beta.12"), ("1.2.3-beta.12", "1.2.3", 12))
        for version in ["v0.49.0", "0.49", "00.49.0", "../0.49.0", "0.49.0-beta", "0.49.0-beta.0",
                        "0.49.0-beta.01", "0.49.0-rc.1", "0.49.0-nightly.1", "0.49.0-beta.1+build",
                        "0.49.0-beta.1\n", "0.49.0~beta.1", ""]:
            with self.assertRaises(ValueError, msg=version):
                parse(version)

    def test_versions_sort_as_semver_sorts_them(self):
        ordered = ["0.48.3", "0.49.0-beta.1", "0.49.0-beta.2", "0.49.0-beta.10", "0.49.0", "0.49.1-beta.1",
                   "0.49.1", "0.50.0-beta.1", "1.0.0"]
        self.assertEqual(sorted(reversed(ordered), key=lambda version: parse(version).order), ordered)

    def test_a_beta_is_written_so_each_package_format_sorts_it_before_its_final(self):
        beta = parse("0.49.0-beta.2")
        self.assertEqual(beta.deb_version, "0.49.0~beta.2")
        self.assertEqual(beta.rpm_release, "0.beta.2")
        final = parse("0.49.0")
        self.assertEqual(final.deb_version, "0.49.0")
        self.assertEqual(final.rpm_release, "1")

    def test_the_step_outputs_name_the_version_and_its_package_forms(self):
        run = lambda *args: subprocess.run([sys.executable, str(ROOT / "scripts" / "release_version.py"), *args],
                                           capture_output=True, text=True)
        self.assertEqual(run("0.49.0-beta.1").stdout,
                         "version=0.49.0-beta.1\nbase=0.49.0\nbeta=true\ndeb_version=0.49.0~beta.1\nrpm_release=0.beta.1\n")
        self.assertEqual(run("0.49.0").stdout,
                         "version=0.49.0\nbase=0.49.0\nbeta=false\ndeb_version=0.49.0\nrpm_release=1\n")
        refused = run("0.49.0-rc.1")
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(refused.stdout, "")
        self.assertIn("X.Y.Z-beta.N", refused.stderr)

    def test_setting_a_version_changes_the_two_files_the_build_reads_and_nothing_else_in_them(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "crates" / "hotline-app").mkdir(parents=True)
            cargo = (ROOT / "Cargo.toml").read_text()
            tauri = (ROOT / "crates" / "hotline-app" / "tauri.conf.json").read_text()
            (root / "Cargo.toml").write_text(cargo)
            (root / "crates" / "hotline-app" / "tauri.conf.json").write_text(tauri)
            write(root, parse("9.8.7-beta.6"))
            current = parse_current(cargo)
            self.assertEqual((root / "Cargo.toml").read_text(),
                             cargo.replace(f'version = "{current}"', 'version = "9.8.7-beta.6"', 1))
            self.assertEqual((root / "crates" / "hotline-app" / "tauri.conf.json").read_text(),
                             tauri.replace(f'"version": "{current}"', '"version": "9.8.7-beta.6"', 1))
            self.assertIn('rust-version = "', (root / "Cargo.toml").read_text())
            (root / "Cargo.toml").write_text("[package]\nname = \"other\"\n")
            with self.assertRaisesRegex(ValueError, "no version to set"):
                write(root, parse("9.8.7"))


def parse_current(cargo: str) -> str:
    return cargo.split('[workspace.package]\nversion = "', 1)[1].split('"', 1)[0]


if __name__ == "__main__":
    unittest.main()
