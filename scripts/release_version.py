#!/usr/bin/env python3
"""The versions a release may carry, and how each package format writes one.

A release is final, `X.Y.Z`, or a beta of a final, `X.Y.Z-beta.N`. The
updater compares these by semver, where a beta is older than its final. Two
package formats order versions by their own rules and need the beta written
their way, or the final would be a downgrade to them:

- deb: `X.Y.Z~beta.N`. dpkg reads `X.Y.Z-beta.N` as version `X.Y.Z`, revision
  `beta.N`, which is newer than `X.Y.Z`; a tilde sorts before the end.
- rpm: version `X.Y.Z`, release `0.beta.N`. `-` cannot appear in an rpm
  version, and release `0.beta.N` is older than the final's release `1`.

Run with a version, this prints those as `name=value` lines for a workflow
step's outputs. With `--set` it also writes the version into `Cargo.toml` and
`tauri.conf.json`; `cargo update --workspace` then brings `Cargo.lock` along.
"""
import argparse
from pathlib import Path
import re
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
NUMBER = r"(?:0|[1-9][0-9]*)"
VERSION = re.compile(rf"({NUMBER}\.{NUMBER}\.{NUMBER})(?:-beta\.([1-9][0-9]*))?")


class Release(NamedTuple):
    version: str
    base: str
    beta: int | None

    @property
    def order(self) -> tuple:
        """Semver's order: by the final's numbers, then betas by number, then the final."""
        final = self.beta is None
        return (*(int(part) for part in self.base.split(".")), final, self.beta or 0)

    @property
    def deb_version(self) -> str:
        return self.version if self.beta is None else f"{self.base}~beta.{self.beta}"

    @property
    def rpm_release(self) -> str:
        return "1" if self.beta is None else f"0.beta.{self.beta}"


def parse(version: str) -> Release:
    found = VERSION.fullmatch(version)
    if found is None:
        raise ValueError(f"{version!r} is not a release version: expected X.Y.Z or X.Y.Z-beta.N")
    return Release(version, found.group(1), int(found.group(2)) if found.group(2) else None)


def write(root: Path, release: Release) -> None:
    """Writes the version where the build reads it, leaving every other byte as it was."""
    for path, pattern in [
        (root / "Cargo.toml", r'(\[workspace\.package\]\nversion = ")[^"]*(")'),
        (root / "crates" / "hotline-app" / "tauri.conf.json", r'(\n  "version": ")[^"]*(")'),
    ]:
        text, count = re.subn(pattern, rf"\g<1>{release.version}\g<2>", path.read_text(), count=1)
        if count != 1:
            raise ValueError(f"{path} has no version to set")
        path.write_text(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("version")
    parser.add_argument("--set", action="store_true", help="write the version into Cargo.toml and tauri.conf.json")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    try:
        release = parse(args.version)
        if args.set:
            write(args.root, release)
    except ValueError as error:
        raise SystemExit(str(error))
    print(f"version={release.version}")
    print(f"base={release.base}")
    print(f"beta={'true' if release.beta is not None else 'false'}")
    print(f"deb_version={release.deb_version}")
    print(f"rpm_release={release.rpm_release}")


if __name__ == "__main__":
    main()
