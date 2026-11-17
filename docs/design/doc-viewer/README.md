# Opening documents on the desk

`doc-viewer.html` is the scenario page George answered on 17 Nov 2026 (every
choice took the pick); `mock.html` draws its scenes with the app's own classes
and `shoot.mjs` shoots them into `shots/`. `mock.html` reads the window's
stylesheet and fonts from `assets/`, which is not committed: rebuild it with
`vite build` (`cd ui && bun run build`) and copy `dist/assets/`'s stylesheet in
as `assets/app.css`, with its fonts beside it.

`shoot.mjs` needs Playwright with its Chromium. It imports it from
`../../ui-climb/tools/node_modules/playwright/index.mjs`, the ui-climb rig in
the workspace it was drawn in, a path that does not exist in this repo: point
the import at any Playwright install before running `bun shoot.mjs`.
