#!/usr/bin/env python3
"""Fails when a package the window ships is under a license nobody reviewed.

The window's release build carries every package its `dependencies` reach,
so this walks that closure in ui/node_modules the way Node resolves it and
reads each package's declared license. Development dependencies are build
tools and are not walked. An expression outside REVIEWED fails, naming the
package and the path that brought it; adding it here and to
THIRD-PARTY-NOTICES.md, with what it obliges, is the review.

    python3 scripts/window_licenses.py
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Every license expression the window's packages declare today, each read
# in THIRD-PARTY-NOTICES.md for what it obliges.
REVIEWED = {
    "MIT",
    "ISC",
    "Apache-2.0",
    "BSD-3-Clause",
    "MIT OR Apache-2.0",
    "Apache-2.0 OR MIT",
    "Unlicense",
    "OFL-1.1",
    "(MPL-2.0 OR Apache-2.0)",
    "EPL-2.0",
}

# A package whose manifest declares no license, with the one its license
# file grants, read by hand.
UNDECLARED = {
    "khroma": "MIT",
}


def find(name, start, ui):
    """The directory `name` resolves to from `start`, as Node looks: its own
    node_modules, then each parent's, up to the window's."""
    for directory in [start, *start.parents]:
        candidate = directory / "node_modules" / name
        if (candidate / "package.json").exists():
            return candidate
        if directory == ui:
            return None
    return None


def unreviewed(ui):
    """Each shipped package with a license outside REVIEWED, as a line."""
    manifest = json.loads((ui / "package.json").read_text())
    pending = [(name, ui, False, "hotline-ui") for name in manifest.get("dependencies", {})]
    seen = set()
    problems = []
    while pending:
        name, start, optional, parent = pending.pop()
        directory = find(name, start, ui)
        if directory is None:
            # An optional package is often another platform's native build.
            if not optional:
                problems.append(f"{name} (needed by {parent}) is not installed: run bun install in ui")
            continue
        if directory in seen:
            continue
        seen.add(directory)
        package = json.loads((directory / "package.json").read_text())
        label = f"{package['name']}@{package['version']}"
        license = package.get("license") or UNDECLARED.get(package["name"])
        if license not in REVIEWED:
            problems.append(f"{label} (needed by {parent}) is under {license or 'no declared license'}")
        for dependency in package.get("dependencies", {}):
            pending.append((dependency, directory, False, label))
        for dependency in package.get("optionalDependencies", {}):
            pending.append((dependency, directory, True, label))
    return problems


def main():
    problems = unreviewed(ROOT / "ui")
    if problems:
        print("The window would ship a license nobody reviewed. Read it, then add it to")
        print("REVIEWED in scripts/window_licenses.py and to THIRD-PARTY-NOTICES.md:")
        for problem in problems:
            print(f"  {problem}")
        sys.exit(1)


if __name__ == "__main__":
    main()
