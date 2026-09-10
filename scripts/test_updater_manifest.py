import tempfile
from pathlib import Path
import unittest
from updater_manifest import PACKAGES, manifest


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        for suffix in PACKAGES.values():
            name = f"toad_0.6.0_{suffix}"
            (self.root / name).write_bytes(b"fixture package")
            (self.root / (name + ".sig")).write_text("fixture signature")

    def test_every_installer_has_its_own_architecture_and_package(self):
        data = manifest(self.root, "0.6.0", "1Broseidon/toad", "Release notes")
        self.assertEqual(len(data["platforms"]), 6)
        windows = data["platforms"]["windows-x86_64-nsis"]
        self.assertTrue(windows["url"].endswith("windows_x86_64-setup.exe"))
        deb = data["platforms"]["linux-x86_64-deb"]
        self.assertTrue(deb["url"].endswith(".deb"))
        self.assertIn("/desktop-v0.6.0/", deb["url"])
        self.assertNotIn("linux-x86_64", data["platforms"])
        self.assertEqual(data["notes"], "Release notes")

    def test_a_missing_target_or_signature_cannot_publish_latest(self):
        for suffix in PACKAGES.values():
            signature = self.root / f"toad_0.6.0_{suffix}.sig"
            signature.unlink()
            with self.assertRaisesRegex(ValueError, "Missing signature"):
                manifest(self.root, "0.6.0", "1Broseidon/toad", "")
            signature.write_text("fixture signature")
        (self.root / "toad_0.6.0_linux_x86_64.deb").unlink()
        with self.assertRaisesRegex(ValueError, "Missing update package"):
            manifest(self.root, "0.6.0", "1Broseidon/toad", "")

    def test_prereleases_and_invalid_versions_are_refused(self):
        for version in ["0.6.0-beta.1", "v0.6.0", "0.6", "00.6.0", "../0.6.0"]:
            with self.assertRaises(ValueError):
                manifest(self.root, version, "1Broseidon/toad", "")


if __name__ == "__main__":
    unittest.main()
