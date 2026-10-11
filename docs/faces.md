# Drawn faces

A drawn face is one SVG the person uploads for a teammate (teammate pane →
Face → Drawn). The window moves it per mood with CSS it puts inside the
drawing, and shows the result as an image. Nothing in the drawing runs: an
SVG shown as an image cannot run script, load anything or follow a link, the
same reason `Mermaid.tsx` draws diagrams that way.

So a drawing only has to be a still picture whose moving parts are named.
Everything here is optional; a part the drawing does not name simply stays
still.

## The canvas

- `viewBox="0 0 200 200"`, square, the face centred. The window shows it in a
  circle, so keep what matters inside a circle of radius 95 around (100, 100).
- One file, under 1 MB. No `<script>`, `<foreignObject>`, `<!DOCTYPE>` or
  entities: the desk turns those away. External images and fonts will not
  load in an image, so draw everything as shapes, and text as paths or with a
  generic font.
- Gradients and filters are fine; refer to them by `url(#id)`.

## The parts

Name a part with `data-part`. A space-separated list names several, as
`class` does. Lengths are in viewBox units.

| Part | What it is | How it moves |
|---|---|---|
| `head` | Everything that tilts with the head: face, ears, eyes, brows | Breathes, bobs, tilts, bounces. Pivots at 50% across, 85% down its box |
| `ear-l`, `ear-r` | Each ear, inside `head` | Flick, perk up, flatten. Pivot at the base, toward the face |
| `brow-l`, `brow-r` | Each brow | Raise, frown, slant sad |
| `eye` | A group per eye holding its iris and pupil, not the white | Glances: moves up to 6 left or right and 5 up or down |
| `pupil` | The pupil, inside `eye` | Grows (attentive 1.25×, needy 1.45×) |
| `lid` | Each upper lid, **drawn closed** over its eye, clipped to it | Raised 30 to open; blinks back down; half-shut when thinking, working or grumpy |
| `lid-low` | Each lower lid, drawn closed, clipped to the eye | Lowered 30 to open; rises to 10 for a happy squint |
| `arms-rest` | Front legs or arms as they rest, holding `arm-l` and `arm-r` | Hidden while the arms are crossed |
| `arm-l`, `arm-r` | Each resting arm, inside `arms-rest` | Tap up and down while working; `arm-r` hides while the paw reaches |
| `paw-reach` | A paw raised toward the viewer | Hidden except when needy, when it rises in and waves |
| `arms-crossed` | Arms folded across the chest | Hidden except when the arms are crossed |
| `zzz` | Three small Z shapes, one per child | Shown asleep, each child floats up in turn |
| `dots` | Three thinking dots, one per child | Shown while thinking, each child pulses in turn |
| `spark` | Three sparkles, one per child | Shown when a turn is done, each child pops in turn |
| `tear` | A small tear | Shown when needy |

## The moods

The window works out the mood itself; the drawing does not need to know it.
Small copies of a face, and every face while the window is in the
background, show the mood's pose without moving.

| Mood | When | What the rig does |
|---|---|---|
| `idle` | Nothing happening | Breathes, blinks, glances around, an ear flicks |
| `sleep` | Window in the background, or long idle | Lids closed, slow breathing, head lowered, `zzz` |
| `attentive` | Just woke, or reading | Pupils grow, ears and brows up |
| `think` | Thinking | Head tilts, one brow up, eyes up and aside, `dots` |
| `work` | Running a tool, or the person typing | Lids half down, head bobs, arms tap, eyes scan |
| `done` | A turn just ended | Happy squint, three bounces, `spark` |
| `grumpy` | An error | Brows down, lids heavy, ears back |
| `crossed-arms` | Stopped | Grumpy, chin up, eyes aside, `arms-crossed` |
| `needy` | Waiting on the person | Big pupils, sad brows, tilt, `paw-reach` waves, `tear` |

## A prompt that draws one

Give an image model or a coding model this, with a photo:

> Draw a front-facing head-and-shoulders portrait of the character in this
> photo as a single SVG, `viewBox="0 0 200 200"`, kept inside a circle of
> radius 95 around the centre. Use only shapes, gradients and filters; no
> scripts, no embedded images, no external fonts, no `foreignObject`.
> Name the moving parts with `data-part` attributes:
> `head` around everything that tilts with the head;
> `ear-l` and `ear-r` on each ear; `brow-l` and `brow-r` on each brow;
> per eye, an `eye` group holding the iris with a `pupil` inside it, then an
> upper eyelid shape marked `lid` drawn **fully closed** over that eye and a
> lower one marked `lid-low` also drawn closed, both clipped to the eye's
> outline so moving them up or down by 30 units opens the eye;
> `arms-rest` around both resting front arms, each marked `arm-l` or `arm-r`;
> a raised paw or hand reaching toward the viewer marked `paw-reach`;
> arms folded across the chest marked `arms-crossed`;
> and, near the top right, a group `zzz` of three small Zs, a group `dots` of
> three small circles and a group `spark` of three four-pointed sparkles, plus
> a small tear under one eye marked `tear`. Draw the extras where they would
> appear; the app hides them until their mood.

Check a drawing by opening it in a browser: with the lids drawn closed it
should look asleep, with the extras all visible at once.
