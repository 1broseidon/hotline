# hotline.dev/docs

The user docs, on [Starlight](https://starlight.astro.build). Pages are
Markdown under `src/content/docs/`; the sidebar is in `astro.config.mjs`.
Facts come from the source and from `../docs/`; a page that says something
the code does not do is a bug.

The build lands in `../site/public/docs/` (untracked), so the docs ship as
part of the landing page's Worker. Links inside pages carry the `/docs/`
base.

```bash
bun install
bun run dev                 # http://localhost:4321/docs/
make -C .. site-deploy      # build, then deploy hotline.dev with the docs under it
```
