# Contas de agente por teammate — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cada teammate de um agente com suporte a contas (hoje Claude Code) roda com a conta escolhida pelo operador, isolada por processo via `CLAUDE_CONFIG_DIR`, com cadastro, login oficial, setup por conta, troca manual (inclusive no limite de uso) e compatibilidade total com o comportamento atual ("System default").

**Architecture:** Metadados das contas no setting de sala `agentAccounts` (só ids, nomes, datas), pastas em `<data>/agent-accounts/<uuid>`. Uma tabela de suporte por agente (`accounts/support.rs`, uma linha: `claude-acp`) parametriza ambiente, login, status, setup e frases de limite; todo o resto é genérico. A conta é resolvida uma vez em `DeskAgents::agent` e aplicada por um `EnvPlan` em todo processo do agente (sessão, restart, status, login); a troca é uma mudança de política aplicada só com o teammate ocioso.

**Tech Stack:** Rust 2024 (hotline-contract/support/store/room/core/app, tokio, ts-rs, serde, portable-pty, windows-sys), React 19 + TypeScript 7 + bun test + biome/knip (ui/), Tauri 2 (shell).

**Spec:** `docs/superpowers/specs/2026-10-21-agent-accounts-design.md` (aprovada em 2026-10-21). Protótipo: https://claude.ai/artifact/2nixmDsgN3esfjkh594aza (telas 1–7). Executores leem a spec e este plano.

## Global Constraints

- Regras do repositório: `AGENTS.md` e `docs/standards.md` — um escritor por stream; todo tipo exportado em `crates/hotline-contract`; `cargo test -p hotline-contract` **sem filtro** regenera `ui/src/generated/contract.ts` e é a última execução antes de commitar esse arquivo; dependências entre crates só descem; delete, don't disable; nenhum trait com uma implementação; suppressions com `#[expect(lint, reason = "…")]`; locks com `unwrap_or_else(PoisonError::into_inner)`; arquivos de produção ≤ 1.000 linhas (1.500 dispara split), testes em `foo/tests.rs` ao passar de 400 linhas.
- Tudo novo é aditivo e opcional: ausente = System default = comportamento de hoje, byte a byte no ambiente do processo (System default não remove nem acrescenta variável).
- Nenhum `std::env::set_var`; argumentos só por `Command::args`; caminhos como `PathBuf`/`OsString`, canonizados com `dunce`.
- Segredos e PII: nunca em stream, tape, log (`eprintln!`), wire para Phone/Link, nem eco de saída bruta do CLI.
- Nomes compartilhados: `docs/superpowers/plans/agent-accounts/CONTRACT_SHEET.md` + `RESOLUTIONS.md` (já incorporados nas Interfaces de cada tarefa; em conflito, vale RESOLUTIONS). Cópia do protótipo aprovado (textos de UI) em `docs/superpowers/plans/agent-accounts/prototype/*.dc.html`.
- Textos de UI em inglês, exatamente os da spec/protótipo; tooltips de 2–3 palavras (`ui/design.md`).
- Desenvolvimento com `HOTLINE_DATA_DIR` isolado; nunca encerrar processos por nome ou porta.
- Windows sem `make`: antes do primeiro build, `python scripts/fetch-speech-engine`; checks por comando (Task 13).
- Commits: uma linha que declara o invariante, terminando com `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; só commitar quando o operador autorizar a execução do plano, num branch `feature/agent-accounts` (nunca em `main`).

## Review Focus

Entradas e falhas que a spec implica, nenhum teste das tarefas cobria, e que mais provavelmente afetam quem usa:

1. **"Start with: My setup" numa máquina sem `~/.claude`** (instalação nova do Claude Code nunca usada) — a criação deve falhar com "There is no Claude Code setup on this computer to copy." e não gravar conta nem deixar meia pasta; "Empty" funciona. Teste na Task 7/8.
2. **Remover uma conta enquanto o login dela está rodando** — recusado com frase clara ("Finish or cancel {account}'s sign-in first."), sem matar a janela nem gravar nada. Teste na Task 8.
3. **Teammate removido ou trocado de agente com troca de conta pendente** — a pendência some sem erro e nada é aplicado a um teammate que não existe. Teste na Task 9.
4. **Nome com Unicode** — o limite de 64 conta caracteres, não bytes ("Conta ção" e 64 emojis aceitos, 65 recusados); duplicidade ignora maiúsculas. Teste na Task 2.
5. **Diretório de dados alcançado por alias** (`HOTLINE_DATA_DIR` via symlink/junction, `%TEMP%` com nome curto 8.3) — `checked_account_dir` aceita a pasta legítima e recusa só um link no lugar da pasta da conta. Teste na Task 3.

---

### Task 1: Contract types and command variants (hotline-contract)

**Files:**
- Create: `crates/hotline-contract/src/accounts.rs`
- Modify: `crates/hotline-contract/src/lib.rs:26-47`. Add `mod accounts;` and `pub use accounts::*;` in alphabetical order.
- Modify: `crates/hotline-contract/src/persona.rs`:
  - `:94-99`: Persona, after `harness_override`.
  - `:159-166`: SessionCheckpoint.
  - `:379-411`: PersonaDraft, after `background_work`.
- Modify: `crates/hotline-contract/src/session.rs:13-60` (SessionInfo, after `processes`).
- Modify: `crates/hotline-contract/src/transcript.rs`: `:287-322` (Side), `:354-392` (Link), `:398-420` (Chapter).
- Modify: `crates/hotline-contract/src/command.rs:878-881` (12 variants after `SecretsPasskeyCancel`).
- Modify: `ui/src/generated/contract.ts` (regenerated) and `ui/src/wire.ts`:
  - `:1-51`: imports.
  - `:137-140` and `:285-288`: `Results`.
  - `Commands` is `[N in CommandName]`, so a missing `Results` row is a tsc error as soon as contract.ts regenerates. This task adds every row; Task 11 adds none.
- Modify (compile fixes; the value to use is in brackets):
  - `crates/hotline-core/src/wire/commands.rs:216-221`: the match at :74 is exhaustive. Add one grouped arm for the 12 new variants, shaped like the agent.auth arm, returning `Err("Agent accounts are not on this desk yet.".into())`. Task 8 replaces it arm by arm.
  - SessionInfo [`account: None`]:
    - `crates/hotline-core/src/session.rs:340-370` (`idle_info`)
    - `crates/hotline-core/src/wire/tests.rs:40-70` (`idle`)
  - Persona [`agent_account_id: None`], under `crates/hotline-core/src/` unless noted:
    - `computer/tests.rs:61-99`, `desk.rs:1483-1513`, `driver/acp/tests.rs:45-75`, `driver/rig/tests.rs:1196-1226`
    - `import/schedules.rs:143-173`, `import/threads.rs:128-158`, `mcp/server.rs:1655-1685`
    - `session/avatar/tests.rs:23-53`, `session/generate/tests.rs:138-168`, `session/schedule.rs:762-792`, `session/tests.rs:490-522`
    - `wire/commands/persona.rs:64-97` (Task 8 puts the validated draft value here)
    - `crates/hotline-room/src/room.rs:711-741`
  - PersonaDraft [`agent_account_id: None`]:
    - `wire/commands/persona.rs:162-174` (MobileDraft→draft). It stays `None` because a phone cannot express it (§8.3).
    - `wire/tests/models.rs:184-196,311-323`
    - `wire/tests/personas.rs:39-51,294-306,402-414`
    - `wire/tests/seats.rs:354-366`
  - SessionCheckpoint [`account_id: None`]:
    - `driver/acp/tests/turns.rs:120-123,788-791`
    - `session/agent.rs:408-411` (Task 5 fills it from the binding), plus the test literals at `:561-564,803-806`
    - `session/sides/tests.rs:691-694`, `session/tests/cards.rs:109-112,251-254`, `session/tests/runs.rs:148-151`, `session/threads.rs:316-323`
    - `crates/hotline-room/src/room.rs:659-662` (Task 2 rewrites it)
  - Chapter, in `session/chapters.rs`:
    - `opened` :125-139 and `reopened` :144-175 get `account_id: None`. Task 5 decides what a resumed marker carries.
    - `closed` :199-238 binds `account_id` in the destructure at :205 and writes it back at :220.
    - `session/peers/tests.rs:1379-1391` gets `None`.
  - Side, in `crates/hotline-store/src/link.rs`: the `read_marker` destructure :227-241 binds `account_id: _`, and `marker()` :301-331 sets `account_id: None`. Task 2 replaces both.
- Test: `crates/hotline-contract/src/tests.rs`, and `crates/hotline-core/src/session/chapters.rs` (`mod tests`, :1038).

**Interfaces:**
- Consumes: the CONTRACT_SHEET hotline-contract section. `serde` and `ts_rs::TS` are already in the crate.
- Produces:
  - In `accounts.rs`, with fields as in the sheet: `AgentAccount`, `AccountSetupOrigin`, `SetupFrom`, `AccountStatusState`, `AccountStatusView`, `LoginAttemptState`, `LoginAttemptView`, `AccountView`, `AccountAgent`, `AccountsList`, `AccountCreated`, `SetupCategoryView`, `AccountSetupView`, `SetupCopyReport`, `AccountHold`, `SessionAccount`.
  - Additive fields:
    - `Persona.agent_account_id: Option<String>` and `PersonaDraft.agent_account_id: Option<String>`
    - `SessionCheckpoint.account_id: Option<String>`
    - `SessionInfo.account: Option<SessionAccount>`
    - `account_id: Option<String>` on `TranscriptEvent::{Chapter, Side, Link}`
  - Command variants:
    - `AccountsList {}`
    - `AccountsCreate { backend_id, name, start_with: SetupFrom, categories: Option<Vec<String>> }`
    - `AccountsRename { account_id, name }`
    - `AccountsRemove { account_id }`
    - `AccountsStatus { account_id }`
    - `AccountsLoginStart { account_id, email: Option<String> }`
    - `AccountsLoginPoll { account_id, id }`
    - `AccountsLoginInput { account_id, id, input }`
    - `AccountsLoginCancel { account_id, id }`
    - `AccountsSetupList { account_id }`
    - `AccountsSetupCopy { account_id, categories: Vec<String> }`
    - `PersonaAccount { persona_id, account_id: Option<String> }`
  - wire.ts: the poll type `SignInStatus` and the 12 `Results` rows. The remove, input and cancel rows are `null`, so Task 8 answers `Value::Null` for them, as commands.rs:836 does.
  - SHEET DEVIATION: `TranscriptEvent::Link` also gets `account_id` after `backend_id`, because it is the typed mirror of `Link::event` (spec §8.4).

- [ ] **Step 1: Write the failing tests.** They go in `crates/hotline-contract/src/tests.rs`, with `const A: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";` and `let config = ts_rs::Config::default();`.
```rust
#[test] fn agent_account_fields_are_additive_and_absent_is_system_default() {
  let old = json!({"backendId":"claude-acp","sessionId":"s1"});
  let c: SessionCheckpoint = serde_json::from_value(old.clone()).unwrap();
  assert_eq!(c.account_id, None); assert_eq!(serde_json::to_value(&c).unwrap(), old);
  let on = json!({"backendId":"claude-acp","sessionId":"s1","accountId":A});
  assert_eq!(serde_json::to_value(serde_json::from_value::<SessionCheckpoint>(on.clone()).unwrap()).unwrap(), on);
  for line in [json!({"kind":"chapter","id":"c1","ts":1,"backendId":"claude-acp","sessionId":"s1"}),
               json!({"kind":"chapter","id":"c1","ts":1,"backendId":"claude-acp","accountId":A})] {
    assert_eq!(serde_json::to_value(serde_json::from_value::<TranscriptEvent>(line.clone()).unwrap()).unwrap(), line); }
  let p: Persona = serde_json::from_value(json!({"id":"ada","name":"Ada","goal":"","backendId":"claude-acp","cwd":"/w",
    "mcpPolicy":{"mode":"none","serverIds":[]},"sessionCheckpoints":[],"createdAt":1,"updatedAt":1})).unwrap();
  assert!(serde_json::to_value(&p).unwrap().get("agentAccountId").is_none());
  assert!(SessionCheckpoint::decl(&config).contains("accountId?: string"));
  assert!(Persona::decl(&config).contains("agentAccountId?: string"));
  assert!(PersonaDraft::decl(&config).contains("agentAccountId?: string"));
  assert!(SessionInfo::decl(&config).contains("account?: SessionAccount"));
  let events = TranscriptEvent::decl(&config);
  assert!(events.contains("backendId: string, accountId?: string"));              // chapter
  assert_eq!(events.matches("backendId?: string, accountId?: string").count(), 2); // side, link
}
#[test] fn account_commands_round_trip_under_their_wire_names() {
  // One json! per variant (the 12 wire names), plus accounts.create with and without `categories`,
  // accounts.login.start with and without `email`, and persona.account with "accountId": A and with
  // "accountId": null. Each goes through from_value::<Command> and to_value, and comes back equal.
  assert!(matches!(serde_json::from_value::<Command>(json!({"cmd":"persona.account","params":{"personaId":"ada"}})).unwrap(),
    Command::PersonaAccount { account_id: None, .. }));
  let commands = Command::decl(&config);
  assert!(commands.contains(r#""cmd": "persona.account", "params": { personaId: string, accountId?: string | null, }"#));
  assert!(commands.contains(r#""cmd": "accounts.login.start", "params": { accountId: string, email?: string, }"#));
}
#[test] fn an_agent_account_reads_back_as_written_and_its_states_are_snake_case() {
  let bare = json!({"id":A,"backendId":"claude-acp","name":"Account 1","createdAt":1});
  let a: AgentAccount = serde_json::from_value(bare.clone()).unwrap();
  assert_eq!((a.login_at, a.setup.is_none()), (None, true)); assert_eq!(serde_json::to_value(&a).unwrap(), bare);
  // `full` = bare + "loginAt":2 + "setup":{"from":"system_default","at":1,"categories":["instructions"]} round-trips too
  assert_eq!(serde_json::to_value(SetupFrom::SystemDefault).unwrap(), "system_default");
  assert_eq!(serde_json::to_value(AccountStatusState::NotChecked).unwrap(), "not_checked");
  assert_eq!(serde_json::to_value(AccountHold::SigningIn).unwrap(), "signing_in");
  assert_eq!(serde_json::to_value(SessionAccount { running: None, wanted: None, held: None }).unwrap(), json!({}));
  assert!(AccountView::decl(&config).contains("folder?: string"));
}
```
  - In `chapters.rs` `mod tests`, add `a_close_keeps_the_account_its_chapter_ran_on`. It calls `closed(&json!({"kind":"chapter","id":"c1","ts":100,"backendId":"claude-acp","accountId":"acc"}), 900, Closing::Empty, ChapterClose::Idle)` and checks that the result serializes with `["accountId"] == "acc"`.
- [ ] **Step 2: Run it to verify it fails.** Run `cargo test -p hotline-contract --lib tests::`. Expected: a compile error (E0412 cannot find type `AgentAccount`; E0599 no variant `Command::PersonaAccount`). Because it does not compile, it writes nothing to contract.ts.
- [ ] **Step 3: Implement.**
  - accounts.rs:
    - Every struct gets `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]`, `#[serde(rename_all = "camelCase")]` and `#[ts(export, export_to = "contract.ts", optional_fields)]`.
    - Every `Option` field gets `#[serde(default, skip_serializing_if = "Option::is_none")]`.
    - The enums also derive `Copy, Eq` and take `#[serde(rename_all = "snake_case")]`.
    - Each type has a one-line doc taken from spec §8.1 and §8.5.
  - Persona and PersonaDraft get `#[serde(skip_serializing_if = "Option::is_none")] pub agent_account_id: Option<String>`. The doc says three things:
    - absent means System default;
    - the value is kept but ignored on an agent without accounts;
    - only `persona.account` and `persona.create` write it.
  - SessionCheckpoint gets `#[serde(default, skip_serializing_if = "Option::is_none")] #[ts(optional)] pub account_id`. The field needs `#[ts(optional)]` because the struct has no `optional_fields`.
  - SessionInfo gets `#[serde(default, skip_serializing_if = "Option::is_none")] pub account: Option<SessionAccount>`.
  - Side and Link: put `account_id` directly after `backend_id`, with the neighbour's serde attribute and no doc of its own. A doc would split the TS string that Step 1 asserts. Extend the `session_id` doc with "and the agent account it ran on".
  - Chapter: add `#[serde(skip_serializing_if = "Option::is_none")] account_id` after `backend_id: String` (:401).
  - Commands:
    - Each doc says what the variant answers and ends "Owner or local desk only."
    - `categories` and `email` get `#[serde(default, skip_serializing_if = "Option::is_none")]`.
    - `PersonaAccount.account_id` is a plain `Option<String>`. Its only attribute is `#[ts(optional = nullable)]`, the same one `cpus` has at command.rs:331. It has no `present_option` and no skip. Null and absent both mean System default.
  - wire.ts:
    - Import `AccountCreated`, `AccountSetupView`, `AccountStatusView`, `AccountView`, `AccountsList`, `SessionAccount` and `SetupCopyReport`.
    - Add `type SignInStatus = { state: "running" | "succeeded" | "failed"; output: string; error?: string }`. Leave it unexported, because knip fails on an unused export; Task 11 exports it when it needs it. Use it for both `"agent.auth.poll"` and `"accounts.login.poll"`.
    - New rows:

      | Command | Result |
      | --- | --- |
      | `accounts.list` | `AccountsList` |
      | `accounts.create` | `AccountCreated` |
      | `accounts.rename` | `AccountView` |
      | `accounts.status` | `AccountStatusView` |
      | `accounts.login.start` | `{ id: string }` |
      | `accounts.setup.list` | `AccountSetupView` |
      | `accounts.setup.copy` | `SetupCopyReport` |
      | `persona.account` | `SessionAccount` |
      | `accounts.remove`, `accounts.login.input`, `accounts.login.cancel` | `null` |
  - Then make every compile fix listed under Files.
- [ ] **Step 4: Run tests to verify they pass.** Run these in order:
  1. `python scripts/fetch-speech-engine` (once; the workspace links it).
  2. `cargo test -p hotline-contract --locked`. Run it unfiltered and first, because it writes contract.ts whole. Expected: PASS.
  3. `cargo check --workspace --all-targets --locked`, then `cargo fmt --all --check`.
  4. `cargo test -p hotline-store --lib link::tests` and `cargo test -p hotline-core --lib session::chapters::tests`. Expected: PASS.
  5. `(cd ui && bun install --frozen-lockfile && bun run typecheck && bun run lint && bun run knip)`. Expected: PASS.
- [ ] **Step 5: Commit.** `git add crates/hotline-contract crates/hotline-core crates/hotline-room/src/room.rs crates/hotline-store/src/link.rs ui/src/generated/contract.ts ui/src/wire.ts`, then `git commit -m "An agent account is an optional id on the teammate, its checkpoints, its session and its chapter and thread lines, and absent is System default."`

---

### Task 2: Room persistence (hotline-room, hotline-store link.rs)

**Files:**
- Create:
  - `crates/hotline-room/src/agent_accounts.rs`: the filter, in the `mcp_servers.rs` pattern, with an in-file `mod tests`.
  - `crates/hotline-room/src/room/tests.rs`: the moved tests.
- Modify: `crates/hotline-room/src/lib.rs:1-30`. The crate doc names `agent_accounts`; add `pub mod agent_accounts;`.
- Modify `crates/hotline-room/src/room.rs`:
  - :32-55: imports, `AGENT_ACCOUNTS` and `defaults()`.
  - :320-343: `settings_from_events`.
  - :557-634 and :695-1299: both test modules, replaced by `#[cfg(test)] mod tests;`.
  - :645-693: the checkpoints and the lock.
- Modify callers (the new argument is `None`):
  - `crates/hotline-core/src/session.rs:1059-1064`
  - `crates/hotline-core/src/session/dm.rs:259-260`
  - `crates/hotline-core/src/session/chapters.rs:702,711,798`
- Modify `crates/hotline-store/src/link.rs`:
  - :49-55 (`AgentBinding`)
  - :153-164 (`event`)
  - :191-223 (`read_link`)
  - :225-265 (`read_marker`)
  - :299-331 (`marker`)
  - :389-408 (fixture)
- Modify the AgentBinding literals:
  - `crates/hotline-core/src/thread/store.rs:107-117`
  - `crates/hotline-core/src/session/job_thread.rs:243-246`
  - `crates/hotline-core/src/session/sides/ending.rs:140-143`
  - tests: `session/agent.rs:675-680`, `session/threads.rs:336-341,374-379`
- Test:
  - `crates/hotline-room/src/agent_accounts.rs`
  - `crates/hotline-room/src/room/tests.rs` (fixtures `scratch`, `persona`, `persona_event`, `setting`, `append`)
  - `crates/hotline-store/src/link.rs` (fixture `closed_side`)

**Interfaces:**
- Consumes (Task 1): `hotline_contract::AgentAccount`, `SessionCheckpoint { backend_id, session_id, account_id: Option<String> }`, `TranscriptEvent::Side { account_id, .. }`.
- Produces:
  - `room::AGENT_ACCOUNTS: &str = "agentAccounts"`, default `[]`.
  - `room::agent_accounts(log: &Log) -> Vec<AgentAccount>`.
  - `room::write_agent_accounts(log: &Log, accounts: &[AgentAccount]) -> std::io::Result<()>`. It writes one `setting` event and does not validate. Task 8 checks `normalize_accounts(&json!(candidate)) == candidate` under `policy_update_lock` before calling it.
  - `hotline_room::agent_accounts::normalize_accounts(value: &serde_json::Value) -> Vec<AgentAccount>` (SHEET ADDITION: public, as the per-entry filter of §8.1). The id must be the canonical uuid: `Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)`.
  - `room::checkpoint_session(log: &Log, persona_id: &str, backend_id: &str, account_id: Option<&str>, session_id: &str) -> Result<(), String>`.
  - `room::clear_checkpoint(log: &Log, persona_id: &str, backend_id: &str, account_id: Option<&str>) -> Result<(), String>`.
  - `room::clear_account_checkpoints(log: &Log, account_id: &str) -> Result<(), String>`, for Task 9's `Room::account_signed_in`.
  - `room::clear_backend_checkpoints(log: &Log, persona_id: &str, backend_id: &str) -> Result<(), String>`, for Task 9's switch apply.
  - `room::persona_write_lock() -> std::sync::MutexGuard<'static, ()>`:
    - It is the innermost lock: never held across `.await`, and nothing is acquired under it.
    - The four checkpoint functions above take it themselves, so a holder never calls them. Task 9's `persona.account` append is one such holder, and holds it under `policy_update_lock`.
    - `Room::checkpoint` reaches it while holding `lifecycle`.
  - `hotline_store::link::AgentBinding { backend_id, session_id, account_id: Option<String> }`. `Link::event` and `Link::read` carry `"accountId"`.
  - SHEET DEVIATION: there is no `Link::line`. The line is written by `Link::event` (link.rs:126) and read by `Link::read`, which dispatches to `read_link` (:191) and `read_marker` (:225).
  - Not in this task: the `settings.update` refusal (Task 8) and docs/development.md (Task 13).
  - Left for Task 5:
    - the `None` passed at session.rs:1059, dm.rs:260, chapters.rs:702/711/798, job_thread.rs:243 and sides/ending.rs:140 becomes the session's account, or the previous chapter marker's;
    - `thread/store.rs` `dm()` matches the account;
    - the guard "(account, login_at) differs ⇒ no checkpoint".

- [ ] **Step 1: Move room.rs's tests.** This step changes no behaviour.
  - Move `mod tests` (:695-1299) to `room/tests.rs` unchanged.
  - Append `cookie_import_tests`' `import()` helper and its 2 tests to it, with `use hotline_contract::{CookieImport, CookieSite};`. `scratch` drops `pub(super)`, because its only outside user moved in.
  - Run `cargo test -p hotline-room --lib room::tests`. Expected: 24 passed.
  - `git add crates/hotline-room && git commit -m "room.rs's tests live in room/tests.rs, past the in-file budget, and every one still runs."`
- [ ] **Step 2: Write the failing tests (agentAccounts setting).**
  - In `agent_accounts.rs`, add `a_bad_account_entry_costs_only_itself`. Helpers: `fn id(n: u8) -> String { format!("00000000-0000-4000-a000-{n:012}") }` and `fn names(accounts: &[AgentAccount]) -> Vec<(String, String)>`, which returns (id, name). The input array is:
    1. `id(1)` on `claude-acp`, named `" Account 1 "`
    2. `"not-a-uuid"`
    3. `id(2).to_uppercase()`
    4. `id(1)` again, named `"Other"`
    5. `id(3)` with `backendId` `""`
    6. `id(4)` named `"   "`
    7. `id(5)` named with 65 `x`
    8. `id(6)` named `"ACCOUNT 1"` on `claude-acp`
    9. `id(7)` named `"Account 1"` on `codex-acp`
    10. `id(8)` named with 64 `x`
    11. `42`
```rust
assert_eq!(names(&normalize_accounts(&input)),
  [(id(1), "Account 1".into()), (id(7), "Account 1".into()), (id(8), "x".repeat(64))]);
assert!(normalize_accounts(&Value::Null).is_empty());
```
  - In `room/tests.rs`, add `agent_accounts_default_to_empty_and_read_back_what_was_written`:
    - `agent_accounts(&log)` is empty, and `settings(&log)[AGENT_ACCOUNTS] == json!([])`.
    - `write_agent_accounts(&log, &[one, two])` grows `log.load(&StreamId::Room)` by exactly one event, of kind `setting`, and the reader returns `[one, two]`.
    - After `append(&log, &setting(AGENT_ACCOUNTS, json!([{"id":"bad"}, one])))`, both `agent_accounts(&log)` and `settings(&log)[AGENT_ACCOUNTS]` give only `one`.
- [ ] **Step 3: Run them to verify they fail.** Run `cargo test -p hotline-room --lib agent_accounts`. Expected: E0425, cannot find `normalize_accounts`, `agent_accounts`, `write_agent_accounts`.
- [ ] **Step 4: Implement.**
  - The filter takes `as_array()`. An element is kept when all of these hold:
    - it passes `serde_json::from_value::<AgentAccount>`;
    - its id is the canonical uuid;
    - `backend_id` is not empty;
    - `name.trim().chars().count()` is in 1..=64.
  - The first id wins, then the first `(backend_id, name.trim().to_lowercase())`. A kept entry keeps its trimmed name.
  - `defaults()` inserts `AGENT_ACCOUNTS: []`. `settings_from_events` re-normalises the key after the `mcpServers` block (:336-341).
  - Add the writer to room.rs: `log.append(&StreamId::Room, &room_event("setting", json!({"id": AGENT_ACCOUNTS, "value": accounts})))`.
  - Run Step 3's command. Expected: PASS.
  - `git add crates/hotline-room && git commit -m "The agentAccounts setting is read entry by entry, so a bad entry costs only itself."`
- [ ] **Step 5: Write the failing tests (checkpoints).** In `room/tests.rs`, `checkpoints_of(log, id) -> Vec<(String, Option<String>, String)>` reads `roster(log)`.
```rust
#[test] fn a_checkpoint_is_kept_per_backend_and_account()  // ada claude-acp: None "s0", Some("acc-1") "s1", then "s2" ⇒
  // [("claude-acp",None,"s0"),("claude-acp",Some("acc-1"),"s2")]; clear_checkpoint(..,Some("acc-1")) ⇒ only s0
#[test] fn a_legacy_checkpoint_is_the_system_default_one()  // persona_event with sessionCheckpoints [{backendId,sessionId:"old"}];
  // a checkpoint on Some("acc-1") leaves "old"; clear_checkpoint(..,None) removes only "old";
  // no Room event serializes "accountId":null
#[test] fn clearing_an_account_reaches_every_teammate_and_clearing_a_backend_every_account()
  // ada {None, acc-1, acc-2 on claude-acp; None on cursor}, bob {acc-1}: clear_account_checkpoints("acc-1") ⇒ ada keeps
  // None/acc-2/cursor, bob none; clear_account_checkpoints("acc-9") appends no event;
  // clear_backend_checkpoints("ada","claude-acp") ⇒ cursor only
#[test] fn checkpoint_writes_on_one_teammate_never_lose_each_other()
  // 16 std threads behind a Barrier, each checkpoint_session(&log.clone(),"ada",&format!("b{i}"),None,"s") ⇒ 16 checkpoints
```
- [ ] **Step 6: Run them to verify they fail.** Run `cargo test -p hotline-room --lib room::tests::a_checkpoint_is_kept_per_backend_and_account`. Expected: E0061 (wrong argument count) and E0425 (`clear_account_checkpoints` not found).
- [ ] **Step 7: Implement.**
  - Add `static PERSONA_WRITES: Mutex<()>`. `persona_write_lock()` takes it with `unwrap_or_else(PoisonError::into_inner)`.
  - Add a private `fn same(c: &SessionCheckpoint, backend: &str, account: Option<&str>) -> bool`. It compares `backend_id` and `account_id.as_deref()`.
  - `with_checkpoints` holds the lock from the roster read to `append_persona`.
  - `clear_account_checkpoints` takes the lock once and reads the roster once. It appends one record, with `updated_at` bumped, per teammate whose checkpoints changed, and writes nothing when none changed.
  - The five callers pass `None`. Behaviour is unchanged, because only `None` checkpoints exist yet.
  - Run `cargo test -p hotline-room --lib room::tests` (expected: PASS) and `cargo check -p hotline-core --all-targets --locked`.
  - `git add crates/hotline-room crates/hotline-core/src/session.rs crates/hotline-core/src/session && git commit -m "A checkpoint belongs to a backend and an account, and its read-modify-write holds the persona write lock so a concurrent one is never lost."`
- [ ] **Step 8: Write the failing test (link).** In link.rs, add `a_binding_keeps_the_account_its_session_ran_on`:
  - `Link { binding: Some(AgentBinding { account_id: Some("acc-1".into()), ..}), ..closed_side() }` round-trips through `event()` and `Link::read`.
  - `event()["accountId"] == "acc-1"`, and `Link::wire(event())["accountId"] == "acc-1"` (the Side marker).
  - `closed_side().event().get("accountId").is_none()`.
  - A legacy `link` line and a `side` marker without `accountId` each read `account_id == None`. A marker with `"accountId":"acc-1"` reads `Some`.
- [ ] **Step 9: Run it to verify it fails.** Run `cargo test -p hotline-store --lib link::tests::a_binding_keeps_the_account_its_session_ran_on`. Expected: E0560, struct `AgentBinding` has no field named `account_id`.
- [ ] **Step 10: Implement.**
  - Add `pub account_id: Option<String>` to `AgentBinding`. Its doc says None is System default, as is every line written before accounts.
  - `event()`: add `put("accountId", …)` after `backendId`.
  - `read_link`: read `text("accountId")`.
  - `read_marker`: bind `account_id`.
  - `marker()`: set it from `binding.account_id`.
  - The fixture at :400 gets `account_id: None`.
  - Literals:
    - `thread/store.rs:113`: `account_id: checkpoint.account_id.clone()`.
    - `job_thread.rs:243` and `sides/ending.rs:140`: `None`.
    - The test literals in agent.rs and threads.rs: `None`.
- [ ] **Step 11: Run tests to verify they pass.**
  - `cargo test -p hotline-store --lib link::tests` (expected: 5 passed)
  - `cargo test -p hotline-room --locked`
  - `cargo test -p hotline-core --lib`
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - Expected: PASS.
- [ ] **Step R (Review Focus 4): Unicode names count characters**

Add to `crates/hotline-room/src/room/tests.rs` (or the file Task 2's tests live in):
```rust
#[test] fn an_account_name_is_measured_in_characters_not_bytes() {
  // "ç".repeat(64) and "🙂".repeat(64) are kept; "ç".repeat(65) is dropped; "Conta Teste ção" and "conta teste ÇÃO" on the
  // same backend keep only the first (case-insensitive duplicate). Assert via normalize_accounts(&json!([...])).len() and names.
}
```
Run: `cargo test -p hotline-room an_account_name_is_measured_in_characters_not_bytes` — Expected: PASS once `chars().count()` and `to_lowercase()` are used.

- [ ] **Step 12: Commit.** `git add crates/hotline-store/src/link.rs crates/hotline-core/src/thread/store.rs crates/hotline-core/src/session`, then `git commit -m "A thread's binding remembers the account its session ran on, and a line without one is System default."`

---

### Task 3: hotline-support: account folder paths, the checked account folder, private directories, the login job

**Files:**
- Create: `crates/hotline-support/src/private_dir.rs` (in-file `mod tests`, under 400 lines)
- Modify: `crates/hotline-support/Cargo.toml:3` (description gains "private directories") and `:23` (add `dunce = "1.0.5"` after `uuid`; core and tools already lock 1.0.5); `Cargo.lock` (the new edge)
- Modify: `crates/hotline-support/src/lib.rs:1-5` (doc gains "the private directory a credential lives in") and `:16` (`pub mod private_dir;` after `pub mod paths;`)
- Modify: `crates/hotline-support/src/paths.rs:149` (new fns after `acp_agents_dir`) and its `mod tests` (:470-554)
- Modify: `crates/hotline-support/src/process_windows.rs:1-2,14-17,25-28,50-92` and its `mod tests` (:95-171)
- Modify: `crates/hotline-core/src/lib.rs:16` → `use hotline_support::{fence, images, private_dir, thumbs};`
- Modify: `crates/hotline-core/src/vault.rs:16-21` (imports), `:1106-1138` (delete both `make_private_directory` and `open_directory`), `:1678-1696` (the symlink test moves out)

**Interfaces:**
- Consumes: `credentials::check_private_path(path: &Path) -> io::Result<()>` (credentials.rs:440-459; PermissionDenied for a symlink or reparse point); `credentials::windows::private_directory(path: &Path) -> io::Result<()>` (credentials/windows.rs:38); `Job`, `owned` and the test helper `members` (process_windows.rs:30-48, :108-130)
- Produces:
  - `paths::agent_accounts_dir(root: &Path) -> PathBuf`; `paths::agent_account_dir(root: &Path, id: &str) -> Option<PathBuf>`
  - `private_dir::make_private_directory(path: &Path) -> io::Result<()>`; `#[cfg(unix)] private_dir::open_directory(path: &Path) -> io::Result<fs::File>`
  - `private_dir::checked_account_dir(root: &Path, id: &str) -> io::Result<PathBuf>`: the only way Tasks 4-8 reach an account folder; no caller builds `expected` itself
  - `process_windows::prepare_visible(command: &mut tokio::process::Command)`
  - `process_windows::Job::attach_without_kill_on_close(id: Option<u32>) -> io::Result<Job>` (an `Option` like `attach`, so a caller passes `child.id()`); `Job::terminate(&self)` unchanged
  - in core: `crate::private_dir::{make_private_directory, checked_account_dir}`
- SHEET DEVIATION: `agent_account_dir` takes only the lowercase hyphenated uuid (`parse_str` also takes urn:, braced, simple and uppercase: a `:` in a Windows path component, two spellings of one folder on a case-blind disk); `real_directory` is private and compares canonical `path` with an `expected` it does not canonicalize, so a junction at `agent-accounts` is refused (§8.2); `open_directory` is exported for the vault's `sync_directory`.

- [ ] **Step 1: Write the failing tests.** Also add the `dunce` line to Cargo.toml, `pub mod private_dir;` to lib.rs, and `private_dir.rs` holding only its `#[cfg(test)] mod tests`.
```rust
// paths.rs
#[test]
fn an_account_folder_is_named_only_by_the_one_spelling_of_a_uuid() {
    let (root, id) = (Path::new("/data"), "89bbac55-e7ac-4409-a709-115f33bb12f0");
    assert_eq!(agent_accounts_dir(root), root.join("agent-accounts"));
    assert_eq!(agent_account_dir(root, id), Some(root.join("agent-accounts").join(id)));
    for bad in ["", "..", "a/b", "not-a-uuid", "89BBAC55-E7AC-4409-A709-115F33BB12F0",
        "89bbac55e7ac4409a709115f33bb12f0", "{89bbac55-e7ac-4409-a709-115f33bb12f0}",
        "urn:uuid:89bbac55-e7ac-4409-a709-115f33bb12f0"] {
        assert_eq!(agent_account_dir(root, bad), None, "{bad}");
    }
}
// private_dir.rs. tmp = tempfile::tempdir(); root = tmp/"Conta Teste ção"; dir = paths::agent_account_dir(&root, ID).unwrap();
// canonical = paths::agent_account_dir(&dunce::canonicalize(&root).unwrap(), ID).unwrap();
// link(target, at): unix symlink, Windows `cmd.exe /d /c mklink /J` (recipe at credentials/windows.rs:270-278)
#[test]
fn a_real_account_folder_answers_its_canonical_path() {
    make_private_directory(&dir).unwrap();
    assert_eq!(checked_account_dir(&root, ID).unwrap(), canonical);
    assert!(!canonical.to_string_lossy().starts_with(r"\\?\"));
}
#[test]
fn a_root_reached_through_a_link_still_finds_its_folder() { // macOS /var → /private/var; a Windows 8.3 %TEMP%
    make_private_directory(&dir).unwrap();
    link(&root, &tmp.path().join("via"));
    assert_eq!(checked_account_dir(&tmp.path().join("via"), ID).unwrap(), canonical);
}
#[test]
fn a_folder_that_is_missing_a_file_a_link_or_elsewhere_is_refused_by_kind() {
    let kind = |root: &Path, id: &str| checked_account_dir(root, id).unwrap_err().kind();
    assert_eq!(kind(&root, "not-a-uuid"), io::ErrorKind::InvalidInput);
    assert_eq!(kind(&root, ID), io::ErrorKind::NotFound);
    // each on a fresh root, one assert per case: a file at `dir` → NotADirectory; link(elsewhere, &dir) → PermissionDenied;
    // link(elsewhere, &agent_accounts_dir(&root)) with elsewhere/ID a real private directory → PermissionDenied
}
#[cfg(unix)] #[test] fn setting_the_directory_private_does_not_follow_a_symlink() // moved verbatim from vault.rs:1678-1696; scratch() → tempfile::tempdir()
#[cfg(unix)] #[test] fn a_private_directory_is_0700_and_makes_its_parents() // tmp/a/b → mode 0o700; a 0o755 dir is set back to 0o700
```
- [ ] **Step 2: Run it to verify it fails.** `cargo test -p hotline-support --lib` (the first time without `--locked`: the dunce edge rewrites Cargo.lock). Expected: FAIL, E0425 for `agent_accounts_dir`, `agent_account_dir`, `make_private_directory` and `checked_account_dir`. hotline-support does not pull in hotline-voice, so no speech engine is needed.
- [ ] **Step 3: Implement**
  - `paths.rs`: `agent_accounts_dir` = `root.join("agent-accounts")`. `agent_account_dir` = `Uuid::parse_str(id).is_ok_and(|uuid| uuid.hyphenated().to_string() == id).then(|| agent_accounts_dir(root).join(id))`; its doc says why there is one spelling.
  - `private_dir.rs`:
    - `make_private_directory`: Unix = vault.rs:1109-1122 plus `open_directory` (vault.rs:1124-1132, now `pub`); Windows = vault.rs:1134-1138, calling `crate::credentials::windows::private_directory`. The doc moves with it, worded for "a credential directory".
    - private `fn real_directory(path: &Path, expected: &Path) -> io::Result<PathBuf>`, in order: `credentials::check_private_path(path)?`; `fs::symlink_metadata(path)?` not a directory → `NotADirectory`; `let canonical = dunce::canonicalize(path)?`; `canonical != expected` → `PermissionDenied`, "The folder no longer resolves to where Hotline made it."; `Ok(canonical)`.
    - `checked_account_dir`: `paths::agent_account_dir(root, id)` is `None` → `InvalidInput`, "Not an account id."; `expected = paths::agent_account_dir(&dunce::canonicalize(root)?, id)`; `real_directory(&dir, &expected)`. Its doc: run before every spawn, status, login and setup write (spec §8.2).
  - `vault.rs`: add `use crate::private_dir::make_private_directory;` and `#[cfg(unix)] use crate::private_dir::open_directory;`; delete :1106-1138 and the test at :1678-1696. `launch.rs`, `shared.rs` and `web_search.rs` keep the name through `use super::*`; the test call at :1312 stays.
- [ ] **Step 4: Run tests to verify they pass.** `cargo test -p hotline-support --lib --locked`: PASS. Then `python scripts/fetch-speech-engine` once (a cold tree needs sherpa-onnx before any core build) and `cargo test -p hotline-core --lib --locked vault`: PASS.
- [ ] **Step 5: Commit**
  - `git add crates/hotline-support/Cargo.toml crates/hotline-support/src/lib.rs crates/hotline-support/src/paths.rs crates/hotline-support/src/private_dir.rs crates/hotline-core/src/lib.rs crates/hotline-core/src/vault.rs Cargo.lock`
  - `git commit -m "An account folder has one uuid spelling, is made private like the vault, and is reached only through checked_account_dir while it is a real directory under the canonical agent-accounts."`
- [ ] **Step 6: Write the failing Windows tests** (the module is `#[cfg(windows)]`). Extract the descendant loop of `children_wait_for_the_job_and_descendants_die_when_it_closes` (:142-151) into `async fn descendant(job: &Job, child: u32) -> u32`; that test calls it.
```rust
#[tokio::test] async fn closing_a_login_job_leaves_its_tree_running() {
    // the existing cmd.exe "ping -n 60" child under prepare(), attached by Job::attach_without_kill_on_close(child.id());
    // handle = owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, descendant(&job, id).await))
    drop(job);
    assert_eq!(unsafe { WaitForSingleObject(handle.as_raw_handle(), 1000) }, WAIT_TIMEOUT); // a floor: still alive
    // clean up only what this test started: TerminateProcess(handle, 1), then child.kill()
}
#[tokio::test] async fn terminating_a_login_job_ends_its_whole_tree() { // same setup
    job.terminate().unwrap();
    assert_eq!(unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) }, WAIT_OBJECT_0);
}
#[tokio::test]
#[ignore = "opens a console window"]
async fn a_visible_child_waits_suspended_until_its_job_resumes_it() {
    let mut command = tokio::process::Command::new("cmd.exe");
    command.args(["/d", "/c", "exit", "3"]);
    prepare_visible(&mut command);
    let mut child = command.spawn().unwrap();
    assert!(child.try_wait().unwrap().is_none());
    let _job = Job::attach_without_kill_on_close(child.id()).unwrap();
    assert_eq!(timeout(Duration::from_secs(30), child.wait()).await.unwrap().unwrap().code(), Some(3));
}
```
  The third is the only proof that `CREATE_NEW_CONSOLE` combines with `CREATE_SUSPENDED` (`CREATE_NO_WINDOW` is the flag it excludes).
- [ ] **Step 7: Run it to verify it fails.** `cargo test -p hotline-support --lib --locked process_windows`. Expected: FAIL, E0599 (`attach_without_kill_on_close`) and E0425 (`prepare_visible`).
- [ ] **Step 8: Implement**
  - Import `CREATE_NEW_CONSOLE` (:14-17).
  - `prepare_visible`: `creation_flags(CREATE_SUSPENDED | CREATE_NEW_CONSOLE)` and `kill_on_drop(false)`. Doc: `CREATE_NO_WINDOW` excludes a console; cancel and timeout end the tree through `Job::terminate`.
  - The body of `attach` (:50-92) moves into private `fn attach_with(id: Option<u32>, kill_on_close: bool) -> io::Result<Job>`, which skips `SetInformationJobObject` when `kill_on_close` is false. `attach` and `attach_without_kill_on_close` wrap it; the latter's doc says a login's browser can be a descendant, so a normal exit only closes the handle.
  - Module doc (:1-2): every child waits for its job; only the default job kills on close.
  - The release-build console check of spec §17 belongs to Task 6.
- [ ] **Step 9: Run tests to verify they pass.** `cargo test -p hotline-support --lib --locked process_windows -- --include-ignored`: PASS (4 tests; one console window opens briefly). Then `cargo clippy -p hotline-support -p hotline-core --all-targets --locked -- -D warnings`: clean.
- [ ] **Step 10: Commit**
  - `git add crates/hotline-support/src/process_windows.rs`
  - `git commit -m "A login's Windows job holds its tree without killing it on close, so a browser the login opened outlives it; only cancel and timeout terminate it."`

---

### Task 4: hotline-core accounts module: support row, env plan, resolve, status parse

**Files:**
- Create: `crates/hotline-core/src/accounts.rs`: the module doc (everything generic about an agent account, parameterized by its agent's row) and `pub(crate) mod env; pub(crate) mod resolve; pub(crate) mod status; pub(crate) mod support;`
- Create: `crates/hotline-core/src/accounts/support.rs` (in-file tests: smaller than the row)
- Create: `accounts/env.rs` + `accounts/env/tests.rs`, `accounts/resolve.rs` + `accounts/resolve/tests.rs`, `accounts/status.rs` + `accounts/status/tests.rs` (each one's tests outgrow its code, docs/standards.md:102-108; Task 6 adds `AccountStatuses` to this `status.rs` and its tests to `status/tests.rs`)
- Modify: `crates/hotline-core/src/lib.rs:9`, before `pub mod boot;`:
  `#[cfg_attr(not(test), expect(dead_code, reason = "the driver, status, login, setup and switch tasks call into accounts; Task 9 removes this"))] pub(crate) mod accounts;`

**Interfaces:**
- Consumes:
  - Task 1: `contract::{AgentAccount, AccountStatusView, AccountStatusState}` (`Clone, Debug, PartialEq`, like every type in contract/persona.rs) and `Persona.agent_account_id: Option<String>`
  - Task 2: `room::agent_accounts(log: &Log) -> Vec<AgentAccount>`; `room::write_agent_accounts(log: &Log, accounts: &[AgentAccount]) -> io::Result<()>` (tests)
  - Task 3: `paths::agent_account_dir`; `private_dir::{make_private_directory, checked_account_dir}`
  - Existing: `crate::session::tests::persona(id) -> Persona` (session/tests.rs:489); `Log::open` (hotline-store/src/log.rs:132); `Launch.env: Vec<(String, String)>` (driver/acp/registry.rs:47-51); `portable_pty::CommandBuilder::{env, env_remove, get_env}` (0.9)
- Produces (sheet names):
  - `support::{AccountSupport, SetupCategory, CLAUDE, account_support}` (fields `pub(crate)`, `#[derive(Debug)]`)
  - `env::{EnvPlan, env_plan}`; `EnvPlan::apply(&self, cmd: &mut tokio::process::Command)`; `#[cfg(unix)] EnvPlan::apply_pty(&self, cmd: &mut portable_pty::CommandBuilder)`
  - `resolve::ResolvedAccount { pub id: String, pub backend_id: String, pub name: String, pub dir: PathBuf, pub support: &'static AccountSupport, pub login_at: Option<i64> }` (`Clone, Debug`; no `PartialEq`, `support` holds a fn pointer)
  - `resolve::AccountRefusal` (the sheet's two variants; `Clone, Debug, PartialEq, Eq`) with `private_sentence` and `public_sentence(&self, teammate: &str, agent: &str) -> String`
  - `resolve::resolve(root: &Path, log: &Log, persona: &Persona) -> Result<Option<ResolvedAccount>, AccountRefusal>`
  - `resolve::account(root: &Path, log: &Log, account_id: &str) -> Result<ResolvedAccount, AccountRefusal>` (for the status, login and setup commands, Tasks 6-8)
  - `status::parse_claude_status(stdout: &[u8], dir: &Path) -> AccountStatusView`, with `checked_at: None` (Task 6's cache stamps it)
- SHEET DEVIATION: `EnvPlan::apply_pty` is `#[cfg(unix)]`; its only caller, Task 6's terminal sign-in, exists only on Unix.

- [ ] **Step 1: Write the failing tests.** Also add the lib.rs line, `accounts.rs`, and each child file holding only `#[cfg(test)] mod tests;` (support.rs: its in-file tests).
```rust
// support.rs
#[test] fn only_claude_code_has_a_row_and_it_carries_the_spec_values() {
    for none in ["codex-acp", "hotline", ""] { assert!(account_support(none).is_none()) }
    let row = account_support("claude-acp").expect("the one row");
    assert_eq!((row.env_var, row.email_flag), ("CLAUDE_CONFIG_DIR", Some("--email")));
    assert_eq!(row.login_args, ["--cli", "auth", "login", "--claudeai"]);
    assert_eq!(row.status_args, ["--cli", "auth", "status", "--json"]);
    assert_eq!(row.scrub.len(), 18);
    assert!(row.scrub.contains(&row.env_var) && row.scrub.contains(&"ANTHROPIC_CUSTOM_HEADERS"));
    assert_eq!(row.usage_limit_prefixes.len(), 12);
    assert!(row.usage_limit_prefixes.contains(&"Your org is out of usage \u{b7} add funds to continue"));
    assert_eq!(row.setup.iter().map(|c| c.id).collect::<Vec<_>>(), ["instructions", "skills", "plugins", "mcp", "settings"]);
    assert_eq!(row.setup.iter().map(|c| c.open).collect::<Vec<_>>(), ["CLAUDE.md", "skills/", "plugins/", ".claude.json", "settings.json"]);
    assert_eq!(row.setup[0].paths, ["CLAUDE.md", "rules"]);
    assert_eq!(row.setup[2].json_keys, [("settings.json", &["enabledPlugins", "extraKnownMarketplaces"][..])]);
    assert_eq!(row.setup[3].json_keys, [(".claude.json", &["mcpServers"][..])]);
    assert_eq!(row.settings_auth_keys, ["apiKeyHelper", "awsAuthRefresh", "awsCredentialExport"]);
    assert_eq!((row.parse_status)(br#"{"loggedIn":false}"#, Path::new(".")).state, AccountStatusState::SignedOut);
}
// env/tests.rs. resolved(dir) = ResolvedAccount with support = account_support("claude-acp"); names come from support.scrub and
// support.env_var, never literals; dir = canonical tempdir/"Conta Teste ção"/<uuid>
#[test] fn system_default_removes_nothing_and_keeps_the_legacy_order()        // env_plan(&[A=1, scrub[0]=cat], &[B=2], None) == EnvPlan { remove: vec![], set: [A=1, scrub[0]=cat, B=2] }
#[test] fn an_account_scrubs_every_listed_name_and_sets_its_folder_last()     // remove == support.scrub as OsString; set.last() == (env_var, dir)
#[test] fn a_catalogue_or_method_that_reintroduces_a_scrubbed_name_loses()    // cat [scrub[0]=cat, lowercased scrub[6]=x, KEEP=1], method [env_var=elsewhere, M=2] → set == [KEEP=1, M=2, env_var=dir]
#[test] fn the_command_applier_removes_then_sets() // BTreeMap of Command::new("agent").as_std().get_envs() after apply:
    // scrub[0] → None, env_var → Some(dir), KEEP → Some("1"); "last" is asserted on plan.set, because get_envs is sorted
#[cfg(unix)] #[test] fn the_pty_applier_removes_then_sets() // b.env(scrub[0], "inherited"); apply_pty → b.get_env(scrub[0]) == None, b.get_env(env_var) == Some(dir)
// resolve/tests.rs. root = tempdir/"Conta Teste ção"; log = Log::open(&root); write_agent_accounts(&log, &[AgentAccount { id: ID,
// backend_id: "claude-acp", name: "Account 1", login_at: Some(1_700_000_100_000), .. }, AgentAccount { id: OTHER, backend_id: "codex-acp", name: "Account 2", .. }]);
// ada(account, backend) = session::tests::persona("ada") with those two fields
#[test] fn no_account_or_an_agent_without_accounts_is_system_default() // ada(None, "claude-acp") and ada(Some(ID), "codex-acp") → matches!(_, Ok(None))
#[test] fn an_account_resolves_to_its_canonical_folder() // after make_private_directory(agent_account_dir(&root, ID)): resolve(ada(Some(ID), "claude-acp"))
    // and account(&root, &log, ID) both give id ID, backend_id "claude-acp", name "Account 1", login_at Some(1_700_000_100_000),
    // support.env_var "CLAUDE_CONFIG_DIR", dir == checked_account_dir(&root, ID).unwrap()
#[test] fn an_account_this_desk_does_not_have_is_refused_in_both_sentences()
    // UnknownAccount { account_id } from: resolve and account with an unknown uuid; resolve(ada(Some(OTHER), "claude-acp")); account(.., OTHER)
    // private_sentence("Ada", "Claude Code") == "Pick a Claude Code account for Ada: the one it was set to is not on this desk."
    // public_sentence("Ada", "Claude Code") == "Ada can't start: its Claude Code account needs attention at the desk."
#[test] fn a_missing_or_replaced_folder_is_refused_with_its_kind()
    // no folder → FolderUnavailable { account_id: ID, name: "Account 1", path: agent_account_dir(&root, ID), kind: NotFound }, from resolve and account
    // private == format!("Can't open Account 1's folder ({}): entity not found. Fix the folder or pick another account.", path.display())
    // the public sentence contains neither the path nor "Account 1"
    // a junction or symlink in its place (mklink /J recipe at credentials/windows.rs:270-278) → kind PermissionDenied
// status/tests.rs. said(json!({...})) = .to_string().into_bytes(), so Windows backslashes stay escaped; dir is the canonical "Conta Teste ção"
#[test] fn signed_in_needs_a_first_party_claude_ai_login_in_this_folder()
    // {loggedIn:true, authMethod:"claude.ai", apiProvider:"firstParty", email:"ada@example.com", orgId, orgName, subscriptionType:"max",
    //  analyticsDisabled:false, configDirectory:dir}
    //   → AccountStatusView { state: SignedIn, checked_at: None, email: Some("ada@example.com"), plan: Some("max") }
    // still SignedIn with: configDirectory spelled dir/../"Conta Teste ção", apiKeySource: null, or a trailing "\n"
#[test] fn logged_out_is_signed_out_whatever_else_it_says() // {loggedIn:false, authMethod:"none", apiProvider:"firstParty", configDirectory:dir} → SignedOut, email and plan None
#[test] fn any_other_answer_is_unavailable_and_says_nothing_more() // each → AccountStatusView { state: Unavailable, checked_at: None, email: None, plan: None }:
    // api_key + apiKeySource:"ANTHROPIC_API_KEY"; oauth_token; third_party + apiProvider:"bedrock";
    // claude.ai + apiKeySource:"apiKeyHelper" (the account's own settings.json replacing the login, spec §7.6);
    // claude.ai in another existing folder; no configDirectory; loggedIn:"true"; b""; b"not json"
```
- [ ] **Step 2: Run it to verify it fails.** `python scripts/fetch-speech-engine` first if Task 3 did not run it, then `cargo test -p hotline-core --lib --locked accounts::`. Expected: FAIL, E0425/E0433/E0422 for `account_support`, `env_plan`, `EnvPlan`, `resolve`, `account` and `parse_claude_status`.
- [ ] **Step 3: Implement**
  - **`support.rs`**: the two structs as in spec §9.1, with `parse_status: fn(&[u8], &Path) -> AccountStatusView`; `account_support` = `match backend_id { "claude-acp" => Some(&CLAUDE), _ => None }`.
    - `CLAUDE.scrub`, in spec §9.2 order: `ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_CUSTOM_HEADERS, CLAUDE_CODE_OAUTH_TOKEN, CLAUDE_CODE_OAUTH_REFRESH_TOKEN, CLAUDE_CODE_OAUTH_SCOPES, ANTHROPIC_BASE_URL, CLAUDE_CODE_USE_BEDROCK, CLAUDE_CODE_USE_VERTEX, CLAUDE_CODE_USE_FOUNDRY, ANTHROPIC_PROFILE, ANTHROPIC_FEDERATION_RULE_ID, ANTHROPIC_ORGANIZATION_ID, CLAUDE_SECURESTORAGE_CONFIG_DIR, CLAUDE_CODE_SIMPLE, CLAUDECODE, CLAUDE_CODE_ENTRYPOINT, CLAUDE_CONFIG_DIR`.
    - `usage_limit_prefixes`, the SDK's list in its order; a comment cites `USAGE_LIMIT_ERROR_PREFIXES` of `@anthropic-ai/claude-agent-sdk` 0.3.270 as bundled by `@agentclientprotocol/claude-agent-acp` 0.78.0 (driver/acp/registry.rs:192):
      `"You've hit your", "You've reached your", "You're out of usage credits", "Your org is out of usage \u{b7} add funds to continue", "Your org is out of usage \u{b7} contact your admin", "Your seat type doesn't include usage credits", "Your seat type doesn't include usage", "Your usage allocation has been disabled by your admin", "Your group's usage limit is set to $0", "Fable 5 requires usage credits", "You're out of extra usage", "Your seat type doesn't include extra usage"`.
    - `setup` (spec §8.6 and §7.6; `json_keys` empty unless listed): `instructions` paths `["CLAUDE.md","rules"]`, open `"CLAUDE.md"`; `skills` paths `["skills","agents","commands","output-styles"]`, open `"skills/"`; `plugins` paths `["plugins"]`, json_keys `[("settings.json", ["enabledPlugins","extraKnownMarketplaces"])]`, open `"plugins/"`; `mcp` paths `[]`, json_keys `[(".claude.json", ["mcpServers"])]`, open `".claude.json"`; `settings` paths `["settings.json"]`, open `"settings.json"`. A trailing `/` marks a folder for Task 7's `open_target` and Task 11's Open.
    - `settings_auth_keys` = `["apiKeyHelper","awsAuthRefresh","awsCredentialExport"]`; Task 7's copy strips the `env.<name>` keys from `scrub` itself.
  - **`env.rs`**: `#[derive(Debug, Default, PartialEq, Eq)] EnvPlan`. No account: `remove` empty, `set` = catalogue then method, in order, as `OsString`. Account: `remove` = `scrub`; `set` = catalogue then method minus every name equal, ASCII case-insensitively (Windows env names ignore case), to a scrub name or `env_var`, then `(env_var, dir)` last. Both appliers run every `env_remove` before any `env`.
  - **`resolve.rs`**:
    - `account(root, log, id)`: no row with that id in `room::agent_accounts(log)` → `UnknownAccount`; `account_support(&row.backend_id)` is `None` → `UnknownAccount`; `paths::agent_account_dir(root, id)` is `None` (an id Task 2's reader admits in another spelling) → `UnknownAccount`; `private_dir::checked_account_dir(root, id)` errs → `FolderUnavailable { account_id, name: row.name, path: agent_account_dir(root, id), kind: error.kind() }`; else `ResolvedAccount` with `dir` = the checked path.
    - `resolve(root, log, persona)`: `agent_account_id` is `None` → `Ok(None)`; `account_support(&persona.backend_id)` is `None` → `Ok(None)` (kept but ignored, spec decision 14); `account(..)?`; its `backend_id != persona.backend_id` → `UnknownAccount`; else `Ok(Some(..))`.
    - Sentences: private = the sheet's two, `{path}` as `path.display()` and `{kind}` as the `io::ErrorKind` `Display`; public, for both kinds = "{teammate} can't start: its {agent} account needs attention at the desk."
  - **`status.rs`**: `serde_json::from_slice::<Value>(stdout.trim_ascii())`. `SignedIn` needs all of `loggedIn == true`, `apiProvider == "firstParty"`, `authMethod == "claude.ai"`, `apiKeySource` absent or null, and `dunce::canonicalize(configDirectory) == dunce::canonicalize(dir)`; it fills `email` from `email` and `plan` from `subscriptionType`. `loggedIn == false` → `SignedOut`. Anything else → `Unavailable` with no email or plan. `"claude.ai"` (spec §10.2) stays strict until the real-account check of spec §14 confirms it.
- [ ] **Step 4: Run tests to verify they pass.** `cargo test -p hotline-core --lib --locked accounts::`: PASS (the Unix PTY test runs on the Linux and macOS legs). Then `cargo clippy -p hotline-core --all-targets --locked -- -D warnings`: clean; the lib.rs `expect` is fulfilled in the non-test build, and the tests read every field.
- [ ] **Step 5: Commit**
  - `git add crates/hotline-core/src/lib.rs crates/hotline-core/src/accounts.rs crates/hotline-core/src/accounts`
  - `git commit -m "A teammate's account resolves before any spawn to its checked folder or a refusal, its env is the launch's with inherited credentials scrubbed and the folder last, and only a first-party subscription login in that folder reads as signed in."`

---

### Task 5: Driver and session integration

**Files:** (all under `crates/hotline-core/src/`)
- Create: `driver/acp/tests/accounts.rs`, `session/tests/accounts.rs` (Task 9 appends to the second)
- Modify driver: `driver/failure.rs:24-60` (Kind), `:72-86` (Failure), `:244-310` (four literals), `:414-423` (name_the_harness), `:425-503` (set_kind), mod tests; `driver/acp.rs:325-404` (field, builder), `:556-583` (start), `:756-772` (prompt failure); `driver/acp/session.rs:17-18` (spawn), `:183-191` (handshake), `:242-246` (open_session); `driver/acp/auth.rs:47-67` (failure), `:78-91` (start), not the Terminal arm at `:193` (Task 6); `driver/acp/tests.rs:1-6` (`mod accounts;`, `resolved()`); `driver/acp/auth/tests.rs:208,212,228`
- Modify session: `accounts.rs` (`account_of`); `session.rs:262-323,372-419,1051-1093`; `session/start.rs:97-157,184`; `session/dm.rs:255-262`; `session/agent.rs:218-228,372-378,405-411,675-701`; `session/chapters.rs:125-240,697-713,747-752,794-800,1272`; `session/sides.rs:150-195,674-701,777`; `session/sides/ending.rs:127-146`; `session/job_thread.rs:131-139,232-247,704-710`; `thread/store.rs:107-126`; `session/tests.rs:24` (`mod accounts;`)
- Modify wire: `wire/commands.rs:391-411`; test `wire/tests/owner_handler.rs`

**Interfaces:**
- Consumes:
  - Task 1: `Persona.agent_account_id`, `SessionCheckpoint.account_id`, `TranscriptEvent::Chapter.account_id` (`accountId`), `AgentAccount`.
  - Task 2: `AgentBinding.account_id`; `room::{checkpoint_session(log, persona_id, backend_id, account_id: Option<&str>, session_id), clear_checkpoint(log, persona_id, backend_id, account_id), clear_backend_checkpoints(log, persona_id, backend_id), agent_accounts, write_agent_accounts}`. This task replaces the `None`s Task 2 passed at session.rs:1059, dm.rs:260, job_thread.rs:243 and sides/ending.rs:140.
  - Task 3: `private_dir::checked_account_dir(root: &Path, id: &str) -> io::Result<PathBuf>`.
  - Task 4: `support::{account_support, CLAUDE}`, `env::{env_plan, EnvPlan::apply}`, `resolve::{resolve, ResolvedAccount { id, backend_id, name, dir, support, login_at }, AccountRefusal::{private_sentence, public_sentence}}`.
- Produces:
  - `pub(crate) fn accounts::account_of(persona: &Persona) -> Option<&str>`: the record's `agent_account_id` when `account_support(&persona.backend_id)` is Some, else None (§8.3).
  - `pub(crate) fn ChildAgent::with_account(self, Option<ResolvedAccount>) -> Self`. The account is fixed for the driver's life.
  - `Kind::UsageLimit` ("usage_limit"), `Kind::AccountUnavailable` ("account_unavailable"); `Failure.account_id: Option<String>`, serialized `accountId`, skipped when None so legacy notices stay byte-identical; a refusal's `code` is "unknown_account" or "folder_unavailable"; `Failure::classify_acp(body: &str, phase: &'static str, usage_limit_prefixes: &[&str]) -> Failure`, `Failure::account_unavailable(refusal: &AccountRefusal, agent: &str, sentence: String) -> Failure`, `acp::auth::failure(live, text, phase, usage_limit_prefixes: &[&str], account_id: Option<&str>)`.
  - `Session { account_id: Option<String>, account_login_at: Option<i64> }`, written only by `start_now` (Task 9 reads them); `Room::checkpoint_still_ours(&Session) -> bool`; `chapters::opened(backend_id, account_id: Option<&str>, id, now)`, `reopened(backend_id, account_id, id, now, from)`, `Room::begin_chapter(persona_id, backend_id, account_id)`, `Room::stamp_chapter_session(persona_id, session_id, account_id)`.
  - Refusal routing: `Agents::agent`'s Err is the public sentence, so every string path is public (side, handoff, pair, job, run, reattach `info.error`, phone retry). The in-start re-check puts a public `account_unavailable` notice on the tape. Only the Desk/Owner `session.start` and `session.prompt` answers become the private card (Task 12 draws it).
  - SHEET DEVIATION: adds `account_of`; `Failure.summary` becomes `Cow<'static, str>` (a usage-limit summary is the agent's own text, and `classify` keeps its 25 callers); `Agents::agent` keeps `Result<Arc<dyn Driver>, String>`.

- [ ] **Step 1: Write the failing test** (failure.rs mod tests; the body is acp `Error` Display, `"{message}: {pretty data}"`)
```rust
const LIMITED: &str = "Internal error: You've hit your limit · resets 3pm (Europe/Lisbon): {\n  \"errorKind\": \"rate_limit\"\n}";
#[test] fn an_agents_usage_limit_is_its_own_kind_in_its_own_words() {
    let p = crate::accounts::support::CLAUDE.usage_limit_prefixes;
    let f = Failure::classify_acp(LIMITED, "acp_prompt", p);
    assert_eq!((f.kind, f.summary.as_ref()), (Kind::UsageLimit, "You've hit your limit · resets 3pm (Europe/Lisbon)"));
    assert!(f.notice().contains(r#""kind":"usage_limit""#)); assert_eq!(f.retry_delay(0), None);
    assert_ne!(Failure::classify_acp(LIMITED, "acp_prompt", &[]).kind, Kind::UsageLimit);
    assert_ne!(Failure::classify_acp(LIMITED, "request", p).kind, Kind::UsageLimit);
    assert_eq!(Failure::classify_acp(r#"{"code":-32000,"message":"Authentication required"}"#, "acp_prompt", p).kind, Kind::AgentAuth);
}
#[test] fn an_account_refusal_card_names_the_account_by_id_alone() {
    let r = AccountRefusal::FolderUnavailable { account_id: "a1".into(), name: "Account 1".into(), path: "C:/d/agent-accounts/a1".into(), kind: std::io::ErrorKind::PermissionDenied };
    let n = Failure::account_unavailable(&r, "Claude Code", r.public_sentence("Ada", "Claude Code")).notice();
    for part in [r#""kind":"account_unavailable""#, r#""accountId":"a1""#, r#""code":"folder_unavailable""#, "Ada can't start: its Claude Code account needs attention at the desk."] { assert!(n.contains(part), "{n}"); }
    assert!(!n.contains("Account 1") && !n.contains("agent-accounts") && !Failure::classify("x", None, "local").notice().contains("accountId"));
}
```
- [ ] **Step 2: Run it to verify it fails.** `python scripts/fetch-speech-engine` if no core build has run yet, then `cargo test -p hotline-core --lib driver::failure::tests`. Expected: FAIL to compile, no variant `UsageLimit` and no function `classify_acp`.
- [ ] **Step 3: Implement** in failure.rs:
  - The two Kinds. `set_kind` arms: `UsageLimit => ("Agent usage limit reached", "The agent stopped at its usage limit.")`, `AccountUnavailable => ("Agent account unavailable", "This teammate's agent account needs attention at the desk.")`. `summary` becomes `Cow<'static, str>` (`set_kind` assigns `Cow::Borrowed`). `name_the_harness` also titles UsageLimit `"{name} usage limit reached"`. `account_id` has `skip_serializing_if = "Option::is_none"`; the four literals set `None`.
  - `classify_acp`: when `phase.starts_with("acp")` and `fn usage_limit_text(body: &str, prefixes: &[&str]) -> Option<String>` is Some, return a UsageLimit failure (details `sanitize(body)`, summary that text) before `classify`, so its AgentAuth rewrite never applies (§10.5); otherwise `classify(body, None, phase)`. The text runs from the earliest prefix match to the end of that line, with a trailing `": {"` cut, trimmed, `redact_text`, clipped to 500 bytes on a char boundary.
  - `account_unavailable`: title `"{agent} account unavailable"`, summary and details `sentence`, phase "acp_start", `code` from the refusal's variant, `account_id` the refusal's id.
- [ ] **Step 4: Run tests to verify they pass.** Same command. Expected: PASS, including the existing `summary.contains` assertions.
- [ ] **Step 5: Commit.** `git add crates/hotline-core/src/driver/failure.rs`, then `git commit -m "A harness's usage limit is its own kind in its own words, and an account refusal names its account by id alone."`
- [ ] **Step 6: Write the failing test** (fixtures `scripted_agent(heard, true)`, `handshake`, `client_transport`, `next`; `UUID` is a canonical uuid)
```rust
// driver/acp/tests.rs
pub(super) fn resolved(id: &str, dir: PathBuf) -> ResolvedAccount { ResolvedAccount { id: id.into(), backend_id: "claude-acp".into(), name: "Account 2".into(), dir, support: &CLAUDE, login_at: Some(1) } }
// driver/acp/tests/accounts.rs
#[tokio::test(flavor = "multi_thread")] async fn a_checkpoint_opens_only_under_the_account_that_wrote_it() {
    for (runs_on, saved_on, opened) in [(Some("b"), Some("a"), "session/new"), (Some("b"), None, "session/new"),
        (None, Some("a"), "session/new"), (Some("b"), Some("b"), "session/load old-session")] {
        // fresh agent per row: ChildAgent::new(scratch(..), "cursor", ..).with_account(runs_on.map(|id| resolved(id, scratch(id))));
        // ada = persona(&scratch_cwd(), vec![SessionCheckpoint { backend_id: "cursor", session_id: "old-session", account_id: saved_on }])
        let info = driver.handshake(&ada, client_transport()).await.unwrap();
        assert_eq!(heard.opened.lock().unwrap().as_slice(), [opened]); assert_eq!(info.context_restored, opened != "session/new");
    } }
#[tokio::test] async fn a_child_whose_folder_is_not_its_accounts_comes_up_refused_in_public_words() {
    // ChildAgent::new(scratch("refused"), "claude-acp", ..).with_account(Some(resolved(UUID, scratch("elsewhere")))); no <root>/agent-accounts/<UUID>
    let info = driver.start(&persona(&scratch_cwd(), vec![])).await.unwrap();
    let n = driver.startup_failure().unwrap();
    for part in [r#""kind":"account_unavailable""#, r#""code":"folder_unavailable""#, UUID, "Ada can't start: its Claude Code account needs attention at the desk."] { assert!(n.contains(part), "{n}"); }
    assert!(!n.contains("Account 2") && !n.contains("elsewhere") && info.session_id.is_none() && lock(&driver.child).is_none());
    let mut updates = driver.prompt("go".into(), Vec::new(), Reach::Workspace).await; // restart finds the stored persona
    assert!(matches!(next(&mut updates).await, Update::Notice { text, .. } if text == n));
}
// driver/acp/auth/tests.rs (the calls at :208, :212, :228 gain `&[], None`)
#[tokio::test] async fn agent_auth_is_refused_for_a_teammate_on_an_account() { // ChildAgent "claude-acp" .with_account(Some(resolved(UUID, ..)))
    assert_eq!(driver.clone().auth_start("login", CancellationToken::new()).await.unwrap_err(), "Sign in Account 2 from its account instead."); assert!(lock(&driver.live.auth).is_none()); }
#[test] fn an_account_teammates_cards_carry_its_account_and_never_the_harness_methods() {
    // live: auth_methods [Agent(AuthMethodAgent::new("login", "Harness login"))] (offered on every OS), agent_name "Claude Code"
    let p = CLAUDE.usage_limit_prefixes;
    assert!(auth::failure(&live, "authentication required", "acp_prompt", p, None).notice().contains("signIn"));
    let n = auth::failure(&live, "authentication required", "acp_prompt", p, Some("b")).notice();
    assert!(n.contains(r#""kind":"agent_auth""#) && n.contains(r#""accountId":"b""#) && !n.contains("signIn"));
    let n = auth::failure(&live, "Internal error: You've hit your limit · resets 3pm (Europe/Lisbon)", "acp_prompt", p, Some("b")).notice();
    assert!(n.starts_with("Claude Code usage limit reached: You've hit your limit") && n.contains(r#""accountId":"b""#), "{n}");
}
```
- [ ] **Step 7: Run it to verify it fails.** `cargo test -p hotline-core --lib driver::acp::`. Expected: FAIL to compile, no method `with_account`, and `auth::failure` takes 3 arguments.
- [ ] **Step 8: Implement.**
  - acp.rs: field `account: Option<ResolvedAccount>` (None in `new`), `with_account`, `fn account_id(&self) -> Option<&str>`, `fn usage_limit_prefixes(&self) -> &'static [&'static str]` from `account_support(&self.backend_id)`, so System default teammates are classified too.
  - `start()`, in this order: `check_capability()?`; `*lock(&self.persona) = Some(persona.clone())`, moved up from :564 because `restart_after_failure` → `abandon_failed_session` reads it and :419 then returns Ok, so the next prompt replays the card instead of "The failed agent has no launch configuration"; then, with an account, `checked_account_dir(&self.root, &account.id)`. On `Err(e)`: `FolderUnavailable { account_id, name, path: account.dir, kind: e.kind() }`, `startup_failure = Failure::account_unavailable(&r, &agent, r.public_sentence(&persona.name, &agent)).notice()` (agent = `registry::name_of(&self.root, &self.backend_id)`, else the id), `failed = true`, `session.id = None`, `info.session_id = None`, return `Ok(self.live.publish_info())`. Then refresh, install and launch as today.
  - `spawn`: `.envs(launch.env…)` becomes `env_plan(&launch.env, &[], self.account.as_ref()).apply(&mut command)`. `open_session`: the checkpoint must also have `account_id.as_deref() == self.account_id()`.
  - `handshake` and `prompt`: pass `self.usage_limit_prefixes()` and `self.account_id()` to `auth::failure` (prompt captures both before `tokio::spawn`); the failed-at-start path holds for `AgentAuth && (sign_in.is_some() || account_id.is_some())`.
  - acp/auth.rs: `failure` calls `classify_acp`, stamps `account_id`, and sets `sign_in` only when `account_id` is None. `start`'s first line returns `Err(format!("Sign in {} from its account instead.", account.name))` when `agent.account` is Some (§9.3).
- [ ] **Step 9: Run tests to verify they pass.** `cargo test -p hotline-core --lib driver::`. Expected: PASS, including `a_checkpoint_reopens_the_agents_own_session_without_replaying_it` and `auth_signed_out_session_is_recoverable_and_success_never_resends`.
- [ ] **Step 10: Commit.** `git add crates/hotline-core/src/driver`, then `git commit -m "A teammate's child runs only in its account's real folder, reopens only a session that account wrote, and never signs in through the harness."`
- [ ] **Step 11: Write the failing test.** In `session/tests/accounts.rs` (fixtures `Fake`, `Scripted`, `scratch`, `enrol`, `write_tape`, `markers`, `saying`, `until`), accounts A and B on "claude-acp" are written with `write_agent_accounts` at `login_at: Some(1)`, and ada = `persona("ada")` on "claude-acp". `saved(room)` lists ada's checkpoints as `(session_id, account_id)`.
```rust
#[tokio::test] async fn a_checkpoint_is_kept_only_while_the_record_names_the_sessions_account_and_sign_in() {
    for (record, login_at, kept) in [(A, 1, true), (B, 1, false), (A, 2, false)] { // fresh room each: Fake over Scripted::turns([saying("t1","one"), saying("t2","two")]), reporting("s1", false); ada on A
        room.start("ada").await.unwrap(); // then enrol ada with agent_account_id = record, and rewrite A with login_at
        // prompt twice; until "two" is on the tape, so turn one's end (where the checkpoint is decided) was handled
        assert_eq!(saved(&room), if kept { vec![("s1".into(), Some(A.into()))] } else { vec![] }); } }
#[tokio::test] async fn a_resume_onto_another_account_opens_fresh() {
    // Scripted::new(vec![]) (no turn, so no checkpoint); ada on B with checkpoints (claude-acp, A, "s-old"), (claude-acp, B, "s-b"); tape: closed c1 {accountId: A, sessionId: "s-old"}, open c2
    room.start("ada").await.unwrap(); room.resume_chapter("ada").await.unwrap();
    assert!(saved(&room).is_empty()); let m = markers(&room, "ada").pop().unwrap();
    assert_eq!((m["accountId"].as_str(), m.get("sessionId")), (Some(B), None)); }
#[tokio::test] async fn an_unknown_account_refuses_the_start_in_public_words() { // ada on an unknown uuid; Room::new(log, Arc::new(DeskKeys)) = the real DeskAgents
    assert_eq!(room.start("ada").await.unwrap_err(), "Ada can't start: its Claude Code account needs attention at the desk."); }
// agent.rs a_session_is_reopened_only_by_the_child_that_issued_it, binding(backend, account):
assert_eq!(resumable(Resume::Binding, binding("claude", Some("a")), "claude", Some("a"), false).as_deref(), Some("s1"));
assert_eq!(resumable(Resume::Binding, binding("claude", Some("a")), "claude", Some("b"), false), None);
assert_eq!(resumable(Resume::Binding, binding("claude", None), "claude", Some("b"), false), None);
// owner_handler.rs: Desk::open_with_store, enrol ada (claude-acp, unknown uuid); for Seat::Desk and Seat::Owner, session.start answers
for part in ["Pick a Claude Code account for Ada: the one it was set to is not on this desk.", r#""kind":"account_unavailable""#, r#""code":"unknown_account""#] { assert!(error.contains(part), "{error}"); }
```
- [ ] **Step 12: Run it to verify it fails.** `cargo test -p hotline-core --lib -- session::tests::accounts session::agent::tests wire::tests::owner_handler`. Expected: FAIL to compile, E0061 on `resumable`.
- [ ] **Step 13: Implement.**

| Site | Change |
| --- | --- |
| `session.rs:262-323` | Before `acp::registry::launch` (:296), so a refusal needs no npx: `resolve(&self.root, &self.log, persona).map_err(\|r\| r.public_sentence(&persona.name, &agent))?` (agent = `registry::name_of`, else the id), then `.with_account(account)`. |
| `session.rs:372-419`, `start.rs:97-157,184` | The two fields go after `backend_id`. `start_now` sets `account_id = account_of(&agent.view)`, the view the driver was built from, so a switch Task 9 holds keeps the old account. `account_login_at` is that account's `login_at` in `room::agent_accounts`. Then `begin_chapter(.., account_id)`. |
| `session.rs:1051-1093` | After `take()`, return unless `self.checkpoint_still_ours(session)`. Pass `session.account_id` to `checkpoint_session` and `stamp_chapter_session`, which sets `"accountId"` on the open marker, or removes it for None. |
| `session/dm.rs` | `impl Room { pub(super) fn checkpoint_still_ours(&self, session: &Session) -> bool }` re-reads the record and the setting: `account_of(record) == session.account_id` and that account's `login_at == session.account_login_at` (§8.4). It lives here because session.rs is 1,445 lines. `:259-260` clears with `session.account_id`. |
| `agent.rs:218-228,372-378,405-411` | `resumable(resume, binding, backend_id, account_id: Option<&str>, in_process)` also matches `binding.account_id`. The caller passes `account_of(&view)`, and the synthetic checkpoint carries it. |
| `thread/store.rs:107-126` | `dm()` picks the checkpoint matching `(persona.backend_id, account_of(&persona))`. |
| `sides.rs:150-195,674-701,777`, `ending.rs:127-146` | `LiveSide.account_id = account_of(&persona)`, and `kept` also matches `saved.account_id`. Sides `Up` gains `account_id` from `account_of(&agent.view)`. `mark_side` writes the up agent's account, else the LiveSide's. |
| `job_thread.rs:131-139,232-247,704-710` | `Up.account_id` from `account_of(&agent.view)`, and `remember_session`'s binding carries it. |
| `chapters.rs:125-240,747-752,1272` | `opened` and `reopened` take `account_id` and write it. `reopened` keeps `sessionId` only when `from["accountId"]` equals it. `closed` keeps the open marker's. |
| `chapters.rs:697-713,794-800` | Resume (§8.4: a different account opens Fresh): a previous `sessionId` whose `accountId == account_of(&persona)` → `checkpoint_session(.., account, id)`, otherwise (non-Hotline) `clear_backend_checkpoints`. Closing a chapter also calls `clear_backend_checkpoints`, so no account keeps a session from a closed chapter. |
| `wire/commands.rs:391-411` | `fn refused_at_the_desk(log: &Log, persona_id: &str, error: String) -> String`: when `resolve(log.root(), log, &persona)` refuses and `error` contains its public sentence, return `Failure::account_unavailable(&r, &agent, r.private_sentence(..)).notice()`; else `error`. `SessionStart` and `SessionPrompt` map their Err through it. Both are Desk/Owner only (`seat.rs:44-49`). |

- [ ] **Step 14: Run tests to verify they pass.** `cargo test -p hotline-core --lib -- session:: thread:: wire::tests`. Expected: PASS, including `resume_points_the_checkpoint_back_at_the_previous_chapters_session`, where cursor has no account on either side. Then `cargo fmt --all --check`, `cargo test -p hotline-core --locked` and `cargo clippy -p hotline-core --all-targets --locked -- -D warnings`. Expected: clean.
- [ ] **Step 15: Commit.** `git add crates/hotline-core/src`, then `git commit -m "A checkpoint, a thread's binding and a chapter belong to the account and sign-in their session started on."`

---

### Task 6: Account status runner and sign-in (`accounts/status.rs`, `accounts/login.rs`, terminal, fixture CLI)

**Files:**
- Create: `crates/hotline-core/src/accounts/login.rs` and `accounts/login/tests.rs`. The process cases outgrow the code (docs/standards.md "Test layout").
- Modify: `crates/hotline-core/src/accounts/status.rs` (`AccountStatuses` beside Task 4's `parse_claude_status`), `accounts/status/tests.rs` (append), `accounts.rs` (`pub(crate) mod login;`, in Step 12), `test_support.rs` (two helpers)
- Modify: `crates/hotline-core/src/driver/auth.rs:47-49` (`Attempt::state`), `:104-149` (signature; delete the env loops and TERM at :142-149), `:216` (`COMMAND_FAILED`), `:263-271` (stub), tests `:280-487`; `driver/acp/auth.rs:193-206` (Terminal arm); `driver/acp.rs:165` (`end_group` becomes `pub(crate)`)
- Modify: `crates/hotline-core/src/bin/hotline-acp-fixture.rs:1-9` (doc), `:35-36` (argv dispatch first in `main`), new fns after `:188`

**Interfaces:**
- Consumes:
  - Task 1: `AccountStatusView, AccountStatusState, LoginAttemptView, LoginAttemptState`
  - Task 2: `room::{agent_accounts, write_agent_accounts, clear_account_checkpoints, checkpoint_session}`
  - Task 3: `private_dir::{checked_account_dir, make_private_directory}`, `paths::agent_account_dir`, `process_windows::{prepare_visible, Job::attach_without_kill_on_close}`
  - Task 4: `resolve::{ResolvedAccount /* backend_id */, account}`, `support::{account_support, CLAUDE}`, `env::{env_plan, EnvPlan}`, `status::parse_claude_status`
- Consumes (existing):
  - `process_windows::{prepare, Job::attach, Job::terminate}` (hotline-support/src/process_windows.rs:25-50)
  - `acp::StartBounds` (acp.rs:123-137)
  - `registry::{install, launch, name_of, started_before}` (registry.rs:410, :378, :362, :67)
  - `room::roster` (hotline-room/src/room.rs:260)
  - `Room::{new, policy_update_lock, log}` (session.rs:653, :822, :951) and `now_ms` (session.rs:1335)
  - `Attempt` (driver/auth.rs:25-102)
  - `test_support::{until, PATIENCE}` and `session::tests::{persona, enrol, DeskKeys}` (session/tests.rs:489, :525, :458)
- Produces (status.rs):
  - `STALE_AFTER_MS: i64 = 600_000`
  - `AccountStatuses::new(bounds: StartBounds)`
  - `async fn check(&self, root: &Path, account: &ResolvedAccount) -> AccountStatusView`
  - `fn cached(&self, account_id: &str) -> AccountStatusView`
  - `fn stale(&self, account_id: &str, now_ms: i64) -> bool`
  - `fn refresh(self: &Arc<Self>, root: PathBuf, account: ResolvedAccount)`
  - `fn refresh_in_use(self: &Arc<Self>, room: &Room)`
  - `#[cfg(test)] fn put(&self, account_id: &str, status: AccountStatusView)`
- Produces (login.rs):
  - `AccountLogins::new(statuses: Arc<AccountStatuses>)`
  - `async fn start(self: &Arc<Self>, room: &Arc<Room>, account: &ResolvedAccount, email: Option<String>, owner: CancellationToken, seat_is_desk: bool) -> Result<String, String>`
  - `fn poll(&self, account_id: &str, id: &str) -> Result<AuthStatus, String>`
  - `fn input(&self, account_id: &str, id: &str, input: &str) -> Result<(), String>`
  - `fn cancel(&self, account_id: &str, id: &str) -> Result<(), String>`
  - `fn running(&self, account_id: &str) -> bool`
  - `fn view(&self, account_id: &str) -> Option<LoginAttemptView>`
  - `pub(crate) fn can_sign_in_here(seat_is_desk: bool) -> bool` = `cfg!(unix) || seat_is_desk`
- Produces (driver/auth.rs):
  - `terminal(launch: Launch, cwd: PathBuf, args: Vec<String>, env: EnvPlan, attempt: Arc<Attempt>) -> Result<(), String>`
  - `Attempt::state(&self) -> (&'static str, Option<String>)`
  - `pub(crate) const COMMAND_FAILED: &str = "The harness sign-in command failed. You can try again."`
- Produces (test_support.rs):
  - `acp_fixture_launch(env: &[(&str, &str)]) -> Launch`: the built fixture, found the way rig/tests.rs:832-848 finds its bin; no args.
  - `account_on_disk(log: &Log, id: &str, name: &str, login_at: Option<i64>) -> ResolvedAccount`: appends a `claude-acp` row, then `make_private_directory`, then `resolve::account`.
- Produces (fixture CLI; Task 10 keeps it and adds `--print-env`, `REPORT_ENV`, `CHILD_ENV`):
  - **Dispatch:** `--cli` anywhere in argv (after `-y <pkg>` under the npx shim) → `std::process::exit(cli(&args[at + 1..]))`. `fn cli(args: &[OsString]) -> i32` runs `auth status [--json]` or `auth login …`, and returns 2 for anything else.
  - **No folder:** with `CLAUDE_CONFIG_DIR` unset nothing is written, so it never writes under `~`. Status prints `{"loggedIn":false}` and exits 1; login exits 1.
  - **Env report:** every run first writes `<dir>/fixture-<status|login>-env.json`, the sorted names of its whole environment, never a value.
  - **status:**
    - `FIXTURE_STATUS_MODE=hang` sleeps 600 s.
    - `FIXTURE_AUTH=api_key` → `{loggedIn:true, authMethod:"api_key", apiProvider:"firstParty", apiKeySource:"ANTHROPIC_API_KEY", configDirectory}`.
    - `FIXTURE_AUTH=bedrock` → `{loggedIn:true, authMethod:"third_party", apiProvider:"bedrock", configDirectory}`.
    - Otherwise loggedIn ⇔ `<dir>/.credentials.json` exists: `{loggedIn, authMethod:"claude.ai"|"none", apiProvider:"firstParty", email /* from that file */, subscriptionType:"max", configDirectory /* the variable as given */}`.
    - Exit 0 when logged in, else 1.
  - **login:** writes `<dir>/fixture-login-args.json` (the args after `login`), prints "Opening the browser to sign in.", then acts by `FIXTURE_LOGIN_MODE` (default `ok`):

    | Mode | Behaviour |
    | --- | --- |
    | `ok` | Writes `.credentials.json` `{"email": <--email or "fixture@example.invalid">}`, exits 0 |
    | `fail` | Exits 7 |
    | `silent` | Exits 0 |
    | `hang` | Prints "waiting", sleeps 600 s |
    | `prompt` | Prints "Paste code:", reads a line; `yes` → as `ok`, else exits 3 |

- For Task 8: the Room fields and getters; `AccountStatuses::new(acp_start)` in `new_with_mcp` and `new(StartBounds::default())` elsewhere; `AccountLogins::new(statuses.clone())`; and the one `refresh_in_use` call when the Desk opens.
- For Task 9: `login::signed_in` ends where Task 9 appends `room.account_signed_in(&account.id);`. This is the direct call, with no `finished` future; Task 9 appends it because it creates that method after this task lands.
- SHEET DEVIATION: `start` takes `room: &Arc<Room>` (for `policy_update_lock`, `log` and Task 9's call) where the sheet has `root`; `new` takes `statuses` / `bounds`.

- [ ] **Step 1: Write the failing status tests** in `accounts/status/tests.rs`. Helpers:
  - `view(state)` has its other fields None; `A` and `B` are uuids; `log` is on `tempdir/"Conta Teste ção"`.
  - `status(account, env).await` = `run_status(&acp_fixture_launch(&[CATALOGUE, env].concat()), account, PATIENCE).await`
  - `CATALOGUE = [("ANTHROPIC_API_KEY", "sk-catalogue"), ("ANTHROPIC_BASE_URL", "https://proxy.invalid"), ("CLAUDE_CODE_USE_BEDROCK", "1")]`
```rust
#[tokio::test] async fn one_status_run_per_account_and_waiters_share_its_answer() {
    let s = AccountStatuses::new(StartBounds::default()); let runs = AtomicUsize::new(0); let (release, held) = oneshot::channel::<()>();
    let first = s.check_with("a", || async { runs.fetch_add(1, SeqCst); held.await.ok(); view(SignedIn) });
    let second = s.check_with("a", || async { runs.fetch_add(1, SeqCst); view(SignedOut) });
    let other = async { until("a runs", || runs.load(SeqCst) == 1).await; assert_eq!(s.cached("a").state, Checking);
        assert_eq!(s.check_with("b", || async { runs.fetch_add(1, SeqCst); view(SignedOut) }).await.state, SignedOut); release.send(()).unwrap() };
    let (a, b, ()) = tokio::join!(first, second, other);
    assert_eq!((runs.load(SeqCst), a.state, b.state, a.checked_at == b.checked_at), (2, SignedIn, SignedIn, true));
    s.check_with("a", || async { runs.fetch_add(1, SeqCst); view(SignedOut) }).await; assert_eq!((runs.load(SeqCst), s.cached("a").state), (3, SignedOut)); // an ended run is not reused
}
#[test] fn only_an_answer_older_than_ten_minutes_is_stale() {
    let s = AccountStatuses::new(StartBounds::default()); s.put("a", AccountStatusView { checked_at: Some(1_000), ..view(SignedIn) });
    assert_eq!(s.cached("never").state, NotChecked); assert!(!s.stale("never", i64::MAX)); // never checked is not stale
    assert!(!s.stale("a", 1_000 + STALE_AFTER_MS) && s.stale("a", 1_001 + STALE_AFTER_MS));
}
#[tokio::test] async fn status_runs_in_the_account_folder_without_inherited_credentials() {
    let account = account_on_disk(&log, A, "Account 1", None); assert_eq!(status(&account, &[]).await.state, SignedOut);
    std::fs::write(account.dir.join(".credentials.json"), r#"{"email":"ada@example.com"}"#).unwrap();
    assert_eq!(status(&account, &[]).await, AccountStatusView { state: SignedIn, checked_at: None, email: Some("ada@example.com".into()), plan: Some("max".into()) });
    let seen: Vec<String> = serde_json::from_slice(&std::fs::read(account.dir.join("fixture-status-env.json")).unwrap()).unwrap();
    assert!(seen.iter().any(|n| n == CLAUDE.env_var)); // the catalogue's three names are gone too: spec §14 row 2
    for name in CLAUDE.scrub.iter().filter(|n| **n != CLAUDE.env_var) { assert!(!seen.iter().any(|n| n == name), "{name}") }
    for auth in ["api_key", "bedrock"] { assert_eq!(status(&account, &[("FIXTURE_AUTH", auth)]).await.state, Unavailable) }
}
#[tokio::test] async fn a_status_that_does_not_answer_in_time_is_unavailable() {
    let (started, account) = (std::time::Instant::now(), account_on_disk(&log, A, "Account 1", None));
    assert_eq!(run_status(&acp_fixture_launch(&[("FIXTURE_STATUS_MODE", "hang")]), &account, Duration::from_millis(300)).await.state, Unavailable); assert!(started.elapsed() < Duration::from_secs(10));
}
#[tokio::test] async fn refresh_rechecks_the_folder_and_covers_only_accounts_in_use() {
    // Room::new(log, DeskKeys); a, b = account_on_disk(A), (B); enrol ada (claude-acp, Some(A)), bob (codex-acp, Some(B)), cy (claude-acp, Some(unknown uuid))
    let s = Arc::new(AccountStatuses::new(StartBounds::default())); s.refresh_in_use(&room);
    std::fs::remove_dir(&a.dir).unwrap(); std::fs::remove_dir(&b.dir).unwrap(); // no await before this, so no CLI ever starts
    until("A rechecked", || s.cached(A).state == Unavailable).await; assert_eq!(s.cached(B).state, NotChecked);
}
```
- [ ] **Step 2: Run to verify they fail.** Run `python scripts/fetch-speech-engine` once per checkout. Then run `cargo build -p hotline-core --locked --bin hotline-acp-fixture && cargo test -p hotline-core --lib --locked accounts::status`; the build comes first because `--lib` builds no bin. Expected: E0599/E0425 for `check_with`, `run_status`, `put`, `refresh_in_use` and `acp_fixture_launch`.
- [ ] **Step 3: Implement** status.rs, the fixture's `auth status`, the test_support helpers and `pub(crate) fn end_group`.
  - **Struct:** `AccountStatuses { bounds, entries: Mutex<HashMap<String, Entry { gate: Arc<tokio::sync::Mutex<()>>, view: Option<AccountStatusView>, runs: u64 }>> }`. The gate works like `Room::start_gate` (session/start.rs:27-32).
  - **`pub(super) async fn check_with<F: Future<Output = AccountStatusView>>(&self, id: &str, run: impl FnOnce() -> F) -> AccountStatusView`:**
    1. Note `runs`, then await the gate.
    2. If `runs` moved while waiting, return the stored view.
    3. Otherwise run, set `checked_at = Some(now_ms())`, store the view and add 1 to `runs`.
  - **`cached`:** while the gate is held, `Checking` with the last `checked_at`/`email`/`plan`. Otherwise the stored view, or `NotChecked`.
  - **`stale`:** true when the gate is free and the stored `checked_at` is `Some(t)` with `now - t > STALE_AFTER_MS`.
  - **`check`** is `check_with` of these steps; any `Err` gives `Unavailable`:
    1. `checked_account_dir(root, &account.id)`.
    2. `install`, then `launch(root, &account.backend_id)`.
    3. The bound is `known` if `started_before(root, &launch)`, else `first`. It is checked on the base launch, and status never calls `mark_started`.
    4. `run_status`.
  - **`pub(super) async fn run_status(launch: &Launch, account: &ResolvedAccount, bound: Duration) -> AccountStatusView`:**
    - The command is the launch plus `account.support.status_args`, with `env_plan(&launch.env, &[], Some(account)).apply` and `current_dir(&account.dir)`.
    - Stdin null, stdout piped (at most 64 KiB read), stderr null, `kill_on_drop(true)`.
    - Unix: `process_group(0)`; a timeout calls `end_group`.
    - Windows: `prepare` plus `Job::attach(child.id())` (a failed attach gives Unavailable); a timeout calls `job.terminate()`.
    - A failed spawn or a timeout gives Unavailable. Otherwise the answer is `(account.support.parse_status)(&stdout, &account.dir)`.
    - The bytes are never stored or logged.
  - **`refresh`** spawns `check`.
  - **`refresh_in_use`:** takes each distinct `agent_account_id` in `room::roster(room.log())` whose `account_support(&backend_id)` is Some, then `resolve::account`. `Ok` → `refresh`; `Err` is skipped.
  - **`put`** stores the view as given.
- [ ] **Step 4: Run to verify they pass.** Same command. Expected: PASS on every platform (Task 4's three tests and these five).
- [ ] **Step 5: Commit.** `git add crates/hotline-core/src/accounts crates/hotline-core/src/driver/acp.rs crates/hotline-core/src/bin/hotline-acp-fixture.rs crates/hotline-core/src/test_support.rs; git commit -m "An account's status comes from its own CLI in its own folder with inherited credentials scrubbed, one run per account shared by every caller."`
- [ ] **Step 6: Change the terminal tests to the new call** in driver/auth.rs. Drop `AuthMethodTerminal`. TERM leads the method env, as both callers pass it; `term()` = `("TERM".into(), "xterm-256color".into())`.
  - The first unix test calls `terminal(launch, std::env::temp_dir(), vec![code.into()], env_plan(&[], &[term(), ("FIXTURE".into(), "advertised-environment".into())], None), attempt.clone())`.
  - The second calls the same with `dir.path().to_path_buf()` and `("PIDS".into(), ..)`.
  - The success script prints `'ready:%s:%s' "$FIXTURE" "$TERM"`, and the test waits for `ready:advertised-environment:xterm-256color`.
  - `output_is_bounded_and_drained_and_disconnection_forgets_it` gains `assert_eq!(attempt.state(), ("running", None))` and, after `attempt.finish(Err("x".into()))`, `assert_eq!(attempt.state(), ("failed", Some("x".into())))`.
- [ ] **Step 7: Run to verify they fail.** Run `cargo test -p hotline-core --lib --locked driver::auth`. Expected: E0599 for `state` on every platform, and E0061 on `terminal` on unix.
- [ ] **Step 8: Implement.**
  - **`terminal`:** `CommandBuilder::new(&launch.command)`, `.args(&launch.args)`, `.args(&args)`, `.cwd(&cwd)`, then `env.apply_pty(&mut command)`. A non-zero exit returns `COMMAND_FAILED`.
  - **Stub:** the non-unix stub takes the same five parameters.
  - **Doc:** args and env come from the harness's method, or from the account row plus a validated email.
  - **acp/auth.rs:** `terminal(launch.clone(), PathBuf::from(cwd), method.args, env_plan(&launch.env, &once(("TERM".into(), "xterm-256color".into())).chain(method.env).collect::<Vec<_>>(), None), attempt)`. The account is None because §9.3 refuses `agent.auth.*` for an account teammate.
  - **`Attempt::state`** reads `(state, error.clone())` without draining output.
- [ ] **Step 9: Run** `cargo test -p hotline-core --lib --locked -- driver::auth driver::acp::auth`. Expected: PASS (the process tests run on unix CI). Commit: `git add crates/hotline-core/src/driver; git commit -m "Terminal sign-in takes its arguments and environment as one plan, TERM after the catalogue, so a harness's login and an account's login run through the same PTY."`
- [ ] **Step 10: Write the failing login tests** in `accounts/login/tests.rs`. Setup:
  - `desk()` = `Room::new(log, Arc::new(DeskKeys))`.
  - `logins` = `Arc::new(AccountLogins::new(Arc::new(AccountStatuses::new(StartBounds::default()))))`.
  - `a` and `b` = `account_on_disk` for "Account 1" and "Account 2", both with `login_at: Some(5)`.
  - ada is enrolled on (claude-acp, Some(A)), with `checkpoint_session` entries (Some(A), "s1"), (Some(B), "s2") and (None, "s3").
  - `login_at(id)` and `sessions()` read the log.
```rust
#[tokio::test] async fn only_a_confirmed_sign_in_moves_login_at_and_clears_that_accounts_checkpoints() {
    assert_eq!(settle(&room, &a, Err("closed".into()), &view(SignedIn), "Claude Code").await, Err("closed".into()));
    assert_eq!(settle(&room, &a, Ok(()), &view(SignedOut), "Claude Code").await, Err("Claude Code finished, but Account 1 is not signed in.".into()));
    assert!(settle(&room, &a, Ok(()), &view(Unavailable), "Claude Code").await.is_err()); assert_eq!(login_at(A), Some(5)); assert_eq!(sessions(), ["s1", "s2", "s3"]);
    assert_eq!(settle(&room, &a, Ok(()), &view(SignedIn), "Claude Code").await, Ok(()));
    assert!(login_at(A) > Some(5)); assert_eq!(login_at(B), Some(5)); assert_eq!(sessions(), ["s2", "s3"]);
}
#[test] fn one_running_attempt_per_account() {
    let first = logins.admit(A, CancellationToken::new()).unwrap();
    assert_eq!(logins.admit(A, CancellationToken::new()).unwrap_err(), "A sign-in is already running for this account."); assert!(logins.admit(B, CancellationToken::new()).is_ok() && logins.running(A));
    assert_eq!(logins.input(A, &first.id, "x"), Err("This sign-in is not accepting terminal input.".into()));
    assert_eq!(logins.cancel(A, "other"), Err("That sign-in is no longer available.".into()));
    logins.cancel(A, &first.id).unwrap(); assert!(first.cancel.is_cancelled()); first.finish(Err("Sign-in cancelled.".into()));
    assert_eq!(logins.view(A).map(|v| (v.state, v.error)), Some((LoginAttemptState::Failed, Some("Sign-in cancelled.".into()))));
    let second = logins.admit(A, CancellationToken::new()).unwrap();
    assert_eq!(logins.poll(A, &first.id).unwrap_err(), "That sign-in is no longer available."); assert!(logins.poll(A, &second.id).is_ok() && logins.poll(B, &second.id).is_err());
}
#[test] fn only_a_plain_email_becomes_an_argument() {
    assert_eq!(login_args(&CLAUDE, Some("ada@example.com")), ["--cli", "auth", "login", "--claudeai", "--email", "ada@example.com"]);
    for refused in ["", "ada", "ada@example", "a b@example.com", "a\"b@example.com", "a%PATH%@example.com", "a&calc@example.com", "-x@example.com", "a@-example.com", "a@example..com"] {
        assert_eq!(login_args(&CLAUDE, Some(refused)), ["--cli", "auth", "login", "--claudeai"]) }
}
#[tokio::test] async fn start_refuses_before_anything_is_spawned() {
    assert_eq!((can_sign_in_here(false), can_sign_in_here(true)), (cfg!(unix), true)); let closed = CancellationToken::new(); closed.cancel();
    assert_eq!(logins.start(&room, &a, None, closed, true).await, Err("The desktop connection has closed.".into()));
    #[cfg(windows)] assert_eq!(logins.start(&room, &a, None, CancellationToken::new(), false).await, Err("Sign in from the desk itself.".into()));
    std::fs::remove_dir(&a.dir).unwrap();
    assert_eq!(logins.start(&room, &a, None, CancellationToken::new(), true).await, Err(format!("Can't open Account 1's folder ({}): entity not found. Fix the folder or pick another account.", a.dir.display())));
    assert!(!logins.running(A));
}
// sign(mode, timeout): admit(A), then logins.run(&room, Run { launch: acp_fixture_launch(&[("FIXTURE_LOGIN_MODE", mode)]), bound: PATIENCE, account: a.clone(), args: login_args(&CLAUDE, Some("ada@example.com")), agent: "Claude Code".into(), timeout }, attempt)
#[tokio::test] #[cfg_attr(windows, ignore = "opens a console window")] async fn a_real_sign_in_succeeds_only_on_exit_zero_with_a_confirming_status() {
    for (mode, signed_before, error) in [("fail", true, Some("Claude Code closed before it was signed in.")), ("silent", false, Some("Claude Code finished, but Account 1 is not signed in.")), ("ok", false, None)] {
        // a fresh desk(); signed_before plants a.dir/.credentials.json; sign(mode, PATIENCE).await
        assert_eq!(logins.view(A).map(|v| (v.state, v.error)), Some((if error.is_some() { Failed } else { Succeeded }, error.map(String::from))));
        assert_eq!((login_at(A) > Some(5), sessions()[0] == "s1"), (error.is_none(), error.is_some()));
        assert_eq!(logins.statuses.cached(A).state, if mode == "silent" { SignedOut } else { SignedIn }); // §10.2: checked after every attempt
    } // "ok": fixture-login-args.json == ["--claudeai", "--email", "ada@example.com"]; fixture-login-env.json has CLAUDE_CONFIG_DIR and no other CLAUDE.scrub name
}
#[tokio::test] #[cfg_attr(windows, ignore = "opens a console window")] async fn cancel_disconnect_and_timeout_end_a_sign_in_as_failed() {
    for (end, error) in [("cancel", "Sign-in cancelled."), ("disconnect", "Sign-in cancelled when the desktop disconnected."), ("timeout", "Sign-in timed out. You can try again.")] {
        // fresh desk() with .credentials.json; spawn sign("hang", 500 ms for "timeout" else PATIENCE); until a.dir/fixture-login-env.json exists; logins.cancel / owner.cancel / nothing; await
        assert_eq!(logins.view(A).map(|v| (v.state, v.error)), Some((Failed, Some(error.into())))); assert_eq!((login_at(A), sessions()[0] == "s1"), (Some(5), true));
    }
}
#[cfg(unix)] #[tokio::test] async fn terminal_input_answers_the_cli_prompt() { // spawn sign("prompt", PATIENCE); until the polled output contains "Paste code:"
    assert_eq!(logins.input(A, &id, "yes\n"), Ok(())); // the run then ends with view(A).state == Succeeded
}
```
- [ ] **Step 11: Run to verify they fail.** Run `cargo build -p hotline-core --locked --bin hotline-acp-fixture && cargo test -p hotline-core --lib --locked accounts::login`. Expected: unresolved `AccountLogins`, `settle`, `login_args` and `Run`.
- [ ] **Step 12: Implement** login.rs and the fixture's `auth login`.
  - **`AccountLogins { statuses: Arc<AccountStatuses>, attempts: Mutex<HashMap<String, Arc<Attempt>>> }`:**
    - The last attempt stays in the map for `view` and `poll`.
    - Private `admit(&self, account_id, owner) -> Result<Arc<Attempt>, String>`.
    - An id other than the stored one gives "That sign-in is no longer available."
    - `view` maps `Attempt::state()`.
  - **`login_args(support, email)`** adds `[email_flag, email]` only for a plain email:
    - At most 254 characters and exactly one `@`.
    - Local part: 1–64 characters from `[A-Za-z0-9._+-]`, not starting with `-` or `.`, and no `..`.
    - Domain: two or more labels of 1–63 characters from `[A-Za-z0-9-]`, none starting or ending with `-`.
    - The email is never stored and never put in a sentence.
  - **`start`**, in order:
    1. `!can_sign_in_here` → "Sign in from the desk itself."
    2. Owner cancelled → "The desktop connection has closed."
    3. `checked_account_dir(root, &account.id)` returns `Err(e)` → "Can't open {name}'s folder ({account.dir}): {e.kind()}. Fix the folder or pick another account."
    4. `install`, then `launch`; an error returns their sentence.
    5. `admit`.
    6. Spawn `run` with `LOGIN_TIMEOUT = Duration::from_mins(10)` and `agent = registry::name_of(..)` (or the id when there is no name), and return `attempt.id`.
  - **`struct Run { launch: Launch, bound: Duration, account: ResolvedAccount, args: Vec<String>, agent: String, timeout: Duration }`** and **`async fn run(self: &Arc<Self>, room: &Arc<Room>, run: Run, attempt: Arc<Attempt>)`:**
    - **Unix:** spawn `terminal(launch, account.dir, args, env_plan(&launch.env, &[("TERM".into(), "xterm-256color".into())], Some(&account)), attempt)`. On timeout, cancel the attempt and await the task, which reaps its process group.
    - **Windows:**
      - Build the command from the launch plus args, with `env_plan(&launch.env, &[], Some(&account)).apply`, `current_dir(&account.dir)` and `prepare_visible`.
      - Call `Job::attach_without_kill_on_close(child.id())`. If it fails, call `child.start_kill()` and answer "{agent}'s sign-in could not be started."
      - `select!` the exit against cancel, owner and timeout. Those three call `job.terminate()`, then `child.wait()`.
      - A normal exit only drops the handle, so a browser the CLI opened keeps running.
      - The §17 manual check stays with the builder: in a release build and in a dev run, confirm the console shows the CLI and takes input.
    - **Result:**
      - Timeout → "Sign-in timed out. You can try again."
      - Owner disconnect → "Sign-in cancelled when the desktop disconnected."
      - Cancel → "Sign-in cancelled."
      - A non-zero exit is `Err(COMMAND_FAILED)` on both platforms (on unix it comes from `terminal`). One shared match turns it into "{agent} closed before it was signed in."
    - **After the process:**
      1. Always run `statuses.check_with(&account.id, || run_status(&launch, &account, bound))`.
      2. A cancel or timeout result stands as it is; anything else goes through `settle`.
      3. Last, call `attempt.finish(result)`.
  - **`settle(room, account, process, status, agent)`:** `Err` passes through. `Ok` with `SignedIn` calls `signed_in`. Any other `Ok` gives "{agent} finished, but {account.name} is not signed in."
  - **`signed_in(room: &Arc<Room>, account: &ResolvedAccount) -> Result<(), String>`:**
    1. Under `room.policy_update_lock()`, set the row's `login_at = Some(now_ms())` with `write_agent_accounts`. If the row was removed meanwhile, nothing is written and the result is `Ok`.
    2. Then call `clear_account_checkpoints(room.log(), &account.id)`.
    3. A failed write gives "Signed in, but Hotline could not record it. Try again."
- [ ] **Step 13: Run to verify they pass.** Same command. Expected: unix 7 passed; Windows 4 passed and 2 ignored (`-- --ignored` runs them through `prepare_visible` and opens consoles). Then `cargo clippy -p hotline-core --all-targets --locked -- -D warnings`: clean.
- [ ] **Step 14: Commit.** `git add crates/hotline-core/src/accounts crates/hotline-core/src/bin/hotline-acp-fixture.rs; git commit -m "A sign-in succeeds only when the CLI exits 0 and its own status confirms the account; anything else leaves login_at and checkpoints alone."`

---

### Task 7: Account setup copy and view (`accounts/setup.rs`)

**Files:**
- Create: `crates/hotline-core/src/accounts/setup.rs` and `crates/hotline-core/src/accounts/setup/tests.rs`. The fixture outgrows the code (docs/standards.md "Test layout").
- Modify: `crates/hotline-core/src/accounts.rs` (`pub(crate) mod setup;`)
- Modify: `crates/hotline-core/src/accounts/support.rs`: `home_dir` and `state_file` on `AccountSupport` and `CLAUDE`, plus one assert in Task 4's `only_claude_code_has_a_row_and_it_carries_the_spec_values`

**Interfaces:**
- Consumes:
  - Task 4 `support::{AccountSupport { env_var, scrub, setup, settings_auth_keys }, SetupCategory { id, paths, json_keys, open }, CLAUDE}`. The ids are `instructions, skills, plugins, mcp, settings`; the `open` values are `CLAUDE.md, skills/, plugins/, .claude.json, settings.json`.
  - Task 1 `SetupCopyReport, AccountSetupView, SetupCategoryView, AccountSetupOrigin`
  - `credentials::{atomic_write, private_read}` (hotline-support/src/credentials.rs:493, :461). `atomic_write` writes a temp file in the same folder and renames it into place. Both refuse a link at the parent or the target.
- Produces:
  - `pub(crate) struct SetupSource { pub dir: PathBuf, pub state_file: Option<PathBuf> }` (`Debug, PartialEq`)
  - `system_default_source(support: &AccountSupport) -> Option<SetupSource>`, which calls the private `source_from(support, var_os(support.env_var), HOME else USERPROFILE)` (the lookup paths.rs:19-21 uses). Tests call `source_from`, so no test sets an environment variable.
  - `copy_setup(source: &SetupSource, account_dir: &Path, support: &AccountSupport, categories: &[String]) -> io::Result<SetupCopyReport>`
  - `setup_view(account_dir: &Path, support: &AccountSupport, origin: Option<AccountSetupOrigin>, with_paths: bool) -> AccountSetupView`. It only reads counts and creates nothing.
  - `nothing_to_copy(agent: &str) -> String` = "There is no {agent} setup on this computer to copy."
  - For Task 8:
    - `account_dir` is the path `private_dir::checked_account_dir` (Task 3) returned to the caller.
    - `accounts.create` with no `categories` passes every row id.
    - A `None` source answers `nothing_to_copy(agent)`.
    - `with_paths` is true for the Desk and Owner seats.
  - For Task 11: each `path` is `account_dir.join(open)` unchanged, so a folder keeps its trailing `/`. The shell's `open_account_setup` creates a missing file.
  - SHEET DEVIATION: `SetupSource { dir, state_file: Option }` and `system_default_source(support)`, because §9.1/§9.1.1 put the setup origin in the row; the row gains `home_dir: &'static str` (CLAUDE `".claude"`) and `state_file: Option<&'static str>` (CLAUDE `Some(".claude.json")`).

- [ ] **Step 1: Write the failing tests.** `fixture()` builds two folders:
  - **Source dir:**
    - Files that must never be copied: `.credentials.json`, `history.jsonl`, `projects/p/s.jsonl`, `sessions/1.json`, `todos/t.json`.
    - Setup: `CLAUDE.md` = "be kind", `rules/style.md`, `skills/review/SKILL.md`, `agents/critic.md`, `plugins/installed_plugins.json`.
    - `settings.json` = `{"apiKeyHelper":"/bin/key","awsAuthRefresh":"aws sso login","env":{"ANTHROPIC_API_KEY":"sk-test","KEEP":"1"},"permissions":{"deny":["Bash(rm:*)"]},"enabledPlugins":{"p@m":true},"extraKnownMarketplaces":{"m":{}}}`.
  - **Source state file:** `{"mcpServers":{"echo":{"command":"echo"}},"oauthAccount":{"emailAddress":"me@home"},"projects":{"/x":{}}}`.
  - **`skills/linked`** links to a folder outside the source:
    - Unix: a `symlink`.
    - Windows: a junction made with `cmd.exe /d /c mklink /J` (recipe at hotline-support/src/credentials/windows.rs:268-279). It needs no admin, and std's `is_symlink()` is true for a junction.
  - **Account dir** (made with `make_private_directory`): `.claude.json` = `{"oauthAccount":{"emailAddress":"acct@x"}}`, `CLAUDE.md` = "mine", `skills/stale/SKILL.md`.
  - Task 4's row test gains `assert_eq!((CLAUDE.home_dir, CLAUDE.state_file), (".claude", Some(".claude.json")));`.
```rust
#[test] fn copy_brings_the_chosen_categories_and_no_secret() {
    let r = copy_setup(&source, acct, &CLAUDE, &ids(&["instructions", "skills", "plugins", "mcp", "settings"])).unwrap();
    assert_eq!((r.copied.clone(), r.links_skipped), (strings(&["instructions", "skills", "plugins", "mcp", "settings"]), 1));
    for never in [".credentials.json", "history.jsonl", "projects", "sessions", "todos"] { assert!(!acct.join(never).exists()) }
    assert_eq!(read(acct.join("CLAUDE.md")), "be kind");
    assert!(acct.join("skills/review/SKILL.md").exists() && !acct.join("skills/stale").exists() && !acct.join("skills/linked").exists());
    let s = json(acct.join("settings.json"));
    assert!(s.get("apiKeyHelper").is_none() && s.get("awsAuthRefresh").is_none());
    assert_eq!((s["env"].clone(), s["permissions"]["deny"][0].clone()), (json!({"KEEP": "1"}), json!("Bash(rm:*)")));
    assert_eq!(sorted(r.removed_keys), ["apiKeyHelper", "awsAuthRefresh", "env.ANTHROPIC_API_KEY"]);
    let st = json(acct.join(".claude.json"));
    assert_eq!((st["mcpServers"].clone(), st["oauthAccount"]["emailAddress"].clone()), (json!({"echo": {"command": "echo"}}), json!("acct@x")));
    assert!(st.get("projects").is_none());
}
#[test] fn copy_again_replaces_only_the_marked_categories() {
    // after the full copy: add acct/skills/mine; set the account's permissions.allow; change the source's enabledPlugins
    copy_setup(&source, acct, &CLAUDE, &ids(&["plugins"])).unwrap();
    // acct keeps permissions.allow and skills/mine; acct enabledPlugins == the source's; no apiKeyHelper
    // delete the source's CLAUDE.md; copy ["instructions"] → !acct.join("CLAUDE.md").exists() (the account mirrors the absence)
}
#[test] fn a_refused_copy_leaves_the_account_as_it_was() {
    assert_eq!(copy_setup(&source, acct, &CLAUDE, &ids(&["instructions", "nope"])).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert_eq!(read(acct.join("CLAUDE.md")), "mine");
    std::fs::write(acct.join(".claude.json"), "not json").unwrap();
    assert_eq!(copy_setup(&source, acct, &CLAUDE, &ids(&["mcp"])).unwrap_err().kind(), ErrorKind::InvalidData);
    assert_eq!(read(acct.join(".claude.json")), "not json"); // the CLI's own state is never overwritten
}
#[test] fn a_link_in_the_account_is_replaced_not_followed() {
    // acct/skills is a link to outside/ holding keep.md; copy ["skills"]
    assert!(outside.join("keep.md").exists() && !std::fs::symlink_metadata(acct.join("skills")).unwrap().is_symlink());
}
#[test] fn the_source_is_the_env_folder_else_the_home_default() {
    assert_eq!(source_from(&CLAUDE, Some(env.clone().into()), Some(home.clone())), Some(SetupSource { dir: env.clone(), state_file: Some(env.join(".claude.json")) }));
    assert_eq!(source_from(&CLAUDE, None, Some(home.clone())), Some(SetupSource { dir: home.join(".claude"), state_file: Some(home.join(".claude.json")) }));
    assert_eq!(source_from(&CLAUDE, Some("".into()), Some(home.clone())), source_from(&CLAUDE, None, Some(home.clone())));
    assert_eq!(source_from(&CLAUDE, Some(home.join("missing").into()), Some(home)), None);
    let this_home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    assert_eq!(system_default_source(&CLAUDE), source_from(&CLAUDE, std::env::var_os(CLAUDE.env_var), this_home)); // reads the row's variable
    assert_eq!(nothing_to_copy("Claude Code"), "There is no Claude Code setup on this computer to copy.");
}
#[test] fn setup_view_counts_items_and_names_paths_only_when_asked() {
    // after the full copy: counts instructions 2, skills 2, plugins 1, mcp 1, settings 1; with_paths false → every path None
    let v = setup_view(empty, &CLAUDE, None, true);
    assert!(v.categories.iter().all(|c| c.count == 0));
    assert_eq!(v.categories.iter().map(|c| c.path.clone().unwrap()).collect::<Vec<_>>(),
               CLAUDE.setup.iter().map(|c| empty.join(c.open).to_string_lossy().into_owned()).collect::<Vec<_>>()); // "skills/" keeps its slash
    for open in ["CLAUDE.md", "settings.json", "skills", "plugins", ".claude.json"] { assert!(!empty.join(open).exists()) } // no side effects
}
```
- [ ] **Step 2: Run to verify they fail.** Run `cargo test -p hotline-core --lib --locked -- accounts::setup accounts::support`. Expected: unresolved `copy_setup`, `setup_view`, `source_from` and `nothing_to_copy`, and no field `home_dir` on `AccountSupport`.
- [ ] **Step 3: Implement** setup.rs and the two row fields.
  - **`source_from`:**
    - An empty env value counts as unset.
    - Env set → `{dir: env, state_file: state_file.map(|f| env.join(f))}`.
    - Otherwise → `{dir: home/home_dir, state_file: state_file.map(|f| home.join(f))}`.
    - A `dir` that is not a folder → `None`.
  - **`copy_setup`:**
    - Every id is checked first; an unknown one gives `InvalidInput` ("Unknown setup item: {id}").
    - Categories run in row order and duplicates are ignored; `copied` lists them in that order.
    - Each `paths` entry: `settings.json` goes to `copy_settings`; any other goes to `mirror(source.dir/p, account/p)`.
  - **`copy_settings`:**
    - Parses the source as a JSON object; invalid JSON is `InvalidData`.
    - Removes the top-level keys in `settings_auth_keys` and `env.{name}` for each name in `scrub`, recording each as `key` or `env.NAME`.
    - When the source has no settings file, the account's file is removed.
  - **`merge_keys`**, for each `json_keys` entry `(file, keys)`:
    - The source file is `source.state_file` when `Some(file) == support.state_file`, else `source.dir/file`. An absent source counts as `{}`; an invalid one is `InvalidData`.
    - The account's file is read with `private_read`. Absent counts as `{}`. Present but not a JSON object is `InvalidData`, and the file is never overwritten.
    - A key present in the source replaces the account's value; a key absent from the source is removed; other account keys stay.
    - The file is written pretty-printed with a trailing newline by `atomic_write`.
  - **`mirror(src, dst)`** starts from `symlink_metadata(src)` and never follows a link:
    - NotFound → `remove(dst)`.
    - A link → `links_skipped += 1` and `remove(dst)`.
    - A file → `remove(dst)` unless it is a regular file, then `atomic_write(dst, read(src))`.
    - A folder → `remove(dst)` unless it is a real folder, then `create_dir_all`, recurse into each child, and remove the children of `dst` that the source lacks.
  - **`remove(p)`** starts from `symlink_metadata`:
    - A link is unlinked itself: unix `remove_file`; Windows `remove_dir` for a directory link or junction, else `remove_file`.
    - A folder gets `remove_dir_all`, a file gets `remove_file`, and NotFound is fine.
  - **`setup_view`:**
    - A category with `json_keys` counts the entries under the first key of the first file in the account (0 when the file is missing or invalid).
    - Any other category sums over `paths`: a folder's direct entries, and 1 for a file that exists.
    - With `with_paths`, `path = Some(account_dir.join(open).to_string_lossy())`; otherwise None.
    - `setup` is `origin`.
- [ ] **Step 4: Run to verify they pass.** Run `cargo test -p hotline-core --lib --locked -- accounts::setup accounts::support`. Expected: 6 passed in setup and support PASS. `links_skipped == 1` on both platforms: a symlink on unix, a junction on Windows.
- [ ] **Step 5: Commit.** `git add crates/hotline-core/src/accounts.rs crates/hotline-core/src/accounts/support.rs crates/hotline-core/src/accounts/setup.rs crates/hotline-core/src/accounts/setup/tests.rs; git commit -m "Copying setup into an account mirrors only the chosen categories, never follows a link and never carries a credential or an auth key; viewing it changes nothing."`

---

### Task 8: Accounts wire

**Files:**
- Create: `crates/hotline-core/src/wire/commands/accounts.rs`, `crates/hotline-core/src/session/accounts.rs`, `crates/hotline-core/src/wire/tests/accounts.rs` (with `mod accounts;` at wire/tests.rs:1111)
- Modify: `crates/hotline-core/src/wire/commands.rs`: `:9-11` (`pub(super) mod accounts;`), `:216-222` (the four `AccountsLogin*` join the agent.auth refusal), the `MobilePersonaCreate` and `MobilePersonaUpdate` arms (`:229-249`), `:262` (`PersonaCreate` under `policy_update_lock`), and Task 1's placeholder arm, which goes
- Modify: `crates/hotline-core/src/wire/commands/persona.rs:46-111` (`build_persona`) and `:403-405` (refusal beside `avatar`)
- Modify: `crates/hotline-core/src/wire/commands/settings.rs:46-48`
- Modify: `crates/hotline-core/src/wire/socket.rs:20-39,174-187` (Outbox), `:444-452` (spawn arm beside `DesksLink`), `:555-598` (owner-checked arms)
- Modify: `crates/hotline-core/src/wire/roster.rs:44-50,198,214-234,323-336` and `crates/hotline-core/src/wire/subscribe.rs:122-133` (`seat` reaches `shaped`)
- Modify: `crates/hotline-room/src/room_handle.rs:108-124` (defaults after `agent_auth_cancel`)
- Modify: `crates/hotline-core/src/session.rs:34` (`mod accounts;`), `:636` and `:760` (two fields), `:679,:699,:703-709` and `session/tests.rs:759` (`acp_start` reaches the constructor)
- Modify: `crates/hotline-core/src/accounts.rs` (glue), `crates/hotline-core/src/desk.rs:209` (refresh on open) and after `:400` (Desk methods)
- Modify: the Outbox literals at `wire/tests.rs:1045-1054`, `wire/tests/seats.rs:636-645`, `wire/tests/asks.rs:59`, `wire/tests/schedules.rs:213`; `Quiet` at `wire/tests.rs:89-123,489-522`
- Test: `wire/tests/{accounts,seats,owner_handler,personas,settings,roster}.rs`; `crates/hotline-core/src/mcp/server.rs` (`mod tests` :1554, with `ada`, `bob`, `write_persona`, `room_with_two`, `through_rig`)

**Interfaces:**
- Consumes:
  - contract types, `Command::Accounts*`, `Command::PersonaAccount` and the placeholder arm (Task 1)
  - `room::{agent_accounts, write_agent_accounts, AGENT_ACCOUNTS}` (Task 2)
  - `paths::{agent_accounts_dir, agent_account_dir}`, `private_dir::{make_private_directory, checked_account_dir}` (Task 3)
  - `support::account_support`, `resolve::{account, ResolvedAccount, AccountRefusal}` (Task 4)
  - `AccountStatuses::{new(StartBounds), cached, check, stale(id, now_ms), refresh(root, account), refresh_in_use(&Room)}`, `AccountLogins::{new(statuses), start(&room, &account, email, owner, seat_is_desk), poll, input, cancel, view}`, `login::can_sign_in_here` (Task 6)
  - `setup::{system_default_source(support), copy_setup, setup_view}` (Task 7)
  - existing: `RoomHandle::backends` (room_handle.rs:378), `off_the_reactor` (commands.rs:60), `registry::name_of`, `Room::federation_roster_entries` (session/links.rs:121)
- Produces:
  - SHEET DEVIATION: the wire itself handles `accounts.{create,rename,remove,setup.*}`, being the only writer of settings on the room stream (commands.rs:1-7); `RoomHandle` carries only live state.
  - `RoomHandle` methods with defaults: `fn account_live(&self, account_id: &str) -> (AccountStatusView, Option<LoginAttemptView>)` (`NotChecked`, None); `fn accounts_refresh_stale(&self, account_ids: &[String]) {}`; `async fn accounts_status(&self, account_id: &str) -> Result<AccountStatusView, String>` ("This room cannot check agent accounts."); `async fn accounts_login_start(&self, account_id: &str, email: Option<String>, owner: CancellationToken, seat_is_desk: bool) -> Result<String, String>` ("This room does not offer account sign-in."); `fn accounts_login_poll(..) -> Result<AuthStatus, String>`, `accounts_login_input(..)`, `accounts_login_cancel(..)` ("That sign-in is no longer available."); `async fn persona_account(&self, persona_id: &str, account_id: Option<String>) -> Result<SessionAccount, String>` ("This room cannot assign agent accounts.")
  - `pub(super) async fn wire::commands::accounts::run(command: Command, log: &Log, room: &Arc<dyn RoomHandle>, seat_is_desk: bool) -> Result<Value, String>`
  - `pub(crate) fn accounts::check_choice(log: &Log, backend_id: &str, account_id: Option<&str>) -> Result<(), String>`, `pub(crate) fn accounts::resolved(log: &Log, account_id: &str) -> Result<ResolvedAccount, String>`
  - `Room::account_statuses(&self) -> &Arc<AccountStatuses>`, `Room::account_logins(&self) -> &Arc<AccountLogins>` (session/accounts.rs)
  - `Outbox.account_logins: Arc<Mutex<HashMap<String, String>>>` (account id → attempt id); `pub(super) fn roster::shaped(sender: &Outbox, seat: Seat, row: Value) -> Value`

- [ ] **Step 1: Write the failing tests.** `Quiet` gains `account_sign_ins: Mutex<Vec<(CancellationToken, bool)>>` and overrides `accounts_login_*` (each asserts `account_id == "acct-1"`; start records `(owner, seat_is_desk)` and answers "fixture-attempt"; poll answers `running` / "ephemeral fixture output"; input asserts "fixture input") and `persona_account` (answers `{wanted: account_id}`). `A` is a uuid.
```rust
// seats.rs, beside only_the_desk_seat_may_touch_stored_secrets (:407)
#[test] fn only_the_desk_and_the_owner_reach_agent_accounts() {
    for c in account_commands() { // the 12 variants
        assert!(Seat::Desk.permits(&c) && Seat::Owner.permits(&c), "{c:?}");
        assert!(!Seat::Phone.permits(&c) && !Seat::Link.permits(&c), "{c:?}");
    }
}
// seats.rs, auth_wire_allows_only_desktop_and_disconnect_revokes_its_owner (:627) over accounts.login.*
#[tokio::test] async fn account_sign_in_belongs_to_its_desktop_socket_and_disconnect_revokes_it()
// 1: start/poll/input/cancel as Seat::Phone → code FORBIDDEN; account_sign_ins empty
// 2: desk(port): start → result.id "fixture-attempt"; poll → output "ephemeral fixture output"; input and cancel ok; seat_is_desk true
// 3: a second desk(port) with the same ids → error contains "another desktop"
// 4: socket.close → the recorded owner is cancelled within 5 s
// commands::run(each AccountsLogin*) == Err("Sign-in requires its owning desktop connection.")
// owner_handler.rs, beside cookie_push_is_operator_only_through_the_real_handler (:393)
#[tokio::test] async fn agent_accounts_are_operator_only_through_the_real_handler()
// Quiet; one frame per command (each seat's create names itself); for seat in [Desk, Owner, Phone]:
//   assert_eq!(answer["code"] == FORBIDDEN, seat == Seat::Phone); the two recorded starts have seat_is_desk [true, false]
#[tokio::test] async fn removing_an_account_in_use_names_its_teammates_and_keeps_its_folder()
// Quiet, TempDir log, Seat::Desk; persona.create Ada and Bob {backendId:"claude-acp", agentAccountId:id}
//   → remove == Err("Account 1 can't be removed while Ada and Bob use it."); persona.delete both → remove answers null;
//   agent_accounts empty; the folder still exists
// accounts.rs (new): commands::run on Quiet and a TempDir log
#[tokio::test] async fn creating_an_account_makes_its_private_folder_and_ignores_a_client_path()
// create {backendId:"claude-acp", name:" Account 1 ", startWith:"empty", path:"/elsewhere"} → account.name "Account 1",
//   folder == agent_account_dir(root, id) and is a directory, copy null, setup.from "empty"
// refused: "  " → "An account needs a name."; 65 chars → "An account name is at most 64 characters.";
//   "ACCOUNT 1" → "There is already an account named ACCOUNT 1 for that agent."; backendId "hotline" → "That agent does not use accounts."
// rename → "Account 2"; list → agents == [cursor, supportsAccounts false] (no Hotline Agent),
//   accounts[0] {usedBy [], folder, canSignInHere == can_sign_in_here(false)}
#[tokio::test] async fn setup_is_read_and_copied_only_in_a_checked_folder()
// setup.list → ids [instructions, skills, plugins, mcp, settings], counts 0, every path inside the folder; the folder is still empty
// setup.copy {categories: []} → "Choose at least one item to copy."
// remove_dir_all(folder): setup.list and setup.copy ["instructions"] → error starts with "Can't open Account 1's folder ("
// personas.rs
#[tokio::test] async fn an_account_is_set_only_by_persona_account()
// write_agent_accounts([A on claude-acp]); persona.create {backendId:"claude-acp", agentAccountId:A} → record carries A; unknown uuid → "That account is not on this desk.";
//   backendId "hotline" + A → "That agent does not use accounts."
// persona.update {agentAccountId:null} → "A teammate's account is set with persona.account."; record unchanged
// mobile.persona_update {id, name:"Ada 2", agentAccountId:null} → ok, reply has no agentAccountId, record keeps A
// mobile.persona_create {requestId: Ada's id, name:"Ada"} (a retry) → reply has no agentAccountId
// settings.rs
#[tokio::test] async fn settings_update_cannot_write_agent_accounts()
// patches {agentAccounts:[…]} and {agentAccounts:null} → "Agent accounts are changed with accounts.*."; agent_accounts empty
// roster.rs (unit; Outbox literal with `lean` stored)
#[test] fn a_phone_roster_never_carries_an_account_lean_or_not()
// row {persona:{agentAccountId:A}, session:{account:{running:A}}}: (Desk,false), (Owner,false) keep both fields;
//   (Phone,false), (Phone,true), (Owner,true) drop both
// mcp/server.rs
#[tokio::test(flavor = "multi_thread")] async fn list_teammates_never_carries_an_agent_account()
// room_with_two; account "Secret Account" (A) written; ada on claude-acp with A; bob's list_teammates text and
//   serde_json::to_string(&room.federation_roster_entries()) contain neither A, "Secret Account" nor "agentAccountId"
```
- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p hotline-core --lib --locked wire::tests`. Expected: a compile error: `Outbox` has no field `account_logins`, and `accounts_login_start` is not a member of `RoomHandle`.
- [ ] **Step 3: Implement**
  - **room_handle.rs**: the defaults above. **session**: `account_statuses: Arc<AccountStatuses>` and `account_logins: Arc<AccountLogins>` beside `policy_updates`, built from a new last parameter `acp_start: acp::StartBounds` of `with_agents_computers_and_vault` (`new_with_mcp` passes its own; `with_agents_and_computers` and session/tests.rs:759 pass `StartBounds::default()`). The getters live in `session/accounts.rs`.
  - **accounts.rs**: `check_choice`: None → Ok; no `account_support(backend_id)` → "That agent does not use accounts."; no account with that id and backend → "That account is not on this desk.". `resolved`: `resolve::account(log.root(), log, id)`; `UnknownAccount` → "That account is not on this desk."; `FolderUnavailable` → its private sentence.
  - **desk.rs**: `open_inner` calls `room.account_statuses().refresh_in_use(&room)` after :209 (§10.2). `account_live` → `(cached(id), account_logins().view(id))`. `accounts_refresh_stale` refreshes each id that is `stale(id, now_ms())` and `resolved`; never-checked is not stale, so `accounts.list` never checks those. `accounts_status` → `resolved`, then `check`. `accounts_login_start` → `resolved`, then `account_logins().start(&self.room, ..)`. Poll, input and cancel go to `account_logins()`. `persona_account` keeps the default until Task 9.
  - **commands.rs**: `Accounts{List, Create, Rename, Remove, Status, SetupList, SetupCopy}` → `accounts::run(command, log, room, false)`; `PersonaAccount` → `room.persona_account(..)` as JSON; `PersonaCreate` takes `policy_update_lock` as `PersonaDelete` does (:264-268). The `MobilePersonaCreate` and `MobilePersonaUpdate` replies drop `agentAccountId` (§8.3: a phone neither sets nor sees it).
  - **wire/commands/accounts.rs** (`run`; any other command → Err). Reads:
    - `list`: `agents` = `room.backends()` without `HOTLINE_BACKEND_ID`, with `supports_accounts`/`takes_email` from `account_support`. One `AccountView` per account: `status`/`attempt` from `account_live`, `used_by` = ids of `room::roster` entries naming it, `folder` = `agent_account_dir`, `can_sign_in_here` = `login::can_sign_in_here(seat_is_desk)`. Then `room.accounts_refresh_stale(&ids)`. `status` → `room.accounts_status`.
  - Writes run under `policy_update_lock`, disk work through `super::off_the_reactor`:
    - Names are trimmed, then refused: blank "An account needs a name."; over 64 characters "An account name is at most 64 characters."; same backend, case-insensitive, not itself "There is already an account named {name} for that agent.". A backend without a row: "That agent does not use accounts.".
    - `create`, before any folder exists: with `system_default`, `system_default_source(support)` or "There is no {agent} setup on this computer to copy.", and `categories` (default: every row id) checked, "Unknown setup item: {id}". Then `Uuid::new_v4()`, `make_private_directory` on `agent_accounts_dir(root)` and the account folder (a client `path` is never read), `checked_account_dir(root, id)`, `copy_setup`. A failed copy writes nothing and leaves the folder: "The setup could not be copied into {name}'s folder: {error}". Then the setting gets `{created_at: now, login_at: None, setup: {from, at: now, categories: copied or []}}`; answer `AccountCreated`.
    - `rename` → `AccountView`. `remove`: unknown → "That account is not on this desk."; in use → "{account} can't be removed while {names} use it." (names "A", "A and B", "A, B and C"); else drop it from the setting, never touch the folder; answer null.
    - `setup.list`: `checked_account_dir`, else "Can't open {account}'s folder ({path}): {kind}. Fix the folder or pick another account."; then `setup_view(dir, support, account.setup, true)` (only Desk and Owner reach it).
    - `setup.copy`: an empty list → "Choose at least one item to copy."; then `checked_account_dir`, the source, `copy_setup`, and `setup = {system_default, now, copied}`; answer the report.
  - **persona.rs**: `update_persona` refuses an `agentAccountId` key with "A teammate's account is set with persona.account." beside `avatar`, before anything is revoked or written; it is not in `persona_patch_reattaches`' KEYS. `build_persona` calls `check_choice` and stores the draft's id in place of Task 1's `None`.
  - **settings.rs:46-48**: after `TEAM_LEAD`, refuse `room::AGENT_ACCOUNTS` with "Agent accounts are changed with accounts.*." (this is the only place).
  - **socket.rs**: the field in all five Outbox literals. A spawned arm next to `DesksLink` for the seven commands above → `commands::accounts::run(command, &desk.log, &desk.room, seat == Seat::Desk)`; `accountId` has no lane, and status, a copy or a slow policy lock must not stall the read loop. `AccountsLoginStart` is spawned too (`registry::install` may fetch an archive): `room.accounts_login_start(.., sender.cancel.clone(), seat == Seat::Desk)`, then on Ok insert `account_id → id` into `account_logins` and answer `{id}`. Poll, input and cancel mirror :555-598 against `account_logins`, refusing "That sign-in belongs to another desktop connection."; input and cancel answer null.
  - **roster.rs/subscribe.rs**: `view` and `send_snapshot` take `seat`; `shaped` removes `persona.agentAccountId` and `session.account` when `seat == Seat::Phone || sender.lean()` (subscribe.rs:229's predicate), before the existing early return for non-lean rows.
- [ ] **Step 4: Run tests to verify they pass.** Run `cargo test -p hotline-core --lib --locked wire::tests`, `cargo test -p hotline-core --lib --locked mcp::server::tests::list_teammates_never_carries_an_agent_account`, `cargo test -p hotline-room --locked`, then `cargo clippy -p hotline-core --all-targets --locked -- -D warnings`. Expected: PASS and clean. If clippy says lib.rs's `expect(dead_code)` on `mod accounts` is unfulfilled, nothing there is dead any more: remove it here and skip Task 9's Step 5.
- [ ] **Step R (Review Focus 1 and 2): nothing to copy, and removal during a sign-in**

Add to the Task 8 wire tests (real Desk on a TempDir, `HOME`/`USERPROFILE` unaffected: point the system-default source at an empty TempDir through the test seam Task 7 exposes):
```rust
#[tokio::test] async fn creating_from_a_missing_setup_writes_no_account_and_no_folder() {
  // accounts.create {startWith:"system_default"} with no source → Err("There is no Claude Code setup on this computer to copy.");
  // accounts.list → no account; <data>/agent-accounts has no new entry. startWith:"empty" then succeeds.
}
#[tokio::test] async fn an_account_whose_sign_in_is_running_cannot_be_removed() {
  // seed AccountLogins with a running attempt (test seam from Task 6) → accounts.remove Err("Finish or cancel {account}'s sign-in first.");
  // the setting still lists the account and the attempt is still running.
}
```
Implement: `accounts.remove` checks `room.account_logins().view(&id)` for `Running` before the in-use check; exact sentence "Finish or cancel {account}'s sign-in first.". Run: `cargo test -p hotline-core --lib wire::tests` — Expected: PASS.

- [ ] **Step L: A linked desk's roster row never carries an account**

Add to `crates/hotline-room/src/linked.rs` `mod tests`:
```rust
#[test] fn a_linked_roster_row_never_carries_an_agent_account() {
  // Build the published row (the same helper `a_published_roster_is_cut_down_to_what_a_roster_row_holds` uses) from a
  // persona with agent_account_id = Some(A); serde_json::to_string(&row) contains neither "agentAccountId" nor A.
}
```
Run: `cargo test -p hotline-room --lib a_linked_roster_row_never_carries_an_agent_account` — Expected: PASS (the row type has no such field; this pins it).

- [ ] **Step 5: Commit.** `git add crates/hotline-core/src crates/hotline-room/src/room_handle.rs`, then `git commit -m "Agent accounts are the desk's and the owner's alone: a sign-in belongs to the socket that started it, only accounts.* writes the setting, persona.update cannot set one, and no phone or colleague is shown one"`.

---

### Task 9: Account switching on Room

**Files:**
- Create: `crates/hotline-core/src/accounts/switch.rs` (pure, tests in the same file)
- Modify: `crates/hotline-core/src/accounts.rs` (`pub(crate) mod switch;`) and `crates/hotline-core/src/accounts/login.rs` (the last line of Task 6's `signed_in`)
- Modify: `crates/hotline-core/src/session/accounts.rs` (Task 8's file: the `impl Room` below)
- Modify: `crates/hotline-core/src/session.rs`: the `account_switches` field and its init (beside Task 8's two), `:856-861` (`info()`), `:878-896` (`list_subagent`)
- Modify: `crates/hotline-core/src/session/agent.rs:236-247` (`thread_agent`), `session/start.rs:85` (`start_now`)
- Modify: the `rest` hooks at `session/dm.rs:194-201` (both branches), `session/sides.rs:376-379`, `session/job_thread.rs:457-462`
- Modify: `crates/hotline-core/src/session/peers.rs:276-281` (`answering_for`) and `:507-510` (`Answering::drop`)
- Modify: `crates/hotline-core/src/desk.rs` (`persona_account`, beside Task 8's methods) and `crates/hotline-core/src/lib.rs` (Task 4's `expect(dead_code)` on `mod accounts`)
- Test: `crates/hotline-core/src/session/tests/accounts.rs` (Task 5's file; append), `accounts/switch.rs`, `crates/hotline-core/src/wire/tests/owner_handler.rs`

**Interfaces:**
- Consumes:
  - `Session.{account_id, account_login_at}`, set in `start_now` from `agent.view` (Task 5)
  - `room::{persona_write_lock, append_persona, clear_backend_checkpoints, agent_accounts, write_agent_accounts, checkpoint_session}` (Task 2)
  - `AccountLogins::view`, `#[cfg(test)] AccountStatuses::put`, `login::signed_in(room, account)` (Task 6)
  - `accounts::check_choice`, `Room::{account_statuses, account_logins}`, `RoomHandle::persona_account` (Task 8)
  - existing: `Room::{this (lifecycle.rs:56), working, start_gate, invalidate, reattach_held, sides, working_jobs, subagents}`
- Produces:
  - SHEET DEVIATION: `assign_account`, `apply_account_when_idle` and `account_signed_in` live in `session/accounts.rs`, because they read private `Room` fields; `accounts/switch.rs` keeps the pure part.
  - `pub(crate) async fn Room::assign_account(self: &Arc<Self>, persona_id: &str, account_id: Option<String>) -> Result<SessionAccount, String>`
  - `pub(crate) fn Room::apply_account_when_idle(&self, persona_id: &str)`; `pub(crate) fn Room::account_signed_in(&self, account_id: &str)`
  - `switch::PendingAccount { from: Option<String>, wanted: Option<String>, relogin: bool }`, `switch::hold(&AccountStatusView, Option<&LoginAttemptView>) -> Option<AccountHold>`, `switch::session_account(running: Option<String>, pending: Option<&PendingAccount>, held: Option<AccountHold>) -> Option<SessionAccount>`; the hold is never stored, it is read live (§10.4)
  - `SessionInfo.account` on `info()` and every `info_changes` send

- [ ] **Step 1: Write the failing tests.** `on_accounts(name, agents) -> (Arc<Room>, String /*A*/, String /*B*/)` writes two `claude-acp` accounts with `login_at: Some(1)`, enrols `Persona { backend_id: "claude-acp".into(), agent_account_id: Some(a), ..super::persona("ada") }` and builds `Room::with_agents(log, Arc::new(DeskKeys), agents)`. "Started" means `room.start("ada")` and `until_state(.., Ready)`. Uses `Fake`, `Scripted`, `saying`, `until`.
```rust
// accounts/switch.rs
#[test] fn a_running_attempt_holds_as_signing_in_and_signed_out_as_not_signed_in()
// hold(SignedOut, None) == Some(NotSignedIn); hold(SignedIn, Some(Running)) == Some(SigningIn);
// hold(NotChecked | Unavailable | SignedIn, None or Some(Failed)) == None
#[test] fn the_session_account_shows_a_switch_until_it_applies()
// session_account(None, None, None) == None; (Some(A), None, None) == {A, A, None};
// (Some(A), Some(&{from A, wanted B}), Some(NotSignedIn)) == {A, B, NotSignedIn}
// session/tests/accounts.rs
#[tokio::test] async fn a_switch_during_a_dm_turn_waits_for_the_turn_and_then_applies() {
    // Scripted::new(saying("1","Working.")).gated(gate); started; prompt
    assert_eq!(room.assign_account("ada", Some(b.clone())).await.unwrap(),
               SessionAccount { running: Some(a.clone()), wanted: Some(b.clone()), held: None });
    assert_eq!(agents.views().len(), 1);
    gate.add_permits(10);
    until("switched", || room.info("ada").account.and_then(|x| x.running) == Some(b.clone())).await;
    assert_eq!(agents.views().last().unwrap().agent_account_id, Some(b));
}
#[tokio::test] async fn a_switch_waits_for_a_side_thread_mid_turn()
// started; room.start_side("ada","Task") gated; until sides("ada")[0].working; assign B → running A;
// a second start_side → views().last().agent_account_id == Some(a); release; until info.account.running == B
#[tokio::test] async fn keeping_the_running_account_clears_the_switch_and_keeps_checkpoints()
// checkpoint_session(log,"ada","claude-acp",Some(&a),"s-a"); agents.reporting("s-a", true); started; DM turn gated;
// assign B, then A → {A, A, None}; release; until Ready; views().len() == 1; the record still has (claude-acp, A, "s-a")
#[tokio::test] async fn a_held_account_waits_for_its_sign_in()
// started; account_statuses().put(&b, SignedOut); assign B → {A, B, Some(NotSignedIn)}; record says B; views().len() == 1
// write B.login_at = Some(2); put(&b, SignedIn) (Task 6 refreshes status first); room.account_signed_in(&b); until running == B
#[tokio::test] async fn applying_clears_the_backends_checkpoints_and_the_next_start_is_new()
// checkpoints (claude-acp, A, "s-a") and (claude-acp, B, "s-b-old"); started; assign B → {B, B, None} at once;
// no claude-acp checkpoint on the record; views().last().session_checkpoints is empty
#[tokio::test] async fn stop_and_switch_now_starts_the_new_account_fresh()
// same checkpoints; started; DM turn gated; assign B (pending); room.stop("ada"); room.start("ada");
// until views().last() is on B with no claude-acp checkpoint
#[tokio::test] async fn a_stale_lease_cannot_write_a_checkpoint_after_the_switch()
// agents.reporting("s-a", false); started; let old = room.session("ada").unwrap();
// assert_eq!(lock(&old.pending_checkpoint).as_deref(), Some("s-a")); assign B (applies); room.checkpoint(&old);
// the record has no checkpoint "s-a"
#[tokio::test] async fn signing_in_again_rebuilds_sessions_that_started_before_it()
// started on A (login_at 1); write A.login_at = Some(2); room.account_signed_in(&a); until views().len() == 2; running still A
#[tokio::test] async fn an_assignment_is_checked_before_anything_is_written()
// "ghost" → "There is no such teammate in this room."; a fresh uuid → "That account is not on this desk.";
// a "hotline" teammate + A → "That agent does not use accounts."; the records are unchanged
// wire/tests/owner_handler.rs (a real Desk::open_with_store)
#[tokio::test] async fn persona_account_reaches_the_room_through_the_real_handler()
// accounts.create (empty) → id; persona.create {backendId:"claude-acp"}; persona.account {accountId:id} → ok, result.wanted == id,
// the record carries id; persona.account {accountId:null} → the record has none
```
- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p hotline-core --lib --locked session::tests::accounts`. Expected: a compile error: no method `assign_account` or `account_signed_in` on `Room`, and no module `switch`.
- [ ] **Step 3: Implement**
  - **switch.rs**: `hold` is `SigningIn` for a `Running` attempt, else `NotSignedIn` for `SignedOut`, else None (so `NotChecked` never holds). `session_account`: an entry gives `{running, wanted: p.wanted, held}`; no entry and Some(running) gives `{running, running, None}`; otherwise None.
  - **The live hold**: a private `Room::account_hold(wanted: Option<&str>) -> Option<AccountHold>` = `hold(&account_statuses().cached(w), account_logins().view(w).as_ref())`, None for System default. Every reader below ("unheld", `info()`) asks it, so a hold ends the moment the status or the attempt does.
  - **`assign_account`**:
    1. `self.persona(pid)?` (a linked teammate is not on this roster), then `check_choice(&self.log, &record.backend_id, account_id.as_deref())?`.
    2. Under `policy_update_lock`: inside `{ room::persona_write_lock() }`, re-read the record and set `agent_account_id`/`updated_at`, appending only if it changed. `current` is the DM session's `account_id`, else the entry's `from`, else the record's previous value. If `account_id == current`, remove the entry unless it is a relogin (Keep: nothing rebuilt, no checkpoint touched). Otherwise insert `{from: current, wanted, relogin: false}`.
    3. Release the policy lock, publish `info_changes.send(self.info(pid))`, and if the entry is unheld, `self.apply_pending_account(pid).await?`. The apply never runs under the policy lock, because `in_this_chapter` takes the gate and then that lock (prompt.rs:36 → chapters.rs:796).
    4. Answer `self.info(pid).account`, or `{running: None, wanted: account_id, held: None}` when there is none.
  - **`apply_pending_account`** (private, async): `self.working()?`, then hold `start_gate(pid)`. Return unless there is an unheld entry and `account_idle(pid)`; the entry stays, and the end of the next turn brings it back. Remove it, `self.invalidate(pid)?` first so the old lease dies (`checkpoint` checks `is_current`, session.rs:1052), then `room::clear_backend_checkpoints(log, pid, backend)` unless a relogin (Task 6 already cleared that account's), then `self.reattach_held(pid).await` and publish.
  - **`account_idle`**: no DM session or `!lock(&session.turns).running`; `sides(pid)` none `working`; `working_jobs(pid)` and `subagents(pid)` empty; `!self.peers.answering_for(pid)`, which is `answering.iter().any(|(_, target)| target == pid)`.
  - **`apply_account_when_idle`** (sync): when there is an unheld entry and `account_idle`, spawn `apply_pending_account` on `self.this()`. Called from each `rest` (dm.rs: both the stopped and the resting branch), from `list_subagent(.., false)`, and from `Answering::drop`, which reads its target from the `(key, target)` pair before the `retain`.
  - **`account_signed_in`**: read `login_at` from `agent_accounts`; for DM sessions with `account_id == Some(id)` and `account_login_at < login_at` and no entry, insert `{from: id, wanted: id, relogin: true}`; then publish and `apply_account_when_idle` for every teammate whose entry wants this account (held ones included, whose hold the refreshed status has ended) and every relogin just inserted. Task 6's `signed_in` gains `room.account_signed_in(&account.id);` as its last line; that is the only caller outside tests.
  - **`thread_agent` (agent.rs:236-247)**: after the destructure, an entry for the teammate sets `persona.agent_account_id = entry.from`. Every thread (DM, side, job, run, pair) is built here, so nothing starts on the wanted account before the switch applies.
  - **`start_now:85`** (gate held): an unheld entry is consumed when `account_idle`, its checkpoints cleared as in the apply, before `self.persona()` is read; so Stop then Start never resumes a stale checkpoint. A held or busy entry stays, and `thread_agent` keeps `from`.
  - **`info()`**: copy `(info, session.account_id)` out of the `sessions` lock, release it, then clone the entry out of `account_switches` and set `info.account = session_account(running, entry.as_ref(), self.account_hold(wanted))`; no two of these locks are ever nested.
  - **desk.rs**: `persona_account` → `self.room.assign_account(persona_id, account_id).await`.
- [ ] **Step 4: Run tests to verify they pass.** Run `cargo test -p hotline-core --lib --locked -- session::tests::accounts accounts::switch`, `cargo test -p hotline-core --lib --locked wire::tests::owner_handler`, then `cargo test -p hotline-core --lib --locked session` (the turn, side, job and computer tests still pass with the `rest` hooks). Expected: PASS.
- [ ] **Step 5: Remove the dead-code expectation.** Delete Task 4's `#[cfg_attr(not(test), expect(dead_code, ..))]` from `mod accounts` in lib.rs, then run `cargo clippy -p hotline-core --all-targets --locked -- -D warnings`. Expected: clean. An item it reports as dead has no caller: delete it, don't allow it.
- [ ] **Step R (Review Focus 3): a pending switch for a teammate that goes away**

Append to `crates/hotline-core/src/session/tests/accounts.rs`:
```rust
#[tokio::test] async fn a_pending_switch_is_dropped_when_its_teammate_is_removed_or_changes_agent() {
  // Ada mid-turn on account A, assign B → pending. Delete Ada (persona.delete) → pending entry gone, no apply task touches her,
  // no panic. Second case: Bob pending A→B, persona.update backendId "hotline" → pending dropped, SessionInfo.account None.
}
```
Implement: the pending-switch map is cleared for a persona in `Room::invalidate`'s delete path and whenever the persona's backend no longer has a support row. Run: `cargo test -p hotline-core --lib session::tests::accounts` — Expected: PASS.

- [ ] **Step 6: Commit.** `git add crates/hotline-core/src`, then `git commit -m "A teammate's account changes only when all its work is idle, as a policy change that clears that backend's checkpoints; Keep and a hold rebuild nothing, and a new sign-in restarts the sessions that began before it Fresh"`.

---

### Task 10: Agent-accounts harness (fixture `--print-env`, `tests/agent_accounts.rs`)

**Files:**
- Modify: `crates/hotline-core/src/bin/hotline-acp-fixture.rs`. Task 6 has already added the `--cli auth status|login` modes; keep them. The module doc (`:1-9`) also names `--print-env` and `REPORT_ENV`. The `--print-env` branch goes first in `main`, before Task 6's `--cli` branch. The `REPORT_ENV` check becomes the first line of `answer` (`:142`, before `calls`). The new fns go at the end of the file.
- Modify: `crates/hotline-core/tests/common/acp_fixture.rs:17` (`fn built()` → `pub fn built()`; `common/mod.rs:54-58` already allows its dead code)
- Create: `crates/hotline-core/tests/agent_accounts.rs` (one binary, `mod common;`, no `#![cfg]` gate)
- Test: `crates/hotline-core/tests/agent_accounts.rs`. `CALL` regression: `crates/hotline-core/tests/threads.rs`, `crates/hotline-core/tests/mcp.rs`

**Interfaces:**
- Consumes (wire, sheet names):
  - `accounts.create {backendId:"claude-acp", name, startWith:"empty"}` → `AccountCreated` (the id is at `result.account.account.id`)
  - `accounts.list {}` → `AccountsList` (`accounts[].account.id`, `.usedBy`, `.status.state`)
  - `accounts.remove {accountId}`
  - `accounts.status {accountId}` → `AccountStatusView`
  - `persona.create {draft:{name, goal, cwd, backendId:"claude-acp", agentAccountId?}}`
  - `persona.account {personaId, accountId|null}`
  - `session.start`, `session.prompt`, `session.stop`
- Consumes (Rust): `hotline_core::paths::agent_account_dir`, `hotline_core::room::write_agent_accounts`, `hotline_core::log::Log::open` (log.rs:132), `hotline_core::driver::acp::end_every_agent()` (acp.rs:153).
- Consumes (Task 6's fixture CLI): `auth status` answers `loggedIn` when `<CLAUDE_CONFIG_DIR>/.credentials.json` exists (with `FIXTURE_AUTH` unset). It also writes `<CLAUDE_CONFIG_DIR>/fixture-status-env.json`, naming the variables it saw. `FIXTURE_LOGIN_MODE` and `FIXTURE_AUTH` steer it.
- Consumes (Task 5): when `resolve` refuses a Desk-seat `session.start` or `session.prompt`, the answer is the private `account_unavailable` notice. It carries `accountId` and `code` and no path.
- Sentences: the sheet's private-unknown and remove-in-use sentences, with `{agent}` = "Claude Code" (registry.rs:188-194).
- Helpers:
  - `common::{open_desk, Client, until, ANSWER}`.
  - `acp_fixture::install(root)` writes a fresh catalogue without `claude-acp`. So `claude-acp` falls back to `npx -y @agentclientprotocol/claude-agent-acp@0.78.0` (registry.rs:296-313, :582-590), and nothing touches the network.
  - Patterns to copy: the door from `tests/threads/together.rs:165-176`; tape reading from `:232-245`; re-exec from `src/driver/acp/auth/tests.rs:294-358`; reopen from `tests/desk/teammates.rs:359-361`.
- Produces (fixture argv):
  - `--print-env NAME...` prints `NAME=value` or `NAME unset` for each name and exits 0.
  - A prompt line containing `REPORT_ENV NAME...` gets one message:
    - one `ENV NAME=value` or `ENV NAME unset` line per name;
    - then `CHILD_ENV ` followed by each line that `current_exe() --print-env NAME...` printed, or `CHILD_ENV failed: <error>`.
  - Every other prompt is answered byte for byte as before.
- Produces: `pub fn acp_fixture::built() -> PathBuf`, and the eight test names below, which Task 13 cites.
- Note for Task 6: `cargo test --lib` builds no bin target, so a test that spawns the fixture belongs in an integration binary.

- [ ] **Step 1: Write the failing test** in `tests/agent_accounts.rs`: its helpers and the first scenario. Make `acp_fixture.rs:17` `pub fn built()` now, so the test compiles.
```rust
mod common;
const TOKEN: &str = "disposable-agent-accounts-test";
const CASE: &str = "HOTLINE_TEST_AGENT_ACCOUNTS"; // set only in the re-exec'd child, to the sentinel dir
const INHERITED: [(&str, &str); 6] = [("ANTHROPIC_API_KEY","fake-api-key"), ("ANTHROPIC_AUTH_TOKEN","fake-auth-token"),
  ("CLAUDE_CODE_OAUTH_TOKEN","fake-oauth-token"), ("ANTHROPIC_BASE_URL","https://inherited.example.invalid"),
  ("ANTHROPIC_CUSTOM_HEADERS","X-Inherited: fake"), ("CLAUDE_CODE_USE_BEDROCK","1")];
async fn isolated(name: &str) -> Option<PathBuf>; // parent: re-runs `name` alone, asserts success, None; child: Some(sentinel)
struct TestDesk { desk: Arc<hotline_core::desk::Desk>, client: Client, log: Log, root: PathBuf, door: JoinHandle<()> } // open, account, teammate(name, Option<&str>), report, close
struct Report { env: BTreeMap<String, Option<String>>, child: BTreeMap<String, Option<String>> }
fn folder(root: &Path, id: &str) -> PathBuf; // dunce::canonicalize(paths::agent_account_dir(root, id).unwrap())
fn same(reported: &Option<String>, dir: &Path) -> bool; // dunce::canonicalize both sides

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_and_the_system_default_start_at_once_each_in_its_own_folder() {
    let Some(sentinel) = isolated("two_accounts_and_the_system_default_start_at_once_each_in_its_own_folder").await else { return };
    // A, B; Ada on A, Bob on B, Cy on None; send session.start ×3, then reply ×3: ok && state != "error"
    // session.prompt ×3 "REPORT_ENV CLAUDE_CONFIG_DIR <INHERITED keys>"; each tape's first agent text starting "ENV "
    assert!(same(&ada.env["CLAUDE_CONFIG_DIR"], &folder(&root, &a)) && same(&bob.env["CLAUDE_CONFIG_DIR"], &folder(&root, &b)));
    assert_ne!(folder(&root, &a), folder(&root, &b));
    assert!(same(&cy.env["CLAUDE_CONFIG_DIR"], &sentinel));
    for (key, value) in INHERITED { assert_eq!(ada.env[key], None); assert_eq!(bob.env[key], None); assert_eq!(cy.env[key].as_deref(), Some(value)); }
    assert_eq!(std::env::var_os("CLAUDE_CONFIG_DIR"), Some(sentinel.into_os_string())); // the desk never set_var'd
    hotline_core::driver::acp::end_every_agent();
}
```
- [ ] **Step 2: Run it to verify it fails.** Run `python scripts/fetch-speech-engine` once, then `cargo test -p hotline-core --locked --test agent_accounts two_accounts -- --nocapture`. Expected: FAIL. The parent's assert prints the child's stderr, `never: Ada reports its environment`, because the fixture answers "On it.".
- [ ] **Step 3: Implement.**
  - **Fixture, first in `main`:**
    ```rust
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--print-env") { print_env(&args[1..]); return; }
    // then Task 6's `--cli` branch, then the ACP agent as before (under the npx shim argv starts `-y <pkg>`)
    ```
  - **Fixture fns:**
    - `fn print_env(names: &[OsString])`. Values go through `to_string_lossy`.
    - `fn reported(said: &str) -> Option<Vec<String>>`. It takes the first line containing `"REPORT_ENV "` and splits it on whitespace after the token.
    - `async fn report(names: &[String]) -> String`. Its child is tokio `Command::new(current_exe()?).arg("--print-env").args(names).stdin(Stdio::null()).output()`, plus `#[cfg(windows)] .creation_flags(0x0800_0000)` (CREATE_NO_WINDOW).
  - **`answer`'s first line:** `if let Some(names) = reported(said) { return report(&names).await; }`.
  - **`isolated`, parent half.** It is the child exactly when `CASE` is set.
    - It makes two `tempfile::tempdir()`s, one for the shims and one for the sentinel. It copies `acp_fixture::built()` to `npx{EXE_SUFFIX}` and `claude{EXE_SUFFIX}` in the shim dir.
    - It runs `tokio::process::Command::new(current_exe())` with:
      - `.args(["--exact", name, "--nocapture"])`
      - `.env(CASE, &sentinel)` and `.env("CLAUDE_CONFIG_DIR", &sentinel)`
      - `.envs(INHERITED)`
      - `.env("PATH", join_paths(once(shims).chain(split_paths(&old))))`
      - `.env_remove("FIXTURE_LOGIN_MODE")` and `.env_remove("FIXTURE_AUTH")`
      - `.kill_on_drop(true)`
    - The run gets `timeout(ANSWER * 5)`. It asserts `status.success()` and prints stdout and stderr on failure.
  - **`TestDesk`:**
    - `open` calls `acp_fixture::install(root)`, then `Arc::new(open_desk(root)?)`, and keeps a clone of the `Arc` it hands to `Door::bind`.
    - `teammate` gives each persona its own `cwd` (`root/work/<name>`).
    - `close`, in order:
      1. Sends `session.stop` to the started teammates.
      2. Waits (`until`) for `desk.prepare_restart()` (session.rs:800: no work running, every stream synced) to answer `Ok`.
      3. Returns that guard for the scenario to hold, so the old desk starts nothing new.
      4. Calls `door.abort()` and drops the client.
- [ ] **Step 4: Run tests to verify they pass.** Run `cargo test -p hotline-core --locked --test agent_accounts -- --nocapture`. Expected: PASS. Then run `cargo test -p hotline-core --locked --test threads --test mcp`. Expected: PASS unchanged, so `CALL` is byte-identical.
- [ ] **Step 5: Commit.** `git add crates/hotline-core/src/bin/hotline-acp-fixture.rs crates/hotline-core/tests/common/acp_fixture.rs crates/hotline-core/tests/agent_accounts.rs`; `git commit -m "Two teammates on two accounts run at once, each agent and its children seeing only its own folder."`
- [ ] **Step 6: Add the remaining scenarios.** Each is a `#[tokio::test(flavor = "multi_thread")]` that opens with `isolated(..)` and ends with `end_every_agent()`.
  - `an_account_teammates_subprocesses_see_the_same_environment_as_the_agent`
    - Setup: Ada on A, Cy on System default; both report `CLAUDE_CONFIG_DIR` and the INHERITED keys.
    - Asserts: `assert_eq!(r.child, r.env)` for both, and Ada's child `CLAUDE_CONFIG_DIR` is `folder(A)`.
  - `an_account_choice_survives_reopening_the_desk`
    - Setup: Ada starts on System default; `persona.account {ada, a}` answers ok; `close()`; reopen on the same root.
    - Asserts: `accounts.list` gives A `usedBy == [ada]`; after `session.start`, the report shows `folder(A)`.
  - `removing_an_unused_account_keeps_its_folder`
    - Setup: write `folder(A)/.credentials.json` = `"left by a login"`; `accounts.remove` answers ok.
    - Asserts: A is no longer listed; the folder `is_dir()`; the file's bytes are unchanged.
  - `removing_an_account_in_use_is_refused_naming_its_teammates`
    - Setup: Ada and Bob are on A.
    - Asserts:
      - `remove` answers `ok == false`. The error starts with "Account 1 can't be removed while ", contains "Ada" and "Bob", and ends with " use it.".
      - The folder stays.
      - After `persona.account {ada,null}` and `{bob,null}`, the remove answers ok.
  - `a_teammate_set_to_an_account_not_on_this_desk_is_refused_and_never_falls_back`
    - Setup:
      - Ada is on A; `close()`.
      - Offline: `room::write_agent_accounts(&Log::open(&root), &[])` and `remove_dir_all(folder(A))`.
      - Reopen.
    - Asserts:
      - `session.start` answers `ok == false`. The error contains "Pick a Claude Code account for Ada: the one it was set to is not on this desk.", `"kind":"account_unavailable"` and `"code":"unknown_account"`. It contains neither the root path nor `agent-accounts`.
      - `session.prompt "REPORT_ENV CLAUDE_CONFIG_DIR"` answers the same refusal.
      - Ada's tape holds no `ENV ` text, and `!agent_account_dir(&root, &a).unwrap().exists()`.
  - `an_account_under_a_data_directory_with_spaces_and_accents_runs`
    - Setup: root is `tempdir/"Conta Teste ção"`; Ada is on A.
    - Asserts:
      - The report's `CLAUDE_CONFIG_DIR` is `folder(A)` and contains "Conta Teste ção".
      - After writing `{"email":"ada@example.invalid"}` to `folder(A)/.credentials.json`, `accounts.status` gives `state == "signed_in"`.
  - `status_says_signed_in_only_for_the_account_the_agent_confirms`
    - Setup: `.credentials.json` goes in A's folder only.
    - Asserts:
      - `accounts.status` gives A `"signed_in"` and B `"signed_out"`.
      - Each result's keys are a subset of `{state, checkedAt, email, plan}`.
      - `accounts.list` shows A's cached `signed_in`.
      - `folder(A)/fixture-status-env.json` contains `"CLAUDE_CONFIG_DIR"` and none of the quoted INHERITED names. This proves the status run gets the account's scrubbed env through the real desk (spec §14 row 2).
- [ ] **Step 7: Run them.** Run `cargo test -p hotline-core --locked --test agent_accounts -- --nocapture`. Expected: 8 PASS. They prove Tasks 2-9 end to end. A failure is a bug in the task that owns that behaviour, and is fixed there, not here.
- [ ] **Step 8: Commit.** `git commit -am "An account outlives a reopened desk, keeps its folder when removed, and is never swapped for System default."`

**Platforms:**
- All eight tests run on Windows, macOS and Linux, including the Windows CI leg. This differs from `acp_startup.rs` and `acp_shutdown.rs`, which are `#![cfg(unix)]`.
- Shim lookup:
  - Shim names carry `EXE_SUFFIX`.
  - On Windows, `which` (registry.rs:828-849) tries each `PATHEXT` suffix one directory at a time. So the shim dir, first on PATH, beats a real `npx.cmd` or `claude.exe`.
  - `claude` is only looked up (`adapter_missing`, registry.rs:575-580); it never runs.
  - On Unix, `fs::copy` keeps the exec bit.
- PATH is built with `join_paths`, never by concatenation. Nothing calls `set_var`: the sentinel and the fake keys exist only in the re-exec'd child.
- Paths are compared through `dunce::canonicalize` on both sides, because of Windows 8.3 temp names and macOS `/var` → `/private/var`.
- On Windows a running fixture locks `npx.exe`, so each scenario ends with `end_every_agent()`. `TempDir` drop errors are ignored.
- Login never runs here, because a Windows sign-in opens a real console. Status is driven by planting `.credentials.json`; Task 6 proves the login process.

---

### Task 11: Settings › Agent accounts, and Open through the desk's shell

**Files:**
- Create: `ui/src/components/settings/AgentAccounts.tsx`, `ui/src/components/settings/AccountSetup.tsx`, `ui/src/components/accounts/AddAccountForm.tsx`, `ui/src/components/accounts/AccountSignIn.tsx`, `ui/src/components/accounts/words.ts`, `crates/hotline-app/src/account_setup.rs`
- Modify: `ui/src/components/AgentSignIn.tsx:14-142`, `ui/src/components/Settings.tsx:28-55, 174-198`, `ui/src/App.tsx:556`, `ui/src/native.ts:393-397`, `ui/src/index.css:2997-3017`, `ui/design.md:122-129` and its Log (:327-337)
- Modify: `crates/hotline-app/src/lib.rs:29, 398-399, 428-429`, `crates/hotline-app/build.rs:33`, `crates/hotline-app/permissions/main.toml:15`, `crates/hotline-app/Cargo.toml:25-50` (`dunce = "1.0.5"`, the version core and tools lock), `crates/hotline-core/src/lib.rs:15-16`, `Cargo.lock`
- Test: `ui/tests/agent-accounts.test.tsx`, `ui/tests/agent-account-setup.test.tsx`, `ui/tests/settings.test.tsx`, `mod tests` in `crates/hotline-app/src/account_setup.rs`; `crates/hotline-app/tests/window_capabilities.rs` (unchanged) keeps the two handler lists, the app manifest and `main-commands` equal

**Interfaces:**
- Consumes: Task 1's contract.ts (`AccountsList`, `AccountAgent`, `AccountView`, `AccountStatusView`, `AccountStatusState`, `AccountCreated`, `AccountSetupView`, `SetupCopyReport`, `Persona.agentAccountId?`) and every wire.ts `Results` row, `agent.auth.poll` and `accounts.login.poll` both answering Task 1's `SignInStatus`; Task 3's `private_dir::checked_account_dir(root: &Path, id: &str) -> io::Result<PathBuf>`; Task 6's last attempt in `AccountView.attempt`; Task 7's `accounts.setup.list` paths (no side effect, a folder's path ends in `/`); Task 8's `AccountView.folder` and `canSignInHere`.
- Produces:
  - `account_setup.rs`: `fn setup_item(root: &Path, path: &str) -> Result<PathBuf, String>`; `#[tauri::command] pub fn open_account_setup(app: tauri::AppHandle, path: String) -> Result<(), String>`. hotline-core lib.rs:15 becomes `pub use hotline_support::{credentials, log_file, paths, private_dir, room_lock};` (and :16 drops `private_dir`).
  - `native.ts`: `export async function openAccountSetup(path: string): Promise<void>`
  - `AgentSignIn.tsx`: `export type SignInJob = { start(): Promise<{ id: string }>; poll(id: string): Promise<Commands["agent.auth.poll"]["result"]>; input(id: string, text: string): Promise<void>; cancel(id: string): Promise<void> }`; `export function SignInTerminal({ job, onDone, onClose, windowNote, cancelLabel = "Cancel sign-in" }: { job: SignInJob; onDone(): void; onClose(): void; windowNote?: string | undefined; cancelLabel?: string })`
  - `accounts/AccountSignIn.tsx`: `export function accountJob(accountId: string, email?: string): SignInJob`; `export function AccountSignIn({ account, agentName, email, cancelLabel = "Cancel", onEnd }: { account: AccountView; agentName: string; email?: string | undefined; cancelLabel?: string; onEnd(signedIn: boolean): void })`
  - `accounts/AddAccountForm.tsx`: `export function AddAccountForm({ backendId, agentName, takesEmail, taken, onAdded, onCancel }: { backendId: string; agentName: string; takesEmail: boolean; taken: string[]; onAdded(account: AccountView, email: string | undefined): void; onCancel(): void })` (Task 12 adds `compact?`)
  - `accounts/words.ts`: `statusWords(state: AccountStatusState): string`, `whoLine(status: AccountStatusView): string`, `nextAccountName(taken: string[]): string`, `SETUP_CATEGORIES: readonly { id: string; choice: string; item: string; noun?: string }[]`
  - `export function AgentAccountsSection({ roster, onBack }: { roster: RosterEntry[]; onBack?: (() => void) | undefined })`; `export function AccountSetup({ accountId, agentName }: { accountId: string; agentName: string })`; `SettingsSection` gains `"accounts"`, `Settings` gains `roster: RosterEntry[]`.
- SHEET DEVIATION: `SignInTerminal` also takes `onClose`, `windowNote?`, `cancelLabel?` (AgentSignIn.tsx:49-54, :140; spec §7.3.4); `AddAccountForm` also takes `taken` and `onCancel` and only creates (the caller owns the sign-in); the window holds the category titles because `AccountAgent` carries none, and the email label says "{agent}", never "Claude" (spec §9.1.1).

- [ ] **Step 1: Write the failing tests**

  `account_setup.rs` `mod tests`, laid out like files.rs:96-166:
```rust
// root = tempdir()/"Conta Teste ção"; dir = root/agent-accounts/ID (create_dir_all); ID = "0b7e2c55-3f6a-4d8e-9c1a-5e4f3a2b1c0d"
// at(rest) = format!("{}/{rest}", dir.display()); REFUSED = "That is not part of an agent account's setup."
#[test] fn only_a_markdown_or_json_file_or_a_folder_directly_in_an_account_folder_opens() {
    std::fs::write(dir.join("CLAUDE.md"), "x").unwrap();
    assert_eq!(setup_item(&root, &at("CLAUDE.md")).unwrap(), dunce::canonicalize(dir.join("CLAUDE.md")).unwrap());
    for path in [at("../../secret.md"), at("run.bat"), at("rules/a.md"), root.join("agent-accounts").display().to_string(),
                 root.join("agent-accounts/not-a-uuid/CLAUDE.md").display().to_string()] {
        assert_eq!(setup_item(&root, &path).unwrap_err(), REFUSED, "{path}");
    }
    assert!(!dir.join("run.bat").exists() && !dir.join("rules").exists());
    #[cfg(unix)] { std::fs::write(root.join("secret.json"), "{}").unwrap();
        std::os::unix::fs::symlink(root.join("secret.json"), dir.join("settings.json")).unwrap();
        assert_eq!(setup_item(&root, &at("settings.json")).unwrap_err(), REFUSED); }
}
#[test] fn a_missing_item_is_made_before_it_opens_and_an_existing_one_is_never_rewritten() {
    assert_eq!(std::fs::read(setup_item(&root, &at("CLAUDE.md")).unwrap()).unwrap(), b"");
    assert_eq!(std::fs::read_to_string(setup_item(&root, &at("settings.json")).unwrap()).unwrap(), "{}\n");
    assert!(setup_item(&root, &at("skills/")).unwrap().is_dir());
    std::fs::write(dir.join(".claude.json"), r#"{"mcpServers":{}}"#).unwrap();
    assert_eq!(std::fs::read_to_string(setup_item(&root, &at(".claude.json")).unwrap()).unwrap(), r#"{"mcpServers":{}}"#);
}
```
  `ui/tests/agent-accounts.test.tsx` uses the remote-section.test.tsx skeleton: `installDom()`, `__hotlineDesk = { platform: "windows" }` (as settings.test.tsx:24 sets `version`), `replaceDesks([{ id: "local", name: "This computer", kind: "local", origin: "", token: "" }])`, `spyOn(wire, "command")` recording `calls` and answering from a `respond` table, `<AgentAccountsSection roster={roster} />` in `act`. Fixture: agents `claude-acp` "Claude Code" (supportsAccounts, takesEmail) and `codex-acp` "Codex" (neither); `a1` "Account 1" signed_in `dev@example.com`/`Max`, `loginAt` set, usedBy `["ada"]`; `a2` "Account 2" signed_out, no `loginAt`, usedBy `[]`; each `folder: "/data/agent-accounts/<id>"`, `canSignInHere: true`; roster Ada (claude-acp, `a1`) and Mack (claude-acp, none). After `accounts.create` answers `a3`, `accounts.list` answers with `a3` too, so its sign-in stands under its row.
```tsx
test("an agent with accounts opens its page; the others use their own sign-in") // expect(row("Claude Code").textContent).toContain("2 accounts · 1 not signed in"); expect(row("Codex").closest("button")).toBeNull(); expect(text()).toContain("Uses its own sign-in")
test("the agent page shows System default and each account's line") // expect(text()).toContain("Used by Mack"), ("Built in"), ("Signed in · dev@example.com · Max · Used by Ada"), ("Not signed in · Not used yet")
test("each status state reads as its words") // not_checked/checking/signed_in/signed_out/unavailable → "Not checked"/"Checking…"/"Signed in"/"Not signed in"/"Status unavailable"; expect(row("Account 2").querySelector(".agent-account-dot")).toBeNull() for not_checked
test("a blank name adds Account 3 and a Windows sign-in shows only Cancel")
  // expect(calls).toContainEqual(["accounts.create", { backendId: "claude-acp", name: "Account 3", startWith: "system_default" }]) and (["accounts.login.start", { accountId: "a3" }])
  // expect(text()).toContain("A Claude Code window opened for Account 3. Finish signing in there; Hotline checks when it closes."); expect(container.querySelector("[data-private-terminal]")).toBeNull()
  // Cancel → expect(calls).toContainEqual(["accounts.login.cancel", { accountId: "a3", id: "l1" }]); accounts.list called again
test("Customize what is copied sends the ticked; Empty sends none") // untick Settings → categories ["instructions", "skills", "plugins", "mcp"]; Empty → params toEqual({ backendId: "claude-acp", name: "Account 3", startWith: "empty" })
test("rename saves the trimmed name") // "  Reviewer seat " → ["accounts.rename", { accountId: "a1", name: "Reviewer seat" }]
test("remove is refused in place while Ada uses it, and frees an unused account")
  // a1: expect(text()).toContain("Account 1 can't be removed while Ada use it."); no accounts.remove
  // a2: expect(text()).toContain("Hotline forgets Account 2. Its folder and the sign-in in it stay at /data/agent-accounts/a2; sign out there with Claude Code's own sign-out if you no longer need it.")
  // Remove → ["accounts.remove", { accountId: "a2" }]; the band reads "Claude Code"
test("Check asks the agent; an owner seat signs in from the desk") // Check → ["accounts.status", { accountId: "a2" }]; canSignInHere false → "Sign in from the desk itself", no "Sign in" button
test("a second sign-in says its teammates start fresh; a first one does not")
  // a1 page → "Sign in again" → poll succeeded → expect(text()).toContain("Account 1 has a new sign-in. Its teammates start fresh once their work ends."); Back → row("Account 1") contains it
  // a2 → "Sign in" → succeeded → expect(row("Account 2").textContent).not.toContain("has a new sign-in")
test("an account's sign-in job speaks accounts.login.*") // accountJob("a1", "dev@example.com"): start/poll("l1")/input("l1","x")/cancel("l1") → .start {accountId, email}, .poll {accountId, id}, .input {accountId, id, input}, .cancel {accountId, id}
test("the next blank name skips a taken one") // expect(nextAccountName(["Account 1", "account 3"])).toBe("Account 4")
```
  `ui/tests/agent-account-setup.test.tsx` mounts `<AccountSetup accountId="a1" agentName="Claude Code" />` with the same DOM and mocks native as remove-teammate.test.tsx:14-22: `mock.module("../src/native", () => ({ ...native, openAccountSetup: async (path: string) => { opened.push(path); if (refuse) throw new Error("That is not part of an agent account's setup."); } }))`. `accounts.setup.list` answers `{ setup: { from: "system_default", at, categories: [...] }, categories }` with a path on all five: instructions 2 `/data/agent-accounts/a1/CLAUDE.md`, skills 6 `…/skills/`, plugins 3 `…/plugins/`, mcp 2 `…/.claude.json`, settings 1 `…/settings.json`.
```tsx
test("lists each item with its count under where it started") // "Started from your Claude Code setup on", "6 skills", "3 plugins", "2 servers", "CLAUDE.md"
test("Open stands only on the Desk seat and opens the item through the shell")
  // local desk: 5 "Open" buttons; "Open Instructions" → expect(opened).toEqual(["/data/agent-accounts/a1/CLAUDE.md"]); refuse = true → the Refusal shows the sentence; only a kind:"remote" desk → no "Open"
test("Copy again ticks all but Settings and copies the ticked") // Choose… → Settings unchecked; "Copy 4 items" → ["accounts.setup.copy", { accountId: "a1", categories: ["instructions", "skills", "plugins", "mcp"] }] → status "Copied 4 items from System default." and accounts.setup.list again
```
  `ui/tests/settings.test.tsx` gains `test("Agent accounts stands right after Providers")`: among the rail's button texts, `expect(names.indexOf("Agent accounts")).toBe(names.indexOf("Providers") + 1)`. The Import tab-order check at :51 stays.

- [ ] **Step 2: Run to verify they fail**
  - `cargo test -p hotline-app --lib account_setup` (without `--locked` this once: the dunce edge rewrites Cargo.lock). Expected: FAIL, E0425 cannot find function `setup_item`.
  - `(cd ui && bun test ./tests/agent-accounts.test.tsx ./tests/agent-account-setup.test.tsx ./tests/settings.test.tsx)`. Expected: FAIL, `Cannot find module '../src/components/settings/AgentAccounts'` (and `…/AccountSetup`), and the rail has no "Agent accounts".

- [ ] **Step 3: Implement**
  - **Shell.** `open_account_setup` is `setup_item(&data_root(), &path)`, then `app.opener().open_path(item.to_string_lossy(), None::<&str>)` as files.rs:63-65, failing with "It could not be opened: {error}". `setup_item`, in order (every refusal is "That is not part of an agent account's setup."):
    1. The path's last component is a plain name and its parent's last component is the account id; `private_dir::checked_account_dir(root, id)` passes and `dunce::canonicalize(parent)` equals it. (E0603 here means the core's `pub use` was missed.)
    2. An existing directory, or a missing path ending in `/` or `\`, is a folder. Any other name must end in `.md` or `.json` (ASCII case-insensitive): the system picks the program by the name, so nothing that runs is ever opened.
    3. A missing item is made in the account folder: a folder with `create_dir`, a file with `OpenOptions::new().write(true).create_new(true)` holding `"{}\n"` for `.json` and nothing otherwise. `AlreadyExists` leaves it untouched; any other error is "It could not be made: {error}". This is spec §10.7's "created before opening", for every row of an Empty account.
    4. `dunce::canonicalize(item)` has the account folder as its parent and is a directory when a folder was asked for, else a file whose own name ends in `.md` or `.json`, so a link out of the folder or to a program is refused. That canonical path is returned.
  - Register it: `mod account_setup;` at lib.rs:29, `account_setup::open_account_setup,` after `files::save_sent_file,` in both `generate_handler!` lists, `"open_account_setup"` after build.rs:33, `"allow-open-account-setup"` after main.toml:15. No capability change: a Rust-side `open_path` is not ACL-gated (files.rs:63).
  - **native.ts** (beside `openSentFile`, :393-397): `openAccountSetup` throws "Open the desktop application to open this." when `!isDesktop()`, else `invoke("open_account_setup", { path })`.
  - **AgentSignIn.tsx.** `SignInAttempt` (:49-142) becomes `SignInTerminal`: its wire calls become `job.*`, its effect depends on `[job]`, with `windowNote` it shows that sentence in place of the xterm host (no xterm import, no input queue, same polling), and its key reads `cancelLabel` while running and "Close" after a failure. `AgentSignIn` (:14-47) builds its `agent.auth.*` job in a `useMemo` on `[personaId, attempt]`; every caller memoizes its job, because the effect restarts when it changes. agent-sign-in.test.tsx stays unchanged and green.
  - **AccountSignIn** memoizes `accountJob` on `[account.account.id, email]`; its start sends `email` only when given. It passes `windowNote` when `!onServer() && platform() === "windows"` (Main.dc.html:141), else the xterm runs. Under either: "The browser uses whatever sign-in is already open there: use Switch account on the page, or open the link from the {agent} window in the right browser profile." (spec §7.3.7). Callers mount it only when `canSignInHere`, and otherwise show "Sign in from the desk itself".
  - **words.ts.** `whoLine` joins email and plan with " · ". `nextAccountName` returns the first "Account N", from N = `taken.length + 1`, that `taken` lacks case-insensitively. `SETUP_CATEGORIES` as id (choice / item, noun): instructions ("Instructions and rules" / "Instructions"), skills ("Skills, agents and commands" / "Skills", "skill"), plugins ("Plugins" / "Plugins", "plugin"), mcp ("MCP servers" / "MCP servers", "server"), settings ("Settings" / "Settings").
  - **AddAccountForm.** "Name" with placeholder `nextAccountName(taken)`; when `takesEmail`, "Email of the {agent} account (optional)", placeholder "name@example.com", hint "Fills in the email on the sign-in page; Hotline does not keep it." "Start with" radios "My {agent} setup" ("A copy of your own {agent} setup, without the sign-in, keys or history") and "Empty" ("{agent}'s defaults; add what this account needs afterwards"); under My setup, "Customize what is copied…" / "Hide what is copied" shows one ticked checkbox per category. Foot: Cancel and primary "Add and sign in" ("Adding…"). Submit sends `{ backendId, name: name.trim() || placeholder, startWith }`, plus `categories` only for `system_default` with a box unticked, then `onAdded(created.account, email.trim() || undefined)`; a refusal shows in `<Refusal>`.
  - **AgentAccountsSection** takes the Providers shape (list Providers.tsx:137-221, page header :384-418) and loads `accounts.list` on open (biome-ignore reason as Providers.tsx:78) and after every create, rename, remove and sign-in end.
    - Level 1 (band "Agent accounts"): one row per agent, initial avatar and name; a `group-row-choice` with chevron and "N account(s)[ · M not signed in]" (M counts `signed_out`, warn ink when M > 0; "No accounts yet" for none) when `supportsAccounts`, else a plain row. Hint "Hotline never sees a password or token: each agent's CLI keeps its own sign-in."
    - Level 2 (BackKey; band = agent): accent "Add account" opens the form in place; "System default" with "{agent}'s own sign-in on this computer, as before · Used by …" (roster entries on that backend without `agentAccountId`) and a "Built in" chip; one row per account: `.agent-account-dot[data-state]` (none for not_checked), name, and "Sign-in did not finish: {error}" for a failed last attempt, else status words [+ " · " + whoLine when signed_in] + " · Used by {names}" or " · Not used yet". While this window signs it in, the detail is "Signing in…" with `AccountSignIn` under the row. Hint "{agent} keeps each account's sign-in, history and settings in its own folder."
    - Level 3 (BackKey; band = account): `nt-card` header with initial, name, quiet "Rename" (field + "Save"; Enter saves, Escape cancels, blank keeps the name) and "{status} · {agent}"; "Sign-in" group (whoLine or status words; "Checked with {agent} · {relativeTime(checkedAt)}" (dock.ts:178), or for signed_out "Teammates on this account do not start until it is signed in"; quiet "Check" reading "Checking…"); "Used by" chips or "No teammate uses it yet. Pick it in a {agent} teammate's settings."; `<AccountSetup>`; "Folder" (when set) in mono with "Show folder" (`showPath`) and hint "{agent} keeps this account's sign-in, history and settings here."; foot quiet danger "Remove account" and primary "Sign in" / "Sign in again" (signed_in), or the text "Sign in from the desk itself".
    - Remove opens a row in place: in use → the refusal from the test, "Give them another account in their settings first; Hotline never moves a teammate to another account for you." and "OK"; free → "Remove {account}?", "Hotline forgets {account}. Its folder and the sign-in in it stay at {folder}; sign out there with {agent}'s own sign-out if you no longer need it." ("on the desk" in place of "at {folder}" when `folder` is absent), Cancel and danger "Remove" → `accounts.remove` → level 2. A core refusal shows in `<Refusal>`.
    - New sign-in (spec §7.5, last row): `renewed: Set<string>` gains an id when its `AccountSignIn` ends with `true` and that account's `account.loginAt` was set when the sign-in started; its level-2 row and level-3 "Sign-in" group then add "{account} has a new sign-in. Its teammates start fresh once their work ends." until the section unmounts.
  - **AccountSetup** loads `accounts.setup.list` on mount and after a copy. Origin: "Started from your {agent} setup on {toLocaleDateString(undefined, { day: "numeric", month: "short" })}", "Started empty", or none. Rows in `SETUP_CATEGORIES` order, unknown ids by id: title = item; detail "{count} {noun}(s)" for a noun, else the path's last segment. "Open" (aria-label "Open {item}") only when `isDesktop() && !onServer()` and `path` is set; it calls `openAccountSetup(path)` and a rejection shows in `<Refusal>`. Hint (spec §7.6) "Opens in your own editor or file manager on this computer. Changes apply from the next session of the teammates on this account." Group "Copy from your own setup": "Copy again from System default", detail "Bring over what you changed in your own {agent} setup since this account was made", key "Choose…" → checkboxes ticked except `settings` (spec §18.16), warn box "The ticked items replace this account's, including your changes to them. The sign-in is not touched.", Cancel and primary "Copy 1 item" / "Copy N items" (disabled at 0); the answer shows `role="status"` "Copied N items from System default." [+ " K links skipped."] [+ " Left out: {removedKeys}."]. Hint "The sign-in, keys and history are never copied."
  - **Settings.tsx / App.tsx.** `| "accounts"` after `"providers"` (:31), `{ id: "accounts", title: "Agent accounts" }` after Providers (:45), prop `roster`, early return `if (section === "accounts") return <AgentAccountsSection roster={roster} onBack={onBack} />;` beside :195; App.tsx:556 passes `roster={roster}`.
  - **index.css.** `.agent-account-dot` is an 8px circle like `.dock-dot`: signed_in `--accent`, checking or signing `--warn`, failed `--danger`, signed_out and unavailable `--ink-4`.
  - **ui/design.md.** :122-129 becomes "Providers, Tools and Agent accounts are one shape…", adding that Agent accounts goes a level deeper (agents, an agent's accounts with System default first, an account's page with Setup and Remove at the foot) and that a Windows desk signs in in the agent's own window, one line with Cancel, never a terminal. Log: "- 2026-10-21 Settings › Agent accounts: a third Providers-shaped list, three levels deep; Open hands a setup item to the system through the shell, which touches nothing outside the account's folder."

- [ ] **Step 4: Run tests to verify they pass**
  - `cargo test -p hotline-app --locked` (the `account_setup` tests and `window_capabilities`) and `cargo clippy -p hotline-app --all-targets --locked -- -D warnings`. Expected: PASS, clean.
  - `(cd ui && bun test ./tests/agent-accounts.test.tsx ./tests/agent-account-setup.test.tsx ./tests/settings.test.tsx ./tests/agent-sign-in.test.tsx && bun run typecheck && bun run lint && bun run knip)`. Expected: PASS; knip is quiet because the tests, its entry, read `accountJob` and `nextAccountName`.

- [ ] **Step 5: Commit**
  - `git add crates/hotline-app crates/hotline-core/src/lib.rs Cargo.lock ui/src/components/AgentSignIn.tsx ui/src/components/accounts ui/src/components/settings/AgentAccounts.tsx ui/src/components/settings/AccountSetup.tsx ui/src/components/Settings.tsx ui/src/App.tsx ui/src/native.ts ui/src/index.css ui/design.md ui/tests/agent-accounts.test.tsx ui/tests/agent-account-setup.test.tsx ui/tests/settings.test.tsx`
  - `git commit -m "An agent account is added, signed in, checked and removed in Settings, one in use cannot be removed, and Open touches nothing outside the account's own folder." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`

---

### Task 12: Teammate surfaces for agent accounts

**Files:**
- Create: `ui/src/components/teammate/AccountRow.tsx`, `ui/src/components/accounts/AccountChoices.tsx`, `ui/src/components/accounts/AccountFailure.tsx`
- Modify: `ui/src/components/accounts/AddAccountForm.tsx` (`compact`), `ui/src/components/accounts/words.ts` (`SYSTEM_DEFAULT`)
- Modify: `ui/src/components/Teammate.tsx:44-62, 266-268`, `ui/src/components/NewTeammate.tsx:143-148, 155-163, 185, 285-298`, `ui/src/components/ErrorCard.tsx:13, 15-54, 56-114`
- Modify: `ui/src/components/Transcript.tsx:100, 150, 414-433, 497-539, 581-587`, `ui/src/components/Conversation.tsx:67, 81, 449-469`
- Modify: `ui/src/components/Settings.tsx:174-198`, `ui/src/components/settings/AgentAccounts.tsx` (`opening`), `ui/src/App.tsx:508-512, 525-532, 556, 573-600, 604-617`, `ui/design.md:271-297` and its Log
- Test: `ui/tests/teammate-agent-account.test.tsx`, `ui/tests/new-teammate-account.test.tsx`, `ui/tests/error-card-accounts.test.tsx`, one test in `ui/tests/agent-accounts.test.tsx`

**Interfaces:**
- Consumes: Task 11's `AddAccountForm`, `AccountSignIn`, `statusWords`, `whoLine`, `nextAccountName`, `AgentAccountsSection`; Task 1's `SessionInfo.account?: SessionAccount { running?, wanted?, held?: "not_signed_in" | "signing_in" }`, `PersonaDraft.agentAccountId?` and `persona.account` params `{ personaId: string; accountId?: string | null }` (`ts(optional = nullable)`, so null needs no cast); Task 5's notice JSON `kind` (`usage_limit`, `account_unavailable`, `agent_auth`), `accountId` and `code`; Task 9's switch (`persona.account` answers the new `SessionAccount`; a wanted account that is `signed_out` or signing in is held).
- Produces:
  - `AccountRow({ persona, session, disabled, onSetup }: { persona: Persona; session: SessionInfo; disabled: boolean; onSetup?: ((accountId: string) => void) | undefined })`
  - `AccountChoices({ agentName, accounts, selected, inUse, signing, name, disabled, onSelect })`: the radio rows only; callers own the `grouped` radiogroup.
  - `AccountFailure({ error, personaId, teammate, backendId, onChooseAccount, onRetry })`
  - Props: Teammate `onAccountSetup?: ((accountId: string) => void) | undefined`; Transcript, Row and ErrorCard `backendId?: string` and `onChooseAccount?(): void`, ErrorCard also `teammate?: string`; Conversation `onChooseAccount(): void`; Settings `account?: string | undefined`; AgentAccountsSection `opening?: string | undefined`; AddAccountForm `compact?: boolean`; words.ts `export const SYSTEM_DEFAULT = ""`.
  - App: `openPanel(personaId: string, deskId?: string)`, the Rail's Edit (App.tsx:525-532) made one function and reused by Choose account. The window has already followed a teammate on a linked desk to its desk to show its card (docs/design.md:840-845), so this opens that teammate's panel there; from the rail the same function crosses desks with `openOnDesk(deskId, { personaId, inspector: true })`.
- SHEET DEVIATION: `AccountRow` also takes `onSetup?` and `AgentAccountsSection` `opening?` for the "{account}'s setup" deep link (Teammate.tsx:44-62 has no navigation prop); AccountRow is its own "{agent} account" section between the profile card and Folders (Teammate.tsx:266-268), where spec §7.2 and Teammate.dc.html:119-175 put it.

- [ ] **Step 1: Write the failing tests**

  `ui/tests/teammate-agent-account.test.tsx` uses the teammate-voice.test.tsx skeleton (`window` with `matchMedia` before `await import("../src/components/Teammate")`, `installDom({ ResizeObserver: NoResize })`, `replaceDesks([local])`, `__hotlineDesk = { platform: "windows" }`). The spy answers `accounts.list` (Task 11's fixture plus `a3` "Account 3", signed_out), `persona.account` (a `SessionAccount`), `session.stop`, `session.start`, `accounts.create` (`a4`), `accounts.login.start` (`{ id: "l1" }`) and `accounts.login.poll` (the test's `poll`); anything else throws. Ada is on `claude-acp` with `a1`, her session ready with `account: { running: "a1", wanted: "a1" }`.
```tsx
test("no account section for Hotline Agent or an agent without accounts") // hotline: expect(calls.map(([cmd]) => cmd)).not.toContain("accounts.list"); codex-acp: expect(text()).not.toContain("Codex account")
test("picking an account sends persona.account; System default sends null") // Account 2 → ["persona.account", { personaId: "ada", accountId: "a2" }]; System default → ["persona.account", { personaId: "ada", accountId: null }]
test("In use marks the running account, and the note says Restored or Fresh")
  // expect(row("Account 1").textContent).toContain("In use"); contextRestored → "On Account 1 · Restored: Claude Code reopened its own session for this account."
  // else "On Account 1 · Fresh: a new Claude Code session for this account, handed this conversation. Another account's session is never reopened here."
test("a pending switch offers Keep and Stop and switch now") // thinking, {running:"a1",wanted:"a2"} → "Switches to Account 2 when its work ends."; "Keep Account 1" → persona.account {accountId:"a1"}; "Stop and switch now" → session.* calls toEqual(["session.stop","session.start"])
test("a held switch keeps the current account") // {running:"a1",wanted:"a3",held:"not_signed_in"} → "Account 3 is not signed in. Ada keeps Account 1 until it is."
test("an account added here is assigned only after its sign-in succeeds") // "Starts with your Claude Code setup · Change or customize"; blank → accounts.create name "Account 4"; poll failed → no persona.account; poll succeeded → ["persona.account", { personaId: "ada", accountId: "a4" }]
test("the setup link opens the account's setup") // "Account 1's setup" → expect(setupOpened).toEqual(["a1"])
```
  `ui/tests/new-teammate-account.test.tsx` mounts `<NewTeammateForm models={[]} onCreated={…} />` with the same DOM, types names as onboarding.test.tsx:140-144, and answers `backends.list` (hotline, `claude-acp` "Claude Code", `codex-acp` "Codex"), `accounts.list` (the fixture), `persona.create` (echoes the draft), `session.start`, `accounts.create` (`a3`) and `accounts.login.*`; anything else throws and the form catches it.
```tsx
test("the Account list shows only for an agent with accounts") // Hotline Agent: no #new-account; Claude Code: radiogroup labelled "Account" with "System default", "Account 1" ("dev@example.com · Max"), "Account 2" ("Not signed in · it will not start until it is"); Codex: none
test("the picked account rides in the draft; System default adds nothing") // Account 1 → persona.create draft.agentAccountId "a1" and Thinks reads "Claude Code · Account 1"; System default → expect("agentAccountId" in draft).toBe(false)
test("Add account starts with your setup, and Change or customize opens Start with") // "Starts with your Claude Code setup · Change or customize" → "My Claude Code setup", "Empty", "Customize what is copied…"
test("an inline account is selected at once, and Sign in later leaves it Not signed in") // blank → accounts.create name "Account 3"; "Sign in later" → accounts.login.cancel; Account 3 radio checked; Add teammate → draft.agentAccountId "a3"
```
  `ui/tests/error-card-accounts.test.tsx` mounts `<ErrorCard text={notice} personaId="ada" teammate="Ada" backendId="claude-acp" onChooseAccount={…} onRunAgain={…} />` with `createRoot` so effects run; `accounts.list` answers `a1` signed_in usedBy `["ada"]`, `a2` signed_in, `a3` signed_out, `accounts.status` echoes each account's status, and `persona.account` answers.
```tsx
test("a usage limit offers Continue on each other signed-in account, System default and Try again")
  // "Account 1 hit its usage limit"; accounts.status for a2 and a3; "Continue on Account 2", "Continue on System default", "Try again", no "Continue on Account 3"
  // Continue on Account 2 → ["persona.account", { personaId: "ada", accountId: "a2" }] → "Now on Account 2: a fresh Claude Code session that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything."; "Try again" stays
test("a usage limit on System default still offers Try again") // no accountId: "Continue on Account 1", "Continue on Account 2", no "Continue on System default", "Try again"; renderToStaticMarkup without backendId → "Try again" only
test("an account-bound sign-in failure offers Sign in and Choose account, not the harness methods")
  // agent_auth + accountId "a3" + signIn → "Account 3 isn't signed in", "Ada runs on Account 3, and Claude Code says nobody is signed in there. Your message is kept; send it again once the account is ready."
  // "Sign in Account 3", "Choose account", no "Sign-in method"; Choose account → onChooseAccount called once; Sign in Account 3 → ["accounts.login.start", { accountId: "a3" }]
test("an unavailable account offers Show folder only for a listed folder on the Desk") // account_unavailable + "a1" (listed, folder) → "Show folder" and "Choose account"; remote desk → no Show folder; accountId "gone" → only "Choose account"
test("a card with no account calls no wire command") // agent_auth without accountId, quota, plain text → expect(calls).toEqual([])
```
  `ui/tests/agent-accounts.test.tsx` gains `test("opening lands on the account's page with its agent behind Back")`: with `opening="a1"` the band reads "Account 1", and after Back it reads "Claude Code".

- [ ] **Step 2: Run to verify they fail**
  - `(cd ui && bun test ./tests/teammate-agent-account.test.tsx ./tests/new-teammate-account.test.tsx ./tests/error-card-accounts.test.tsx ./tests/agent-accounts.test.tsx)`
  - Expected: FAIL: there is no "Claude Code account" section and no `#new-account`, the card's title is the core's rather than "Account 1 hit its usage limit", and `opening` is ignored.

- [ ] **Step 3: Implement**
  - **AccountChoices.** Rows are `<label className="group-row group-row-choice"><input type="radio" className="radio" …>` like `Choice` (BackendPicker.tsx:192-235). Details: System default "{agent}'s own sign-in on this computer, as before"; the account being signed in here "Signing in…"; signed_in whoLine or "Signed in"; signed_out "Not signed in · it will not start until it is" in warn ink; else statusWords. The `inUse` row adds a trailing "In use".
  - **AddAccountForm `compact`.** No visible labels: aria-label "New account name", and when `takesEmail` an email field with aria-label "Email of the {agent} account (optional)" and placeholder "Email (optional)". One line "Starts with your {agent} setup · " and a quiet "Change or customize" that opens the full form's Start with radios and categories. Same foot.
  - **AccountRow.**
    - Loads `accounts.list` on mount and after an inline sign-in ends, with a cancelled flag (Teammate.tsx:98-119). Renders null on a refusal or when no `agents` entry for `persona.backendId` has `supportsAccounts`; no backend id is written in the window (spec §9.1.1).
    - `<h3 className="label">{agent} account</h3>` over a `grouped` radiogroup with aria-label "{agent} account", holding in order: AccountChoices (`selected` = `persona.agentAccountId ?? SYSTEM_DEFAULT`; `inUse` = `running` while the session is ready or thinking and `running === wanted`); `AccountSignIn` while a new account signs in; the compact AddAccountForm while adding (`taken` = this agent's account names); the accent "Add account" row otherwise; the pending box when the session is up, `held` is unset and `(wanted ?? "") !== (running ?? "")` ("Switches to {wanted} when its work ends.", "What is running now stays on {running}. Stop and switch now stops all of {teammate}'s work.", quiet "Keep {running}", key "Stop and switch now"); the held box. An unlisted id shows as itself; none is "System default".
    - Picking sends `persona.account { personaId, accountId }`, `accountId: null` for System default, nothing when it is already wanted. Keep sends the running account. Stop and switch now sends `session.stop`, then `session.start`, both `{ personaId }` (spec §10.4). Inline add: `onAdded` mounts AccountSignIn when `canSignInHere`; `onEnd(true)` sends `persona.account` with the new id and reloads; `onEnd(false)` only reloads (spec §7.2).
    - While the session is ready or thinking, a `group-hint` shows the Restored or Fresh note, by `session.contextRestored`. With `agentAccountId` and `onSetup`, a quiet "{account}'s setup" key is followed by " · instructions, settings, skills, opened in your editor. It is shared by every teammate on {account}." Its own `<Refusal>` stands under the section.
  - **Teammate.tsx.** Prop `onAccountSetup?` (:44-62); after :266, `{!hotline && <AccountRow persona={persona} session={session} disabled={busy} onSetup={onAccountSetup} />}`.
  - **NewTeammate.tsx.** The first non-Hotline backend picked calls `accounts.list` in an effect on `onHotline` with a cancelled flag, again after an inline sign-in ends; a refusal reads as no accounts. `accountId` starts at `SYSTEM_DEFAULT` and resets when `backendId` changes. After the BackendPicker block (:285-298), for an agent with accounts: `<p className="label" id="new-account">Account</p>` over a radiogroup labelled by it holding AccountChoices, AccountSignIn (`cancelLabel="Sign in later"`), the compact form and the "Add account" row, with hint "Rename or remove accounts later in Settings › Agent accounts." `onAdded` selects the new id at once, then signs in (spec §7.2). With `agent` the `AccountAgent` for `backendId` in the `accounts.list` answer, the draft (:155-163) adds `if (!onHotline && agent !== undefined && accountId !== SYSTEM_DEFAULT) draft.agentAccountId = accountId;`; `thinks` (:185) reads "{backendName} · {account}" when one is picked.
  - **ErrorCard.tsx.** `errorDetails` also returns `accountId` when it is a string; `WORTH_ANOTHER_TRY` (:13) gains `"usage_limit"`, still gated by `failedLast`. New props `teammate`, `backendId`, `onChooseAccount`. With `personaId` and `backendId` set and kind `usage_limit`, `account_unavailable`, or `agent_auth` with an `accountId`, `<AccountFailure>` replaces the title line, summary and AgentSignIn; Try again and Error details stay. Every other card is today's and calls no wire command.
  - **AccountFailure.**
    - Loads `accounts.list` on mount; for `usage_limit` it also calls `accounts.status` for each other account of `backendId`, merging the answers as they land (spec §10.2, §10.5). The current account is the one whose `usedBy` holds `personaId`, else System default; the card reads as switched when that differs from the failed one (`accountId ?? SYSTEM_DEFAULT`), so a reload keeps it.
    - `usage_limit`: title "{account} hit its usage limit" (the notice's title for an unlisted account), body the notice's summary, "Continue on {account}" for each other signed_in account (first primary, the rest `control btn btn-sm`), then "Continue on System default" when it failed on an account; a Continue sends `persona.account`, then the "Now on…" text.
    - `agent_auth` with an account: title "{account} isn't signed in", body as the test; primary "Sign in {account}" mounts `AccountSignIn` in place, which on success shows "Signed in. The teammate is ready for another message." and, with `onRetry`, "Review failed message" (AgentSignIn.tsx:23-31); without `canSignInHere`, the text "Sign in from the desk itself". Then "Choose account".
    - `account_unavailable`: the notice's title and summary; "Show folder" (`showPath(folder)`) when the account is listed with a `folder` and `!onServer()`; "Choose account".
  - **Wiring.** Transcript takes `backendId?` and `onChooseAccount?` beside `onProviders` (:100, :150) and hands them to Row only for the main conversation (`!speakers && thread === undefined`, :414-433); Row hands them to ErrorCard with `teammate={ownerName}` (:581-587). Conversation (:67, :81, :449-469) passes `backendId={persona.backendId}` and `onChooseAccount`. App extracts the Rail's `onEdit` (:525-532) into `openPanel`, which also closes the dock (the inspector and the dock are one at a time, :270), and gives Conversation (:573-600) `onChooseAccount={() => openPanel(selected.persona.id)}`.
  - **Deep link.** App holds `const [accountPage, setAccountPage] = useState<string | null>(null)`. Teammate (:604-617) gets `onAccountSetup={(id) => { setAccountPage(id); setSettingsSection("accounts"); setPane("settings"); }}`; Settings (:556) gets `account={accountPage ?? undefined}`; the SettingsRail `onSection` (:510) clears it, and so does an effect when the pane leaves "settings". Settings renders `<AgentAccountsSection key={account ?? ""} … {...(account !== undefined ? { opening: account } : {})} />`, which starts on level 3 for `opening` with its agent's level 2 behind Back.
  - **ui/design.md.** In the inspector paragraph (:280-297): under the profile card, a teammate on an agent with accounts has an "{agent} account" list whose radio rows carry the status, "In use" on the running one and Add account in place; a pending switch (Keep / Stop and switch now) or a held one sits under the rows, and the list's one note says Restored or Fresh. After Needs you (:271-279), a bullet "Account cards": a usage limit names the account and offers Continue on each other signed-in account and System default before Try again; a missing sign-in offers Sign in {account} and Choose account; nothing switches an account on its own. Log: "- 2026-10-21 The teammate pane's account list and the account cards; a switch waits for the teammate's work and for the account's sign-in."

- [ ] **Step 4: Run tests to verify they pass**
  - `(cd ui && bun test ./tests/teammate-agent-account.test.tsx ./tests/new-teammate-account.test.tsx ./tests/error-card-accounts.test.tsx ./tests/agent-accounts.test.tsx ./tests/teammate-voice.test.tsx ./tests/folders.test.tsx ./tests/onboarding.test.tsx ./tests/agent-sign-in.test.tsx ./tests/no-provider.test.tsx ./tests/refusals.test.tsx ./tests/remove-teammate.test.tsx && bun run typecheck && bun run lint && bun run knip && bun run test)`
  - Expected: PASS. The existing suites stay green because every new fetch reads a refusal as nothing to show; folders.test mounts `backendId: "claude"`.

- [ ] **Step 5: Commit**
  - `git add ui/src/components/teammate/AccountRow.tsx ui/src/components/accounts ui/src/components/Teammate.tsx ui/src/components/NewTeammate.tsx ui/src/components/ErrorCard.tsx ui/src/components/Transcript.tsx ui/src/components/Conversation.tsx ui/src/components/Settings.tsx ui/src/components/settings/AgentAccounts.tsx ui/src/App.tsx ui/design.md ui/tests/teammate-agent-account.test.tsx ui/tests/new-teammate-account.test.tsx ui/tests/error-card-accounts.test.tsx ui/tests/agent-accounts.test.tsx`
  - `git commit -m "A teammate changes account only when the person picks one, and the switch waits for its work to end and for the account to be signed in." -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"`

---

### Task 13: Docs, security matrix and the Windows check run

**Files:**
- Modify: `docs/security.md`
  - Matrix rows go after `:721` (the last row, "Inside the computer…"), before the blank line `:722`.
  - A new `## Agent accounts` section goes before `:1079` (`## The owner phone changes a teammate's access`), after Harness sign-in (`:996-1078`).
- Modify: `docs/sessions.md:807-811` (An ACP child) and `:1590-1593` (Checkpoints). Add a new paragraph after `:811` and another after `:1605`.
- Modify: `docs/wire.md`
  - `:158` (`agent.auth.start`)
  - New rows after `:230` (`secrets.passkey.cancel`)
  - A paragraph before `:299` (`backends.list`)
  - `:982-984` (the phone roster sentence)
- Modify: `docs/design.md`: one paragraph after `:406`, before `### 4.` (`:408`).
- Modify: `docs/development.md`. Only this task edits it for the plan.
  - `:60`, plus a new line after it (hotline-support)
  - `:66` (link.rs)
  - after `:79` (hotline-room), after `:98` (hotline-core) and after `:118` (bins)
  - a data-directory paragraph after `:508`
- Create: `docs-site/src/content/docs/setup/agent-accounts.md` (frontmatter as in `teammates.md:1-4`)
- Modify: `docs-site/src/content/docs/setup/teammates.md:44-46`, `docs-site/src/content/docs/setup/providers.md:42-48`, and `docs-site/astro.config.mjs` (after `:53`: `{ label: 'Agent accounts', slug: 'setup/agent-accounts' }`)
- Modify: `CHANGELOG.md:9` (`## [Unreleased]` gets `### Added`)
- Test: `scripts/security-citations` (reads the matrix); `grep -n '‹' docs/security.md`

**Interfaces:**
- Consumes:
  - the test names Tasks 1-12 declare, at the files RESOLUTIONS fixes;
  - the sheet's commands, fields and sentences;
  - spec §8.6, §9.2, §10, §12, §13 and §14;
  - Task 11's remove sentence and its `open_account_setup` shell command;
  - the agent name "Claude Code".
- Produces: no code. The `citations` leg checks the matrix rows.

- [ ] **Step 1: Write the failing test: the matrix rows.** Insert them after `docs/security.md:721` in the `| Must hold | Proof | Needs |` voice. Needs is `—` unless a row gives it.
  - **How the checker reads a row:**
    - It takes a backticked `.rs` path followed by backticked `[a-z_0-9]+` names. It greps for `fn <name>` in the first root that holds the file (`crates/hotline-core/src`, `crates/hotline-store/src`, `crates`, `crates/hotline-core`, `crates/hotline-app/src`, …).
    - So `accounts/…`, `driver/…`, `session/…`, `wire/…`, `mcp/server.rs` and `tests/agent_accounts.rs` are written bare. Other crates take their crate prefix.
    - UI tests are named in prose, or in backticks with spaces as `:711` does; the checker skips both.
    - A `‹…›` slot is unchecked text. Fill it with the owning task's final test name.
  - **Row 1: isolation.**
    - Must hold:
      - two accounts and System default run at once, each agent and its children seeing only its own folder;
      - none of the six inherited variables reaches an account teammate, while System default inherits them unchanged;
      - a catalogue or method variable cannot bring back a scrubbed one;
      - both appliers apply one plan;
      - the desk's own environment is never set.
    - Proof: `tests/agent_accounts.rs` `two_accounts_and_the_system_default_start_at_once_each_in_its_own_folder`, `an_account_teammates_subprocesses_see_the_same_environment_as_the_agent`; `accounts/env/tests.rs` `system_default_removes_nothing_and_keeps_the_legacy_order`, `an_account_scrubs_every_listed_name_and_sets_its_folder_last`, `a_catalogue_or_method_that_reintroduces_a_scrubbed_name_loses`, `the_command_applier_removes_then_sets`, `the_pty_applier_removes_then_sets`.
    - Needs: Linux or macOS for the pty applier.
  - **Row 2: status.**
    - Must hold:
      - "Signed in" only when the agent's CLI confirms first party, `claude.ai`, no key source and the account's canonical folder;
      - the status run gets the account's scrubbed env;
      - raw output never reaches the wire;
      - one run per account, shared by every caller;
      - a status that runs out of time is `unavailable`.
    - Proof: `tests/agent_accounts.rs` `status_says_signed_in_only_for_the_account_the_agent_confirms`; `accounts/status/tests.rs` `signed_in_needs_a_first_party_claude_ai_login_in_this_folder`, `logged_out_is_signed_out_whatever_else_it_says`, `any_other_answer_is_unavailable_and_says_nothing_more`, `one_status_run_per_account_and_waiters_share_its_answer`; `accounts/status/tests.rs` `status_runs_in_the_account_folder_without_inherited_credentials`, `a_status_that_does_not_answer_in_time_is_unavailable`, `refresh_rechecks_the_folder_and_covers_only_accounts_in_use`.
  - **Row 3: refusal.**
    - Must hold:
      - an unknown, missing, linked, junctioned or moved folder refuses the start before any spawn and never falls back;
      - the private sentence goes to desk and owner, and the public one everywhere else, with no path, account name or email;
      - a data directory with spaces and Unicode works.
    - Proof: `tests/agent_accounts.rs` `a_teammate_set_to_an_account_not_on_this_desk_is_refused_and_never_falls_back`, `an_account_under_a_data_directory_with_spaces_and_accents_runs`; `accounts/resolve/tests.rs` `no_account_or_an_agent_without_accounts_is_system_default`, `an_account_resolves_to_its_canonical_folder`, `an_account_this_desk_does_not_have_is_refused_in_both_sentences`, `a_missing_or_replaced_folder_is_refused_with_its_kind`; `hotline-support/src/paths.rs` `an_account_folder_is_named_only_by_the_one_spelling_of_a_uuid`; `hotline-support/src/private_dir.rs` `a_real_account_folder_answers_its_canonical_path`, `a_folder_that_is_missing_a_file_a_link_or_elsewhere_is_refused_by_kind`; `driver/acp/tests/accounts.rs` `a_child_whose_folder_is_not_its_accounts_comes_up_refused_in_public_words`; `session/tests/accounts.rs` `an_unknown_account_refuses_the_start_in_public_words`; `driver/failure.rs` `an_account_refusal_card_names_the_account_by_id_alone`.
  - **Row 4: persistence and old data.**
    - Must hold:
      - the choice survives a reopen;
      - `agentAccounts` entries are filtered one by one;
      - a teammate, checkpoint, `link` line or chapter without `accountId` is System default, and writing none keeps old bytes.
    - Proof: `tests/agent_accounts.rs` `an_account_choice_survives_reopening_the_desk`; `hotline-room/src/agent_accounts.rs` `a_bad_account_entry_costs_only_itself`; `hotline-room/src/room/tests.rs` `agent_accounts_default_to_empty_and_read_back_what_was_written`, `a_legacy_checkpoint_is_the_system_default_one`; `hotline-contract/src/tests.rs` `agent_account_fields_are_additive_and_absent_is_system_default`; `hotline-store/src/link.rs` `a_binding_keeps_the_account_its_session_ran_on`; `session/chapters.rs` `a_close_keeps_the_account_its_chapter_ran_on`.
  - **Row 5: switching and checkpoints.**
    - Must hold:
      - a switch applies only when the whole teammate is idle and the account is not held;
      - it applies as a policy change that revokes the old lease and clears that backend's checkpoints for every account;
      - Keep clears nothing;
      - A's checkpoint never opens under B;
      - a late, other-account or old-`loginAt` checkpoint is not written;
      - a binding or chapter from another account opens Fresh;
      - signing in again rebuilds older sessions Fresh.
    - Proof: `session/tests/accounts.rs` `a_switch_during_a_dm_turn_waits_for_the_turn_and_then_applies`, `a_switch_waits_for_a_side_thread_mid_turn`, `keeping_the_running_account_clears_the_switch_and_keeps_checkpoints`, `a_held_account_waits_for_its_sign_in`, `applying_clears_the_backends_checkpoints_and_the_next_start_is_new`, `a_stale_lease_cannot_write_a_checkpoint_after_the_switch`, `signing_in_again_rebuilds_sessions_that_started_before_it`, `a_checkpoint_is_kept_only_while_the_record_names_the_sessions_account_and_sign_in`, `a_resume_onto_another_account_opens_fresh`; `accounts/switch.rs` `a_running_attempt_holds_as_signing_in_and_signed_out_as_not_signed_in`; `driver/acp/tests/accounts.rs` `a_checkpoint_opens_only_under_the_account_that_wrote_it`; `session/agent.rs` `a_session_is_reopened_only_by_the_child_that_issued_it`; `hotline-room/src/room/tests.rs` `a_checkpoint_is_kept_per_backend_and_account`, `clearing_an_account_reaches_every_teammate_and_clearing_a_backend_every_account`, `checkpoint_writes_on_one_teammate_never_lose_each_other`.
  - **Row 6: sign-in.**
    - Must hold:
      - sign-in belongs to the socket that started it (start, poll, input, cancel);
      - one attempt per account, from the desk seat only on Windows;
      - success is exit 0 plus a confirming status, and only success moves `loginAt` and clears that account's checkpoints;
      - a non-zero exit, silence, cancel or timeout is `failed` and changes nothing;
      - only a plain `--email` is passed;
      - `agent.auth.*` refuses an account teammate.
    - Proof: `wire/tests/seats.rs` `account_sign_in_belongs_to_its_desktop_socket_and_disconnect_revokes_it`; `accounts/login/tests.rs` `a_real_sign_in_succeeds_only_on_exit_zero_with_a_confirming_status`, `only_a_confirmed_sign_in_moves_login_at_and_clears_that_accounts_checkpoints`, `one_running_attempt_per_account`, `only_a_plain_email_becomes_an_argument`, `start_refuses_before_anything_is_spawned`, `cancel_disconnect_and_timeout_end_a_sign_in_as_failed`, `terminal_input_answers_the_cli_prompt`; `driver/acp/auth/tests.rs` `agent_auth_is_refused_for_a_teammate_on_an_account`.
    - Needs: Windows for the desk-seat test, Unix for input, and the visible console checked by hand (Step 5).
  - **Row 7: remove.**
    - Must hold: removing an account in use is refused, naming its teammates; removing an unused one keeps the folder and its sign-in.
    - Proof: `tests/agent_accounts.rs` `removing_an_account_in_use_is_refused_naming_its_teammates`, `removing_an_unused_account_keeps_its_folder`; `wire/tests/owner_handler.rs` `removing_an_account_in_use_names_its_teammates_and_keeps_its_folder`.
  - **Row 8: authorization and privacy.**
    - Must hold:
      - `accounts.*`, `accounts.setup.*` and `persona.account` answer Desk and Owner and refuse Phone and Link through the real handler;
      - `persona.update`, `settings.update` and `mobile.persona_update` cannot set an account;
      - the colleague view, a linked desk's roster and the Phone roster carry no account, lean or not, and neither does the `mobile.persona_update` reply;
      - folders and setup paths go to desk and owner only.
    - Proof: `wire/tests/seats.rs` `only_the_desk_and_the_owner_reach_agent_accounts`; `wire/tests/owner_handler.rs` `agent_accounts_are_operator_only_through_the_real_handler`; `wire/tests/personas.rs` `an_account_is_set_only_by_persona_account`; `wire/tests/roster.rs` `a_phone_roster_never_carries_an_account_lean_or_not`; `mcp/server.rs` `list_teammates_never_carries_an_agent_account`; `wire/tests/settings.rs` `settings_update_cannot_write_agent_accounts`; `hotline-room/src/linked.rs` `a_linked_roster_row_never_carries_an_agent_account`.
  - **Row 9: usage limit.**
    - Must hold:
      - the agent's usage-limit prefixes classify as `usage_limit`, in the agent's own words, and are never rewritten to `agent_auth`;
      - the notice carries `accountId`;
      - an account teammate's sign-in card carries its account, not the harness methods.
    - Proof: `driver/failure.rs` `an_agents_usage_limit_is_its_own_kind_in_its_own_words`; `driver/acp/auth/tests.rs` `an_account_teammates_cards_carry_its_account_and_never_the_harness_methods`.
  - **Row 10: setup copy.**
    - Must hold:
      - it takes only the categories asked for;
      - it never copies `.credentials.json`, `history.jsonl`, `projects/`, `sessions/`, `todos/` or a link;
      - it strips and lists the auth keys;
      - it gives the account's `.claude.json` only `mcpServers`;
      - Copy again replaces only the ticked categories;
      - listing the setup writes nothing.
    - Proof: `accounts/setup/tests.rs` `copy_brings_the_chosen_categories_and_no_secret`, `copy_again_replaces_only_the_marked_categories`, `a_refused_copy_leaves_the_account_as_it_was`, `a_link_in_the_account_is_replaced_not_followed`, `the_source_is_the_env_folder_else_the_home_default`, `setup_view_counts_items_and_names_paths_only_when_asked`.
  - **Row 11: Open.**
    - Must hold: "Open" goes through the desk shell's `open_account_setup`. It opens only a path that canonically lies under `<data>/agent-accounts/`, it creates only a missing `CLAUDE.md` or `settings.json`, and only the Desk seat offers it.
    - Proof: `hotline-app/src/account_setup.rs` `only_a_markdown_or_json_file_or_a_folder_directly_in_an_account_folder_opens`, `a_missing_item_is_made_before_it_opens_and_an_existing_one_is_never_rewritten`; the Desk-only Open test in `ui/tests/agent-account-setup.test.tsx`.
  - **Row 12: UI**, cited in prose: `ui/tests/agent-accounts.test.tsx`, `ui/tests/agent-account-setup.test.tsx`, `ui/tests/teammate-agent-account.test.tsx`, `ui/tests/new-teammate-account.test.tsx` and `ui/tests/error-card-accounts.test.tsx` (System default's usage-limit card keeps Try again).
- [ ] **Step 2: Run it to verify it fails.** Run `PYTHONUTF8=1 python3 scripts/security-citations` and `grep -n '‹' docs/security.md`.
  - Expected: FAIL.
    - grep prints the slot rows (2, 6, 8 and 11).
    - The checker prints a `missing` line for any test that a task's final text renamed or moved.
  - Fix: find each test across the tree (`grep -rn "fn <name>" crates`) and cite the file it is in. Never drop a claim.
  - Done when the checker says `checked N, missing 0` and grep prints nothing.
- [ ] **Step 3: Implement the docs.**
  - **`docs/security.md` `## Agent accounts`**, written from spec §13 in the review rule's order:
    - **Default:** no teammate has an account. These are all System default, which inherits the desk's environment exactly:
      - a record without `agentAccountId`;
      - a checkpoint, `link` line or chapter without `accountId`;
      - a room without `agentAccounts`.
    - **Grant source:** the operator, through `accounts.*` and `persona.account`, on the Desk or Owner seat, under `policy_update_lock`. The setting holds no secret and no path; `<data>/agent-accounts/<id>` is derived, and private from creation.
    - **Enforcement:** the eight §13 points:
      - the folder check before every spawn, restart, status, sign-in and setup write;
      - one `EnvPlan` with the §9.2 scrub;
      - a status that confirms provider, method and folder;
      - refusals in `persona.update`, `settings.update` and `agent.auth.*`;
      - the companion allowlist;
      - socket-bound sign-in;
      - a switch applied as a policy change that revokes the old lease;
      - checkpoints owned by `(backend, account, loginAt)`.
      
      Add Open's path check (row 11). Say that Hotline never reads or checks what the operator writes in the account's setup.
    - **Private data:**
      - the private sentence goes to desk and owner, and the public one everywhere else;
      - a notice names the account by `accountId` and `code` alone;
      - raw status JSON is discarded after parsing;
      - the `--email` value is validated, passed only as an argument and never stored.
    - **Tests:** the matrix rows.
    - **Revocability:** Remove is refused while a teammate uses the account, and never deletes the folder. Its confirmation reads "Hotline forgets {account}. Its folder and the sign-in in it stay at {folder}; sign out there with {agent}'s own sign-out if you no longer need it."
    - **Residual risk:** the nine §13 bullets, in order, in English.
  - **`docs/sessions.md`:**
    - At `:809`, "these agents sign themselves in" becomes "…in, into a folder of their own when the teammate has an agent account".
    - New paragraph after `:811`:
      - Accounts are `Persona.agentAccountId`.
      - The child gets `CLAUDE_CONFIG_DIR=<data>/agent-accounts/<id>` last, after the catalogue, the login method and the §9.2 scrub.
      - One `EnvPlan` serves start, restart, status and sign-in; System default adds and removes nothing.
      - A refused folder never falls back.
      - A live process never changes account; a switch restarts it Fresh.
    - At `:1591`, "one per backend" becomes "one per backend and account (`accountId`, absent for System default)".
    - New paragraph after `:1605`:
      - Applying a switch clears that backend's checkpoints for every account.
      - A confirmed sign-in clears that account's checkpoints on every teammate, and rebuilds older sessions Fresh once the teammate is idle.
      - A session whose `(account, loginAt)` no longer matches the record writes no checkpoint.
      - `link` lines and chapter markers carry `accountId`, and a resume across accounts opens Fresh.
  - **`docs/wire.md` rows after `:230`:** one row per sheet command, in the existing `| cmd | params | result |` voice, each ending "Owner or local desk only."
    - `accounts.list`: the cached `status`, `attempt`, `usedBy` ids, `folder` (desk and owner only) and `canSignInHere`. A status checked more than 10 minutes ago is rechecked in the background; one never checked stays `not_checked` until Check.
    - `accounts.create`: `startWith: "system_default" | "empty"`. A client `path` is ignored. A failed copy writes nothing and leaves the folder.
    - `accounts.remove`: refused with the remove-in-use sentence; the folder stays.
    - `accounts.login.start` → `{id}`. The attempt is this socket's, one per account, and on Windows it starts from the desk seat only ("Sign in from the desk itself.").
    - `accounts.login.poll` → `{state, output, error?}`.
    - `accounts.setup.list` → `AccountSetupView`, with `path` for desk and owner only; nothing is written.
    - `accounts.setup.copy` → `SetupCopyReport` `{copied, removedKeys, linksSkipped}`.
    - `persona.account` → `SessionAccount` `{running?, wanted?, held?}`.
  - **`docs/wire.md`, other edits:**
    - Paragraph before `:299`:
      - the companion, a linked desk and agent tools cannot reach these commands;
      - status, sign-in and setup are answered off the read loop, like `link.preview`;
      - the desk rechecks the accounts in use once when it opens;
      - a switch applies when the whole teammate is idle and the account is not held, and `held` says why it waits.
    - At `:158`: "refused for a teammate with an agent account ("Sign in {account} from its account instead.")".
    - At `:982`: "A phone seat, lean or not, is sent each row without `agentAccountId` or `info.account`, and so is the reply to `mobile.persona_update`."
  - **`docs/design.md`**, one paragraph after `:406`:
    - The decision: the agent's own config folder is the account, one row per agent in `accounts::support`, given only to that child. Hotline holds no credential, and a switch is always Fresh.
    - The additive optional fields: `Persona.agentAccountId`, `SessionCheckpoint.accountId`, `TranscriptEvent::{Chapter, Side, Link}.accountId`, the `link` line's `accountId` (`hotline-store/src/link.rs`), and the `agentAccounts` setting.
    - The importer copies them unchanged, because the reference tree has no accounts.
    - Downgrade: an older build drops the field, and the teammate returns to System default.
    - An old window draws a generic card for `usage_limit` and `account_unavailable`. The `account_unavailable` notice carries `accountId` and `code`.
  - **`docs/development.md`:**
    - `:60` becomes "…kill-on-close job, and a sign-in console's job that outlives a normal exit".
    - After `:60`: `private_dir.rs` "a credential folder made private from its first instant, and the real-directory check before each use".
    - `:66` (link.rs) gains "…and the agent account its session ran on".
    - After `:79`: `agent_accounts.rs` "the `agentAccounts` setting, read entry by entry".
    - After `:98`: `accounts/` "an agent account: its agent's row, folder, env, status, sign-in, setup and switch".
    - After `:118`: `bin/hotline-acp-fixture.rs` "a scripted ACP agent and its CLI, for the harnesses".
    - New paragraph after `:508`: `<data dir>/agent-accounts/<uuid>/` is one agent account's own config folder, private from creation (`hotline_support::private_dir`). The agent writes its sign-in, settings and history there, and Remove leaves it.
  - **`docs-site/.../setup/agent-accounts.md`** covers:
    - adding an account (Settings › Agent accounts, New teammate › Thinks with, the teammate pane);
    - Starts with System default or Empty, and what is never copied;
    - signing in: a console window from the desk on Windows, a terminal on macOS and Linux, "Signed in" only after Claude Code confirms, email optional;
    - assigning, switching, Keep, and Stop and switch now;
    - limits (Continue on …, Try again);
    - setup:
      - Open, on the desk, opens the file in your editor or the folder in your file manager, creating a missing CLAUDE.md or settings.json first;
      - Hotline does not read or check what you write;
      - Copy again;
    - what the folder holds, and the sign-out the remove confirmation mentions (`claude auth logout` with `CLAUDE_CONFIG_DIR` set to the folder);
    - the workspace-settings caveat.
  - **`teammates.md:44-46`:** "never asks for or holds your Claude login" becomes "never asks for or relays your Claude login", plus a link to Agent accounts. `providers.md` gets one sentence with that link after `:48`.
  - **CHANGELOG**, under `### Added`: "Agent accounts: give each Claude Code teammate its own Claude login. Add an account in Settings › Agent accounts or where you pick a teammate's agent, sign it in with Claude Code's own sign-in, and choose what of your Claude Code setup it starts with. Teammates on different accounts run at the same time; a switch waits until the teammate is idle and starts fresh with the conversation; a usage-limit card offers another signed-in account. System default works as before."
- [ ] **Step 4: Run the checks** (spec §15; Git Bash at the repo root; `python3` here is the spec's `python`).

  | Command | Expected |
  | --- | --- |
  | `python3 scripts/fetch-speech-engine` | sherpa-onnx v1.13.8 in `.speech-engine/`; its SHA matches `scripts/speech-engine.sha256` |
  | `(cd ui && bun install --frozen-lockfile)` | no lockfile change |
  | `cargo fmt --all --check` | no diff |
  | `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 warnings |
  | `cargo test --workspace --locked` | all pass, including the 8 in `agent_accounts`; ignored tests (Task 6's console test) show as ignored |
  | `cargo test -p hotline-core --locked --lib import` | importer unchanged |
  | `cargo test -p hotline-contract --locked` (unfiltered, last) | `ui/src/generated/contract.ts` written whole and committed |
  | `(cd ui && bun run typecheck && bun run lint && bun run knip && bun run test)` | pass; knip reports no unused export |
  | `PYTHONUTF8=1 python3 scripts/security-citations` | `checked N, missing 0` |
  | `python3 -m unittest discover -s scripts -p 'test_*.py'` | OK (`test_githooks` needs git on PATH) |
  | `PYTHONUTF8=1 bash scripts/precheck fmt lockfile citations contract clippy ui` | every leg passes |

  - Not run here; report each with its cause:
    - precheck `windows`: cargo-xwin cross-compiles from a Mac, and native clippy covers it.
    - `linux`: needs Apple `container`.
    - `actionlint`: not installed.
    - `bare-path`: needs a Unix `/usr/bin` PATH.
    - `shuffle`: unverified on Windows.
    - the deps gate: needs cargo-machete and cargo-deny; it is a Linux CI job.
  - If `aws-lc-sys` asks for nasm, retry with `AWS_LC_SYS_PREBUILT_NASM=1`.
- [ ] **Step 5: Manual real-account validation.** This needs the operator's two Claude logins, so the builder stops and asks.
  1. **Isolated data dir.** `export HOTLINE_DATA_DIR="$TEMP/hotline-accounts-validation"`, then `cd crates/hotline-app && cargo tauri dev`. The installed Hotline stays untouched, and nothing is killed by name or port.
  2. **Two accounts.**
     - Add "Account 1" (Starts with System default) and "Account 2". Sign in each one in its console window, with its own email.
     - Each row reads "Signed in · <its email>".
     - `<data>/agent-accounts/<id>/.credentials.json` exists for both.
     - The mtime of `%USERPROFILE%\.claude\.credentials.json` is unchanged.
  3. **Three teammates at once.** Developer on Account 1, Reviewer on Account 2, Researcher on System default. Prompt all three at once to run `claude auth status --text`.
     - Expect emails 1, 2 and the operator's default, each with `claude.ai`.
     - Each account's usage page on claude.ai shows only its own work.
  4. **Switch.** Switch Reviewer to Account 1 while it is idle. The next answer comes from email 1, the briefing is Fresh, and there is no "Restored".
  5. **Open.** Open on Instructions opens the account's `CLAUDE.md` in the system editor. The Owner seat shows no Open.
  6. **Release-build console.** Run `cargo build -p hotline-app --release`, start it on the same data dir, and Sign in again on Account 2. The console must take input and show the CLI's output. If it does not, use the §17 fallback: `conhost.exe`, or `CreateProcessW` without `STARTF_USESTDHANDLES`.
  7. **Remove.**
     - Removing Account 2 while Reviewer uses it is refused, naming Reviewer.
     - After moving Reviewer, removal succeeds, the folder stays, and the confirmation names the folder.
     - `CLAUDE_CONFIG_DIR=<folder> claude auth logout` signs it out.
  8. **Usage limit.** Only if a limit is hit naturally: the card offers Continue on Account 1 and Try again.
- [ ] **Step 6: Report real vs fixture**, as a three-column table.
  - **By fixture:**
    - isolation and the scrub;
    - `CHILD_ENV`;
    - persistence;
    - remove;
    - the unknown-account refusal;
    - the Unicode root;
    - status;
    - Task 6's login runs;
    - every unit, wire and UI test.
  - **Real:** Step 5, items 2-7.
  - **Not validated unless it happens:**
    - a real usage-limit card;
    - macOS and Linux terminal sign-in on this machine;
    - the §3 session-carry spike, which is outside this plan.
- [ ] **Step 7: Commit.** `git add docs/security.md docs/sessions.md docs/wire.md docs/design.md docs/development.md docs-site CHANGELOG.md`; `git commit -m "Every agent-account claim in the docs names the test that proves it."`

---

## Desvios

Desvios em relação à spec e ao plano, registrados pelo coordenador durante a execução. Nenhum muda uma decisão da spec; cada um preenche uma lacuna ou segue o código real.

### Já previstos em RESOLUTIONS
1. **`SetupFrom` como `"system_default" | "empty"`** em todo o contrato (a spec §10.6 escreve `systemDefault` uma vez).
2. **O notice `account_unavailable` leva `accountId` e `code`** (o `kind` da recusa), para o `ErrorCard` desenhar título e ações.
3. **"Open" passa pelo comando do shell `open_account_setup(path)`** em `crates/hotline-app`, não pelo opener da janela.
4. **`TranscriptEvent::Link` também ganha `accountId`**, como espelho tipado de `Link::event` (spec §8.4).

### Da varredura pré-execução (2026-10-21)
5. **`SessionAccount.held` aceita `not_signed_in` e `signing_in`.** A spec §10.4 tipa só `"signing_in"`, mas §7.2 e §10.4 retêm a troca também para uma conta sem login.
6. **`open_account_setup` aceita só os itens de setup do agente e cria qualquer um que falte**: `.md` vazio, `.json` como `{}`, ou a pasta. A spec §7.6 cita só CLAUDE.md e settings.json; uma conta "Empty" não tem `skills/`, `plugins/` nem `.claude.json`, e §18.16 diz que o item ausente é criado antes de abrir. A lista fechada mantém `.credentials.json`, `projects/` etc. fora (§8.6).
7. **O texto do cartão de limite vem da mensagem do erro ACP, antes da dica de stderr**, que fica só nos detalhes.
8. **O seat Phone (e qualquer seat `lean`) também perde `account`/`agentAccountId` nas respostas de `session.set_model`, `session.set_config`, `mobile.persona_access` e `mobile.persona_computer`**, com o mesmo predicado do roster.
9. **A linha de conta no nível 2 mostra status · email · Used by** (spec §7.1); o plano acrescentava o plano de assinatura.
10. **Títulos do cartão `account_unavailable` por `code`**, do protótipo (States): "Pick a {agent} account" e "Can't open {account}'s folder"; os corpos são as frases da spec §10.1.
11. **Testes vão para `<módulo>/tests.rs` quando passam do tamanho do código** (docs/standards.md), e a matriz de segurança cita o arquivo real.
12. **`test_githooks` não roda neste Windows** (privilégio de symlink, WinError 1314; exec de hook `#!/bin/sh`, WinError 193); é informado, não desabilitado.
13. **A validação real usa `bun run --cwd ui dev` + `cargo run -p hotline-app`**, sem instalar o `tauri-cli`.
14. **A linha de suporte `CLAUDE` é um `static`, não `const`** (CONTRACT_SHEET), para que `&CLAUDE` tenha um só endereço e a comparação por `ptr::eq` valha.
15. **A lista de itens que o "Open" aceita é a união das linhas de suporte**, porque o shell recebe só o caminho; com uma linha (`claude-acp`) é idêntica à lista do agente.
16. **A troca pendente vive só em memória.** Se o Hotline reinicia com uma troca retida, o teammate sobe na conta escolhida (mesmo sem login) e o primeiro prompt mostra o cartão de login. Quando a partida encontra um checkpoint de outra conta naquele backend (só uma troca perdida deixa isso), apaga os checkpoints do backend, para nunca dizer "Restored" à toa.
17. **`persona.update` grava o registro também sob o lock curto do registro** (`persona_write_lock`), para não desfazer nem ser desfeito pela limpeza de checkpoints de uma troca.
18. **Um novo login apaga os checkpoints do backend do teammate**, não só os da conta; com a regra do item 16 eles só nomeiam a conta do registro, então o efeito é o mesmo.
19. **Um `session.prompt` depois de uma partida recusada responde "That teammate is not running."** A recusa da conta acontece no `session.start` (§10.1), e uma partida recusada não deixa sessão; o cartão privado vem na resposta do start.
20. **Uma conta sem login diz "Not signed in · sign in to use it"** (decisão do operador), no lugar do "it will not start until it is" do protótipo: o teammate inicia e pede o login na primeira mensagem.
21. **Ocioso inclui uma chamada no meio de um turno** (`working_calls`), e aplicar a troca arquiva as side threads e as threads de chamada ociosas, como qualquer mudança de política. Uma chamada ainda conectada continua falando, mas sem buscas nem trabalho: a pessoa desliga e liga de novo.
22. **Os oito comandos `accounts.*` passam pelo `independent()` do 0.42.0**, e a desconexão do celular do dono os cancela: um `accounts.create` pode deixar uma pasta sem conta, e um `accounts.setup.copy`, arquivos copiados sem a nova origem.
23. **O harness `agent_accounts` vive no binário `integration`**: `--test agent_accounts` não existe mais, e cada caso roda com filtro exato e exige "1 passed".
24. **`start_account` fica em `session/accounts.rs`**, para o `session.rs` ficar abaixo de 1.500 linhas.
25. **A página de uma conta sem login diz "Its teammates ask you to sign in at their next message"** (decisão do operador), no lugar de "Teammates on this account do not start until it is signed in".
26. **O console do login no Windows recebe três handles nulos**, então o std não liga `STARTF_USESTDHANDLES` e o console novo dá ao filho os próprios handles. É o fallback da §17 feito pelo std, verificado com sonda; antes, num build release, a saída do CLI ia para o `hotline.log`.
27. **A retenção da troca usa a última resposta gravada** (`AccountStatuses::last_answer`), não o "Checking" de uma checagem em andamento; o plano citava `cached(w)`.
28. **Num cancel, numa desconexão ou num timeout, a tentativa termina `failed` antes da rechecagem do status**, assim que ela toma a vez; o plano dizia "por último, attempt.finish", o que agora vale só para um CLI que saiu.
29. **Todo escritor do registro inteiro do persona toma `persona_write_lock`** (foto, remetentes permitidos, grants de um desk ligado, servidores MCP removidos, passkey e a lápide do delete), estendendo o item 17.
30. **Lacuna da spec, aceita como risco residual:** marcadores de capítulo e bindings de `link` guardam `accountId` mas não `loginAt`, então, depois de um novo login na mesma conta, retomar um capítulo ou uma side/job estacionada pode reabrir uma sessão anterior ao login (mesma pasta, identidade agora logada). A §10.3 pede Fresh.
31. **Um nome em criação fica reservado** (raiz de dados, agente e nome em minúsculas, numa lista do processo) desde antes da primeira checagem até a conta ser gravada, e é liberado em qualquer saída: outro `accounts.create` ou um `accounts.rename` para esse nome é recusado na hora com "An account named {name} is still being made for that agent.", antes de pasta ou cópia, porque copiar um setup grande leva minutos. O `AddAccountForm` ganhou `onRefused`, e cada tela que o usa lê o `accounts.list` de novo depois de uma recusa. Achado na validação real.
32. **No Windows, `atomic_write` grava por um caminho verbatim (`\\?\`)**: o `persist` do `tempfile` passa caminhos crus ao Win32 e falhava com os error 3 acima de 260 caracteres, o que quebrava "My Claude Code setup" com plugins ou skills fundos. A recusa de junction continua no caminho verbatim. Achado na validação real.
