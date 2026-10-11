---
id: task-2
title: "macOS: cookie import finds no browsers (works on Linux)"
column: todo
position: 1
priority: high
tags:
  - bug
  - macos
  - computer
  - cookies
  - 0.36.1
relatedFiles:
  - crates/hotline-core/src/computer/cookies.rs
createdAt: "2026-10-11T20:47:12.297Z"
updatedAt: "2026-10-11T21:24:48.421Z"
---

## Description
Settings › Computer › import cookies lists no browsers on macOS even with Brave installed and signed in. The same flow works on Linux.

Cause (crates/hotline-core/src/computer/cookies.rs): macOS privacy protection blocks reading another app's data dir. On George's Mac, listing ~/Library/Application Support/BraveSoftware/Brave-Browser and reading its `Local State` both fail with EPERM "Operation not permitted" (com.apple.macl), while stat on Default/Cookies succeeds. chromium_profiles() finds profiles only via read_dir + Local State, both fail silently, so profiles is empty and detect() drops the browser. Copying the profile for the headless read would hit the same wall.

Fix:
- Detect EPERM and say so ("macOS is blocking Hotline from reading Brave's data") with a button to Privacy & Security › Full Disk Access (x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles), instead of showing nothing.
- Probe Default / Profile N directly as a fallback so the browser at least shows as found.
- Cover more macOS browsers: Arc, Vivaldi, Chrome Beta/Canary, Edge Beta/Dev/Canary, Chromium, Helium/Dia. Safari (binarycookies in a sandboxed container) is a separate decision.

Done when: on a clean Mac with Brave or Chrome, the import lists the profiles or says exactly which permission to grant.

## Log
- 2026-10-11T22:00:59.155Z: [claude] Fixed on release/0.36.1 (54f70eb): detect() lists a refused browser as blocked (HostBrowser.blocked), the picker names it and opens Full Disk Access; macOS Firefox root fixed; Chrome Beta/Dev/Canary, Chromium, Edge previews, Arc, Vivaldi, ~/Applications added. Verified on this Mac: Brave now shows as blocked instead of missing.
