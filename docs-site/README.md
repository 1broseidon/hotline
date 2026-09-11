# docs.toad.team

The user docs, on [Starlight](https://starlight.astro.build). Pages are
Markdown under `src/content/docs/`; the sidebar is in `astro.config.mjs`.
Facts come from the source and from `../docs/`; a page that says something
the code does not do is a bug.

```bash
bun install
bun run dev                     # http://localhost:4321
bun run build                   # dist/
CLOUDFLARE_ACCOUNT_ID=… npx wrangler deploy   # Worker `toad-docs`, docs.toad.team
```
