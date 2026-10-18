import json
from pathlib import Path
import tempfile
import unittest
from window_licenses import unreviewed


def package(directory, name, license=None, dependencies=None, optional=None):
    directory.mkdir(parents=True)
    manifest = {"name": name, "version": "1.0.0"}
    if license:
        manifest["license"] = license
    if dependencies:
        manifest["dependencies"] = {dependency: "*" for dependency in dependencies}
    if optional:
        manifest["optionalDependencies"] = {dependency: "*" for dependency in optional}
    (directory / "package.json").write_text(json.dumps(manifest))


class WindowLicenses(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.ui = Path(self.temporary.name) / "ui"
        self.modules = self.ui / "node_modules"
        self.ui.mkdir()
        (self.ui / "package.json").write_text(
            json.dumps({"dependencies": {"app": "*"}, "devDependencies": {"tool": "*"}})
        )
        package(self.modules / "tool", "tool", "GPL-3.0-only")

    def tearDown(self):
        self.temporary.cleanup()

    def test_a_reviewed_closure_passes_and_build_tools_are_not_walked(self):
        package(self.modules / "app", "app", "MIT", dependencies=["khroma"], optional=["native-other-os"])
        package(self.modules / "khroma", "khroma")
        self.assertEqual(unreviewed(self.ui), [])

    def test_an_unreviewed_license_deep_in_the_closure_fails_naming_its_parent(self):
        package(self.modules / "app", "app", "MIT", dependencies=["middle"])
        package(self.modules / "middle", "middle", "ISC", dependencies=["deep"])
        package(self.modules / "deep", "deep", "GPL-3.0-only")
        self.assertEqual(
            unreviewed(self.ui), ["deep@1.0.0 (needed by middle@1.0.0) is under GPL-3.0-only"]
        )

    def test_a_nested_copy_is_the_one_its_parent_gets(self):
        package(self.modules / "app", "app", "MIT", dependencies=["shared"])
        package(self.modules / "app" / "node_modules" / "shared", "shared", "SSPL-1.0")
        package(self.modules / "shared", "shared", "MIT")
        self.assertEqual(unreviewed(self.ui), ["shared@1.0.0 (needed by app@1.0.0) is under SSPL-1.0"])

    def test_a_missing_license_or_package_fails(self):
        package(self.modules / "app", "app", dependencies=["absent"])
        self.assertEqual(
            unreviewed(self.ui),
            [
                "app@1.0.0 (needed by hotline-ui) is under no declared license",
                "absent (needed by app@1.0.0) is not installed: run bun install in ui",
            ],
        )


if __name__ == "__main__":
    unittest.main()
