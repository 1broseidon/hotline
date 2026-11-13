import base64
import json
import unittest
from beta_pointer import BRANCH, FILE, checked, move, moves, published
from release_version import parse
from updater_manifest import PACKAGES

REPOSITORY = "1Broseidon/hotline"


def manifest(version: str) -> bytes:
    """A release's own manifest, as scripts/updater_manifest.py writes it."""
    base = f"https://github.com/{REPOSITORY}/releases/download/desktop-v{version}/"
    platforms = {target: {"url": f"{base}hotline_{version}_{suffix}", "signature": f"signed {version}"}
                 for target, suffix in PACKAGES.items()}
    return (json.dumps({"version": version, "notes": "Notes", "platforms": platforms}, indent=2) + "\n").encode()


class Repository:
    """GitHub's git data, as far as the pointer uses it: one branch, one file, fast-forward only."""

    def __init__(self):
        self.commits = {}
        self.trees = {}
        self.head = None
        self.calls = []
        self.before_ref_moves = lambda: None

    def content(self):
        return None if self.head is None else self.trees[self.commits[self.head]["tree"]].get(FILE)

    def release(self, version):
        """A release published before this script existed, or by another run."""
        tree = f"tree{len(self.trees)}"
        self.trees[tree] = {FILE: manifest(version)}
        commit = f"commit{len(self.commits)}"
        self.commits[commit] = {"tree": tree, "parents": [self.head] if self.head else []}
        self.head = commit

    def call(self, method, path, body=None):
        self.calls.append(f"{method} {path}")
        if (method, path) == ("GET", f"git/ref/heads/{BRANCH}"):
            return (404, {"message": "Not Found"}) if self.head is None else (200, {"object": {"sha": self.head}})
        if method == "GET" and path.startswith(f"contents/{FILE}?ref="):
            held = self.trees[self.commits[path.split("=", 1)[1]]["tree"]].get(FILE)
            if held is None:
                return 404, {"message": "Not Found"}
            return 200, {"content": base64.encodebytes(held).decode()}
        if (method, path) == ("POST", "git/trees"):
            sha = f"tree{len(self.trees)}"
            self.trees[sha] = {entry["path"]: entry["content"].encode() for entry in body["tree"]}
            return 201, {"sha": sha}
        if (method, path) == ("POST", "git/commits"):
            sha = f"commit{len(self.commits)}"
            self.commits[sha] = {"tree": body["tree"], "parents": body["parents"], "message": body["message"]}
            return 201, {"sha": sha}
        if (method, path) == ("POST", "git/refs"):
            self.before_ref_moves()
            if self.head is not None or body["ref"] != f"refs/heads/{BRANCH}":
                return 422, {"message": "Reference already exists"}
            self.head = body["sha"]
            return 201, {}
        if (method, path) == ("PATCH", f"git/refs/heads/{BRANCH}"):
            self.before_ref_moves()
            if body["force"] or self.commits[body["sha"]]["parents"] != [self.head]:
                return 422, {"message": "Update is not a fast forward"}
            self.head = body["sha"]
            return 200, {}
        raise AssertionError(f"not a call the pointer makes: {method} {path}")


class BetaPointerTests(unittest.TestCase):
    def publish(self, repository, version):
        return move(repository, manifest(version), parse(version))

    def test_the_first_beta_makes_the_branch_and_each_release_after_it_moves_the_pointer(self):
        repository = Repository()
        self.assertIn("now names 0.49.0-beta.1", self.publish(repository, "0.49.0-beta.1"))
        self.assertEqual(repository.content(), manifest("0.49.0-beta.1"))
        self.assertEqual(repository.commits[repository.head]["parents"], [])
        self.assertEqual(repository.commits[repository.head]["message"], "Beta track: 0.49.0-beta.1")
        first = repository.head
        self.publish(repository, "0.49.0-beta.2")
        self.assertEqual(repository.content(), manifest("0.49.0-beta.2"))
        self.assertEqual(repository.commits[repository.head]["parents"], [first])

    def test_a_final_after_its_betas_reaches_the_beta_track_and_the_next_beta_follows_it(self):
        repository = Repository()
        for version in ["0.49.0-beta.1", "0.49.0-beta.2", "0.49.0", "0.50.0-beta.1", "0.50.0"]:
            self.assertIn(f"now names {version}", self.publish(repository, version))
            self.assertEqual(repository.content(), manifest(version))
        self.assertEqual(list(repository.trees[repository.commits[repository.head]["tree"]]), [FILE])

    def test_the_pointer_never_moves_back_to_an_older_release(self):
        repository = Repository()
        self.publish(repository, "0.49.0-beta.2")
        head = repository.head
        for older in ["0.48.4", "0.49.0-beta.1", "0.49.0-beta.2"]:
            self.assertIn("left as it is", self.publish(repository, older))
            self.assertEqual(repository.head, head)
            self.assertEqual(repository.content(), manifest("0.49.0-beta.2"))
        self.assertTrue(moves(manifest("0.49.0-beta.2"), parse("0.49.0")))
        self.assertFalse(moves(manifest("0.49.0"), parse("0.49.0-beta.3")))
        self.assertTrue(moves(None, parse("0.1.0")))

    def test_a_pointer_that_names_no_release_is_left_for_a_person(self):
        for content in [b"not json", b"{}", b'{"version": "0.49.0-rc.1"}', b"[]"]:
            with self.assertRaisesRegex(ValueError, "does not name a release"):
                moves(content, parse("0.49.0"))

    def test_a_release_that_moved_the_branch_meanwhile_is_a_refusal_not_a_lost_update(self):
        repository = Repository()
        self.publish(repository, "0.49.0-beta.1")
        repository.before_ref_moves = lambda: (repository.release("0.49.0"), setattr(repository, "before_ref_moves", lambda: None))
        with self.assertRaisesRegex(ValueError, "moving update-tracks was refused .422.: Update is not a fast forward"):
            self.publish(repository, "0.49.0-beta.2")
        self.assertEqual(repository.content(), manifest("0.49.0"))
        # The same race for the first release of all: the branch appears before it is made.
        fresh = Repository()
        fresh.before_ref_moves = lambda: (fresh.release("0.49.0-beta.1"), setattr(fresh, "before_ref_moves", lambda: None))
        with self.assertRaisesRegex(ValueError, "creating update-tracks was refused"):
            self.publish(fresh, "0.49.0-beta.2")
        self.assertEqual(fresh.content(), manifest("0.49.0-beta.1"))

    def test_a_run_that_stops_before_the_branch_moves_leaves_the_old_manifest(self):
        for failing in ["POST git/trees", "POST git/commits", f"PATCH git/refs/heads/{BRANCH}"]:
            repository = Repository()
            self.publish(repository, "0.49.0-beta.1")
            head, working = repository.head, repository.call

            def call(method, path, body=None, failing=failing, working=working):
                if f"{method} {path}" == failing:
                    return 502, {"message": "Bad Gateway"}
                return working(method, path, body)

            repository.call = call
            with self.assertRaisesRegex(ValueError, "refused .502."):
                self.publish(repository, "0.49.0-beta.2")
            self.assertEqual(repository.head, head)
            self.assertEqual(repository.content(), manifest("0.49.0-beta.1"))

    def test_only_a_releases_own_complete_manifest_is_accepted(self):
        release = parse("0.49.0-beta.1")
        good = json.loads(manifest("0.49.0-beta.1"))
        self.assertEqual(checked(manifest("0.49.0-beta.1"), release, REPOSITORY), good)

        def broken(change):
            data = json.loads(manifest("0.49.0-beta.1"))
            change(data)
            return json.dumps(data).encode()

        deb = "linux-x86_64-deb"
        for change, reason in [
            (lambda data: data.update(version="0.49.0"), "is for '0.49.0'"),
            (lambda data: data["platforms"].pop(deb), "exactly the nine"),
            (lambda data: data["platforms"].update({"linux-x86_64": data["platforms"][deb]}), "exactly the nine"),
            (lambda data: data["platforms"][deb].update(url="https://example.com/hotline_0.49.0-beta.1_linux_x86_64.deb"), "not this release's"),
            (lambda data: data["platforms"][deb].update(url=data["platforms"][deb]["url"].replace("-beta.1/", "/")), "not this release's"),
            (lambda data: data["platforms"][deb].update(url=data["platforms"]["linux-x86_64-rpm"]["url"]), "not this release's"),
            (lambda data: data["platforms"][deb].update(signature=" "), "has no signature"),
            (lambda data: data["platforms"][deb].pop("signature"), "has no signature"),
            (lambda data: data.pop("platforms"), "exactly the nine"),
        ]:
            with self.assertRaisesRegex(ValueError, reason):
                checked(broken(change), release, REPOSITORY)
        with self.assertRaisesRegex(ValueError, "not this release's"):
            checked(manifest("0.49.0-beta.1"), release, "someone/else")

    def test_the_pointer_waits_until_the_release_and_every_package_answer(self):
        release = parse("0.49.0-beta.1")
        asked = []
        fetch = lambda url: (asked.append(url), manifest("0.49.0-beta.1"))[1]
        self.assertEqual(published(REPOSITORY, release, fetch, lambda url: True), manifest("0.49.0-beta.1"))
        self.assertEqual(asked, [f"https://github.com/{REPOSITORY}/releases/download/desktop-v0.49.0-beta.1/updater-0.49.0-beta.1.json"])
        gone = f"https://github.com/{REPOSITORY}/releases/download/desktop-v0.49.0-beta.1/hotline_0.49.0-beta.1_linux_aarch64.rpm"
        with self.assertRaisesRegex(ValueError, "do not answer, so the pointer stays: " + gone.replace(".", r"\.") + "$"):
            published(REPOSITORY, release, fetch, lambda url: url != gone)
        # A draft serves nothing, so its manifest is never read at all.
        def unpublished(url):
            raise ValueError("404")
        with self.assertRaises(ValueError):
            published(REPOSITORY, release, unpublished, lambda url: True)


if __name__ == "__main__":
    unittest.main()
