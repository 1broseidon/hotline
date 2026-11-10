#!/usr/bin/env python3
"""A release's notes: the short ones written for the person using Hotline
(docs/releases/<version>.md), shown in the app's Settings › Updates, and the
GitHub Release's body, which is those notes with the version's CHANGELOG.md
section beneath them as the full record. A version without both is refused.

A beta has no notes of its own. `X.Y.Z-beta.N` carries the notes being
written for `X.Y.Z`, as they stand when it is tagged, and the changelog's
section for `X.Y.Z`, or `Unreleased` while the release branch still files its
changes there. Its GitHub Release says first that it is a beta and how a desk
gets one, and compares against the last final release."""
import argparse
from pathlib import Path
import re

from release_version import Release, parse

ROOT = Path(__file__).resolve().parents[1]
UNRELEASED = "Unreleased"


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


def beta_section(changelog: str, release: Release) -> str:
    """A beta's changelog: its final's section once that is written, else what is unreleased."""
    try:
        return section(changelog, release.base)
    except ValueError:
        try:
            return section(changelog, UNRELEASED)
        except ValueError:
            raise ValueError(f"CHANGELOG.md has nothing for {release.version}: no section for {release.base} "
                             f"and none under {UNRELEASED}") from None


def last_final(changelog: str, release: Release) -> str | None:
    """The newest final release a beta follows: the first version heading that is not its own final."""
    versions = re.findall(r"^## \[(\d+\.\d+\.\d+)\]", changelog, re.MULTILINE)
    return next((version for version in versions if version != release.base), None)


def notes(release: str, version: str) -> str:
    """The release notes as written, which must say something."""
    body = release.strip()
    if not body:
        raise ValueError(f"docs/releases/{version}.md is empty")
    return body + "\n"


def github_body(release: str, changelog: str, version: str, repository: str) -> str:
    """What the GitHub Release says: the notes, then the full changelog and the comparison."""
    parsed = parse(version)
    if parsed.beta is None:
        body = notes(release, version) + "\n## Full changelog\n\n" + section(changelog, version)
        before = previous(changelog, version)
    else:
        body = (f"Beta {parsed.beta} of {parsed.base}. A desk gets betas with **Settings › Updates › Track** set to Beta.\n\n"
                + notes(release, parsed.base) + "\n## Full changelog\n\n" + beta_section(changelog, parsed))
        before = last_final(changelog, parsed)
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
    try:
        written = parse(args.version).base
        path = args.root / "docs" / "releases" / f"{written}.md"
        if not path.is_file():
            raise ValueError(f"docs/releases/{written}.md is missing: write the release notes before tagging")
        release = path.read_text()
        app = notes(release, written)
        github = github_body(release, changelog, args.version, args.repository)
    except ValueError as error:
        raise SystemExit(str(error))
    args.app.write_text(app)
    args.github.write_text(github)


if __name__ == "__main__":
    main()
