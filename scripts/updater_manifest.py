#!/usr/bin/env python3
"""Publish one complete updater manifest, after every supported build has landed.

Every release gets its manifest under its own version, `updater-<version>.json`.
Only a final release also gets `latest.json`, the name the Stable track reads
from the release GitHub marks latest: a beta writes none, so nothing a beta
uploads can answer at that address. The Beta track's manifest is a copy of a
published release's own (scripts/beta_pointer.py)."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
from urllib.parse import quote

from release_version import parse

# These are deliberately installer-specific, matching updater::target_for.
PACKAGES = {
    "darwin-aarch64-app": "macos_aarch64.app.tar.gz",
    "darwin-x86_64-app": "macos_x86_64.app.tar.gz",
    "linux-x86_64-appimage": "linux_x86_64.AppImage",
    "linux-x86_64-deb": "linux_x86_64.deb",
    "linux-x86_64-rpm": "linux_x86_64.rpm",
    "linux-aarch64-appimage": "linux_aarch64.AppImage",
    "linux-aarch64-deb": "linux_aarch64.deb",
    "linux-aarch64-rpm": "linux_aarch64.rpm",
    "windows-x86_64-nsis": "windows_x86_64-setup.exe",
}


# Well under the desktop's MAX_NOTES (updater.rs), which refuses an offer whose notes are longer.
MAX_NOTES = 12_000


def shortened(notes: str) -> str:
    """Notes past the cap end at the last whole line before it, so markdown is not cut mid-item."""
    if len(notes) <= MAX_NOTES:
        return notes
    cut = notes.rfind("\n", 0, MAX_NOTES)
    return notes[:cut if cut > 0 else MAX_NOTES].rstrip() + "\n\n…\n"


def manifest(directory: Path, version: str, repository: str, notes: str) -> dict:
    parse(version)
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Expected a GitHub owner/repository")
    platforms = {}
    for target, suffix in PACKAGES.items():
        name = f"hotline_{version}_{suffix}"
        artifact = directory / name
        signature = directory / f"{name}.sig"
        if not artifact.is_file() or artifact.stat().st_size == 0:
            raise ValueError(f"Missing update package: {name}")
        if not signature.is_file():
            raise ValueError(f"Missing signature: {signature.name}")
        signed = signature.read_text().strip()
        if not signed or len(signed) > 4096:
            raise ValueError(f"Invalid signature file: {signature.name}")
        platforms[target] = {
            "url": f"https://github.com/{repository}/releases/download/desktop-v{version}/{quote(name)}",
            "signature": signed,
        }
    return {"version": version, "notes": shortened(notes),
            "pub_date": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "platforms": platforms}


def write(directory: Path, value: dict) -> list[str]:
    """Writes the manifest under its version, and as latest.json for a final release alone."""
    encoded = json.dumps(value, indent=2) + "\n"
    names = [f"updater-{value['version']}.json"]
    if parse(value["version"]).beta is None:
        names.append("latest.json")
    for name in names:
        (directory / name).write_text(encoded)
    return names


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--notes", type=Path, required=True)
    args = parser.parse_args()
    write(args.directory, manifest(args.directory, args.version, args.repository, args.notes.read_text()))


if __name__ == "__main__":
    main()
