"""Disabling Cargo autodiscovery must never silently drop a new harness."""

from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent


class IntegrationTargets(unittest.TestCase):
    def test_every_harness_is_registered_once(self):
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
            cwd=ROOT, text=True,
        ))
        for package in metadata["packages"]:
            if package["name"] not in {"hotline-core", "hotline-cli", "hotline-app"}:
                continue
            with self.subTest(crate=package["name"]):
                folder = Path(package["manifest_path"]).parent / "tests"
                roots = [Path(target["src_path"]) for target in package["targets"]
                         if "test" in target["kind"]]
                integration = folder / "integration.rs"
                self.assertIn(integration, roots)
                modules = re.findall(r'#\[path = "([^"]+)"\]', integration.read_text())
                registered = Counter(roots + [folder / module for module in modules])
                self.assertEqual(registered, Counter(folder.glob("*.rs")),
                                 "add each harness to integration.rs or an explained [[test]]")


if __name__ == "__main__":
    unittest.main()
