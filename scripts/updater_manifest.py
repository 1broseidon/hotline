#!/usr/bin/env python3
"""Publish one complete updater manifest, after every supported build has landed."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
from urllib.parse import quote

# These are deliberately installer-specific, matching updater::target_for.
PACKAGES = {
    "darwin-aarch64-app": "macos_aarch64.app.tar.gz",
    "darwin-x86_64-app": "macos_x86_64.app.tar.gz",
    "linux-x86_64-appimage": "linux_x86_64.AppImage",
    "linux-x86_64-deb": "linux_x86_64.deb",
    "linux-x86_64-rpm": "linux_x86_64.rpm",
}


def manifest(directory: Path, version: str, repository: str, notes: str) -> dict:
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError("Updater releases must use a stable major.minor.patch version")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Expected a GitHub owner/repository")
    platforms = {}
    for target, suffix in PACKAGES.items():
        name = f"toad_{version}_{suffix}"
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
    return {"version": version, "notes": notes[:4000],
            "pub_date": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "platforms": platforms}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--notes", type=Path, required=True)
    args = parser.parse_args()
    value = manifest(args.directory, args.version, args.repository, args.notes.read_text())
    # The same manifest is kept at its version and at latest for future hosting portability.
    encoded = json.dumps(value, indent=2) + "\n"
    (args.directory / f"updater-{args.version}.json").write_text(encoded)
    (args.directory / "latest.json").write_text(encoded)


if __name__ == "__main__":
    main()
