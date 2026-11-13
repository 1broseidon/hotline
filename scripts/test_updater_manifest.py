import json
import tempfile
from pathlib import Path
import unittest
from updater_manifest import MAX_NOTES, PACKAGES, manifest, write


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.build("0.6.0")

    def build(self, version):
        """What the build jobs hand over for one version: each package and its signature."""
        for suffix in PACKAGES.values():
            name = f"hotline_{version}_{suffix}"
            (self.root / name).write_bytes(b"fixture package")
            (self.root / (name + ".sig")).write_text(f"fixture signature {version}")

    def manifests(self):
        return sorted(path.name for path in self.root.glob("*.json"))

    def test_every_installer_has_its_own_architecture_and_package(self):
        data = manifest(self.root, "0.6.0", "1Broseidon/hotline", "Release notes")
        self.assertEqual(len(data["platforms"]), 9)
        windows = data["platforms"]["windows-x86_64-nsis"]
        self.assertTrue(windows["url"].endswith("windows_x86_64-setup.exe"))
        deb = data["platforms"]["linux-x86_64-deb"]
        self.assertTrue(deb["url"].endswith(".deb"))
        self.assertIn("/desktop-v0.6.0/", deb["url"])
        self.assertNotIn("linux-x86_64", data["platforms"])
        arm = data["platforms"]["linux-aarch64-rpm"]
        self.assertTrue(arm["url"].endswith("_linux_aarch64.rpm"))
        self.assertNotIn("linux-aarch64", data["platforms"])
        self.assertEqual(data["notes"], "Release notes")

    def test_long_notes_end_on_a_whole_line_under_the_desktops_limit(self):
        notes = "".join(f"- Item {n} of a long changelog.\n" for n in range(1000))
        short = manifest(self.root, "0.6.0", "1Broseidon/hotline", notes)["notes"]
        self.assertLessEqual(len(short), MAX_NOTES + 4)
        self.assertTrue(short.endswith(".\n\n…\n"))
        self.assertTrue(notes.startswith(short[: -len("\n\n…\n")]))

    def test_a_missing_target_or_signature_cannot_publish_latest(self):
        for suffix in PACKAGES.values():
            signature = self.root / f"hotline_0.6.0_{suffix}.sig"
            signature.unlink()
            with self.assertRaisesRegex(ValueError, "Missing signature"):
                manifest(self.root, "0.6.0", "1Broseidon/hotline", "")
            signature.write_text("fixture signature")
        for suffix in PACKAGES.values():
            package = self.root / f"hotline_0.6.0_{suffix}"
            package.unlink()
            with self.assertRaisesRegex(ValueError, "Missing update package"):
                manifest(self.root, "0.6.0", "1Broseidon/hotline", "")
            package.write_bytes(b"fixture package")

    def test_a_beta_is_published_under_its_own_tag_and_leaves_the_stable_manifest_alone(self):
        stable = write(self.root, manifest(self.root, "0.6.0", "1Broseidon/hotline", "Final notes"))
        self.assertEqual(stable, ["updater-0.6.0.json", "latest.json"])
        latest = (self.root / "latest.json").read_bytes()
        self.build("0.7.0-beta.1")
        data = manifest(self.root, "0.7.0-beta.1", "1Broseidon/hotline", "Beta notes")
        self.assertEqual(write(self.root, data), ["updater-0.7.0-beta.1.json"])
        self.assertEqual(self.manifests(), ["latest.json", "updater-0.6.0.json", "updater-0.7.0-beta.1.json"])
        self.assertEqual((self.root / "latest.json").read_bytes(), latest)
        self.assertEqual(data["version"], "0.7.0-beta.1")
        self.assertEqual(len(data["platforms"]), 9)
        for target, suffix in PACKAGES.items():
            self.assertEqual(
                data["platforms"][target],
                {"url": "https://github.com/1Broseidon/hotline/releases/download/desktop-v0.7.0-beta.1/"
                        f"hotline_0.7.0-beta.1_{suffix}",
                 "signature": "fixture signature 0.7.0-beta.1"})

    def test_a_beta_alone_writes_no_latest_manifest(self):
        self.build("0.7.0-beta.1")
        write(self.root, manifest(self.root, "0.7.0-beta.1", "1Broseidon/hotline", ""))
        self.assertEqual(self.manifests(), ["updater-0.7.0-beta.1.json"])

    def test_a_final_after_its_betas_becomes_the_latest_manifest(self):
        for beta in ["0.7.0-beta.1", "0.7.0-beta.2"]:
            self.build(beta)
            write(self.root, manifest(self.root, beta, "1Broseidon/hotline", ""))
        self.assertNotIn("latest.json", self.manifests())
        self.build("0.7.0")
        data = manifest(self.root, "0.7.0", "1Broseidon/hotline", "Final notes")
        self.assertEqual(write(self.root, data), ["updater-0.7.0.json", "latest.json"])
        self.assertEqual((self.root / "latest.json").read_bytes(), (self.root / "updater-0.7.0.json").read_bytes())
        latest = json.loads((self.root / "latest.json").read_text())
        self.assertEqual(latest["version"], "0.7.0")
        for entry in latest["platforms"].values():
            self.assertIn("/desktop-v0.7.0/hotline_0.7.0_", entry["url"])
            self.assertEqual(entry["signature"], "fixture signature 0.7.0")

    def test_a_beta_missing_a_package_or_signature_has_no_manifest(self):
        self.build("0.7.0-beta.1")
        (self.root / "hotline_0.7.0-beta.1_linux_x86_64.rpm.sig").unlink()
        with self.assertRaisesRegex(ValueError, "Missing signature"):
            manifest(self.root, "0.7.0-beta.1", "1Broseidon/hotline", "")
        # The final's packages beside it do not stand in for the beta's.
        (self.root / "hotline_0.7.0-beta.1_linux_x86_64.rpm").unlink()
        with self.assertRaisesRegex(ValueError, "Missing update package"):
            manifest(self.root, "0.7.0-beta.1", "1Broseidon/hotline", "")

    def test_other_prereleases_and_invalid_versions_are_refused(self):
        for version in ["0.6.0-beta", "0.6.0-beta.0", "0.6.0-rc.1", "0.6.0-nightly.1", "0.6.0+1",
                        "v0.6.0", "0.6", "00.6.0", "../0.6.0"]:
            with self.assertRaises(ValueError):
                manifest(self.root, version, "1Broseidon/hotline", "")


if __name__ == "__main__":
    unittest.main()
