---
id: decision-39
title: Cookie offers default to on for all sites; Off prevents automatic host discovery and expires…
type: decision
status: current
claimKey: cookies.offer-preference
kind: decision
tags:
  - cookies
  - settings
createdAt: "2026-10-22T21:12:46.946Z"
---

## Description
Cookie offers default to on for all sites; Off prevents automatic host discovery and expires pending cards before the settings write replies.
The room owns cookieOffers {enabled, sites?}: absent sites means all, a list means only those, and checking every site saves no list. The policy gate orders automatic discovery, settings changes and answers. Sites discovery is a separate explicit desk/owner page read, available even while offers are off; the phone may read the preference but cannot change it or discover host sites. Never remains per teammate and is undone from Settings → Computer → Sites. This controls offers, never grants a lend without Use.
