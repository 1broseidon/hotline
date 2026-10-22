---
id: decision-33
title: A credential capture is bound to one live session lease, teammate and requested origin; only an…
type: decision
status: current
claimKey: secrets.capture-authority
kind: decision
recheck: Revisit if credential capture must survive a stopped session or restart; that would need an explicit durable grant.
createdAt: "2026-10-22T05:25:48.977Z"
---

## Description
A credential capture is bound to one live session lease, teammate and requested origin; only an owner or phone answer can store and grant it.
The capture command never accepts a replacement site or teammate. Its values have no transcript representation and are redacted from debug/parse errors. One request lock serializes answering, expiry, withdrawal and passkey storage. Saved cards cannot recreate that lease after restart. Call cards remain in home chat and voice receives metadata only. A matching existing key is granted without replacing its values or its saved Ask me policy.
