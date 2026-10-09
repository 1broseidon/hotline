from pathlib import Path
import unittest
from changelog_notes import section

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


class ChangelogNotesTests(unittest.TestCase):
    def test_a_version_is_its_own_section_without_its_heading(self):
        self.assertEqual(section(CHANGELOG, "0.35.1"), "### Fixed\n\n- A fix.\n")

    def test_the_last_section_runs_to_the_end(self):
        self.assertEqual(section(CHANGELOG, "0.35.0"), "### Added\n\n- A feature.\n")

    def test_a_version_without_a_section_or_with_an_empty_one_cannot_be_released(self):
        with self.assertRaisesRegex(ValueError, "no section for 0.36.0"):
            section(CHANGELOG, "0.36.0")
        with self.assertRaisesRegex(ValueError, "no section for 0.3"):
            section(CHANGELOG, "0.3")
        with self.assertRaisesRegex(ValueError, "is empty"):
            section("## [0.1.0] - 2026-01-01\n\n## [0.0.9]\n- old\n", "0.1.0")

    def test_the_real_changelog_has_the_last_release(self):
        real = (Path(__file__).resolve().parents[1] / "CHANGELOG.md").read_text()
        self.assertIn("welcome", section(real, "0.35.0"))


if __name__ == "__main__":
    unittest.main()
