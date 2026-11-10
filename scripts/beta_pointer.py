#!/usr/bin/env python3
"""Point the Beta track at a release that is already published.

A desk on the Beta track reads one fixed address: `beta.json` on the
`update-tracks` branch, a branch that holds that file and nothing else. This
puts a published release's own manifest there, after every release, beta or
final, so a beta desk is offered both.

What is written is never built here. It is the `updater-<version>.json` the
release itself serves, fetched back from the published release, checked to
name that version's nine packages under that release and nowhere else, and
moved only once every one of those packages answers. So the pointer can name
only packages that are public and whose signatures were present when the
release was put together.

The move is one fast-forward of the branch: a desk reads the old manifest or
the new one, never half of either, and a run that stops anywhere before it
leaves the old one in place. The pointer only moves forward by semver, so a
patch to an older line does not take the Beta track back from a newer beta.
"""
import argparse
import base64
import json
import os
import re
import urllib.error
import urllib.request

from release_version import Release, parse
from updater_manifest import PACKAGES

BRANCH = "update-tracks"
FILE = "beta.json"
API = "https://api.github.com"


def downloads(repository: str, release: Release) -> str:
    return f"https://github.com/{repository}/releases/download/desktop-v{release.version}/"


def checked(content: bytes, release: Release, repository: str) -> dict:
    """The manifest, if it is this release's own: its version, its nine packages, each signed."""
    manifest = json.loads(content)
    if manifest.get("version") != release.version:
        raise ValueError(f"the manifest is for {manifest.get('version')!r}, not {release.version}")
    platforms = manifest.get("platforms")
    if not isinstance(platforms, dict) or set(platforms) != set(PACKAGES):
        raise ValueError("the manifest does not name exactly the nine installer targets")
    for target, suffix in PACKAGES.items():
        entry = platforms[target]
        expected = downloads(repository, release) + f"hotline_{release.version}_{suffix}"
        if entry.get("url") != expected:
            raise ValueError(f"{target} points at {entry.get('url')!r}, not this release's {expected}")
        if not isinstance(entry.get("signature"), str) or not entry["signature"].strip():
            raise ValueError(f"{target} has no signature")
    return manifest


def moves(current: bytes | None, release: Release) -> bool:
    """Whether the pointer moves: only to a version newer than the one it names."""
    if current is None:
        return True
    try:
        named = parse(json.loads(current)["version"])
    except (ValueError, KeyError, TypeError) as error:
        raise ValueError(
            f"{FILE} on {BRANCH} does not name a release ({error}); put a published release's "
            "manifest there by hand, or delete the file, then run this again") from error
    return release.order > named.order


class GitHub:
    """The few calls this makes, each answering its status and its JSON."""

    def __init__(self, repository: str, token: str):
        self.repository = repository
        self.token = token

    def call(self, method: str, path: str, body: dict | None = None) -> tuple[int, dict]:
        request = urllib.request.Request(
            f"{API}/repos/{self.repository}/{path}",
            data=None if body is None else json.dumps(body).encode(),
            method=method,
            headers={"Authorization": f"Bearer {self.token}", "Accept": "application/vnd.github+json",
                     "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "hotline-release"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.status, json.loads(response.read() or b"{}")
        except urllib.error.HTTPError as error:
            text = error.read()
            try:
                return error.code, json.loads(text or b"{}")
            except ValueError:
                return error.code, {"message": text.decode(errors="replace")}


def expect(answer: tuple[int, dict], what: str) -> dict:
    status, body = answer
    if not 200 <= status < 300:
        raise ValueError(f"{what} was refused ({status}): {body.get('message', body)}")
    return body


def pointer(github) -> tuple[str | None, bytes | None]:
    """The branch's head and the manifest there; none of either before the first release."""
    status, ref = github.call("GET", f"git/ref/heads/{BRANCH}")
    if status == 404:
        return None, None
    head = expect((status, ref), f"reading {BRANCH}")["object"]["sha"]
    status, file = github.call("GET", f"contents/{FILE}?ref={head}")
    if status == 404:
        return head, None
    return head, base64.b64decode(expect((status, file), f"reading {FILE}")["content"])


def move(github, content: bytes, release: Release) -> str:
    """Moves the pointer, or says why it stays. Nothing is visible until the last call."""
    head, current = pointer(github)
    if not moves(current, release):
        return f"{FILE} already names {json.loads(current)['version']}, which is not older than {release.version}: left as it is"
    tree = expect(github.call("POST", "git/trees", {
        "tree": [{"path": FILE, "mode": "100644", "type": "blob", "content": content.decode()}]}), "writing the manifest")
    commit = expect(github.call("POST", "git/commits", {
        "message": f"Beta track: {release.version}", "tree": tree["sha"], "parents": [head] if head else []}),
        "writing the commit")
    # The branch moves only from the head that was read: a second release
    # that moved it meanwhile makes this a refusal, never a lost update.
    if head is None:
        expect(github.call("POST", "git/refs", {"ref": f"refs/heads/{BRANCH}", "sha": commit["sha"]}), f"creating {BRANCH}")
    else:
        expect(github.call("PATCH", f"git/refs/heads/{BRANCH}", {"sha": commit["sha"], "force": False}), f"moving {BRANCH}")
    return f"{FILE} on {BRANCH} now names {release.version}"


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "hotline-release"}), timeout=60) as response:
        return response.read()


def answers(url: str) -> bool:
    """Whether a download is there, asked without fetching it."""
    request = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "hotline-release"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status == 200
    except urllib.error.URLError:
        return False


def published(repository: str, release: Release, fetch=fetch, answers=answers) -> bytes:
    """The release's own manifest, once it and every package it names are public."""
    content = fetch(downloads(repository, release) + f"updater-{release.version}.json")
    manifest = checked(content, release, repository)
    missing = [entry["url"] for entry in manifest["platforms"].values() if not answers(entry["url"])]
    if missing:
        raise ValueError("these packages do not answer, so the pointer stays: " + ", ".join(missing))
    return content


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    try:
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository):
            raise ValueError("Expected a GitHub owner/repository")
        release = parse(args.version)
        content = published(args.repository, release)
        print(move(GitHub(args.repository, os.environ["GH_TOKEN"]), content, release))
    except (ValueError, urllib.error.URLError) as error:
        raise SystemExit(f"The Beta track was not moved: {error}")


if __name__ == "__main__":
    main()
