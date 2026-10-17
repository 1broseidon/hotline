"""The pre-commit hook, run against a clone of this checkout with something
staged in it: a clean change passes, and each thing it guards is refused."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = "ui/src/generated/contract.ts"


def run(args, cwd):
    # A test run from inside a hook would otherwise point git at the
    # outer repository's index.
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    return subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True)


class PreCommit(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.mkdtemp()
        self.repo = Path(self.temp) / "hotline"
        cloned = run(["git", "clone", "--quiet", "--shared", str(ROOT), str(self.repo)], ROOT)
        self.assertEqual(cloned.returncode, 0, cloned.stderr)
        # The hook under test is the one in this checkout, committed or not.
        shutil.copytree(ROOT / ".githooks", self.repo / ".githooks", dirs_exist_ok=True)
        modules = ROOT / "ui" / "node_modules"
        if modules.is_dir():
            (self.repo / "ui" / "node_modules").symlink_to(modules)

    def tearDown(self):
        shutil.rmtree(self.temp)

    def stage(self, path, text):
        (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
        (self.repo / path).write_text(text)
        added = run(["git", "add", path], self.repo)
        self.assertEqual(added.returncode, 0, added.stderr)

    def pre_commit(self):
        return run([".githooks/pre-commit"], self.repo)

    def test_clean_change_passes(self):
        lib = "crates/hotline-core/src/lib.rs"
        self.stage(lib, (self.repo / lib).read_text() + "\n// A formatted line.\n")
        self.stage(CONTRACT, (self.repo / CONTRACT).read_text() + "\n")
        result = self.pre_commit()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_misformatted_rust_is_refused(self):
        lib = "crates/hotline-core/src/lib.rs"
        self.stage(lib, (self.repo / lib).read_text() + "\npub fn   misformatted( ){}\n")
        result = self.pre_commit()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("run cargo fmt --all", result.stdout)

    def test_truncated_contract_is_refused(self):
        lines = (self.repo / CONTRACT).read_text().splitlines(keepends=True)
        self.stage(CONTRACT, "".join(lines[len(lines) // 2 :]))
        result = self.pre_commit()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cargo test -p hotline-core, unfiltered", result.stdout)

    def test_contract_that_does_not_compile_is_refused(self):
        # As long as HEAD's, so only the compile catches it: a type named
        # that nothing defines.
        text = (self.repo / CONTRACT).read_text()
        self.stage(CONTRACT, text + "\nexport type Orphan = { missing: NotWritten };\n")
        result = self.pre_commit()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not compile on its own", result.stdout)

    def test_key_file_is_refused(self):
        self.stage("AuthKey_TEST.p8", "-----BEGIN PRIVATE KEY-----\n")
        result = self.pre_commit()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("AuthKey_TEST.p8", result.stdout)


if __name__ == "__main__":
    unittest.main()
