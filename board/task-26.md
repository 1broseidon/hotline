---
id: task-26
title: A proper dark sign-in page for the MCP OAuth callback, with the mark
column: todo
position: 28
priority: low
tags:
  - improvement
  - bro-250
createdAt: "2026-10-11T21:05:35.422Z"
description: |-
  After signing in to an MCP server, the browser lands on Hotline's loopback callback, which answers with a bare unstyled page: "Hotline sign-in complete. You may close this tab." The two failure pages are just as bare. All three are inline strings in `crates/hotline-core/src/mcp/oauth.rs` (\~787, \~792, \~806), written by `write_callback_response`.

  Make them look like Hotline.

  * **One self-contained HTML page**, templated for success and failure. It must be inline: no external fonts, scripts or images, since it is served from a short-lived `127.0.0.1` listener. Keep it under a few KB and set a strict CSP header in `write_callback_response`.
  * **Dark by default**, using the desk's tokens (`ui/src/tokens.css`: well/canvas, ink, mint accent), with a `prefers-color-scheme: light` variant to match the app.
  * **The mark** inlined as SVG from `assets/hotline-mark.svg` (or `-small`), centred above the message.
  * **Success copy:** a mint check, "Signed in to {server name}", and "You can close this tab and go back to Hotline." The server's name is available on the pending login, so it can be shown; escape it.
  * **Failure copy:** a quiet danger mark, "Couldn't finish signing in to {server}", the escaped reason in smaller ink-3 text, and "Go back to Hotline and try again."
  * Optionally try `window.close()` after a moment on success. Browsers only allow it for script-opened tabs, so the text must still read right if the tab stays open.
  * Keep the 200/400 statuses and `html_escape` on every interpolated value. Add a test that the reason is escaped and that the page contains no external URLs.

  If other loopback callbacks get pages later (provider or agent sign-ins), they should reuse the same template. Today only MCP OAuth has one.

  Migrated from Linear BRO-250: https://linear.app/1broseidon/issue/BRO-250/a-proper-dark-sign-in-page-for-the-mcp-oauth-callback-with-the-mark (2026-10-11).
updatedAt: "2026-10-11T21:05:35.709Z"
---

## Log
- 2026-10-14T01:44:31.408Z: [claude] Consolidated into task-25 as subtask task-25-1 (same mcp/oauth.rs surface); closing as duplicate, not done.
