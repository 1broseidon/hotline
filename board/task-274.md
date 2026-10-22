---
id: task-274
title: "Computer (hotline-computer): GET /pages lists the sites its browser has open, for the desk's cookie offer"
column: todo
position: 3
priority: high
tags:
  - credentials
  - cookies
  - hotline-computer
parentId: epic-24
createdAt: "2026-10-22T15:31:07.467Z"
---

## Description
In ~/Projects/personal/hotline-computer (ext/v3, rc.8). A bearer door, GET /pages, answers {"pages": [{"origin": "https://porkbun.com", "url": "https://porkbun.com/account", "title": "…", "openedAt": <ms>}]} for each top-level page tab of the managed Chromium (from CDP target info), web origins only (http/https), newest first. No extension change. 404 from an older computer tells the desk there is no offer.

Done when:
- The door lists the open top-level pages and drops a closed one: contract test (tests/contract.rs).
- Non-web targets (chrome://, the extension, devtools) are never listed: unit test.
- make check passes; image built as rc.8 and the door checked against it.
