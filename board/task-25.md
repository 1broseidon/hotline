---
id: task-25
title: "MCP gateway: sign in with a Client ID Metadata Document (CIMD) when the server supports it"
column: todo
position: 27
priority: medium
tags:
  - feature
  - bro-248
createdAt: "2026-10-11T21:05:34.595Z"
description: |-
  When an HTTP MCP server needs OAuth, Hotline's MCP gateway signs in as an OAuth client (`crates/hotline-core/src/mcp/oauth.rs`, `authorization_session`). Today it uses one of two identities. The first is a stored or configured client id. Otherwise it calls rmcp's `register_client("Hotline MCP Gateway", …)` for dynamic client registration and stores the result in the vault. A server without a `registration_endpoint`, and no configured client id, cannot be signed in to.

  The MCP authorization spec (2025-11-25) makes Client ID Metadata Documents (SEP-991) the preferred way to register. The `client_id` is an https URL serving a JSON document that describes the client. The authorization server fetches it, so nothing is registered per server and there is no registration to lose. A server opts in with `client_id_metadata_document_supported: true`.

  **rmcp already does the protocol.** We pin `rmcp =3.2.0`. Its `AuthorizationRequest::with_client_metadata_url` and `AuthorizationSession::new` choose in this order: preregistered client, then CIMD when the server advertises it, then DCR. It also keeps a portable CIMD client id across issuer changes. We bypass that order because we call `register_client` ourselves, so that we can keep a returned client secret.

  **Do**

  * Host Hotline's client metadata document at a stable https URL, e.g. `https://hotline.dev/oauth/client-metadata.json`, served from the site repo. Its `client_id` must equal that URL. Include `client_name` "Hotline", `client_uri`, `logo_uri`, `redirect_uris` \[`http://127.0.0.1/callback`\] (plus the path we actually use), `grant_types` \[authorization_code, refresh_token\], `response_types` \[code\], `token_endpoint_auth_method` "none" and `application_type` "native".
  * In `authorization_session`, when there is no stored or configured client and the discovered metadata advertises `client_id_metadata_document_supported`, use the CIMD URL as the client id instead of calling `register_client`. Save it as the server's `McpOAuthRegistration` with `client_secret: None`, so restarts, the issuer/resource binding checks and refresh behave exactly as they do for a DCR client. Fall back to DCR when CIMD isn't advertised. Then refuse with a message naming neither CIMD nor DCR.
  * `validate_resolution` already checks the registration endpoint; it needs nothing extra for CIMD, but should not require a `registration_endpoint` either.
  * Existing DCR registrations keep working; don't migrate them.

  **To verify before relying on it**

  * Loopback redirects. Our redirect is `http://127.0.0.1:{port}{path}` with a random port. RFC 8252 §7.3 says the authorization server should allow any port for a loopback redirect, but CIMD servers may match `redirect_uris` exactly. Test against at least one real CIMD server, and decide whether we need a fixed port, a fallback to DCR on a redirect mismatch, or both.
  * Which servers we can use for testing. Any real MCP server that advertises CIMD, or a mock in `mcp/oauth.rs` tests that serves metadata with the flag, fetches our document and checks `redirect_uri`.

  **Tests**

  * Discovery with the CIMD flag picks the URL client id and never calls `/register`; without it, DCR; with a configured client id, that id.
  * A stored CIMD registration survives a restart and refreshes with no secret.

  (Moved from runprism: this is Hotline's gateway, not runprism's.)

  Migrated from Linear BRO-248: https://linear.app/1broseidon/issue/BRO-248/mcp-gateway-sign-in-with-a-client-id-metadata-document-cimd-when-the (2026-10-11).
updatedAt: "2026-10-14T01:44:25.074Z"
subtasks:
  - id: task-25-1
    title: "Dark MCP OAuth callback page with the mark (from task-26): templated success/failure inline HTML in mcp/oauth.rs, escaped server name/reason, strict CSP, no external URLs"
    completed: false
---

## Log
- 2026-10-14T01:44:30.845Z: [claude] Consolidated task-26 (dark MCP OAuth callback page) into this task as subtask task-25-1; both live in mcp/oauth.rs.
