import gzip
import io
import tarfile
import unittest
from deb_version import MAGIC, members, name, rewrite
from release_version import parse

CONTROL = """Package: hotline
Version: {version}
Architecture: amd64
Installed-Size: 1234
Maintainer: Hotline
Depends: libwebkit2gtk-4.1-0, libgtk-3-0
Description: Hotline
 Version: is also a word in this line, and stays.
"""


def packed(files: dict[str, bytes]) -> bytes:
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as zipped, tarfile.open(fileobj=zipped, mode="w") as tar:
        folder = tarfile.TarInfo("./")
        folder.type, folder.mode = tarfile.DIRTYPE, 0o755
        tar.addfile(folder)
        for path, data in files.items():
            info = tarfile.TarInfo(path)
            info.size, info.mode, info.mtime = len(data), 0o755 if path.endswith("postinst") else 0o644, 1_700_000_000
            tar.addfile(info, io.BytesIO(data))
    return out.getvalue()


def member(title: str, data: bytes) -> bytes:
    header = f"{title:<16}{'0':<12}{'0':<6}{'0':<6}{'100644':<8}{len(data):<10}`\n".encode()
    return header + data + b"\n" * (len(data) % 2)


def deb(version: str, control_name="control.tar.gz", control=CONTROL) -> bytes:
    """A package laid out as the bundler lays one out, with a member of odd length."""
    files = {"./control": control.format(version=version).encode(), "./md5sums": b"d41d8  usr/bin/hotline\n",
             "./postinst": b"#!/bin/sh\nexit 0\n"}
    data = packed({"./usr/bin/hotline": b"the app, which says it is " + version.encode()})
    return (MAGIC + member("debian-binary", b"2.0\n") + member(control_name, packed(files))
            + member("data.tar.gz", data) + member("_gpgorigin", b"odd"))


def control_files(package: bytes) -> dict[str, tuple[bytes, int]]:
    archive = dict((name(header), data) for header, data in members(package))["control.tar.gz"]
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        return {info.name: (tar.extractfile(info).read() if info.isfile() else b"", info.mode) for info in tar}


class DebVersionTests(unittest.TestCase):
    def test_a_betas_package_sorts_before_its_final_and_nothing_else_in_it_changes(self):
        before = deb("0.49.0-beta.1")
        after = rewrite(before, parse("0.49.0-beta.1"))
        self.assertNotEqual(after, before)
        self.assertEqual([name(header) for header, _ in members(after)], ["debian-binary", "control.tar.gz", "data.tar.gz", "_gpgorigin"])
        old, new = dict(members(before)), dict(members(after))
        for header in old:
            if name(header) != "control.tar.gz":
                self.assertEqual(new[header], old[header])
        was, now = control_files(before), control_files(after)
        self.assertEqual(list(now), list(was))
        self.assertEqual(now["./control"][0].decode(), CONTROL.format(version="0.49.0~beta.1"))
        self.assertEqual(len(was), 4)
        for path in was:
            if path != "./control":
                self.assertEqual(now[path], was[path])
        self.assertEqual(now["./postinst"][1], 0o755)
        # The app inside is the bytes the bundler packed: it still says its semver.
        self.assertIn(b"0.49.0-beta.1", gzip.decompress(dict((name(h), d) for h, d in members(after))["data.tar.gz"]))

    def test_every_member_starts_on_an_even_byte_whatever_the_new_length(self):
        for version in ["0.49.0-beta.1", "0.49.0-beta.10", "1.0.0-beta.2"]:
            after = rewrite(deb(version), parse(version))
            sizes = [len(data) for _, data in members(after)]
            self.assertEqual(len(after), len(MAGIC) + sum(60 + size + size % 2 for size in sizes))
            self.assertIn(f"Version: {parse(version).deb_version}\n".encode(), control_files(after)["./control"][0])

    def test_a_final_package_and_one_already_written_dpkgs_way_are_left_as_they_are(self):
        final = deb("0.49.0")
        self.assertIs(rewrite(final, parse("0.49.0")), final)
        once = rewrite(deb("0.49.0-beta.1"), parse("0.49.0-beta.1"))
        self.assertIs(rewrite(once, parse("0.49.0-beta.1")), once)

    def test_a_package_of_another_version_or_shape_is_refused(self):
        beta = parse("0.49.0-beta.1")
        for package, reason in [
            (deb("0.49.0-beta.2"), "not 0.49.0-beta.1"),
            (deb("0.49.0"), "not 0.49.0-beta.1"),
            (deb("0.49.0-beta.1", control="Package: hotline\n"), "not 0.49.0-beta.1"),
            (deb("0.49.0-beta.1", control="Version: {version}\nVersion: {version}\n"), "not 0.49.0-beta.1"),
            (deb("0.49.0-beta.1", control_name="control.tar.xz"), "control.tar.xz, not control.tar.gz"),
            (deb("0.49.0-beta.1", control_name="other.tar.gz"), "no control.tar.gz"),
            (deb("0.49.0-beta.1")[:-2], "cut short"),
            (deb("0.49.0-beta.1")[:-30], "damaged"),
            (b"\xed\xab\xee\xdb an rpm", "not a .deb"),
        ]:
            with self.assertRaisesRegex(ValueError, reason):
                rewrite(package, beta)


if __name__ == "__main__":
    unittest.main()
