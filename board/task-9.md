---
id: task-9
title: Add controlled privileged-service API for Linux, macOS, and Windows
column: todo
position: 11
priority: low
tags:
  - hotline-computer
  - bro-133
createdAt: "2026-10-11T21:05:20.503Z"
description: |-
  ## Goal

  Allow Hotline agents to request narrowly scoped host-administration operations without receiving administrator passwords, an elevated shell, or unrestricted sudo. Use the same product architecture on Linux, macOS, and Windows: an unprivileged agent communicates with a privileged service through a controlled, typed API.

  ## Agreed architecture

  * The privileged service is the security boundary. It validates callers, operations, parameters, policy, and approval before executing anything. Hotline presents requests; direct service calls must not bypass approval.
  * Share a capability-discovery and request/result contract across platforms. Native installation, caller verification, IPC, and authorization are platform-specific.
  * Expose semantic operations with constrained parameters, not arbitrary command execution. Choose the initial small operation set during design; service restart is an example, not a finalized commitment.
  * Keep the main app and agent tools unprivileged. A dedicated unprivileged agent account may strengthen isolation, but is not the authorization gate.

  ## Platform adapters

  | Platform | Privileged executor | Authorization / integration |
  | -- | -- | -- |
  | Linux | Root-owned system service over system D-Bus or a protected Unix socket | Use polkit when available; OS-derived caller identity and administrator-approved provisioning |
  | macOS | Privileged launch daemon registered through SMAppService on supported versions | XPC with audit-token-backed client identity/signing validation; implement per-operation authorization separately from service registration approval |
  | Windows | Privileged Windows service using least privilege appropriate to supported operations | UAC-approved installation; restricted authenticated IPC, such as an ACL-protected named pipe with caller-token verification; service independently enforces per-operation authorization |

  Polkit is a Linux implementation detail, not the cross-platform foundation. Polkit presence alone is insufficient: Hotline's helper must also be installed, authorized, and compatible. SMAppService registration and UAC setup likewise do not constitute per-request approval.

  ## User experience / fallback

  * Privileged support is optional and initial setup requires native administrator approval. Never collect or expose administrator credentials to the model.
  * If the helper or required authorization support is absent, disabled, incompatible, or unavailable, silently continue ordinary unprivileged work. No unsolicited dependency-install prompt, setup interruption, or nagging.
  * When a task actually requires unavailable elevation, the agent explains that it lacks elevated access and offers a manual step.
  * A denied or cancelled request stays denied. Never try sudo or another escalation route as a fallback.
  * Surface accurate capabilities to the agent, distinguishing unavailable, available-with-approval, and request outcomes.

  ## Security requirements

  * Enforce authorization inside the service, independently of agent-writable code/configuration and UI state. Do not trust an agent-supplied approval boolean or an approval secret accessible to the agent.
  * Bind one-shot approval to the exact validated operation, parameters, requester, expiry, and a single-use nonce. Reject replay and parameter substitution.
  * Define a trusted consent/authentication path that the agent cannot satisfy through desktop automation. A same-user confirmation window alone is not a security boundary. Document the supported threat model and isolation assumptions.
  * Reject arbitrary shells/interpreters and broad passwordless sudo. Treat package installers, scripts, plugins, and other indirect execution paths as privileged code execution; do not include them casually in the initial API.
  * Harden file operations against symlinks, content replacement, and approval/execution races; use controlled executable paths and sanitized environments.
  * Protect service binaries, configuration, IPC endpoints, and audit records from agent modification. Define secure install/update/uninstall, revocation, and failure behavior. Audit requests and outcomes without recording secrets.

  ## Acceptance criteria

  - [ ] Document the threat model, trusted approval channel, and smallest initial operation set before implementation.
  - [ ] Provide one shared typed capability/request/result contract and platform adapters for Linux, macOS, and Windows.
  - [ ] Demonstrate that approved operations work without elevating the main app/agent or exposing credentials to it.
  - [ ] Demonstrate that direct IPC calls without authorization, spoofed callers, replayed/expired approvals, and altered parameters fail closed.
  - [ ] Cover denial/cancellation, missing helper/polkit, helper failure, and protocol incompatibility; ordinary unprivileged work remains functional with no alternative escalation attempts.
  - [ ] Validate file-race and indirect-execution protections for each supported operation.
  - [ ] Verify native setup/approval, revocation, and audit behavior on all three platforms.

  ## Out of scope

  Unrestricted host-root commands, NOPASSWD: ALL, elevating the whole Hotline application, reusable blanket root sessions, and VM provisioning. Split platform implementation into follow-up subissues as needed; this task captures the common architecture and expected parity.

  Migrated from Linear BRO-133: https://linear.app/1broseidon/issue/BRO-133/add-controlled-privileged-service-api-for-linux-macos-and-windows (2026-10-11).
updatedAt: "2026-10-11T21:05:20.793Z"
---
