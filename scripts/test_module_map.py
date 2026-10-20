"""scripts/module-map on a small crate whose every reference is known."""

from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / "module-map"


def load():
    loader = SourceFileLoader("module_map", str(SCRIPT))
    module = module_from_spec(spec_from_loader("module_map", loader))
    loader.exec_module(module)
    return module


FILES = {
    "lib.rs": "mod a;\nmod b;\nmod c;\nmod d;\n",
    # a names b in code, c only in a comment and a string, d in test code.
    "a.rs": '''
use crate::b::Thing;
/// See [`crate::c`].
fn f() -> &'static str { "crate::c::nope" }
#[cfg(test)]
mod tests {
    use crate::d;
}
''',
    # b and c name each other: one cycle. c reaches b with a brace import,
    # and from a nested file with `super::super`.
    "b.rs": "use crate::c;\n",
    "c.rs": "use crate::{b::Thing, d};\nmod inner;\n",
    "c/inner.rs": "use super::super::b;\n",
    "d.rs": "",
}


class ModuleMap(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.module_map = load()
        self.module_map.SRC = Path(scratch.name)
        for name, text in FILES.items():
            path = self.module_map.SRC / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.modules, self.edges = self.module_map.collect()

    def sites(self, origin, target):
        return self.edges[(origin, target)]

    def test_code_is_a_reference_and_comments_and_strings_are_not(self):
        self.assertEqual(len(self.sites("a", "b")["production"]), 1)
        self.assertNotIn(("a", "c"), self.edges)

    def test_cfg_test_code_counts_apart_from_production(self):
        self.assertEqual(len(self.sites("a", "d")["test"]), 1)
        self.assertEqual(self.sites("a", "d")["production"], [])

    def test_brace_imports_and_super_chains_are_references(self):
        self.assertEqual(len(self.sites("c", "b")["production"]), 2)
        self.assertEqual(len(self.sites("c", "d")["production"]), 1)

    def test_modules_that_reach_each_other_are_one_component(self):
        successors = self.module_map.successors_of(
            self.modules, self.edges, ("production",)
        )
        components = self.module_map.strongly_connected(self.modules, successors)
        self.assertIn(["b", "c"], components)
        self.assertIn(["d"], components)


if __name__ == "__main__":
    unittest.main()
