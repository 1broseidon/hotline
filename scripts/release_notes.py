#!/usr/bin/env python3
"""A release's notes: the short ones written for the person using Hotline
(docs/releases/<version>.md), shown in the app's Settings › Updates, and the
GitHub Release's body, which is those notes with the version's CHANGELOG.md
section beneath them as the full record. A version without both is refused."""
import argparse
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


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


def previous(changelog: str, version: str) -> str | None:
    """The version released before this one: the next `## [x.y.z]` heading below it."""
    versions = re.findall(r"^## \[(\d+\.\d+\.\d+)\]", changelog, re.MULTILINE)
    if version not in versions:
        return None
    index = versions.index(version)
    return versions[index + 1] if index + 1 < len(versions) else None


def notes(release: str, version: str) -> str:
    """The release notes as written, which must say something."""
    body = release.strip()
    if not body:
        raise ValueError(f"docs/releases/{version}.md is empty")
    return body + "\n"


def github_body(release: str, changelog: str, version: str, repository: str) -> str:
    """What the GitHub Release says: the notes, then the full changelog and the comparison."""
    body = notes(release, version) + "\n## Full changelog\n\n" + section(changelog, version)
    before = previous(changelog, version)
    if before is not None:
        body += f"\n**Compare:** https://github.com/{repository}/compare/desktop-v{before}...desktop-v{version}\n"
    return body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--app", type=Path, required=True, help="where the app's notes go")
    parser.add_argument("--github", type=Path, required=True, help="where the GitHub Release body goes")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    changelog = (args.root / "CHANGELOG.md").read_text()
    path = args.root / "docs" / "releases" / f"{args.version}.md"
    if not path.is_file():
        raise SystemExit(f"docs/releases/{args.version}.md is missing: write the release notes before tagging")
    release = path.read_text()
    args.app.write_text(notes(release, args.version))
    args.github.write_text(github_body(release, changelog, args.version, args.repository))


if __name__ == "__main__":
    main()
