#!/usr/bin/env python3
"""Write a beta's version into its .deb the way dpkg sorts it.

The bundler writes the app's semver into the package: `0.49.0-beta.1`. dpkg
reads that as version `0.49.0` with revision `beta.1`, which is newer than
`0.49.0`, so the final release would be a downgrade. This changes the one
`Version:` line of the package's control file to `0.49.0~beta.1`, which dpkg
sorts before `0.49.0`, and leaves every other byte of the package's contents
as the bundler wrote it. The app inside still calls itself `0.49.0-beta.1`,
which is the version the updater compares.

The package's bytes change, so its updater signature must be made again
after this runs. A final release's package is left untouched.
"""
import argparse
import gzip
import io
from pathlib import Path
import re
import tarfile

from release_version import Release, parse

MAGIC = b"!<arch>\n"
HEADER = 60
CONTROL = "control.tar.gz"


def members(deb: bytes) -> list[tuple[bytes, bytes]]:
    """The archive's members in order, each as its 60-byte header and its data."""
    if not deb.startswith(MAGIC + b"debian-binary"):
        raise ValueError("not a .deb package")
    found, at = [], len(MAGIC)
    while at < len(deb):
        header = deb[at:at + HEADER]
        if len(header) != HEADER or header[58:] != b"`\n":
            raise ValueError("the .deb's archive is damaged")
        size = int(header[48:58])
        data = deb[at + HEADER:at + HEADER + size]
        if len(data) != size:
            raise ValueError("the .deb's archive is cut short")
        found.append((header, data))
        at += HEADER + size + size % 2
    return found


def name(header: bytes) -> str:
    return header[:16].decode().rstrip(" /")


def with_version(control: bytes, release: Release) -> bytes | None:
    """The control archive with the version dpkg should read, or None when it already says it."""
    out = io.BytesIO()
    changed = False
    with tarfile.open(fileobj=io.BytesIO(control), mode="r:gz") as source, \
            gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as packed, \
            tarfile.open(fileobj=packed, mode="w", format=tarfile.GNU_FORMAT) as target:
        for info in source:
            data = source.extractfile(info).read() if info.isfile() else None
            if info.isfile() and info.name.removeprefix("./") == "control":
                text = data.decode()
                versions = re.findall(r"^Version: (.*)$", text, re.MULTILINE)
                if versions == [release.deb_version]:
                    return None
                if versions != [release.version]:
                    raise ValueError(f"the package's Version is {versions}, not {release.version}")
                data = text.replace(f"Version: {release.version}\n", f"Version: {release.deb_version}\n").encode()
                info.size = len(data)
                changed = True
            target.addfile(info, None if data is None else io.BytesIO(data))
    if not changed:
        raise ValueError("the package has no control file")
    return out.getvalue()


def rewrite(deb: bytes, release: Release) -> bytes:
    """The package with its beta version written dpkg's way; a final's package as it was."""
    if release.beta is None:
        return deb
    out = [MAGIC]
    seen = False
    for header, data in members(deb):
        if name(header).startswith("control.tar"):
            if name(header) != CONTROL:
                raise ValueError(f"the package's control archive is {name(header)}, not {CONTROL}")
            seen = True
            rewritten = with_version(data, release)
            if rewritten is None:
                return deb
            data = rewritten
            header = header[:48] + f"{len(data):<10}".encode() + header[58:]
        out += [header, data, b"\n" * (len(data) % 2)]
    if not seen:
        raise ValueError(f"the package has no {CONTROL}")
    return b"".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("package", type=Path)
    parser.add_argument("version", help="the app's version, as the updater compares it")
    args = parser.parse_args()
    try:
        release = parse(args.version)
        before = args.package.read_bytes()
        after = rewrite(before, release)
    except (ValueError, tarfile.TarError, OSError) as error:
        raise SystemExit(f"{args.package}: {error}")
    if after != before:
        args.package.write_bytes(after)
    print(f"{args.package.name}: Version: {release.deb_version}")


if __name__ == "__main__":
    main()
