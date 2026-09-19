#!/usr/bin/env python3
"""Render og.html to site/public/og.png, and check it against the card it replaces.

The fonts go into the page as data URIs first. Headless Chromium will quietly
substitute a system grotesque for a webfont that has not arrived — every
measurement moves and nothing says so — and embedding removes both the network
and the file:// question.

The geometry below is the original Toad card's, measured off it. The wordmark
is a different word now, so its width is expected to differ; everything else is
held in place.
"""
import base64
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent / "public" / "og.png"

# Ink boxes the render has to land on: x, y, width, height. None where the
# word itself changed and only the left edge and baseline still mean anything.
EXPECTED = [
    ("mark", 96, 196, 150, 77),
    ("wordmark", 96, 315, None, 67),
    ("subtitle", 96, 417, 520, 27),
    ("footer", 96, 554, None, 15),
]


def page() -> pathlib.Path:
    html = (HERE / "og.html").read_text()
    css = (HERE / "fonts.css").read_text()
    css = re.sub(
        r"url\('fonts/([^']+)'\) format\('woff2'\)",
        lambda m: "url('data:font/woff2;base64,%s') format('woff2')"
        % base64.b64encode((HERE / "fonts" / m.group(1)).read_bytes()).decode(),
        css,
    )
    html = html.replace('<link rel="stylesheet" href="fonts.css">', f"<style>\n{css}\n</style>")
    out = HERE / ".og.inlined.html"
    out.write_text(html)
    return out


def bands(path: pathlib.Path, threshold: int = 40) -> list[tuple[int, int, int, int]]:
    """The ink of each row of the card, as (x, y, width, height) boxes.

    The background is a gradient, so ink is what stands out from the darkest
    pixel on its own row rather than from one fixed number.
    """
    from PIL import Image

    im = Image.open(path).convert("L")
    width, height = im.size
    px = im.load()
    rows = []
    for y in range(height):
        floor = min(px[x, y] for x in range(width))
        rows.append([x for x in range(width) if px[x, y] - floor > threshold])
    out, run = [], None
    for y, xs in enumerate(rows):
        if xs and run is None:
            run = [y, y, min(xs), max(xs)]
        elif xs:
            run[1], run[2], run[3] = y, min(run[2], min(xs)), max(run[3], max(xs))
        elif run is not None:
            out.append(run)
            run = None
    if run:
        out.append(run)
    return [(x0, y0, x1 - x0 + 1, y1 - y0 + 1) for y0, y1, x0, x1 in out if y1 - y0 >= 3]


def main() -> int:
    html = page()
    subprocess.run(
        ["chromium", "--headless", "--disable-gpu", "--hide-scrollbars",
         "--force-device-scale-factor=1", "--window-size=1200,630",
         "--virtual-time-budget=6000", f"--screenshot={OUT}", str(html)],
        check=True, capture_output=True,
    )
    html.unlink()

    found = bands(OUT)
    ok = True
    print(f"{OUT}")
    for (name, ex, ey, ew, eh), (x, y, w, h) in zip(EXPECTED, found + [(0, 0, 0, 0)] * 4):
        notes = []
        if x != ex:
            notes.append(f"left {x} want {ex}")
        if y != ey:
            notes.append(f"top {y} want {ey}")
        if ew is not None and w != ew:
            notes.append(f"width {w} want {ew}")
        if h != eh:
            notes.append(f"height {h} want {eh}")
        ok &= not notes
        mark = "ok " if not notes else "OFF"
        print(f"  {mark} {name:9} x {x:4} y {y:4} w {w:4} h {h:3}"
              + (f"   {'; '.join(notes)}" if notes else ""))
    if len(found) != len(EXPECTED):
        print(f"  OFF {len(found)} bands of ink, expected {len(EXPECTED)}")
        ok = False
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
