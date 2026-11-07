# A desk browser as a live screen (task-390, phase 2)

Design only; nothing here is built. Phase 1 made a server's headless browser
a live screen (`docs/wire.md`, Browser screens): X DAMAGE says what changed on
each page's window, MIT-SHM reads it, PNG rectangles go to the viewer in the
computer viewer's protocol, and XTEST is the person's hands. Phase 2 is the
same socket for a desk browser on the person's own computer, the pane's
native web view, so a phone or a linked desktop watches and drives it live
too. A native web view has no X display to watch, so each platform pictures
the page itself and finds what changed by comparing tiles.

## What stays the same

- The door and the bridge route are phase 1's: `/v2/browser/{personaId}/ws`
  and `/browser/{personaId}/ws`, the `browserStream` capability, the viewer
  token minted for the browser, owner-only `control` mapped to
  `browser_take`, the grace hand-back, and the wire commands for the bar.
- The seam is `DeskBrowser::streams` and `DeskBrowser::screen`, which the
  app's `Native` would implement in process (no helper socket): a
  `screen::Link` whose `shown` queue holds one message, so a slow viewer
  holds capture back instead of queueing frames.
- Input: the platforms already make the person's own events
  (`browser/native/{macos,windows,linux}.rs` `input`); phase 2 widens them
  from click, hover and key to move, button down and up (drag), wheel and
  any key, each fire-and-forget.

## How each platform pictures the page

Common: while a viewer watches, picture the page into a frame buffer; split
it into 64×64 tiles; hash each tile (xxh3 of its rows); send the changed
tiles' bounding rectangles (merged when close, so a scroll is one rectangle)
as PNGs. Picture on a clock that follows activity: at the platform's best
rate for a second after the person's input or a load, falling to 2 a second
when the last pictures were unchanged, and nothing while nobody watches. A
page script can mark change sooner (a `MutationObserver`, a
`requestAnimationFrame` that ran) so a quiet page is not pictured blind.

| Platform | Pictured by | Cost of one 1280×800 picture | Expected frames a second while scrolling | Idle |
| --- | --- | --- | --- | --- |
| macOS 14+ (WKWebView) | `takeSnapshot(with:)` into a `CGImage`, read as BGRA | about 15–30 ms on Apple silicon (the web process paints, then the image crosses to the app) | 20–30 | 2 snapshots a second, nothing sent when unchanged |
| Windows (WebView2) | `Windows.Graphics.Capture` on the pane's own visual (`GraphicsCaptureItem` from the composition controller), which says when it drew and, on Windows 11, which regions (`DirtyRegions`); `CapturePreviewAsync` where that is missing | a GPU texture copy per frame, 2–5 ms; `CapturePreviewAsync` 40–80 ms | 30–60 with capture, 10–15 with `CapturePreviewAsync` | none with capture (frames come only on drawing); 2 a second otherwise |
| Linux desktop (WebKitGTK window) | on X11, the server's own way: DAMAGE and MIT-SHM on the pane's window; on Wayland, `webkit_web_view_get_snapshot` | X11: as the server, 5–10 ms; snapshot 20–40 ms in software | X11: 40–60, as the server; Wayland: 15–25 | X11: none; Wayland: 2 snapshots a second |

Encoding is the server's: PNG (fdeflate, fast) of the changed rectangles,
about 150–200 KB for a whole scrolled 1280×800 page and a few KB for a
caret or a hover. The phone's link is what limits a scroll from away, not
capture: on a 20 Mbit/s link a whole-page frame takes about 70 ms, so a
remote scroll runs at 10–15 frames a second with frames dropped, not queued.

## Open questions

- macOS: is `takeSnapshot` steady at 30 a second without disturbing the
  page, or does it need a cap and back-off while the page animates?
- Windows: capture's yellow border on Windows 10, and whether the app may
  turn it off (`IsBorderRequired`) without a prompt.
- Any platform: a pane that is hidden (the person closed it) must keep
  painting for its pictures; the macOS and Windows panes are parked offscreen
  today and their throttling while hidden needs checking.
- A JPEG mode for a slow phone link, which the computer viewer protocol does
  not have yet: a header flag, or a second binary kind.
