# hotline.dev

The landing page: one static file, `public/index.html`, carrying the launch
film as its hero, plus `og.png`, `favicon.svg`, `_headers`, `robots.txt` and
`sitemap.xml`. The docs are built into `public/docs/` from `../docs-site`
and are not tracked. Download links resolve against the latest GitHub
release at load time, so a release needs no change here.

```bash
make site-deploy   # from the repo root: builds the docs, deploys Worker `hotline-site`
```
