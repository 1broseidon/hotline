# Rig 0.42 → 0.45 migration for Hotline

**The plan changed on 2026-10-20.** S1 now jumps straight from Rig 0.42 to 0.45: it is the report's S1, S2 and S3 in one slice, and S2 and S3 are retired, so read S1's row in §6 together with S2's and S3's. The `/tmp/rig-*` paths were the researcher's scratch checkouts and are not kept; read them as Rig's source at those tags (`/tmp/rig-src` and `rig/…` at v0.45.0, `/tmp/rig-0.42.0`, `/tmp/rig-0.43.0` and `/tmp/rig-0.44.0` at theirs). `/tmp/rig-notes` held the researcher's working notes, which are gone; what they found is in this report.

Researched 2026-10-20. Nothing in `~/Projects/personal/hotline-wt-release` was modified or built; the only command run there was a read-only `cargo tree --locked --offline`. All Hotline citations are against that checkout. Its HEAD is `e05716c4` ("Merge pull request #192 from 1broseidon/release/0.41.0"), which is what the detached release/0.42.0 branch points at.

Paths are abbreviated:
- `core/` means `crates/hotline-core/src/`
- `tools/` means `crates/hotline-tools/src/`
- `rig/…` means `/tmp/rig-src/…` (tag v0.45.0)
- `rig-0.42/…`, `rig-0.43/…`, `rig-0.44/…` mean the worktrees at `/tmp/rig-0.42.0`, `/tmp/rig-0.43.0` and `/tmp/rig-0.44.0`

---

## 1. Sources found

| What | Where |
|---|---|
| **0.42 → 0.43 guide** | `MIGRATING.md`, section "## 0.42 → 0.43" (in v0.45.0's file at line 1415; in the origin/HEAD copy at line 2374). The 0.43 "Behavior Changes" list sits at the end of that section. |
| **0.43 → 0.44 guide** | `MIGRATING.md` "## 0.43 → 0.44", added by [PR #2760](https://github.com/0xPlaygrounds/rig/pull/2760). It merged *after* the v0.44.0 tag, so it ships in v0.45.0's `MIGRATING.md` (line 820). |
| **0.44 → 0.45 guide** | **Not in the v0.45.0 tag.** That tag's `MIGRATING.md` still says it "covers … 0.30 through 0.44". The guide landed after the release in [PR #2866](https://github.com/0xPlaygrounds/rig/pull/2866) "docs: 0.45.0 changelog and 0.44 → 0.45 migration guide" (merged 2026-10-20 03:30Z). I saved a copy as `/tmp/rig-notes/MIGRATING-main.md` from `git show origin/HEAD:MIGRATING.md`. The 0.45 silent changes are at lines 821-985 and the guide (17 sections) at lines 987-1778. |
| Silent-behaviour index | `MIGRATING.md` "## Silent behavior changes": subsections `### 0.43`, `### 0.44` and `### 0.45`. Read these first. |
| Changelogs | `rig/CHANGELOG.md` (facade; 0.45.0 at line 11, 0.44.0 at 79, 0.43.0 at 159). Per-crate: `rig/crates/rig-core/CHANGELOG.md`, `rig-agent/CHANGELOG.md`, `rig-reqwest/CHANGELOG.md`, `rig-rmcp/CHANGELOG.md`, `rig-http/CHANGELOG.md`. |
| Releases | [v0.43.0](https://github.com/0xPlaygrounds/rig/releases/tag/v0.43.0) (2026-09-30), [v0.44.0](https://github.com/0xPlaygrounds/rig/releases/tag/v0.44.0) (2026-10-07), [v0.45.0](https://github.com/0xPlaygrounds/rig/releases/tag/v0.45.0) (2026-10-19). All are on crates.io and none is yanked. 0.43/0.44 have `rust_version` 1.95.0; **0.45.0 has 1.97.1**. |
| API review follow-up | [PR #2845](https://github.com/0xPlaygrounds/rig/pull/2845) "fix!: API review follow-up: defects, async storage, restored capabilities" (merged 2026-10-18). Added `StoreLock::new(guard)` and `crates/rig-core/tests/custom_token_store.rs`. |
| Token storage | Issue [#2481](https://github.com/0xPlaygrounds/rig/issues/2481) (George's; closed 2026-10-15). [PR #2815](https://github.com/0xPlaygrounds/rig/pull/2815) "feat(auth): pluggable token storage". |
| Agent PRs relevant to Hotline | [#2756](https://github.com/0xPlaygrounds/rig/pull/2756) every dispatched completion gets exactly one `on_outcome`; [#2762](https://github.com/0xPlaygrounds/rig/pull/2762) runs survive a restart; [#2763](https://github.com/0xPlaygrounds/rig/pull/2763) tool items as a projection of committed history; [#2755](https://github.com/0xPlaygrounds/rig/pull/2755) usage observed once; [#2758](https://github.com/0xPlaygrounds/rig/pull/2758) in-band error classification; [#2754](https://github.com/0xPlaygrounds/rig/pull/2754) `DocumentData::Text`; [#2846](https://github.com/0xPlaygrounds/rig/pull/2846) compaction stop (harness only); [#2812](https://github.com/0xPlaygrounds/rig/pull/2812) rmcp 3; [#2814](https://github.com/0xPlaygrounds/rig/pull/2814) transcription segments. |
| Design contract for typed options | `rig/crates/rig-core/TYPED_OPTIONS.md` |

Local working copies: the clone is `/tmp/rig-src` (depth 300, all four tags fetched), plus the worktrees `/tmp/rig-0.42.0`, `/tmp/rig-0.43.0` and `/tmp/rig-0.44.0`. Research notes are in `/tmp/rig-notes/{inventory,tokens,resume}.md`.

---

## 2. Hotline's Rig surface (by file)

**Dependency.** `Cargo.toml:89` declares `rig = { version = "0.42.0", default-features = false, features = ["agent","reqwest","rustls"] }`. It is used by `crates/hotline-core/Cargo.toml:21` and `crates/hotline-tools/Cargo.toml:15`.
- `Cargo.lock` holds `rig` (line 8368), `rig-agent` (8382) and `rig-core` (8403), all 0.42.0. `rig-fastembed`, `rig-helixdb`, `rig-lancedb` and `rig-milvus` are lock-only weak optionals and are never compiled.
- Workspace `rust-version = "1.95"` (`Cargo.toml:9`), with resolver 3.

**Not used at all:**
- `OneOrMany`/`NonEmpty`, embeddings and vector stores, transcription, audio, image generation
- extractors, the `rig::telemetry` API, rig's `rmcp` feature, `rig-derive`

MCP reaches Rig only as `DynamicTool` wrappers. Hotline's own `rmcp` 3.2.0 is pinned in `core/../Cargo.toml:80` and `hotline-mcp/Cargo.toml:29`.

**Caution.** `crate::driver::rig` is Hotline's own module (`Said`, `InProcess`, `complete`, `completion_builder*`). `session.rs:321 rig::complete(..)` is Hotline code, not the crate.

### Provider construction
- **`core/driver/rig/build.rs`**
  - 8-11: `rig::prelude::*` (for `CompletionClient`, `AgentClientExt`) and the providers `anthropic, chatgpt, deepseek, gemini, groq, mistral, openai, openrouter, xai, zai`.
  - 79-89: `anthropic::Client::completion_model(m).with_prompt_caching().with_automatic_caching()`.
  - 94-105: `openrouter::CompletionModel::with_prompt_caching()` for `anthropic/` models.
  - 114: returns `rig::agent::AgentBuilder`.
  - 136/138: custom client `.agent(model)` and `.completions_api().agent(model)`.
  - 145-151: `chatgpt::Client::builder().oauth().auth_file(token_dir.join("auth.json")).allow_device_flow(false).build()?.agent(model)`.
  - 152-159: Copilot dispatch.
  - 161-208: `{openrouter,anthropic,openai,gemini,xai,groq,deepseek,mistral}::Client::new(key)`.
  - 169-175: `.agent()` on `xai::Client<SubscriptionHttp>` and `ollama::Client`.
  - 210-221: `zai::Client::builder().api_key(k).general()|.coding().build()`.
  - 225/229: `.max_tokens()` / `.additional_params()`.
  - `effort_params` at 19-48 sends reasoning as raw `additional_params` JSON per client.
- **`core/providers.rs`** 13 and 38-51: `ollama::Client::builder().api_key().base_url().http_client(reqwest::Client).build()`. This is the native `/api/chat` route.
- **`core/providers/custom.rs`**
  - 5-7: `rig::client::{ApiKey, ClientBuilder, ProviderBuilder}`, `http_client::{self, HttpClientExt}`, `openai`.
  - 49-55: `impl ApiKey for OptionalKey` (uses `http_client::make_auth_header`).
  - 61-74: `impl ProviderBuilder for CompatibleBuilder` (`Extension<H> = openai::OpenAIResponsesExt`).
  - 79-101: `ClientBuilder::<CompatibleBuilder>` returns `openai::Client<compat::Http>` or `Client<DiscoveryHttp>`.
- **`core/providers/discovery.rs`**
  - 5-7: `rig::client::ModelListingClient`, `http_client::{…}`, and 8 providers.
  - 115-189: `impl HttpClientExt for DiscoveryHttp`.
  - 191-208: `client.list_models()` (`ModelList.data`, the model fields).
  - 226-237: macro `$p::Client::builder().api_key().http_client(DiscoveryHttp)…build()`.
  - 255-260: Ollama with the same pattern.
  - 112/140/167: `http_client::Error::{Instance, InvalidStatusCode}`.
- **`core/providers/xai.rs`**
  - 11-14: `http_client::{Error, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse, bearer_auth_header}`.
  - 300-337: matches `Error::{Instance, InvalidStatusCode, InvalidStatusCodeWithMessage, InvalidStatusCodeWithDetails}`.
  - 371/379: `bearer_auth_header`.
  - 387-435: `impl HttpClientExt for SubscriptionHttp`, delegating to rig's `impl HttpClientExt for reqwest::Client`.
  - 490-498: `xai::Client::builder().api_key("oauth").http_client(SubscriptionHttp)`.
  - 584-606: tests use `prelude`, `client.agent("grok-4")` and `prompt`.
- **`core/providers/copilot.rs`**
  - 16-17: imports.
  - 67-76: `rig_client` = `copilot::Client::builder().oauth().token_dir(dir).allow_device_flow(false).build()`.
  - 82-106: `authorize()`, then **reads `api-key.json` itself**.
  - 264-279: `chat_agent` = `copilot::Client::builder().http_client(ChatBody).oauth().token_dir()…agent(model)` with `AgentClientExt`.
  - 338-372: `impl HttpClientExt for ChatBody`, which flattens assistant text to a string.
  - 380-389: `AgentBuilder::new(openai::responses_api::ResponsesCompletionModel::<Transport>::new(client, model).with_strict_tools())`.
  - 394-404: `openai::Client::builder().api_key().base_url().http_client(Transport).build().map(rig::client::Client::with_system_instructions_as_messages)`.
  - 466-571: `http_client::Error::{Instance, InvalidStatusCodeWithDetails, Protocol}` and `http_client::sse::BoxedStream`.
  - 498-573: `impl HttpClientExt for Transport`, which signs each request after `authorize()`.
- **`core/providers/chatgpt.rs`** 6, 105: `HttpClientExt::send`. 109-132 `refreshed_auth`: `chatgpt::Client::builder().oauth().auth_file(&auth_file).allow_device_flow(false).build()?.authorize()`, then **reads `auth.json` itself** (`vault::read_model_file`).
- **`core/providers/compat.rs`** 10-13: `http_client::{…, sse::BoxedStream}` and `openai::responses_api::streaming::StreamingCompletionChunk` (a decode probe at 191, 421). 37-86: `impl HttpClientExt for Http`.
- **`core/desk.rs`** 24 imports `chatgpt, copilot`. Sign-in:
  - 723-733: `chatgpt::Client::builder().oauth().auth_file(token_dir.join("auth.json")).on_device_code(..).allow_device_flow(true).build()?.authorize()`.
  - 735-745: Copilot with `.token_dir(&token_dir)`.

### Turn loop (Hotline drives its own loop over the raw model)
- **`core/driver/rig.rs`**
  - 41-45: `rig::message::{DocumentSourceKind, Image, ImageMediaType, Message, ToolResultContent, UserContent}`, `prelude::*` (for `Prompt`), and `tool::{DynamicTool, ToolExecutionError, ToolOutput}`.
  - 160-167: `agent.prompt(p).await`.
  - 175/186: `rig::agent::AgentBuilder`.
  - 754-765: `rig::tool::ToolSet::default()`, `.add_tool` ×6, `.add_dynamic_tool`.
  - **782-794: `CompletionRequest { preamble, tools, max_tokens, additional_params, model, chat_history, documents, temperature, tool_choice, output_schema, record_telemetry_content }` literal.**
  - **795: `turn::run(agent.model_handle(), …)`.**
- **`core/driver/rig/turn.rs`**
  - 7-10: `completion::{CompletionModel, CompletionRequest}`, `message::{AssistantContent, ToolCall, UserContent}`, `streaming::StreamedAssistantContent`, `tool::{ToolContext, ToolResult, ToolSet}`.
  - 143-163: `model: &impl CompletionModel`; writes `template.preamble`; `template.tools.extend`.
  - 288: `Message::Assistant { id, content }`.
  - 401-404: overwrites `request.chat_history = history`.
  - 422: `model.stream(request)` returns `Result<StreamingCompletionResponse, CompletionError>`.
  - 460/466/484: `stream.cancel()`.
  - 489-515: matches `StreamedAssistantContent::{Text, ReasoningDelta{id,reasoning}, Reasoning{id,reasoning}}` and `ReasoningContent::Text`.
  - 517-526: `stream.usage()`, `stream.response`, `Usage::has_values()`, `input_tokens + output_tokens`, `+=`.
  - 537-576: `stream.choice`, `response.finish_reason` (`FinishReason::{Length, ContentFilter}`), `stream.identity().message_id`.
  - 700-707: `rig::message::ToolResult { call, provider, name, content }`.
  - 729-828: `ToolResult::{skipped, success, failed}`, `ToolContext::new()`, `tools.execute(..)`.
  - 962-976: `Usage` reads.
  - **986-997: `every_input_token` adds `cached_input_tokens + cache_creation_input_tokens` into Anthropic `input_tokens`.**
- **`core/driver/rig/recovery.rs`**
  - 2, 34-90, 129-158: exhaustive matches on `Message`, `AssistantContent{Text,ToolCall,Image,Reasoning}` and `ToolResultContent{Text,Json,Image}`.
  - 121-126: reads `template.preamble` and `.tools`.
  - Tests at 166-305:
    - provider wire conversions: `openai::responses_api::InputItem` try_into, `anthropic::completion::Message::try_from`, `gemini_api_types::Content::try_from`
    - `Reasoning { id, content: [ReasoningContent::Encrypted] }`, `Reasoning::new_with_signature`
    - `ToolCall::from_wire`, `call.signature`
    - `ToolResult { call, provider, .. }` and `Image` literals
- **`core/driver/rig/tool_output.rs`** 11-86: `DynamicTool::new(.., |_ctx, args| ..)`, `ToolOutput::{text, content}`, `ToolResultContent::{text, image_base64}` with an exhaustive match, `MimeType::to_mime_type`, `DocumentSourceKind::Base64`.
- **`core/driver/rig/images.rs`** 9-223: the `Image { data, media_type, detail, additional_params }` literal (63, 204, 371); `Message::User { content }` literals; `UserContent::{Image, ToolResult, text}`.
- **`core/driver/rig/granted.rs`** 74-135: `DynamicTool::new(name, desc, schema, move |_context, args| ..)`, `ToolOutput::text`, `ToolExecutionError::other`.
- **`core/driver/failure.rs`**
  - 6, 89-114: `Failure::provider(CompletionError)` uses `provider_response_status/body/headers`, `retry-after` and `matches!(CompletionError::HttpError(_))`.
  - Tests 746-797: a `chatgpt::Client::builder()…auth_file(..).base_url(..)` client, `completion_model`, `completion_request(..).build()`.
  - Tests 879-884: `CompletionError::ProviderResponse(ProviderResponseError::new(..).with_headers(Some(Box::new(h))))`.
- **`core/pricing.rs`** 10, 91-114: `cost(&ModelCost, &Usage, cache_beside_input)` uses `u64` arithmetic on `input/output/total/cached_input/cache_creation_input`.

### Agent runtime (voice and one-shots)
- **`core/voice/dispatcher.rs`**
  - 20-27: `agent::MultiTurnStreamItem`; `agent::hook::{AgentHook, CompletionCall, CompletionCallAction, CompletionResponse, HookContext, ModelTurnAction, ModelTurnFinished, ObservationAction}`; `completion::{Prompt, Usage}`; `streaming::{StreamedAssistantContent, StreamingPrompt}`.
  - 365-381: `.dynamic_tools(..)`, then `agent.stream_prompt(p).max_turns(4).tool_concurrency(1).add_hook(StreamMeter)`.
  - 393-409: matches `MultiTurnStreamItem::{CompletionCall, FinalResponse, StreamAssistantItem(StreamedAssistantContent::Text), ModelTurnRetried}`.
  - 444, 788, 839: `agent.prompt(p).max_turns(n).add_hook(meter)`.
  - 476: `cache_beside_input` set for Anthropic.
  - 672-675: `settle` checks `total==0 && input==0 && output==0`.
  - **689-725: `impl AgentHook for Meter` (`on_completion_call`, `on_completion_response`)**.
  - 728-746: `StreamMeter` (`on_model_turn_finished`).
  - 941/990: `DynamicTool::new(.., |_, args| ..)`.
  - Tests 1075-1083: `Usage {..}` literal.
- **`core/voice/front_tools.rs`** 21, 117-128: `DynamicTool::new`, `ToolOutput::text`.
- **`core/mcp/server.rs`** 1147-1170: `TeammateTools::as_dynamic()` uses `DynamicTool::new(.., |_context, args| ..)`. Tests at 1561 and 1958-1966 use `ToolSet::from_dynamic_tools` and `.execute`.
- **`core/session/jobs.rs`** 24, 229-256: `ToolDefinition { name, description, parameters }` literals ×5; `Tool::parameters()`.
- **`core/session/generate/tests.rs`** 393-398, **`core/driver/rig/tests.rs`** 856-973: `ToolSet::from_dynamic_tools`.

### Tools crate
- **`tools/lib.rs`** 11, 63-67: `ToolExecutionError::{invalid_args, refused, other}`.
- **`tools/shell.rs`** 25, 88-130: `impl Tool for RunCommand` (`NAME`, `Args`, `Output = String`, `Error`, `description`, `parameters`, `map_error`, `call(&self, &mut ToolContext, Args)`).
- **`tools/workspace/rig_tools.rs`** 7, 20/72/135/207/273/338: `impl Tool` for `ListDirectory`/`ReadFile`/`SearchFiles`/`FindFiles`/`WriteFile`/`EditFile`.

### Tests of note
- **`core/driver/rig/turn/tests.rs`**
  - 16-43: `impl CompletionModel for ScriptModel` built from `StreamingCompletionResponse::stream`, `RawStreamingChoice::*`, `StreamFinal`, `ToolCallDeltaContent`.
  - 66-78: a `CompletionRequest` literal.
  - 158, 1285, 1307, 1382: `Usage {..}` literals.
  - 526-551: `impl Tool for Nod`.
  - 903-1263: `ProviderResponseError::new`.
  - 1185: `Message::Assistant{id, content}` with `Reasoning{id, content}`.
- **`core/driver/rig/tests.rs`**
  - Axum stub servers.
  - `anthropic::Client::builder().base_url()`, `openrouter::Client::builder()`.
  - `completion_request(..).send()`.
  - `agent.model_handle().completion(request)` (796).
  - The `ToolDefinition` literal (777).
  - `every_input_token` assertions (187-197).

**Cross-cutting summary**
- **Struct literals:** `CompletionRequest` (3 places), `ToolDefinition` (6), `message::ToolResult` (3), `Image` (4), `Message::Assistant{id,…}` (3), `Reasoning{id,content}` (3), `ToolFunction` (2), `Usage{..}` (5).
- **Rig trait impls:** `Tool` ×8, `CompletionModel` ×1 (test), `AgentHook` ×2, `ApiKey`/`ProviderBuilder` ×1 each, `HttpClientExt` ×5.
- **Every custom `HttpClientExt` delegates to rig's `impl HttpClientExt for reqwest::Client`.**

---

## 3. Breaking changes mapped to call sites

Each item says which release breaks it, which Hotline lines it touches, and the 0.45 replacement, checked against `rig/` source. **[silent]** marks behaviour that changes without a compile error.

### 3.1 Cargo, features, toolchain
1. **Feature `reqwest` is now a separate crate** (rig-reqwest; rig-core has no default transport, 0.43). Hotline's `default-features = false, features = ["agent","reqwest","rustls"]` still works. In 0.45 the facade's `reqwest = ["dep:rig-reqwest", "rig-core/reqwest", …]` and `agent` is unchanged (`rig/Cargo.toml:482+`). `rmcp` no longer implies `agent`, which doesn't matter here. `memory` is needed only for `rig::memory`; Hotline doesn't need it.
2. **MSRV 1.97.1 in 0.45** (`rig/Cargo.toml:217`; 0.43/0.44 need 1.95.0). Bump Hotline's `Cargo.toml:9` `rust-version` to `1.97.1`. CI uses `dtolnay/rust-toolchain@stable`, so it is fine; local `rustc` is 1.99.0. Under resolver 3 a stale `rust-version` is a lie, so bump it in the 0.45 slice.
3. **New transitive crates:** rig-http, rig-reqwest, `schemars` in the public API (`CompletionRequest.output_schema: Option<schemars::Schema>`), and in 0.45 `rig-memory` (only with `memory`). `make check` runs `cargo deny check advisories licenses` and `cargo machete`. Re-run them, and update `THIRD-PARTY-NOTICES.md` if Hotline generates it from the lock.
4. **rig-core enables serde_json `preserve_order` + `float_roundtrip`** (0.44). Hotline already sets `preserve_order` (`Cargo.toml:94`). **[silent]** `float_roundtrip` is now unified into every crate. That's harmless, but tape float bytes could differ if any JSON float is re-serialized; check the tape byte-compat tests.

### 3.2 Provider clients (0.43 removed every `Client::builder()`, generic `Client<Ext,H>`, `ClientBuilder`, `ProviderBuilder`, `ApiKey`, `CompletionClient`, `AgentClientExt`, `ModelListingClient`)

| Hotline site | 0.45 replacement (verified) |
|---|---|
| `build.rs:186` `anthropic::Client::new(k)` + `completion_model` + `with_prompt_caching().with_automatic_caching()` | `let mut m = Anthropic::new(k).completion(model); m.wire = m.wire.with_prompt_caching();` (`rig/crates/rig-core/src/providers/anthropic/wire.rs:451`). `with_automatic_caching` was **removed in 0.44**, so set `.cache(CacheRetention::Short)` **on the request** (`CompletionRequest.options`). Because Hotline builds its own `CompletionRequest`, agent-level `.cache()` would never reach `turn.rs`. |
| `build.rs:94-105, 161, 192` OpenRouter `with_prompt_caching()` | `openrouter::new(k)` returns an `OpenAI` client with the `OPENROUTER` dialect (`providers/openrouter/mod.rs`). The request needs `.cache(CacheRetention::Short)`: `openrouter_cache` sends top-level `cache_control: {type: ephemeral}` for non-automatic upstreams such as `anthropic/`, and omits it for `openai/`, `deepseek/`, `x-ai/`, `groq/` (`providers/openai/options.rs:615-639`). **Only set it for `anthropic/` models.** For `CacheRetention::None`/`Long` the mapping can be *unsupported*, and with the default `OnUnsupported::Error` that refuses the request. |
| `build.rs:196-208` `{openai,gemini,xai,groq,deepseek,mistral}::Client::new(k)` | `OpenAI::new(k)`, `Gemini::new(k)`, `xai::new(k)`, `groq::new(k)`, `deepseek::new(k)`, `mistral::new(k)` (the vendor macro `client/macros.rs:68-80`), then `.completion(model)`. |
| `build.rs:210-221` zai general/coding | `OpenAIConfig::with_key(&openai::wire::ZAI, k).with_base_url(zai::CODING_API_BASE_URL)` (`providers/zai.rs:21-23`), then `.connect(http)` / `.client()`. |
| `providers.rs:38-51` Ollama | `OllamaConfig::new().with_base_url(u).with_api_key(k).connect(ReqwestClient::from(http))` (`providers/ollama/wire.rs:31-75`, `client/ollama.rs`). **[silent]** In 0.44 `Ollama::completion` targets `/v1` (OpenAI Chat). Hotline sends `{"think": effort}` (`build.rs:42-45`), which is native-only, so use `native_completion(model)` to keep today's `/api/chat` behaviour. |
| `custom.rs:49-101` `ApiKey`/`ProviderBuilder`/`ClientBuilder` | Delete them. Use `OpenAIConfig::with_key(&dialect, key.unwrap_or_default()).with_base_url(base).connect(compat::Http)`, with a custom `Dialect` (`Dialect::gateway(..)`) whose auth is `Auth::OptionalBearer` ("omitted entirely when the key is empty", `providers/openai/wire/auth.rs:19-26`). For the Chat route (`completions_api()`), use `.with_route(Route::Chat)` (`providers/openai/wire.rs:740`). |
| `discovery.rs:5, 191-237` `ModelListingClient` + builders | `list_models()` is inherent on each client (e.g. `Ollama::list_models`, `client/ollama.rs`). `collect(&impl ModelListingClient)` becomes a closure or enum over concrete clients. Errors become `ProviderError::ProviderResponse(response)` with `response.route`. |
| `xai.rs:490-498` `xai::Client<SubscriptionHttp>` | `OpenAIConfig::with_key(&xai::DIALECT, "oauth").connect(SubscriptionHttp)` (`providers/xai/mod.rs:44`). |
| `copilot.rs:380-404` `ResponsesCompletionModel::<Transport>::new(..).with_strict_tools()` + `with_system_instructions_as_messages` | `OpenAIConfig::with_key(&copilot::DIALECT, token).with_route(Route::Responses).connect(Transport)`. The 0.45 Copilot dialect already sets `strict_tools_by_default: true` and `SystemInstructionsPlacement::InputSystemMessages`, and routes per model (`providers/copilot/wire.rs:12-60`). Hotline's override for models "the account offered nowhere else" stays as `.with_route(Route::Responses)`. |
| `copilot.rs:264-279` `copilot::Client::builder().http_client(ChatBody)` | `OpenAIConfig::with_key(&copilot::DIALECT, token).connect(ChatBody)`; `CopilotConfig`/`CopilotWire`/intent were removed in 0.45. **Probably obsolete:** 0.45's Chat wire joins an assistant turn's text into a single `content` string (`providers/openai/wire/chat.rs:367-386`), which is exactly what `ChatBody` patches. Keep it until a request-body test proves otherwise, then delete it (slice S7). |
| ChatGPT/Copilot OAuth clients (`build.rs:145`, `chatgpt.rs:111`, `copilot.rs:67,269`, `desk.rs:723-745`, `failure.rs:786`) | **0.43/0.44:** `chatgpt::auth::Authenticator::new(source, Some(path), handler, allow)` and `copilot::auth::Authenticator::new(source, access_token_file, api_key_file, handler, allow)`, plus `OpenAI::authenticate(&auth)` (`rig-0.43/crates/rig-core/src/client/openai.rs:130`, `client/copilot.rs:67`). Passing today's paths keeps users signed in. **0.45:** `Authenticator::new(source, impl TokenStore, handler, allow)`, served per request by `client::signed_in::SignedInModel::new(spec, auth, http)` or by `auth.auth_context(&http)` in Hotline's own signing transport. See §4. |

### 3.3 HTTP transport
- **`reqwest::Client` no longer implements `HttpClientExt`** (0.43). Only rig-reqwest's newtype does: `http_client_ext!(ReqwestClient)` (`rig/crates/rig-reqwest/src/lib.rs:415`; also true at 0.43). All five Hotline impls delegate to the raw reqwest impl:
  - `compat.rs:47,69,80`
  - `copilot.rs:347,358,369` (and `Transport`)
  - `xai.rs:399,414,430`
  - `discovery.rs`

  Each must hold `rig::http_client::ReqwestClient::from(reqwest_client)` and delegate to it. The redirect/timeout policy stays on the inner `reqwest::Client`.
- **The trait itself is unchanged.** `send`/`send_multipart`/`send_streaming` have the same signatures (`rig/crates/rig-http/src/http_client/mod.rs:204-230`). It moved to `rig-http`, still re-exported at `rig::http_client`.
- **`http_client::Error`** is `#[non_exhaustive]` and loses `InvalidStatusCode(StatusCode)` and `InvalidStatusCodeWithMessage(..)` (already gone in 0.43). Remaining variants: `Protocol`, `InvalidStatusCodeWithDetails{..}`, `InvalidHeaderValue`, `NoHeaders`, `StreamEnded`, `InvalidContentType`, `Instance`, and `Unbuilt` (0.45). Affects:
  - `xai.rs:300-337` (match, plus constructing `InvalidStatusCode`)
  - `discovery.rs:112,140,167`
  - `copilot.rs:466-571`

  Use `InvalidStatusCodeWithDetails` and add `_` arms.
- **`http_client::sse` was removed (0.43).** `BoxedStream` is now `rig::http_client::BoxedStream` (`rig-http/src/http_client/mod.rs:174`). Affects `compat.rs:10`, `copilot.rs` (`sse::BoxedStream`).
- **`make_auth_header`/`bearer_auth_header` were removed in 0.45** (they still exist in 0.43/0.44 at `rig-0.43/crates/rig-http/src/http_client/mod.rs:156,163`). Affects `custom.rs:49-55` (gone with the builder anyway) and `xai.rs:371,379`. Insert `AUTHORIZATION: Bearer …` with a validated `HeaderValue`.
- `LazyBytes` was removed (0.45); Hotline uses `LazyBody<Vec<u8>>`, so no change.

### 3.4 The turn loop (`driver/rig.rs`, `turn.rs`)
- **There is no public way to get the model out of an `Agent`.** `Agent::model_handle` is `pub(crate)` from 0.43 on (`rig/crates/rig-agent/src/agent/completion.rs:323`; 0.43 line 313). `CompletionModel` and `ModelHandle` are gone as public traits. `rig.rs:767-795` and `turn.rs:143-203` must take a `DynModel<Completion>`.
  - Restructure `build.rs` to `completion_model(keys, id, reuse) -> Result<DynModel<Completion>, String>`, using `Model::erase()` (`rig/crates/rig-core/src/driver/dyn_model.rs:92`).
  - `agent_builder` becomes `AgentBuilder::new(completion_model(..)?)` plus settings (`AgentBuilder::new(impl Into<DynModel<Completion>>)`).
- **`CompletionRequest` literal:** `rig.rs:782`, `rig/tests.rs:775`, `turn/tests.rs:66`.
  - 0.43 removed the `preamble` field; the system prompt is a leading `Message::System`.
  - 0.44 made the struct `#[non_exhaustive]` (`rig/crates/rig-core/src/completion/request.rs:1199`).
  - Replacement: `CompletionRequest::from(history).tools(defs).max_tokens(n)` / `.preamble(..)` / `.additional_params(..)` / `.cache(..)`.
  - **Behaviour trap:** `turn.rs:150` appends job instructions to `template.preamble`, and `turn.rs:402` *replaces* `request.chat_history` every round. With the preamble now living in `chat_history`, that overwrite would silently drop the system prompt. Keep the preamble as a Hotline `String` and prepend `Message::system(preamble)` each round. `recovery.rs:121-126` reads `template.preamble` for token estimates; read it from the same string.
- **`chat_history: Vec<Arc<Message>>` and `tools: Vec<Arc<ToolDefinition>>`** (0.45, `request.rs:1214,1219`). `turn.rs:402-404` push `.into()`. `strict::nullable_optionals(&mut tool.parameters)` (`turn.rs:160-162`) needs `Arc::make_mut`.
- **Streaming** (`turn.rs:422-576`; 0.43 changed it, 0.44 changed argument fragments):
  - `model.stream(req)` returns `Result<CompletionStream, ProviderError>` synchronously (`dyn_model.rs:180`). A transport refusal can also arrive as the **first stream item**, so both paths need the retry.
  - The items are `Item<StreamEvent>`, with `StreamEvent::{Start, Text{text}, Reasoning{text}, Arguments{json}, End{content}}` (`rig/crates/rig-core/src/streaming/event.rs:81-124`).
  - Map `StreamedAssistantContent::Text` to `StreamEvent::Text`, and `ReasoningDelta` to `StreamEvent::Reasoning`. A whole `Reasoning` block now arrives as `End{content: AssistantContent::Reasoning(r)}` and `r.text` (0.44 made `Reasoning { text, redacted, native }`, `message.rs:373`). Hotline's "only if no deltas" dedup (`reasoning_deltas`) becomes "show `End` reasoning text only if that part had no `Reasoning` deltas", keyed by `part`.
  - `stream.cancel()` becomes dropping the stream. `stream.usage()` / `stream.response` / `stream.choice` / `stream.identity().message_id` become `stream.finish().await` → `CompletionResponse { usage, choice, finish_reason, … }`, `response.response_id()` (`request.rs:431`) and `response.message()` (`request.rs:544`, an `AssistantMessage` with `origin`/`stop`).
  - After stop or steer, use `stream.partial()` (`streaming/mod.rs:286`) for whatever arrived.
  - **[silent]** A stream cut before its terminal record is `ProviderError::Truncated` (retryable, 0.43). This replaces Hotline's own `stream.response.is_none()` check (`turn.rs:537-546`). Make sure `Failure::provider` classifies `Truncated` as retryable rather than "provider".
- **`Message::Assistant { id, content }`** (`turn.rs:288`, `recovery.rs:205`, tests) becomes `Message::Assistant(AssistantMessage)`; push `response.message()` (0.44). `recovery.rs:34-90, 129-158` match on `Message::Assistant(turn)` and `turn.content`.
- **Exhaustive matches** need a `_` arm (0.44 made `AssistantContent`, `FinishReason`, `StopReason`, `ProviderError`, `PromptError`, `MultiTurnStreamItem` `#[non_exhaustive]`; `AssistantContent` gained `Opaque`). Affects `recovery.rs:38-55` (`AssistantContent`), `tool_output.rs:65`, `recovery.rs:68` (`ToolResultContent`, not non-exhaustive, but check).
- **Tool calls and results**
  - `ToolCall { id: CallId, function: ToolFunction { name: ToolName, arguments: serde_json::Map, invalid_arguments }, native }` (0.43 introduced `CallId` and `ToolName`; 0.44 made arguments a `Map` and removed the separate `provider` id). Affects `turn.rs:658-704` and `strict::without_nulls(&call.function.arguments)` (`strict.rs`), which now takes a `Map`; use `arguments_value()` if a `Value` is needed.
  - The result `rig::message::ToolResult { call, provider, name, content }` (`turn.rs:701`) becomes `ToolResult { call, name, content, is_error }` (`message.rs:403`). Build it with `call.result(content)` and set `is_error`, or with `call.answer(&exec_result)` (`message.rs:512-523`).
  - **[silent]** A call with non-object arguments carries `invalid_arguments`. Rig's agent answers it with an error; Hotline's loop must too (today it would dispatch it).
- **`FinishReason::{Length, ContentFilter}`** (`turn.rs:557-566`) still exists (`request.rs:248-257`). **[silent]** 0.44 maps `Other`/`ContentFilter` to `StopReason::Error` and adds `CompletionResponse.error`/`.aborted` (`request.rs:322-327`). Treat `response.error.is_some()` like truncation (no tool execution). This mirrors 0.44's "a turn that ends in an error runs none of its tool calls".
- **Errors** (`failure.rs:6, 89-114`): `CompletionError` became `ProviderError` (0.43). `provider_response_status/body/headers` keep their names (`rig/crates/rig-core/src/error.rs:503-546`), and `HttpError` became `Http`. **[silent]**
  - Auth failures may now be `ProviderError::InvalidAuthentication(response)`; `provider_response()` covers it.
  - 0.45 classifies in-band 2xx error envelopes and may report `status` 2xx for them (`error.rs:515`).
  - Numeric nested codes add a `status N:` Display prefix.
  - `Failure::classify` keys on status plus body text, so re-run the `failure.rs:652-721` table against real 0.45 errors.
  - `ProviderError::is_retryable()` and `retry_after()` (`error/retry.rs:14`) could replace part of Hotline's parsing; that is optional.
- **`ProviderResponseError::with_headers(Option<HeaderMap>)`** is no longer boxed (0.43, `provider_response.rs:165`). Affects `failure.rs:884`.

### 3.5 Usage (0.43 `Option<u64>`, 0.44 constructors and `cost`, 0.45 emitted once)
- Fields become `Option<u64>` (`rig/crates/rig-core/src/completion/request.rs:679-713`). The struct is `#[non_exhaustive]`, gains `cost: Option<Cost>`, and loses `Eq`. `has_values()` is now `is_reported()`. `AddAssign` treats unreported as identity (`request.rs:952-975`). Build with `Usage::new().input_tokens(n)…`.
- Affected sites:
  - `turn.rs:517-526` (`has_values`, `saturating_add`)
  - `turn.rs:962-976` → `TokenUsage { input_tokens: usage.input_tokens.map(|n| n as i64), … }`. The wire type is already `Option<i64>`, so `contract.ts` is unchanged, but **[silent]** `total_tokens` may now be `null` where it was `0`: it is "absent unless both are reported".
  - `pricing.rs:91-114`
  - `dispatcher.rs:672-675` → `!usage.is_reported()`
  - Test literals at `turn/tests.rs:158,1285,1307,1382`, `dispatcher.rs:1076`
- **[silent, must fix] Anthropic double count.** 0.43 makes Anthropic, its gateways and Bedrock report `input_tokens` *including* cache reads and writes, and Gemini `output_tokens` including thoughts (MIGRATING "Token usage"; the 0.45 silent note repeats it). Hotline adds them again in two places:
  - `turn.rs:986-997` `every_input_token`, which feeds the context threshold (`turn.rs:525`), the Chat meter and the turn's `TokenUsage`.
  - `voice/dispatcher.rs:476` `cache_beside_input: true` → `pricing.rs:95-99`.

  Delete `every_input_token`'s Anthropic branch and the `cache_beside_input` flag. Price as `fresh = input − read − written`, after checking that `cached/creation ≤ input`. Update `rig/tests.rs:187-197`. The Gemini "unreported = total − input − output" term in `pricing.rs:106-109` becomes 0, which is correct.
- **[silent]** 0.45 emits usage once per completed attempt. Failed or truncated attempts report none (MIGRATING 0.45 "Usage observations…"). Hotline already keeps the reservation when usage is absent.

### 3.6 Tools
- **`DynamicTool::new`**
  - 0.43: the closure takes only `args` (`|args|`); use `new_with_context` for context.
  - 0.44: the name is a `ToolName` (`rig/crates/rig-core/src/tool/contextual.rs`, `DynamicTool::new(name: ToolName, ..)`).
  - Sites: `granted.rs:74,92`, `tool_output.rs:17`, `mcp/server.rs:1153`, `front_tools.rs:117`, `dispatcher.rs:941,990,1245`, plus tests. `ToolName::new(s)?` validates. **[silent]** MCP tool names with odd characters now fail at construction instead of at the provider; surface that in `mcp/server.rs:1147`.
- **`ToolDefinition` literals** (`session/jobs.rs:229-256`, `rig/tests.rs:777`): 0.44 `name: ToolName`, 0.45 adds `annotations`. Use `ToolDefinition::new(name, desc, params)`.
- **`impl Tool`** (×8). The trait moved to `rig_core::tool` (`contextual.rs`) and is still at `rig::tool::Tool`. The signature is unchanged (`call(&self, &mut ToolContext, Args)`); 0.45 adds defaulted `annotations()` and `footprint()`. **No change expected.**
- **`ToolSet::from_dynamic_tools` was removed in 0.45.** Use `ToolSet::default()` plus `add_dynamic_tool` (tests: `mcp/server.rs:1958`, `generate/tests.rs:393`, `rig/tests.rs:856`). `tools.get_tool_definitions()` now returns shared `Arc` definitions (0.45 §3).
- **`ToolResultContent::Image(Image)`:** `Image` loses `additional_params` and gains `native` (0.44), and is now `{data, media_type, detail, native}` (`message.rs:733`). Affects literals at `images.rs:63,204,371`, `recovery.rs:288`. Prefer `ToolResultContent::image_base64` / `UserContent::image_base64`.
- **`DocumentSourceKind::String` was removed in 0.45** (documents now hold `DocumentData::{Text, File}`). Hotline only matches `DocumentSourceKind::Base64` on *images* (`images.rs`, `tool_output.rs:72-80`), so there is no impact beyond the `_` arms.

### 3.7 Agent API (voice dispatcher, `rig.rs:160` one-shot)
- **Prompt traits.** `Prompt`/`StreamingPrompt` were removed (0.43); the methods are inherent. `agent.stream_prompt(p)…​.await` becomes `agent.prompt(p).max_turns(4).tool_concurrency(1).add_hook(h).stream()` (`rig/crates/rig-agent/src/agent/streaming.rs:181`; runner `max_turns`/`tool_concurrency`/`add_hook` at `runner.rs:133-426`).
- **Prompt results.** `agent.prompt(p).await` returns `PromptResponse`; take `.output()` (a method since 0.44).
- **Stream items.** `MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(t))` becomes `StreamAssistantItem(Item::Event(StreamEvent::Text { text, .. }))` (`agent/streaming.rs:47`). `CompletionCall`, `FinalResponse` and `ModelTurnRetried{turn}` keep their names (`streaming.rs:105-128`). Add a `_` arm.
- **Hooks** (`dispatcher.rs:689-746`):
  - `CompletionCall<'_>` became `CompletionCallEvent<'_>` (0.43), and its `history` is `&[Arc<Message>]` (0.45, `hook.rs:337-344`). `serde_json::to_vec(&(event.prompt, event.history))` still serializes.
  - `on_completion_response` was **removed in 0.43**. Usage settles in `on_outcome` (`event.completion()` gives `Option<&CompletionResponse>`, `hook.rs:820`) or `on_model_turn_finished` (`ModelTurnFinished.usage`, `hook.rs:398-419`).
  - **[silent, 0.45]** `on_outcome` fires exactly once per dispatched completion, including rejected, retried and abandoned attempts (PR #2756). An `Ok` outcome is not acceptance, and `OutcomeAction` cannot stop the run (`hook.rs:758-765`). Billing all attempts is the correct semantics for a provider bill; see §5 and slice S6.
- **`PromptError`** variants were renamed (0.44) and the type is `#[non_exhaustive]`. Hotline only stringifies it, so there is no change. **[silent]** The Display text changed ("the run was cancelled: …"); check `dispatcher.rs` error-to-speech mapping, which only uses `to_string`.
- **[silent, 0.44]** A text turn with an unknown finish reason fails the run. Voice runs could start failing on gateways with nonstandard finish reasons. Consider `.accept_unknown_finish_reasons(true)` on voice agents, where no tools run on text-only replies.

### 3.8 Typed generation options (0.44): Hotline's `additional_params` effort JSON
- `additional_params` still merges last, key by key (0.44 "Request bodies merge in one order"). Hotline's effort JSON (`build.rs:19-48`) keeps working without a code change. **[silent]** Behaviour changes:
  - **Gemini:** Rig no longer deserializes `additional_params` into its typed config; it merges raw JSON into the body. `{"generationConfig":{"thinkingConfig":{"thinkingLevel":"low"}}}` now reaches Google verbatim (the comment at `build.rs:30-33` describes the old typed path). Verify that the Gemini REST API accepts the lowercase level; Google's docs show `"low"`/`"high"`.
  - **ChatGPT:** the backend no longer strips `temperature`/`top_p`/`max_output_tokens` from `additional_params`.
  - **Non-object `additional_params`** is now an error.
  - **Model facts from the catalog:** a listed model without image input gets `(image omitted: model does not support images)`, and OpenAI Chat chooses `max_tokens` vs `max_completion_tokens` from the catalog.
- **Optional later:** move to `.reasoning(Effort::…)` on the request. Under the default `OnUnsupported::Error` an unmappable effort *refuses* the request, so pair it with `.on_unsupported(OnUnsupported::Ignore)` or keep the raw JSON.

### 3.9 Things removed with no direct replacement (Hotline impact)
- **Provider-wire conversions** were removed in 0.43/0.44: `TryFrom<Message>` for `openai::responses_api::InputItem`, `anthropic::completion::Message`, `gemini_api_types::Content`, and the typed provider structs. They are used only by the `recovery.rs:166-305` tests to prove replay shape. Rewrite those tests as request-body snapshots against the axum stubs Hotline already uses (`rig/tests.rs:49,112`).
- **`Reasoning::new_with_signature`, `ReasoningContent::Encrypted`, `ToolCall.signature`** were removed in 0.44; signatures live in `native`. Rebuild those test fixtures with `AssistantContent::reasoning(text).with_native(json)`.
- **`impl CompletionModel for ScriptModel`** (`turn/tests.rs:16-43`): hand-written models are gone since 0.43. Use `rig::test_utils::MockCompletionModel::from_turns(..)` (`rig/crates/rig-core/src/test_utils/completion.rs:400-409`; needs `rig = { features = ["test-utils"] }` as a hotline-core **dev-dependency**). Or drive `turn::run` through an axum SSE stub like the other tests. The mock is more stable across Rig releases, so prefer it.
- **`openai::responses_api::streaming::StreamingCompletionChunk`** (`compat.rs:13,191,421`) is used as a decode probe; the typed Responses types were removed in 0.44. Probe with `serde_json::Value` and check `type`/`response` keys instead.
- **`http_client::make_auth_header`, `bearer_auth_header`** (0.45): see §3.3.

### 3.10 Other silent changes relevant to Hotline
- **Telemetry:** `chat_streaming` became `chat` with `gen_ai.request.stream=true` (0.44). Hotline has no span consumers.
- **0.43 "Other providers' reasoning is dropped from requests instead of being replayed".** This interacts with `recovery::fresh` and model switches (`rig.rs:639-646`). It's beneficial, but re-run the model-switch tests.
- **0.45 ChatGPT sends the `session-id` header (not `session_id`) and forwards the cache key as `prompt_cache_key`.** Hotline's own ChatGPT model listing (`chatgpt.rs:88-107`) sets headers itself, so it is unaffected.
- **0.45 "Copilot's `access-token` and `api-key.json` are no longer read … Nonstandard ChatGPT filenames must be moved to `chatgpt.json`."** Hotline uses `auth.json`, and both files are exactly Hotline's layout. That makes §4 mandatory in the same slice as the 0.45 bump.
- **0.45 "MCP tools with an output schema return structured content alone".** This only applies to rig-rmcp, which Hotline doesn't use.
- **Persisted formats:** `RUN_FORMAT` 5, `rig-session/4`, new document JSON. **Hotline doesn't persist Rig JSON.** History is seeded from the tape as text (`rig.rs:190-201`), so none of these break Hotline.

---

## 4. Token storage adoption (ChatGPT and Copilot into the vault)

### Today (Rig 0.42)
- **Location.** Tokens live in `<data>/vault/logins/<credential-id>/`, and `vault.rs:757-772` sets the path; Rig's `~/.config` defaults are never used.
- **Pre-created files.** `begin_login` pre-creates the Rig files 0600 (`vault.rs:291-306`):
  - ChatGPT: `auth.json` = `{}`.
  - Copilot: `api-key.json` = `{}` plus an empty `access-token`.
- **Plaintext.** These files are plaintext. `vault.rs:1-2` says they "remain private files until Rig exposes a store hook".
- **Formats** (rig-0.42 `providers/chatgpt/auth/native.rs:29-36`, `copilot/auth/native.rs:25-36, 262-301`):
  - ChatGPT `auth.json`: `{access_token, refresh_token, id_token, expires_at, account_id}`.
  - Copilot `access-token`: the raw GitHub token as plain text.
  - Copilot `api-key.json`: `{token, expires_at, endpoints:{api}, bootstrap_token_fingerprint}`.
- **Hotline reads both files directly** in `chatgpt.rs:120` (`refreshed_auth`) and `copilot.rs:90-106` (`read_session`).
- **Status and lifecycle:**
  - Status: `ProviderAuth::Login { token_dir }` (`vault.rs:430`; enum at `crates/hotline-room/src/provider_auth.rs:9-27`, internal, not wire).
  - Revoke only marks the row (`vault.rs:253-265`). Delete removes the dir (`vault.rs:270-285, 328-336`).
  - There is no provider-side revocation.
- **Other secrets already go to the keychain.** `CredentialFile` (`crates/hotline-support/src/credentials.rs:178-438`) writes a reference file on disk and chunks the secret in the keyring (service `dev.hotline.credentials`). **`read()` migrates a plaintext JSON file into the keychain on first read** (`:335-356`) and keeps the plaintext if the keychain write fails.
  - xAI and OpenRouter already use `ProviderAuth::StoredLogin { tokens: CredentialFile }` (`vault.rs:425-428`).
  - `xai.rs:225-282` keeps a process-wide `Arc` registry with an async mutex so a rotated refresh token is not spent twice.
  - `SecretStore` is synchronous (`credentials.rs:29-33`).

### Rig 0.45 API (verified)
- **Trait** (`rig/crates/rig-core/src/providers/internal/auth/store.rs:30-53`), async with boxed futures:
  - `load(key) -> Result<Option<String>, BoxError>`
  - `save(key, record)`
  - `delete(key)` (a missing record is `Ok`)
  - `lock(key) -> StoreLock`, default no-op
- **`StoreLock::new(guard)`** releases on drop and must not block (#2845). `Arc<dyn TokenStore>` is a store.
- **Keys and records.**
  - `"chatgpt"` holds the same `AuthRecord` fields as the 0.42 `auth.json` (`chatgpt/auth/native.rs:36-43`), so **the 0.42 file can be stored verbatim**.
  - `"copilot"` holds one record `{github_token?, token, expires_at, endpoints:{api}, bootstrap_token_fingerprint}` (`copilot/auth/mod.rs:192-201`).
- **Shipped stores:** `MemoryTokenStore`, and `FileTokenStore::new(dir)`, which writes `<dir>/<key>.json` atomically, 0600, with an OS file lock (`store.rs:107-289`). There is no default store; `Authenticator::new(AuthSource::OAuth, store, DeviceCodeHandler, allow_device_flow)` (`chatgpt/auth/mod.rs:111-123`, `copilot/auth/mod.rs:113-125`).
- **Shared state and refresh.**
  - All clones share a refresh lock plus an in-memory cache. While the cached credential is valid, the store isn't touched (`internal/auth.rs:99-147`).
  - A ChatGPT `invalid_grant` re-reads the store, which picks up a token another client rotated (`chatgpt/auth/native.rs:130-140`).
  - Copilot saves the GitHub token before the exchange and clears it on 401/403 (`copilot/auth/native.rs:53-89`).
- **ChatGPT sign-in and sign-out** (`chatgpt/auth/native.rs:160-289`):
  - Sign-in: `sign_in(&http, SignInMethod::{Browser, DeviceCode}, prompt)`, `sign_in_with_device_code`.
  - `sign_out(&http)` revokes at OpenAI, then calls `store.delete`.
  - `forget()`.
- **Copilot has no `sign_in`/`sign_out`/`forget`.** Sign-in means calling `auth_context(&http)` with `allow_device_flow = true`, and it holds the store lock during the device flow (`copilot/auth/native.rs:42-51`).
- **Using the credential:** `SignedInModel::new(spec: ModelSpec, auth, http: DynHttpClient)` (`rig/crates/rig-core/src/client/signed_in.rs:71-96`) or `auth.auth_context(&http) -> AuthContext { api_key: Secret, account_id, api_base }`.

### Design
1. **`VaultTokenStore`.** Add `core/providers/rig_login.rs`, with one store per credential directory. A sketch against the real 0.45 signatures is in `/tmp/rig-notes/tokens.md` §4.1.
   - Keys map to `CredentialFile`s: `"chatgpt"` → `logins/<id>/auth.json` (same path as today), `"copilot"` → `logins/<id>/copilot.json`.
   - `load`/`save`/`delete` call `CredentialFile::{read, write, delete}` under `tokio::task::spawn_blocking`, because keyring calls block and may wait on a macOS prompt.
   - `lock` returns `StoreLock::new(Arc<tokio::sync::Mutex<()>>::lock_owned().await)`, taken from a per-credential registry keyed by path, like `xai.rs:231-249`.
2. **Install points.** Replace `ProviderAuth::Login { token_dir }` for these two providers with a variant that carries the login handle (e.g. `RigLogin { dir, auth }`). Keep one `Authenticator` per credential in a registry. Each is built as `Authenticator::new(AuthSource::OAuth, VaultTokenStore, DeviceCodeHandler::default(), false)` and shared by:
   - turns (`build.rs:145-159`)
   - discovery (`chatgpt.rs:93`, `copilot.rs:135`, `discovery.rs:343`)
   - voice (`voice/connected.rs:93-101`)
   - image generation (`imagegen/chatgpt.rs:145`)
   - `desk.rs:1058-1065`
   - the failure probe (`failure.rs:339-344`)
   - the Copilot `Transport` (`copilot.rs:394-475`)

   `refreshed_auth` and `read_session` become `auth.auth_context(&http)`, with no file reads.
3. **Model wiring: keep Hotline's per-request signing transport instead of `SignedInModel`.**
   - `SignedInModel` needs a catalog `ModelSpec`. Hotline discovers ChatGPT/Copilot model ids dynamically (e.g. the test's `gpt-6-luna`), which would mean building custom `ModelSpec::new` rows.
   - Hotline already signs per request for Copilot Responses (`copilot.rs:498-573`) and xAI (`xai.rs:387-435`). Generalize that `Transport` to call `auth.auth_context(&http)` and set `Authorization` (plus `ChatGPT-Account-Id`).
   - Build models with `OpenAIConfig::with_key(&chatgpt::DIALECT | &copilot::DIALECT, "").connect(SigningTransport)`, with the base URL from `api_base` for Copilot.
4. **Sign-in** (`desk.rs:722-756`):
   - ChatGPT: `Authenticator::new(OAuth, store, DeviceCodeHandler::new(emit), true).sign_in_with_device_code(&http)`. The browser method fits `LoginPrompt`'s empty-`user_code` case (`crates/hotline-contract/src/credentials.rs:111-120`) but is a product decision.
   - Copilot: `Authenticator::new(OAuth, store, handler, true).auth_context(&http)`.
   - Stop pre-creating Rig files in `begin_login` (`vault.rs:297-304`; tests at `vault.rs:1743-1780`).
5. **Sign-out and delete.**
   - ChatGPT: call `auth.sign_out(&http)` (best effort) before `vault.delete`.
   - `abandon_login` must `delete()` the `copilot.json` `CredentialFile`, otherwise its keychain chunks are orphaned.
   - On revoke or delete, evict the registry entry and call ChatGPT `forget()`.
   - Copilot has no `forget()`. Wrap it in a Hotline `CredentialSource`, or check before each `auth_context` that the login still exists, so a revoked Copilot login stops serving its cached session token.
6. **Migration that keeps users signed in.**
   - **ChatGPT: no code needed.** The first `load("chatgpt")` reads `auth.json` through `CredentialFile::read()`, which moves plaintext into the keychain, verifies it, and swaps in the reference. It is idempotent and keeps plaintext on failure.
   - **Copilot: `migrate_copilot(dir)`.**
     1. If `copilot.json` exists, stop.
     2. Read and trim `access-token`; if it is empty or missing, stop (not signed in).
     3. Write `{"github_token": t, ..api-key.json fields if they parse}` through `CredentialFile::write`. Writing only `github_token` also works, at the cost of one extra exchange. The fingerprint is a `DefaultHasher` value that isn't stable across builds, so dropping it is safe.
     4. Only after the write succeeds, delete `access-token` and `api-key.json`.
   - **When it runs:** eagerly and best-effort from `Vault::open_with_store` next to `migrate_mcp_settings` (`vault.rs:206-215`), and lazily inside `load()` under Rig's `hold()`, so it can't interleave with a refresh.
   - **Failures.** A locked keychain leaves the files untouched and surfaces as `AuthError::Store`. Map it in `failure.rs` to an "unlock your keychain" message, not the current "Sign in again" strings (`chatgpt.rs:119`, `copilot.rs:74`).
7. **Contract.** No change to `crates/hotline-contract` or `ui/src/generated/contract.ts`; tokens never cross the wire. The internal changes are `ProviderAuth`, the `&Path`-taking helpers, and the comment at `vault.rs:1-2`.

**Staging.**
- **0.43/0.44:** the path-based `Authenticator::new(.., Some(dir.join("auth.json")), ..)` and Copilot's `(access_token_file, api_key_file)` keep today's files, so users stay signed in with no migration.
- **0.45:** the bump *requires* a `TokenStore`, and Rig's own `FileTokenStore` would read `chatgpt.json`/`copilot.json`, signing everyone out. The 0.45 slice therefore ships a Hotline store that at least maps to the existing files (S3). The keychain move is a small follow-up (S4).

---

## 5. Resumable runs evaluation (recommendation only)

**Recommendation: don't adopt `AgentRun`, resumable runs or committed-history streaming for the Hotline Agent turn loop. Do adopt exactly-once `on_dispatch`/`on_outcome` for voice metering.**

- **Hotline's loop isn't Rig's loop.** `turn.rs:223-338` streams the raw model and writes into history between and during rounds:
  - steering input (`turn.rs:230-233`)
  - job results (`turn.rs:224-229, 294-300`)
  - `keep_spoken` notes (`turn.rs:807-823`)
  - `recovery::fresh` rewrites and chapter notes at index 0 (`turn.rs:239-271, 590-611`)

  Rig 0.45's `CommittedLog::commit` is `pub(crate)` (`rig/crates/rig-agent/src/run/committed.rs:10-33`), so a host can't inject messages into a run, and the classic loop has no steering.
- **The restart policies are opposite.** Rig: "Unanswered tool calls re-execute on resume … (at least once)" (`rig/crates/rig-agent/src/agent/completion.rs:629-631`, PR #2762). Hotline and ADR 0008: never re-run side-effecting work after a restart.
  - `session/stopping.rs:1-27, 98-118` writes "It was not run again".
  - `session/exchanges.rs:724-736`.
  - `recovery.rs:44` tells the model that missing results "must not be repeated automatically".
  - `thread/policy.rs:109-245` holds the per-thread-kind restart rules.
  - Shells and subagents die with the process (`session/jobs.rs:1-16`).
- **Persistence ownership.** Hotline's record is the append-only tape (`crates/hotline-store/src/log.rs:1-37`), kept byte-compatible across editions. Rig moved `RUN_FORMAT` 2 → 5 in one release with no upgrade shim (`run/mod.rs:355-370`), and `rig-session/4` refuses older sessions. Adopting Rig persistence would drop in-flight runs on every Rig bump.
- **Compaction.** PR #2846's "stop during a held summary cancels it" touches only the rig-ecs harness and `rig-basics`. The classic agent never compacts, so this doesn't apply. Hotline's chapter rotation is product behaviour.
- **Worth taking anyway:**
  1. **Voice billing by dispatch id.** Reserve in `on_dispatch` and settle in `on_outcome` with a `HashMap<EffectId, Reservation>` (`hook.rs:743-756, 1064-1100`). This replaces the single slot (`dispatcher.rs:653-654, 705`) that overwrites an earlier attempt's reservation and misses attempts `on_model_turn_finished` skips (comment at `dispatcher.rs:725-726`).
     - Budget refusal moves to the next `on_dispatch` (deny), because `OutcomeAction` can't stop the run.
     - Tests: a provider error keeps its reservation; a hook-retried turn settles both attempts; a dropped stream stays charged; a refusal happens on the next dispatch.
  2. **Optional:** `rig::conversation::pairing::validate` (`rig/crates/rig-core/src/conversation/pairing.rs:62`) as a `debug_assert!` before `model.stream` in `turn.rs`.
- **If the product ever wants interrupted turns to continue:** spike with `AgentRun` on a thread kind without steering (e.g. a job thread). Store it as an opaque sidecar keyed by tape position, and on resume answer pending calls with `ToolResult::skipped("Hotline restarted …")` rather than executing them.

---

## 6. Proposed slices (brainfile epic on release/0.42.0)

**Path: 0.42 → 0.43 → 0.44 → 0.45, one bump per slice, each green.**

Why stepwise rather than a direct jump:
- 0.43 alone rewrites provider construction, transport, streaming, errors, usage and hooks, and brings the biggest silent change (Anthropic usage).
- 0.44 and 0.45 are each moderate.
- Isolating them lets QA attribute regressions.

The cost is touching some code twice:
- `Usage` test literals: 0.43 `Option`, then 0.44 constructors.
- `Message::Assistant`: 0.43, then 0.44.
- Auth wiring: 0.43 path-based, then 0.45 `TokenStore`.
- `DynamicTool` names: 0.43 closure arity, then 0.44 `ToolName`.

The guide supports skipping intermediate forms. A team in a hurry could fold S2 into S3, but not S1. Each bump slice must keep `contract.ts` byte-identical; no wire type changes anywhere in this plan.

**Gate for every slice:** `make check` (fmt, clippy `-D warnings`, `cargo test --workspace`, `cargo deny`/`machete`, ui-check) and `make precheck` (fmt, lockfile `--locked`, actionlint, citations, contract, clippy, the Windows cross-check, ui). Also `git diff --exit-code ui/src/generated/contract.ts`.

| # | Title | What changes | Files | Proof | Risk | Effort |
|---|---|---|---|---|---|---|
| **S0** | Narrow Hotline's Rig seam (still 0.42) | No dependency bump. Helper and test changes only:<br>• One `usage` adapter (rig `Usage` → Hotline counters, `TokenUsage`, pricing input), so 0.43's `Option` change touches one function.<br>• One `stream_event` mapper in `turn.rs` (text, thought, final), so 0.43's event change is local.<br>• Keep the preamble as a `String` on `Turn`, prepended per round, instead of `CompletionRequest.preamble`.<br>• `build.rs`: split `completion_model(..)` from `agent_builder(..)` where 0.42 allows.<br>• Behaviour-pinning tests that must survive the bump: Anthropic usage totals (cache tokens priced once), request-body snapshots for cache markers (Anthropic `cache_control`, OpenRouter `anthropic/`), Copilot chat assistant content as a string, Ollama `think` reaching `/api/chat`, and the `Failure::classify` table on real error shapes. | `driver/rig.rs`, `driver/rig/{turn,build,recovery}.rs`, `pricing.rs`, `voice/dispatcher.rs`, tests | `make check`, `make precheck`; the new tests pass on 0.42 | Low | 1–1.5 d |
| **S1** | Rig 0.43: providers, transport, streaming, errors, usage, hooks | • `Cargo.toml` rig 0.43.0; lockfile.<br>• Providers per §3.2: `Anthropic::new`/`OpenAI::new`/vendor `new`, `OllamaConfig…connect`, `OpenAIConfig` for custom/xAI/zai/Copilot.<br>• ChatGPT/Copilot via path-based `Authenticator::new` with **today's files** plus `OpenAI::authenticate`.<br>• `HttpClientExt` impls hold `ReqwestClient`; `http_client::Error` variants; `sse::BoxedStream` → `BoxedStream`.<br>• `DynModel<Completion>` into `turn::run`; `CompletionRequest::new` builder; system message per round.<br>• Streaming per §3.4 (`Item<StreamEvent>`, `finish()`, `partial()`, drop to cancel, `Truncated`).<br>• `ProviderError` in `failure.rs`.<br>• `Usage` `Option`, and **delete the Anthropic fold in `every_input_token` and `cache_beside_input`**.<br>• `CallId`/`ToolName`/`Message::tool_result`.<br>• `DynamicTool` closures drop the ctx arg.<br>• Voice: `prompt().stream()`, `CompletionCallEvent`, settle in `on_outcome` (minimal port).<br>• Tests: `ScriptModel` → `MockCompletionModel` (dev-dep `test-utils`); recovery replay tests → body snapshots. | `Cargo.toml`, `Cargo.lock`, `driver/rig.rs`, `driver/rig/*`, `driver/failure.rs`, `providers.rs`, `providers/*`, `desk.rs`, `pricing.rs`, `voice/dispatcher.rs`, `voice/front_tools.rs`, `mcp/server.rs`, `session/jobs.rs`, tests | `make check`/`precheck`; S0 pinning tests unchanged; manual: one turn on each of Anthropic, OpenAI, ChatGPT (existing sign-in), Copilot chat and Responses (existing sign-in), OpenRouter `anthropic/`, Ollama, a custom server, xAI subscription; voice call metering; budget numbers equal to the 0.42 baseline for the same transcript | **High**: largest diff; silent usage and truncation semantics; Copilot/ChatGPT transport rewiring | 3–5 d |
| **S2** | Rig 0.44: constructors, assistant messages, typed options | • rig 0.44.0.<br>• `CompletionRequest`/`Usage`/`Text` constructors.<br>• `Message::Assistant(AssistantMessage)` via `response.message()`.<br>• `ToolFunction.arguments: Map`, `invalid_arguments` answered with an error; `ToolResult.is_error`.<br>• `ToolName` in `DynamicTool::new`/`ToolDefinition`.<br>• Anthropic `.with_automatic_caching()` → request `.cache(CacheRetention::Short)`; OpenRouter `with_prompt_caching` → request `.cache` for `anthropic/` only.<br>• Ollama → `native_completion`.<br>• `_` arms on non-exhaustive enums; `Reasoning{text,..}`; `response.error` treated as no-tools.<br>• `PromptResponse::output()`; voice `accept_unknown_finish_reasons(true)` (decide).<br>• Test fixtures for reasoning/native. | `driver/rig/{turn,recovery,build,images,tool_output,granted,strict}.rs`, `driver/rig.rs`, `session/jobs.rs`, `voice/*`, `mcp/server.rs`, `providers.rs`, tests | `make check`/`precheck`; cache-marker snapshot tests from S0 still pass; manual Anthropic multi-round turn shows `cache_read` > 0 on round 2 | Medium: caching regressions are invisible without the snapshot tests; Gemini effort raw-JSON | 1.5–2 d |
| **S3** | Rig 0.45 + login-dir `TokenStore` | • rig 0.45.0.<br>• `rust-version = "1.97.1"`.<br>• `Arc<Message>`/`Arc<ToolDefinition>` in requests; `ToolDefinition::new`; `ToolSet::from_dynamic_tools` → `add_dynamic_tool`; `bearer_auth_header` → manual header.<br>• Hooks: `CompletionCallEvent.history: &[Arc<Message>]`.<br>• **Auth:** `Authenticator::new(OAuth, LoginDirStore, ..)`. `LoginDirStore` is Hotline's `TokenStore` over the existing login dir (`chatgpt` → `auth.json`, plaintext as today; `copilot` → `copilot.json`, migrated once from `access-token` + `api-key.json`) with per-credential `lock()`.<br>• Per-credential authenticator registry.<br>• Signing transport calls `auth.auth_context()`; `refreshed_auth`/`read_session` stop reading files.<br>• Sign-in via `sign_in_with_device_code` / `auth_context`. | `Cargo.toml`, `Cargo.lock`, `providers/{chatgpt,copilot,rig_login(new)}.rs`, `desk.rs`, `vault.rs`, `driver/rig/build.rs`, `driver/failure.rs`, `voice/connected.rs`, `imagegen/chatgpt.rs`, `discovery.rs`, `voice/dispatcher.rs`, tests | `make check`/`precheck`; new tests: a 0.42-layout Copilot dir migrates to `copilot.json` and old files are removed only after the write; a ChatGPT `auth.json` loads unchanged; concurrent refresh via two clients spends the refresh token once (stub server); manual: upgrade a profile signed in under 0.42 and confirm both providers work without re-sign-in | **High**: sign-in continuity; toolchain bump on the Windows precheck leg | 2–3 d |
| **S4** | Move ChatGPT/Copilot tokens into the vault | Swap `LoginDirStore`'s body for `CredentialFile` (keychain) under `spawn_blocking`. ChatGPT `auth.json` migrates automatically on first `read()`. Copilot `copilot.json` is written through `CredentialFile::write`. Stop pre-creating files in `begin_login`. `abandon_login` deletes the `copilot.json` credential. Map `AuthError::Store` to "unlock your keychain". Update `vault.rs:1-2`. | `providers/rig_login.rs`, `vault.rs`, `driver/failure.rs`, tests (`MemoryStore` backend) | `make check`/`precheck`; tests: plaintext → reference swap, keychain failure keeps plaintext and errors, delete removes chunks; manual on macOS: a signed-in profile upgrades, `auth.json` becomes a reference file, and keychain entries exist | Medium: keychain prompts on unsigned dev builds; a downgrade to an older Hotline would show sign-in required (release note) | 1 d |
| **S5** | Sign-out revocation and revoked-login hygiene | ChatGPT `sign_out(&http)` (best effort) on delete; registry eviction plus `forget()` on revoke/delete; a Copilot guard (`CredentialSource` wrapper checks the login is still live). | `vault.rs`, `desk.rs`, `providers/rig_login.rs` | Tests with stub revoke endpoint; revoked Copilot login stops serving within one request | Low | 0.5–1 d |
| **S6** | Voice metering keyed by dispatch id | Replace `Meter`/`StreamMeter` with one hook: `on_dispatch` reserves (deny on budget), `on_outcome` settles by `EffectId`. Remove the single reservation slot. | `voice/dispatcher.rs` | The four tests listed in §5 | Low–medium (billing) | 0.5–1 d |
| **S7** | Cleanups unlocked by 0.45 | Delete Copilot `ChatBody` if the S0 snapshot test passes without it. Copilot `/responses` override via `with_route(Route::Responses)` on the shared dialect. `compat.rs` probe on `Value`. Optional: typed `.reasoning(Effort)` with `OnUnsupported::Ignore` replacing `effort_params`; `pairing::validate` debug assert. | `providers/{copilot,compat}.rs`, `driver/rig/build.rs`, `turn.rs` | `make check`/`precheck`; S0 snapshots unchanged | Low | 1 d |

**Total:** about 11–16 engineering days. S1 and S3 are the critical path. S4–S7 can land in any order after S3, and S6 could also follow S1 in a minimal form.

Suggested brainfile layout (per the board conventions): an epic "Rig 0.45 upgrade" on the hotline board, with S0–S7 as child tasks in To do, tagged for the release after 0.42.0. Check the column names before moving anything.

---

## 7. Risks and open questions

1. **Silent usage change (S1).** If the Anthropic double count isn't removed, both budgets (`turn.rs` Chat meter, voice dispatcher) over-charge on every cached Claude request, and the context threshold (`turn.rs:240-245`) trips early, forcing needless chapter rotations. The S0 pinning test is the guard.
2. **Hand-built request vs agent options.** Hotline bypasses `Agent` for turns, so any setting configured on `AgentBuilder` (`.cache`, `.reasoning`, `.max_tokens`, `.additional_params`) must also be set on Hotline's `CompletionRequest`. Today `request_settings` duplicates it (`rig.rs:776-794`); keep that invariant explicit.
3. **Preamble loss.** With 0.43's system-as-message model and `turn.rs:402` overwriting `chat_history`, the system prompt silently disappears unless it is re-prepended each round.
4. **Sign-in continuity at 0.45.** Rig's own stores read `chatgpt.json`/`copilot.json`. Hotline must ship its store in the same slice (S3), or every ChatGPT and Copilot user is signed out.
5. **Copilot device flow holds the store lock** for up to 15 minutes. Key locks per credential path, never per provider.
6. **Refresh-token races** exist today: every 0.42 builder has its own lock, so voice, turns, image generation and discovery can spend the same rotated ChatGPT refresh token. The shared registry plus `lock()` fixes this. Rig's re-read after a spent token is a second net.
7. **`SignedInModel` needs catalog `ModelSpec`s.** Unclear for dynamically discovered ChatGPT/Copilot ids; hence the per-request signing transport. Open question: does `ModelSpec::new` with minimal facts behave correctly for unknown ids (default max tokens, image support)? Verify before choosing `SignedInModel`.
8. **Catalog facts now influence requests** (0.44/0.45): image omission for models listed without vision, `max_tokens` vs `max_completion_tokens`, unknown-model defaults. Hotline's own catalogue (`models.rs`) and Rig's may disagree. Watch Copilot and OpenRouter models that Rig's catalog lists differently.
9. **Gemini effort JSON** now passes through raw (0.44). Confirm Google accepts `thinkingLevel: "low"` in lowercase, or move to typed `.reasoning()`.
10. **Unknown finish reasons fail agent runs** (0.44); voice may start failing on gateways. Decide on `accept_unknown_finish_reasons(true)` for voice.
11. **Error classification drift** (0.45 in-band envelopes, `InvalidAuthentication`, Display prefixes). Re-validate `Failure::classify` (`failure.rs:117-210`) with recorded 0.45 errors, especially the ChatGPT "provider refused vs sign-in broken" test (`failure.rs:742-800`).
12. **MSRV 1.97.1 and Rig 0.45's pace.** 0.43 → 0.45 took three weeks with heavy churn, and `MIGRATING.md` for 0.45 shipped a day after the tag. Pin `=0.45.0` until 0.45.x settles. The upstream README now carries a stability warning (PR #2864); worth reading before committing.
13. **Release-branch state.** The checkout HEAD is the release/0.41.0 merge (`e05716c4`), even though the branch is named release/0.42.0. Confirm that's the intended base before cutting the epic's branches.
14. **Open product questions.**
    - Adopt the ChatGPT browser sign-in (`SignInMethod::Browser`, PKCE on 127.0.0.1:1455/1457) now that Rig supports it?
    - Revoke tokens at the provider on credential delete (S5)?
