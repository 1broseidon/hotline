from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from release_notes import github_body, notes, previous, section

CHANGELOG = """# Changelog

## [Unreleased]

### Added

- Not out yet.

## [0.35.1] - 2026-10-09

### Fixed

- A fix.

## [0.35.0] - 2026-10-08

### Added

- A feature.
"""

NOTES = "## Hotline 0.35.1: calmer\n\n- **Settings.** Redone.\n"


class ReleaseNotesTests(unittest.TestCase):
    def test_a_version_is_its_own_changelog_section_without_its_heading(self):
        self.assertEqual(section(CHANGELOG, "0.35.1"), "### Fixed\n\n- A fix.\n")
        self.assertEqual(section(CHANGELOG, "0.35.0"), "### Added\n\n- A feature.\n")

    def test_a_version_without_a_section_or_with_an_empty_one_cannot_be_released(self):
        with self.assertRaisesRegex(ValueError, "no section for 0.36.0"):
            section(CHANGELOG, "0.36.0")
        with self.assertRaisesRegex(ValueError, "no section for 0.3"):
            section(CHANGELOG, "0.3")
        with self.assertRaisesRegex(ValueError, "is empty"):
            section("## [0.1.0] - 2026-01-01\n\n## [0.0.9]\n- old\n", "0.1.0")

    def test_the_app_gets_the_notes_and_github_gets_them_over_the_changelog(self):
        self.assertEqual(notes(NOTES, "0.35.1"), NOTES)
        body = github_body(NOTES, CHANGELOG, "0.35.1", "1broseidon/hotline")
        self.assertTrue(body.startswith(NOTES))
        self.assertIn("## Full changelog\n\n### Fixed\n\n- A fix.\n", body)
        self.assertIn("compare/desktop-v0.35.0...desktop-v0.35.1", body)
        self.assertEqual(previous(CHANGELOG, "0.35.0"), None)
        self.assertNotIn("Compare", github_body(NOTES, CHANGELOG, "0.35.0", "1broseidon/hotline"))
        with self.assertRaisesRegex(ValueError, "is empty"):
            notes("  \n", "0.35.1")

    def test_a_tag_without_written_notes_is_refused(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "CHANGELOG.md").write_text(CHANGELOG)
            run = lambda: subprocess.run(
                [sys.executable, str(Path(__file__).with_name("release_notes.py")), "0.35.1",
                 "--repository", "1broseidon/hotline", "--app", str(root / "app.md"),
                 "--github", str(root / "github.md"), "--root", str(root)],
                capture_output=True, text=True)
            refused = run()
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("release notes", refused.stderr)
            (root / "docs" / "releases").mkdir(parents=True)
            (root / "docs" / "releases" / "0.35.1.md").write_text(NOTES)
            self.assertEqual(run().returncode, 0)
            self.assertEqual((root / "app.md").read_text(), NOTES)
            self.assertIn("## Full changelog", (root / "github.md").read_text())

    def test_the_real_release_has_its_notes_and_changelog(self):
        root = Path(__file__).resolve().parents[1]
        changelog = (root / "CHANGELOG.md").read_text()
        release = (root / "docs" / "releases" / "0.35.1.md").read_text()
        self.assertIn("Try again", github_body(release, changelog, "0.35.1", "1broseidon/hotline"))


if __name__ == "__main__":
    unittest.main()
