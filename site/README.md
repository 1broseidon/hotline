# toad.team

The landing page: one static file, `public/index.html`, carrying the launch
film as its hero, plus `og.png`, `favicon.svg`, `_headers`, `robots.txt` and
`sitemap.xml`. Download links resolve against the latest GitHub release at
load time, so a release needs no change here.

```bash
CLOUDFLARE_ACCOUNT_ID=… npx wrangler deploy   # Worker `toad-team`, toad.team
```
