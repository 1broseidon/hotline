---
id: task-13
title: Harden the public tunnel and served desk
column: todo
position: 15
priority: high
tags:
  - bro-167
createdAt: "2026-10-11T21:05:23.997Z"
description: |-
  These are findings from the 29 Sep 2026 remote-access security assessment of a served desk behind a Cloudflare tunnel. None of them is an authentication bypass or a data exposure. The children are:

  * BRO-163: unauthenticated connections can use up paired devices' connection budget behind one proxy IP (DoS).
  * BRO-164: Rustls advisory RUSTSEC-2026-0285.
  * BRO-165: HTTP to HTTPS redirect on the tunnel hostname.

  BRO-163 and BRO-164 go in 0.30. BRO-165 is config and can follow.

  ---

  Backlog review 2026-10-11: BRO-163 and BRO-164 shipped in 0.30.0. The only thing left is BRO-165's edge rule, merged here: the docs and rule file exist (`docs/serve.md` \~182-220, `packaging/cloudflare/grizzly-https-redirect.json`) but the rule isn't deployed — `http://grizzly-tunnel.hotline.dev/foo?x=1` returns 403 `native_clients_only` instead of a 301. **Done when:** the Cloudflare redirect rule is deployed and its rule ID recorded.

  Migrated from Linear BRO-167: https://linear.app/1broseidon/issue/BRO-167/harden-the-public-tunnel-and-served-desk (2026-10-11).
updatedAt: "2026-10-11T21:20:18.483Z"
---

## Log
- 2026-10-12T01:51:04.940Z: [claude] Won't do (2026-10-11 sort): the only thing left is an HTTP→HTTPS redirect on George's grizzly tunnel hostname; native clients already require HTTPS and Noise protects the session.
