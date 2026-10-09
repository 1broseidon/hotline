#!/usr/bin/env python3
"""Print one version's CHANGELOG.md section: the release's notes and the updater's."""
import argparse
from pathlib import Path
import re


def section(changelog: str, version: str) -> str:
    """The body under `## [version]`, up to the next version's heading, without the heading."""
    heading = re.compile(rf"^## \[{re.escape(version)}\][^\n]*\n", re.MULTILINE)
    found = heading.search(changelog)
    if found is None:
        raise ValueError(f"CHANGELOG.md has no section for {version}")
    after = changelog[found.end():]
    following = re.search(r"^## \[", after, re.MULTILINE)
    body = (after[:following.start()] if following else after).strip()
    if not body:
        raise ValueError(f"CHANGELOG.md's section for {version} is empty")
    return body + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--changelog", type=Path, default=Path(__file__).resolve().parents[1] / "CHANGELOG.md")
    args = parser.parse_args()
    print(section(args.changelog.read_text(), args.version), end="")


if __name__ == "__main__":
    main()
