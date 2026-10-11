---
id: task-40
title: "Drawn (2D) teammate faces from an uploaded SVG: the risks, for you to weigh"
column: todo
position: 42
tags:
  - bro-276
createdAt: "2026-10-11T21:06:03.137Z"
description: |-
  Follow-up to #175 / [#176](<https://github.com/1broseidon/hotline/issues/176>), which let the person give a teammate a photo or a video clip per mood.

  A fourth style, **drawn**, would let any teammate have a living face from a single file: an SVG the person uploads, drawn to a small rig of named parts (`data-part="lid"`, `"eye"`, `"ear-l"`, `"paw-reach"`, …) that the window moves per mood with CSS. No video to make, and a model can draw one from a photo and a prompt. I have a branch for it and will open a PR, but an uploaded SVG is markup, not a picture, so I'd like you to weigh the risks first. This is my reading of them.

  ## The big choice: how the drawing reaches the screen

  **What the PR does: show it as an image.** The window copies the drawing, puts the rig's CSS and the current mood inside the copy, and shows it with `<img>`, the way `Mermaid.tsx` shows diagrams. An SVG shown as an image cannot run script, load anything or follow a link, and its CSS cannot touch the rest of the window. So there is no sanitizer to get wrong: the worst a hostile drawing can do is look wrong inside its own circle.

  What that costs:

  * A mood change swaps one image for the next instead of easing between them. Entrances like the paw reaching or arms folding are keyframes, so they still play.
  * The eyes can't follow the caret while you type, and blinking is on a fixed loop rather than random. Both need script reaching into the drawing.

  **The alternative: inline the SVG into the page** so CSS and script can drive its parts live. It looks better, but every one of these then becomes a way in:

  * **No CSP.** `tauri.conf.json` has `"csp": null`, so a script that slips past a sanitizer runs with the window's full access to the Tauri commands and the desk wire.
  * **A sanitizer has to be right about all of it:** `<script>`; `on*` handlers; `javascript:` and external `href`s; `<foreignObject>` carrying HTML; SMIL `<set>`/`<animate>` rewriting an `href` after cleaning; mutation XSS from re-parsing serialized markup; and `<use>` pointing outside the drawing.
  * **CSS inside it is global.** An inlined `<style>` restyles the whole app, and `url()` in it or in attributes can fetch from the network, which is a tracking beacon.
  * **Layout escape.** A root `class` or `style` (Tailwind `fixed inset-0 z-50`, say) or `overflow: visible` lets a drawing paint over the window and imitate its UI.
  * **ids** in the drawing collide with the window's own, and with other copies of the same face.

  I don't think the inline route is worth it for a cosmetic feature without a CSP, which is why the PR doesn't take it.

  ## Smaller risks in what the PR does

  * **The desk's check is a screen, not a guard.** `look.drawing_set` refuses anything over 1 MB, anything that isn't an SVG, and anything with a doctype, entities, `<script>` or `<foreignObject>`, by substring and without parsing. It keeps billion-laughs entity expansion and the obvious payloads off disk. The image display is what makes a drawing safe.
  * **Other readers.** Any seat, the phone included, can read the drawing with `look.clip_read`, like a picture. A future client that inlined it would bring back everything above, so the rule "a drawing is only ever shown as an image" is written into `docs/wire.md` and `docs/design.md`.
  * **Cost on screen.** A face shows in the rail, the conversation header and the activity mark at once. A 1 MB drawing with animated blur filters could be heavy, and each drawing keeps up to nine mood copies as blob URLs, which last as long as the window, as pictures already do. `prefers-reduced-motion` stops every animation.
  * **The rig becomes an API.** The part names, the 200 × 200 viewBox and lids that open by 30 units are a contract with every drawing anyone has uploaded. Changing `rig.css` later changes all of them, so it needs to be treated as versioned.
  * **Parsing.** The window parses the drawing once per mood with `DOMParser` on the main thread, up to 1 MB each. That's once per hash and mood, then cached.
  * **Review size.** 23 files, +630 −50 on top of [#176](<https://github.com/1broseidon/hotline/issues/176>): the core command pair with tests, `rig.css`, a 20-line function that bakes a mood into a copy, the component, the Face section row, and `docs/faces.md` with the rig spec and a prompt for drawing one.

  If you'd rather not take on a fourth style at all, [#176](<https://github.com/1broseidon/hotline/issues/176>) stands on its own, and a drawn face can always be recorded as a video clip instead.

  Migrated from Linear BRO-276: https://linear.app/1broseidon/issue/BRO-276/drawn-2d-teammate-faces-from-an-uploaded-svg-the-risks-for-you-to (2026-10-11).

  ## Status (2026-10-11 review)

  External PRs hotline#176 and #178 (iggyuga) are open; need a review decision.
updatedAt: "2026-10-11T21:06:03.424Z"
---
