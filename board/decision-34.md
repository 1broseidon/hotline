---
id: decision-34
title: A login or passkey's sidecar owns Auto/Ask; only a live held use on the teammate's current…
type: decision
status: current
claimKey: secrets.use-approval
kind: decision
recheck: Revisit if a held use must carry an originating thread or survive desk/session restart with durable authority.
createdAt: "2026-10-22T06:43:10.263Z"
---

## Description
A login or passkey's sidecar owns Auto/Ask; only a live held use on the teammate's current computer can be answered from desk or phone, never voice.
The computer owns the hold deadline and one-key/one-site/one-teammate ten-minute allowance. The core serializes whole-set delivery, polling and answers per computer, and checks the current grant and captured capability lease under the policy lock. Cards carry metadata only. Old records are Automatic; a 404 on the use door means Automatic on that computer, with a single diagnostic, without rewriting the stored choice.
