# Contas por provedor — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Qualquer jeito de entrar num provedor (assinatura de CLI, login OAuth, chave de API, servidor local) pode existir várias vezes com nome, e cada teammate escolhe um dos que servem para ele, em Settings › Providers, em New teammate, no painel e no cartão de limite.

**Architecture:** Os dois armazenamentos ficam como estão: credenciais no stream da sala e no vault, contas de CLI no setting `agentAccounts` com a sua pasta. O que se une é:
- um campo, `Persona.accountId` (o antigo `agentAccountId`);
- uma regra de quais contas servem para um teammate;
- uma leitura, `accounts.list`, que passa a trazer as credenciais com `usedBy`, `default` e seção;
- o motor de troca do #202, que passa a valer também para o Hotline Agent;
- uma página por provedor em Providers.

No Hotline Agent, o driver guarda a conta escolhida e a aplica a cada turno, só quando o modelo do turno é do provedor dela, substituindo a entrada daquele provedor no mapa de autenticação.

**Tech Stack:** Rust 2024 (hotline-contract, hotline-room, hotline-core), tokio, ts-rs, serde, axum (harness); React 19 + TypeScript 7 + bun test + biome/knip (ui/).

**Spec:** `docs/superpowers/specs/2026-10-22-provider-accounts-design.md`, aprovada em 2026-10-22. Ela se apoia em `docs/superpowers/specs/2026-10-21-agent-accounts-design.md` (o #202). Os executores leem a spec e este plano.

## Global Constraints

- **Regras do repositório:** `AGENTS.md` e `docs/standards.md`.
  - Um escritor por stream.
  - Todo tipo exportado fica em `crates/hotline-contract`.
  - `cargo test -p hotline-contract --locked`, **sem filtro**, regenera `ui/src/generated/contract.ts` e é a última execução antes de commitar esse arquivo.
  - Dependências entre crates só descem.
  - Delete, don't disable.
  - Nenhum trait com uma só implementação.
  - Supressões com `#[expect(lint, reason = "…")]`.
  - Locks com `unwrap_or_else(PoisonError::into_inner)`.
  - Arquivo de produção até 1.000 linhas; 1.500 dispara divisão. **`session.rs` tem 1.497 e `desk.rs` 1.708:** nada novo entra neles. O código novo de sessão vai para `session/accounts.rs`, e o de credenciais da desk para `desk/credentials.rs` (`desk.rs` + `desk/`).
  - Testes vão para `foo/tests.rs` quando passam de 400 linhas ou do tamanho do código.
- **Comentários só quando estritamente necessários:** uma restrição ou razão que o código não expressa. Sem narração, sem doc que repete o nome. Isto vale mais que qualquer passo do plano que peça doc.
- **Commits:** uma linha que declara o invariante, no estilo do repositório, **sem `Co-Authored-By` e sem nenhuma atribuição a Claude ou Claude Code**. Só no branch `claude/provider-accounts-206744`. Nunca push.
- **Ausente quer dizer hoje:**
  - sem `accountId`, um agente de CLI é "This computer", com o ambiente idêntico;
  - sem `accountId`, o Hotline Agent usa a conta padrão (`Vault::connections_from`);
  - uma sala de uma conta só escreve os mesmos bytes de notice de hoje.
- **Segredos e nomes:**
  - nenhum segredo, fragmento de chave ou JSON bruto de CLI em stream, tape, log (`eprintln!`), wire ou contrato;
  - nomes de conta nunca em notice nem em recusa pública: notices levam só o id.
- **Ambiente e processos:**
  - nunca `std::env::set_var`;
  - desenvolvimento com `HOTLINE_DATA_DIR` isolado (`.hotline-dev` no checkout); nunca tocar `%APPDATA%\Hotline` nem o app instalado;
  - nunca matar processo por nome ou porta, só um PID que você mesmo criou.
- **Textos de UI** em inglês, exatamente os da spec; tooltips de 2 a 3 palavras (`ui/design.md`).
- **Esta máquina (Windows, Git Bash, sem `make`):**
  - `cargo` com `-j 4` a `-j 6`;
  - `cargo build --workspace --bins -j 4` antes de testes `--lib` que lançam `hotline-acp-fixture`;
  - nunca `npx biome`, use `cd ui && bun run lint`;
  - antes do primeiro build: `python scripts/fetch-speech-engine` e `cd ui && bun install --frozen-lockfile`.
- **Um teste que precisa de conta real** é `#[ignore = "needs …"]`. Nenhum check ou teste é desligado para ficar verde.

## Review Focus

Entradas que a spec implica, que nenhum teste de tarefa cobria e que mais provavelmente atingem quem usa:

1. **Duas chaves antigas com o mesmo rótulo** ("Anthropic", "Anthropic", como o importador e a janela antiga gravavam). O que se espera:
   - a página e o seletor distinguem as duas pela data;
   - o rename para um nome livre funciona;
   - "+ Add API key" com nome em branco vira "Anthropic 2".

   Teste na Task 2 (`a_blank_name_takes_the_next_free_one`) e na Task 9.
2. **Teammate sem `modelId` no registro**, que roda no modelo preferido da sala. `usedBy`, a regra de escolha e `persona.account` usam o provedor do modelo preferido. Teste na Task 5.
3. **Keychain trancado** (todas as chaves `Unavailable`). Um teammate com conta escolhida falha com a frase do keychain, sem pegar outra credencial. Teste na Task 3.
4. **Entrar de novo num login, cancelado ou com falha, enquanto teammates usam aquele login.** Os tokens antigos ficam e o turno seguinte funciona. Teste na Task 2.
5. **O celular troca o modelo para outro provedor e volta, com uma troca de conta pendente.** A troca aplica quando o teammate fica ocioso, e a conta volta a valer quando o modelo volta. Teste na Task 4.

---

### Task 1: `accountId` no lugar de `agentAccountId`, em todas as camadas

Renomeação mecânica, sem mudança de comportamento.

**Files:**
- Modify: `crates/hotline-contract/src/persona.rs`:
  - `Persona.agent_account_id` → `account_id`;
  - `PersonaDraft.agent_account_id` → `account_id`;
  - a doc dos dois diz o que a spec §6.3 diz, em uma frase: num agente de CLI, uma conta de agente; no Hotline Agent, uma credencial do provedor do modelo; ausente é o padrão; uma de outro backend ou provedor fica guardada e é ignorada.
- Modify: todo uso em `crates/` e `ui/src/`.
  - Rode `grep -rn "agent_account_id\|agentAccountId" crates ui/src --include=*.rs --include=*.ts --include=*.tsx`; são cerca de 74 linhas.
  - Inclui os literais de JSON: `wire/roster.rs:345` (`persona.remove("agentAccountId")`), `wire/socket.rs:925`, `wire/commands.rs:286,292` (`without(persona, "agentAccountId")`), `wire/commands/persona.rs:409` (a recusa em `persona.update`), e `hotline-room/src/linked.rs:628-634` (teste).
- Modify: `docs/design.md`, `docs/security.md`, `docs/wire.md`, `docs/sessions.md`, só onde nomeiam o campo (`grep -n agentAccountId docs/*.md`). As specs e planos antigos ficam.

**Interfaces:**
- Produces: `Persona.account_id: Option<String>` (`accountId` no JSON e no TS) e `PersonaDraft.account_id`. Nenhum alias para o nome antigo.

- [ ] **Step 1: Renomear** e rodar `grep -rn "agent_account_id\|agentAccountId" crates ui/src docs/design.md docs/security.md docs/wire.md docs/sessions.md`. Esperado: nenhuma linha.
- [ ] **Step 2: Rodar**
  - `cargo build --workspace --bins -j 4`
  - `cargo test --workspace --locked -j 4`
  - `cargo test -p hotline-contract --locked` (sem filtro, por último)
  - `cd ui && bun run typecheck && bun run test`

  Esperado: tudo passa, incluindo `a_phone_roster_never_carries_an_account_lean_or_not` e `list_teammates_never_carries_an_agent_account`. `PYTHONUTF8=1 python scripts/security-citations` passa.
- [ ] **Step 3: Commit:** `git commit -am "A teammate names its account as accountId, whichever store the account lives in."`

---

### Task 2: Vault — conta padrão, mapa com conta, atualização no lugar, nomes

**Files:**
- Create: `crates/hotline-core/src/vault/accounts.rs`, com os testes em `mod tests` no mesmo arquivo ou em `vault/accounts/tests.rs` se passarem de 400 linhas.
- Modify: `crates/hotline-core/src/vault.rs`:
  - `mod accounts;`
  - torna `fn credentials(events)` utilizável pelo módulo;
  - o código novo fica no módulo novo, não em `vault.rs`.
- Modify: `crates/hotline-contract/src/accounts.rs`: `#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)] #[serde(rename_all = "snake_case")] #[ts(export, export_to = "contract.ts")] pub enum AccountSection { Subscription, ApiKey, Server }`. Regenerate `ui/src/generated/contract.ts`.

**Interfaces:**
- Produces, em `impl Vault`:
  - `pub(crate) fn default_ids(&self) -> HashMap<String, String>`: provedor → id, pela mesma regra de `connections_from` (primeira não revogada que tem segredo ou pasta de login). `connections_from` e esta função compartilham o mesmo laço, sem duplicar a regra.
  - `pub fn provider_auth_for(&self, account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth>`: o mapa de `provider_auth()`, com a entrada de `account.provider_id` substituída.
    - Conta viva e utilizável: a autenticação dela.
    - Conta revogada, sumida ou sem segredo: `ProviderAuth::Unavailable(ACCOUNT_SIGNED_OUT.into())`.
    - Erro de keychain ao ler o segredo da conta: `ProviderAuth::Unavailable(<o erro>)`, como `connections_from` faz.
  - `#[derive(Clone, Debug, PartialEq, Eq)] pub struct ChosenAccount { pub id: String, pub provider_id: String }`, em `crate::session` (perto de `ProviderAuth`), porque o driver e o `ProviderKeys` a usam.
  - `pub(crate) const ACCOUNT_SIGNED_OUT: &str = "The account this teammate is set to is signed out or gone. Choose another account."`, em `driver/failure.rs`.
  - `pub(crate) fn update(&self, id: &str, change: CredentialChange) -> io::Result<Credential>`, com `pub(crate) struct CredentialChange { pub label: Option<String>, pub secret: Option<String>, pub base_url: Option<(String, Vec<String>)> }`:
    - `label` regrava o evento com o mesmo id;
    - `secret` (só `ApiKey`, não personalizada) troca a entrada em `secrets.json` sob o mesmo id;
    - `base_url` (só `Local`) troca a URL e regrava `models.json` da pasta com os ids descobertos, apagando o cache de descoberta daquele id;
    - qualquer outra combinação: `io::ErrorKind::InvalidInput`;
    - o evento novo mantém `id`, `created_at` e `revoked` e muda `updated_at`. O log dobra pelo id e mantém a posição, então o padrão não muda.
  - `pub(crate) fn adopt_login(&self, from: &str, into: &str) -> io::Result<Credential>`: os registros de login de `from` (o `login_tokens(from)`, o `COPILOT_RECORD` quando existir, e os arquivos sem segredo `models.json` e `endpoints.json`) são escritos sobre os de `into`, sob `login_lock(&login_dir(into))`. Depois:
    - `rig_login::forget(&login_dir(into))`;
    - o cache de descoberta de `into` é apagado;
    - `abandon_login(from)`;
    - um evento novo de `into` com `updated_at`.

    `into` precisa ser `Oauth` do mesmo provedor que `from` vai gravar; senão `InvalidInput`.
  - `pub(crate) fn free_label(&self, provider_id: &str, base: &str) -> String`: `base` se nenhuma credencial viva do provedor tem esse rótulo (sem diferenciar maiúsculas), senão `"{base} 2"`, `"{base} 3"`, …
  - `pub(crate) fn check_label(&self, provider_id: &str, label: &str, except: Option<&str>) -> Result<String, String>`:
    - rótulo aparado, de 1 a 64 caracteres (contados com `chars()`);
    - único entre as credenciais vivas do provedor na mesma seção (§6.1), sem diferenciar maiúsculas, ignorando `except`;
    - erros: `"Give the account a name of 1 to 64 characters."` e `"{provider} already has an account named {label}."`.
  - `pub(crate) fn section_of(credential: &Credential) -> AccountSection`:
    - `Local` → `Server`;
    - `Oauth` com `provider_id == "openrouter"` → `ApiKey`;
    - outro `Oauth` → `Subscription`;
    - `ApiKey` → `ApiKey`.

- [ ] **Step 1: Escrever os testes que falham** em `vault/accounts.rs`. Use os fixtures de `vault.rs` (`vault(name)`, `create`, `connect_local` com ids fixos):
  - `the_default_is_the_first_usable_credential_and_skips_a_revoked_or_empty_one`: três chaves anthropic; revogar a primeira e apagar o segredo da segunda deixa a terceira como `default_ids()["anthropic"]`.
  - `a_chosen_account_replaces_only_its_providers_entry`: anthropic A e B, openai C; `provider_auth_for(Some(&ChosenAccount{id:B,provider_id:"anthropic"}))["anthropic"] == ApiKey(b)` e `["openai"] == ApiKey(c)`.
  - `a_chosen_account_that_is_revoked_or_gone_never_becomes_another`: revogada → `Unavailable(ACCOUNT_SIGNED_OUT)`; id inexistente com `provider_id: "anthropic"` → `Unavailable(ACCOUNT_SIGNED_OUT)`; nunca `ApiKey(a)`.
  - `changing_a_key_or_a_server_keeps_its_id_its_place_and_the_default`: `update(A, secret)` mantém `default_ids()["anthropic"] == A`, `list()` na mesma ordem, e `provider_auth()` com a chave nova; `update(local, base_url)` troca `baseUrl` e os ids de `account_models()`.
  - `a_sign_in_adopted_in_place_keeps_its_id_and_a_failed_one_leaves_the_old_tokens`: grava tokens "old" em `login_tokens(into)` e "new" em `login_tokens(from)`; `adopt_login(from, into)` deixa "new" em `into`, apaga a pasta de `from` e mantém o id; sem chamar `adopt_login` (login cancelado), `into` continua "old".
  - `a_blank_name_takes_the_next_free_one`: com duas credenciais rotuladas "Anthropic", `free_label("anthropic","Anthropic") == "Anthropic 2"`.
  - `a_name_is_unique_per_provider_and_section_and_counted_in_characters`: "work" recusado quando existe "Work" na mesma seção; aceito quando o outro "Work" é um servidor; 64 emojis aceitos e 65 recusados.
  - `sections_follow_what_the_account_bills`: `section_of` para os quatro casos acima.
- [ ] **Step 2: Rodar e ver falhar:** `cargo test -p hotline-core --lib vault::accounts -j 4`. Esperado: erro de compilação, porque as funções não existem.
- [ ] **Step 3: Implementar** as funções do bloco Interfaces em `vault/accounts.rs`.
- [ ] **Step 4: Rodar e ver passar:**
  - `cargo test -p hotline-core --lib vault -j 4`
  - `cargo test -p hotline-contract --locked` (sem filtro)
  - `cd ui && bun run typecheck`

  Esperado: todos os testes de `vault` passam, os antigos inclusive.
- [ ] **Step 5: Commit:** `"A teammate's chosen credential stands in for its provider's alone, and a key or sign-in changed in place keeps its id and the default."`

---

### Task 3: Hotline Agent aplica a conta escolhida a cada turno

**Files:**
- Modify: `crates/hotline-core/src/session.rs`:
  - em `trait ProviderKeys`, `fn provider_auth_for(&self, account: Option<&ChosenAccount>) -> HashMap<String, ProviderAuth> { self.provider_auth() }`;
  - em `DeskAgents::agent`, uma linha que chama a resolução da conta do Hotline Agent em `session/accounts.rs` e passa o resultado a `InProcess::with_account`. Não cresça o arquivo: mova para `session/accounts.rs` o que precisar.
- Modify: `crates/hotline-core/src/desk.rs`: `DeskCredentials::provider_auth_for` chama `Vault::provider_auth_for`.
- Modify: `crates/hotline-core/src/driver/rig.rs`:
  - `account: Option<ChosenAccount>`;
  - `pub(crate) fn with_account(self, account: Option<ChosenAccount>) -> Self`;
  - cada `self.keys.provider_auth()` vira `self.keys.provider_auth_for(self.account.as_ref())` (`rig.rs:338,353,477,503,612`);
  - `Turn` ganha `account_on_notice: Option<String>`.
- Modify: `crates/hotline-core/src/driver/failure.rs`:
  - `checked_against_sign_in` guarda o veredito por credencial, não por provedor: a chave é o `token_dir` ou o caminho dos `tokens` da autenticação, em vez de `provider`;
  - um `Unavailable(ACCOUNT_SIGNED_OUT)` vira `Kind::Auth`.
- Modify: `crates/hotline-core/src/session/accounts.rs`:
  - `pub(super) fn hotline_account(log: &Log, persona: &Persona) -> Result<Option<ChosenAccount>, String>`;
  - `crates/hotline-core/src/accounts/resolve.rs`: as regras novas de "ignorado".
- Test: `crates/hotline-core/src/driver/rig/tests.rs`, `crates/hotline-core/src/accounts/resolve/tests.rs`, `crates/hotline-core/src/session/tests/accounts.rs`.

**Interfaces:**
- Consumes: `ChosenAccount`, `Vault::provider_auth_for` e `ACCOUNT_SIGNED_OUT` (Task 2).
- Produces:
  - `hotline_account`:
    - `Ok(None)` sem `accountId`, ou quando o id é uma conta de agente;
    - `Ok(Some(ChosenAccount { id, provider_id }))` quando o id é uma credencial, revogada ou não, de qualquer provedor (o driver aplica só quando o modelo do turno é daquele provedor);
    - `Err(<frase pública>)` quando o id não existe em lugar nenhum. A frase vem de `AccountRefusal::UnknownAccount.public_sentence(&persona.name, "Hotline Agent")`, e o notice da partida é `Failure::account_unavailable` com `code: "unknown_account"`, como no #202.
  - `accounts::resolve::resolve` para agentes de CLI:
    - id que é conta de agente de outro backend → `Ok(None)`;
    - id que é uma credencial → `Ok(None)`;
    - id inexistente → `UnknownAccount`.

    O #202 recusava conta de outro backend; isso muda.
  - Notice de falha de turno do Hotline Agent: `failure.account_id = turn.account_on_notice`. O valor é `Some(id da credencial que serviu o turno)` quando há escolha: a conta escolhida se aplica ao modelo do turno, ou o provedor do modelo tem mais de uma credencial viva. Caso contrário, `None`. O id que serviu sem escolha é o de `default_ids()`, lido por um método novo `ProviderKeys::default_account(&self, provider_id: &str) -> Option<(String, bool)>` (id e "há mais de uma"), com default `None`.

- [ ] **Step 1: Escrever os testes que falham:**
  - `driver/rig/tests.rs` `a_chosen_account_signs_only_its_providers_turns`: `TestKeys` com `provider_auth_for` que registra o `ChosenAccount` recebido; um `InProcess` `.with_account(Some(ChosenAccount{id:"b",provider_id:"anthropic"}))` envia `Some(b)` em todo turno; um sem conta envia `None`.
  - `driver/rig/tests.rs` `a_turn_on_a_signed_out_account_fails_as_a_sign_in_and_names_it_by_id`: o mapa traz `Unavailable(ACCOUNT_SIGNED_OUT)` para o provedor do modelo; o notice tem `"kind":"auth"` e `"accountId":"b"`, e o texto do notice não contém o rótulo da conta.
  - `driver/rig/tests.rs` `a_locked_keychain_fails_the_chosen_account_with_its_own_words` (Review Focus 3): o mapa traz `Unavailable("keychain locked …")` para a conta escolhida; o turno falha como `keychain`, e nenhuma outra autenticação do provedor é usada.
  - `driver/failure.rs` `two_sign_ins_of_one_provider_do_not_share_a_verdict`: um veredito guardado para `token_dir` A não responde por `token_dir` B.
  - `accounts/resolve/tests.rs` `an_account_of_another_backend_or_a_credential_is_ignored_and_an_unknown_one_refused`.
  - `session/tests/accounts.rs` `an_unknown_hotline_account_refuses_the_start_in_public_words`: `ada` no Hotline Agent com `accountId` inexistente; `session.start` recusa com a frase pública, e o notice tem `code: "unknown_account"`.
- [ ] **Step 2: Rodar e ver falhar:** `cargo test -p hotline-core --lib -j 4 -- a_chosen_account_signs a_turn_on_a_signed_out a_locked_keychain two_sign_ins an_account_of_another an_unknown_hotline`. Esperado: falha de compilação ou de asserção.
- [ ] **Step 3: Implementar** o bloco Interfaces.
- [ ] **Step 4: Rodar e ver passar:**
  - `cargo build --workspace --bins -j 4`
  - `cargo test -p hotline-core --lib -j 4`

  Esperado: passa, inclusive os testes de contas do #202.
- [ ] **Step 5: Commit:** `"Hotline Agent signs a turn with its teammate's chosen account whenever the turn's model is that account's provider, and never with another of its accounts."`

---

### Task 4: A troca de conta vale para o Hotline Agent

**Files:**
- Modify: `crates/hotline-core/src/accounts.rs`:
  - `account_of(persona)` devolve `persona.account_id` também no Hotline Agent; num agente de CLI, só com linha de suporte, como hoje;
  - `check_choice` muda de assinatura.
- Modify: `crates/hotline-core/src/session/accounts.rs`. Os gates `account_support(backend).is_some()` em `switch_to_apply` e `settle_account_switch` passam a `uses_accounts(backend)`.
- Modify: `crates/hotline-core/src/wire/commands/persona.rs:64`: `persona.create` chama o `check_choice` novo com `draft.model_id`.
- Test: `crates/hotline-core/src/session/tests/accounts.rs`.

**Interfaces:**
- Produces:
  - `pub(crate) fn uses_accounts(backend_id: &str) -> bool`: `backend_id == HOTLINE_BACKEND_ID || account_support(backend_id).is_some()`.
  - `pub(crate) fn check_choice(log: &Log, backend_id: &str, model_id: Option<&str>, account_id: Option<&str>) -> Result<(), String>`:
    - `None` sempre passa;
    - num agente de CLI, como hoje;
    - no Hotline Agent, `account_id` tem de ser uma credencial não revogada cujo `provider_id` é o prefixo do modelo. `model_id` é o modelo da sessão viva, senão o `modelId` do registro, senão `models::preferred_model(&room::settings(log))`.
    - Erros:
      - credencial inexistente: `NOT_ON_THIS_DESK`;
      - revogada: `"That account is signed out. Sign in again first."`;
      - de outro provedor: `"That account is not for this teammate's model."`.
  - `Room::assign_account` passa o modelo efetivo. A comparação `account_id != current`, `on_running_account`, `apply_pending_account` e `clear_lost_switch` não mudam: no Hotline Agent não há checkpoints, então `clear_backend_checkpoints` não faz nada.

- [ ] **Step 1: Escrever os testes que falham** em `session/tests/accounts.rs`, com `Room::new(log, keys)` e um teammate `hotline` em `anthropic/claude-x`, com as credenciais A e B no stream:
  - `a_hotline_switch_waits_for_the_turn_and_rebuilds_on_the_new_account`: durante um turno, `assign_account(B)` deixa `info.account == {running: None, wanted: B}`; ao fim do turno a sessão é reconstruída, e o driver novo recebe `ChosenAccount{id:B,..}`.
  - `keeping_the_running_hotline_account_rebuilds_nothing`.
  - `going_from_no_choice_to_the_default_is_a_switch`: o driver novo guarda A (spec §8).
  - `a_hotline_choice_is_checked_against_the_teammates_model`: credencial de outro provedor, revogada ou inexistente é recusada com as três frases; com o registro sem `modelId`, a regra usa o modelo preferido da sala (Review Focus 2).
  - `a_pending_switch_survives_a_model_change_to_another_provider_and_back` (Review Focus 5): com uma troca pendente para B, `session.set_model` para `openai/…` e de volta para `anthropic/…` mantém a pendência; ao ficar ocioso, o driver novo guarda B, e os turnos em `anthropic` usam B.
- [ ] **Step 2: Rodar e ver falhar:** `cargo test -p hotline-core --lib session::tests::accounts -j 4`.
- [ ] **Step 3: Implementar** o bloco Interfaces.
- [ ] **Step 4: Rodar e ver passar:** `cargo test -p hotline-core --lib -j 4`. Esperado: passa, inclusive os testes de troca do #202.
- [ ] **Step 5: Commit:** `"An account change on a Hotline Agent teammate waits for its work to end and starts it fresh, like one on a harness."`

---

### Task 5: Wire — leitura única, atualização no lugar, nome e remoção

**Files:**
- Modify: `crates/hotline-contract/src/accounts.rs`:
  - `pub struct CredentialAccount { pub credential: Credential, pub section: AccountSection, pub used_by: Vec<String>, pub default: bool }`;
  - `AccountsList.credentials: Vec<CredentialAccount>`;
  - `AccountAgent.provider_id: Option<String>`.
- Modify: `crates/hotline-contract/src/command.rs`:
  - `CredentialLogin { provider_id, #[serde(default)] label: Option<String>, #[serde(default)] id: Option<String> }`;
  - `CredentialConnectLocal { base_url, #[serde(default)] label: Option<String> }`;
  - novo `#[serde(rename = "credential.update")] CredentialUpdate { id: String, label: Option<String>, secret: Option<String>, base_url: Option<String> }`.
- Modify: `crates/hotline-core/src/accounts/support.rs`: `AccountSupport.provider_id: &'static str`, `"anthropic"` na linha `CLAUDE`.
- Modify: `crates/hotline-room/src/room_handle.rs`:
  - `fn credential_update(&self, _id: &str, _change: CredentialUpdate) -> Result<Credential, String>`, com uma recusa padrão;
  - `credential_login` e `credential_connect_local` ganham `label: Option<&str>`, e `credential_login` ganha `into: Option<&str>`;
  - `fn default_credentials(&self) -> HashMap<String, String> { HashMap::new() }`.
- Create: `crates/hotline-core/src/desk/credentials.rs`. Mova para lá o `impl` das funções `credential_*` de `desk.rs` que esta tarefa toca, para `desk.rs` não crescer.
  - `credential_update`:
    - `label` passa por `check_label`;
    - `secret` não pode ser vazio;
    - `base_url` passa por `providers::ollama_url` e redescobre os modelos, como `credential_connect_local`.
  - `credential_login` com `into`: verifica que `into` é um `Oauth` do provedor, entra numa pasta nova como hoje e, no sucesso, `record_login` chama `vault.adopt_login(new, into)` em vez de `finish_login`. `LoginStatus.credential` é a credencial `into`.
  - O rótulo dado passa por `check_label`. Sem rótulo, `free_label(provider, <nome do catálogo>)`. Em `connect_local`, `free_label("ollama","Ollama Local")`.
- Modify: `crates/hotline-core/src/wire/commands.rs`:
  - `CredentialDelete` toma `room.policy_update_lock()` e recusa enquanto `room::roster(log)` tiver um teammate local com `account_id == id`: `"{label} can't be removed while {names} use it."`, com `names` juntos como em `accounts.remove`;
  - `CredentialCreate` passa o rótulo por `check_label`;
  - `CredentialUpdate` despacha para `room.credential_update`.
- Modify: `crates/hotline-core/src/wire/commands/accounts.rs` `list`:
  - `credentials` a partir de `room.credentials()` e `room.default_credentials()`;
  - `section` por `section_of`;
  - `used_by` = os teammates locais com `account_id == id` ∪ os teammates do Hotline Agent cuja escolha não se aplica ao provedor e cujo modelo (o do registro, senão o preferido da sala) é do provedor de que esta credencial é a padrão;
  - `default` = é o valor de `default_credentials()` do seu provedor.
- Modify: `crates/hotline-core/src/accounts/status.rs:252`: o refresh das contas em uso filtra os `account_id` que são contas de agente (uma credencial não tem status de CLI).
- Test: `crates/hotline-core/src/wire/tests/` (o arquivo de credenciais existente, ou `wire/tests/accounts.rs`).
- Regenerate: `ui/src/generated/contract.ts`, com o `cargo test -p hotline-contract --locked` sem filtro, por último. Atualize `ui/src/wire.ts` (`"credential.update": Credential`).

**Interfaces:**
- Consumes: `Vault::{update, adopt_login, free_label, check_label, default_ids, section_of}` (Task 2) e `check_choice` (Task 4).
- Produces: os tipos de contrato acima, `credential.update`, `credential.login {label?, id?}` e `credential.connect_local {label?}` no wire, e `AccountsList.credentials`.

- [ ] **Step 1: Escrever os testes que falham** (pelo handler real, `CoreHandle` sobre um `Desk` de teste):
  - `accounts_list_names_every_credential_with_its_section_users_and_default`:
    - duas chaves anthropic (A, B), um login `openai-codex` e um servidor ollama;
    - `ada` com `accountId: B`;
    - `mack` sem `accountId` em `anthropic/x`;
    - `eve` sem `modelId`, com a sala preferindo `anthropic/x` (Review Focus 2);
    - esperado: A `{default:true, usedBy:[mack, eve], section:"api_key"}`; B `{default:false, usedBy:[ada]}`; login `{section:"subscription"}`; servidor `{section:"server"}`; agente `claude-acp` com `providerId:"anthropic"`.
  - `removing_a_chosen_credential_is_refused_naming_its_teammates_and_the_default_can_go`: delete B recusado com `"Work can't be removed while Ada use it."` (frase exata montada como em `accounts.remove`); delete A aceito, e B vira o padrão.
  - `an_account_is_renamed_and_changed_in_place`: `credential.update {id, label:"Prod"}` e `{id, secret}` mantêm id e padrão; `{label}` repetido é recusado; `{base_url}` numa chave é recusado.
  - `a_blank_or_taken_name_is_settled_on_create`: `credential.create` com um rótulo já usado é recusado; `credential.connect_local` sem rótulo, quando já existe "Ollama Local", grava "Ollama Local 2".
  - `persona_create_checks_a_hotline_account_against_its_model`.
  - Estenda `only_the_desk_and_the_owner_reach_agent_accounts` (`wire/tests/seats.rs`) com `credential.update` (Desk e Owner sim, Phone e Link não).
- [ ] **Step 2: Rodar e ver falhar:** `cargo test -p hotline-core --lib wire -j 4`.
- [ ] **Step 3: Implementar** o bloco Interfaces.
- [ ] **Step 4: Rodar e ver passar:**
  - `cargo test --workspace --locked -j 4`
  - `cargo test -p hotline-contract --locked` (sem filtro)
  - `cd ui && bun run typecheck`
- [ ] **Step 5: Commit:** `"One accounts list names every credential with who uses it and which one is the default, and a credential in use cannot be removed."`

---

### Task 6: Limite do ChatGPT é limite de uso

**Files:**
- Modify: `crates/hotline-core/src/driver/failure.rs` `classify`: status `Some(429)` e `/error/type == "usage_limit_reached"` → `Kind::UsageLimit`, antes do ramo de `rate_limit`.
  - `retry_after_seconds` = `resets_at` (segundos desde a época) menos agora, quando houver.
  - Título: `"ChatGPT usage limit reached"`.
  - Resumo: `"This ChatGPT sign-in reached its usage limit."`.
  - A fonte do formato (`openai/codex`, `codex-rs/codex-api/src/api_bridge.rs`, `UsageErrorBody`) fica citada numa linha de comentário, porque o código não diz de onde veio.

- [ ] **Step 1: Escrever o teste que falha:**

```rust
#[test]
fn a_chatgpt_usage_limit_is_a_usage_limit_and_says_when_it_resets() {
    let resets = chrono::Utc::now().timestamp() + 3_600;
    let body = format!(r#"{{"error":{{"type":"usage_limit_reached","plan_type":"plus","resets_at":{resets}}}}}"#);
    let failure = Failure::classify(&body, Some(429), "stream");
    assert_eq!(failure.kind, Kind::UsageLimit);
    assert!(failure.retry_after_seconds.is_some_and(|s| (3_500..=3_600).contains(&s)));
    assert!(failure.retry_delay(0).is_none());
}
```

- [ ] **Step 2: Rodar e ver falhar:** `cargo test -p hotline-core --lib a_chatgpt_usage_limit -j 4`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar:** `cargo test -p hotline-core --lib failure -j 4`.
- [ ] **Step 5: Commit:** `"A ChatGPT sign-in at its usage limit fails as a usage limit with its reset time, and is not retried as a rate limit."`

---

### Task 7: Harness — duas contas reais do mesmo provedor pelo wire

**Files:**
- Create: `crates/hotline-core/tests/desk/accounts.rs`, registrado como `mod accounts;` no `mod desk` de `crates/hotline-core/tests/desk.rs`. Reaproveite o servidor Ollama falso de `tests/desk/connections.rs:6-43`; extraia um helper `fake_ollama(name: &str) -> (String /*url*/, mpsc::Receiver<Value>, JoinHandle<()>)` para os dois arquivos.

**Interfaces:**
- Consumes: tudo das Tasks 2 a 5, pelo wire.

- [ ] **Step 1: Escrever o teste** `#[tokio::test(flavor = "multi_thread")] async fn a_teammate_runs_on_its_chosen_account_and_others_on_the_default()`:
  1. Dois servidores falsos, `one` e `two`; `credential.connect_local` em cada um. O primeiro é o padrão.
  2. `ada` é criada com `accountId` = a credencial de `two` (`persona.create`); `mack` é criada sem.
  3. `session.prompt` nos dois: o pedido de `ada` chega a `two`, e o de `mack` a `one`.
  4. `credential.delete` da credencial de `two` é recusado, nomeando Ada.
  5. `credential.update {id: two, label: "Spare"}` mantém o padrão em `one` (`accounts.list`).
  6. `credential.revoke` da credencial de `two`: o próximo prompt de `ada` produz um notice com `"kind":"auth"` e `"accountId":<two>`, e **`one` não recebe nada** nesse intervalo.
  7. Um socket de seat Phone (com `lean`) não vê `accountId` no roster, e `persona.account` é recusado para ele.
- [ ] **Step 2: Rodar:** `cargo test -p hotline-core --test integration -j 4 -- desk_harness::desk::accounts::a_teammate_runs_on_its_chosen_account --exact`. Esperado: `1 passed` (filtro exato; o binário é `integration`).
- [ ] **Step 3: Commit:** `"Two servers of one provider prove over the wire that a teammate's chosen account answers its turns and nothing falls back to the other."`

---

### Task 8: Janela — quais contas servem, e o seletor sobreposto

**Files:**
- Create: `ui/src/components/accounts/fits.ts`, puro.
- Create: `ui/src/components/accounts/AccountPicker.tsx`, o seletor sobreposto da tela 3 e da tela 5.
- Modify: `ui/src/components/accounts/words.ts`: `SYSTEM_DEFAULT` passa a se chamar "This computer" na UI. A constante fica `""` e o texto vem de `THIS_COMPUTER = "This computer"`.
- Test: `ui/tests/account-fits.test.ts` e `ui/tests/account-picker.test.tsx`.

**Interfaces:**
- Consumes: `AccountsList` (Task 5).
- Produces:
  - `export type AccountChoice = { id: string; name: string; section: AccountSection; detail: string; payPerUse: boolean; isDefault: boolean }`. O id `""` é "This computer".
  - `export function accountsFor(list: AccountsList, teammate: { backendId: string; modelId: string | undefined }): AccountChoice[]`:
    - CLI com linha de suporte: as contas daquele backend, depois "This computer";
    - Hotline Agent: as credenciais não revogadas do provedor do modelo, na ordem do stream;
    - os outros casos: `[]`.
  - `export function inUse(choices: AccountChoice[], running: string | undefined): string`: `running`, se estiver em `choices`, senão o `isDefault` (Hotline Agent), senão `""`.
  - `export function suggested(choices: AccountChoice[], failed: string): string | undefined`: a primeira assinatura que não é `failed` (e, para CLI, está "Signed in"), senão `""` se estiver em `choices` e não for `failed`, senão a primeira chave.
  - `AccountPicker`:
    ```tsx
    AccountPicker({ choices, selected, label, onSelect, adds }: {
      choices: AccountChoice[];
      selected: string;
      label: string;
      onSelect(id: string): void;
      adds?: { subscription?: () => void; apiKey?: () => void };
    })
    ```
    - um botão que mostra a escolha;
    - abre um menu `position: absolute` ancorado nele (não aumenta o pai);
    - campo "Search {n} accounts" quando `choices.length > 8`;
    - cabeçalhos "Subscriptions", "API keys · pay per use" e "Servers";
    - rodapé com "+ Add subscription" e "+ Add API key" quando `adds` traz cada um;
    - Esc e clique fora fecham;
    - navegação por setas, com `role="listbox"`.

- [ ] **Step 1: Escrever os testes que falham:**
  - `account-fits.test.ts`:
    - Claude Code com duas contas → 3 escolhas, a última `""`;
    - Hotline Agent em `anthropic/x` com chaves A (padrão) e B e um login `openai-codex` → `[A, B]`;
    - `openai-codex/y` → só o login;
    - `inUse` sem `running` → A;
    - `suggested` com uma assinatura que falhou → a outra assinatura; sem nenhuma → `""` (CLI) ou a primeira chave (xAI).
  - `account-picker.test.tsx`:
    - abre e fecha sem mudar a altura do contêiner pai;
    - busca aparece com 9 contas e filtra por nome;
    - chaves mostram "pay per use";
    - as linhas de adicionar chamam `adds`.
- [ ] **Step 2: Rodar e ver falhar:** `cd ui && bun test tests/account-fits.test.ts tests/account-picker.test.tsx`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar:** `cd ui && bun test tests/account-fits.test.ts tests/account-picker.test.tsx && bun run typecheck && bun run lint`.
- [ ] **Step 5: Commit:** `"The window picks a teammate's account from one list of the accounts that fit it, in an overlay that never grows its card."`

---

### Task 9: Settings › Providers lista todas as contas de cada provedor

**Files:**
- Modify: `ui/src/components/settings/Providers.tsx`. Passa a ler `accounts.list` e `providers.list`. As linhas são por provedor, conforme a spec §10.1.
  - "Add provider" lista os provedores que ainda não estão na lista.
  - Escolher a Anthropic pergunta "Claude subscription" ou "API key" quando `accounts.list` traz um agente com `supportsAccounts` e `providerId: "anthropic"`. Sem isso, vai direto para a chave, como hoje.
- Create: `ui/src/components/settings/ProviderPage.tsx`. A página de um provedor (§10.2):
  - cabeçalho;
  - "Subscriptions", "API keys" e "Servers";
  - "This computer";
  - "Default";
  - a dica;
  - Rename (inline, com `credential.update {label}` ou `accounts.rename`), "Change key", "Change server", "Sign in again", "Sign out", "Remove key" e "Disconnect";
  - uma credencial revogada aparece com "Signed out" e só "Remove".
- Create: `ui/src/components/settings/ProviderModels.tsx`. A seção "Models in the picker", movida de `Providers.tsx` sem mudança de comportamento.
- Create: `ui/src/components/settings/CliAccountPage.tsx`. O `AccountPage` de `settings/AgentAccounts.tsx` (o nível 3 do #202), com o "voltar" levando à página do provedor.
- Delete: `ui/src/components/settings/AgentAccounts.tsx`. Os níveis 1 e 2 deixam de existir.
- Modify: `ui/src/components/ConnectProvider.tsx`:
  - campo "Name", com placeholder do nome livre que a janela calcula a partir da lista, para chave, login e servidor;
  - `credentialId?: string` para "Change key" (`credential.update {secret}`), "Change server" (`credential.update {baseUrl}`) e "Sign in again" (`credential.login {id}`);
  - sai a prop `replacing` e o "adiciona e depois apaga".
- Modify: `ui/src/components/Settings.tsx`: sai `{ id: "accounts", title: "Agent accounts" }`. Uma abertura de conta (o `account` que abria a página antiga) passa a abrir Providers › {provedor da linha de suporte} › conta.
- Modify: `ui/src/components/NewTeammate.tsx:415`: a dica diz `"Rename or remove accounts later in Settings › Providers."`.
- Test: crie `ui/tests/provider-accounts.test.tsx` levando de `ui/tests/agent-accounts.test.tsx` os casos de conta de CLI (adicionar, nome em branco, rename, remover recusado e livre, login, estados), depois apague o arquivo antigo. Atualize `ui/tests/settings.test.tsx`.

**Interfaces:**
- Consumes: `AccountsList.credentials`, `CredentialAccount`, `AccountSection` e `AccountAgent.providerId` (Task 5); `credential.update` e `credential.login {id}`; `AccountChoice` e `THIS_COMPUTER` (Task 8).

- [ ] **Step 1: Escrever os testes que falham** em `provider-accounts.test.tsx`, com `wire.command` substituído por espiões como no teste antigo:
  - `a provider row counts its subscriptions and keys`: "2 subscriptions · 2 API keys · All models" para Anthropic com duas contas `claude-acp` e duas chaves.
  - `the provider page lists every way in, with who uses each and the default`: as três seções, "This computer · Built in", "Default" na primeira chave, "Used by Ada".
  - `adding a key with no name takes the next free one`: com duas "Anthropic", o placeholder e o rótulo enviado são "Anthropic 2" (Review Focus 1).
  - `two old keys with one name are told apart by date` (Review Focus 1).
  - `change key updates the same credential`: envia `credential.update {id, secret}` e nunca `credential.delete`.
  - `remove refused names its teammates`: mostra a recusa do core.
  - `adding anthropic asks which way in when Claude Code takes accounts`: com o agente `claude-acp` (`providerId: "anthropic"`), "Add provider" › Anthropic oferece "Claude subscription" e "API key"; sem ele, abre direto a chave.
  - Os casos de conta de CLI movidos do teste antigo, agora sob Providers › Anthropic.
  - Em `settings.test.tsx`: a navegação não tem "Agent accounts".
- [ ] **Step 2: Rodar e ver falhar:** `cd ui && bun test tests/provider-accounts.test.tsx tests/settings.test.tsx`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar:** `cd ui && bun run typecheck && bun run lint && bun run knip && bun run test`. Esperado: tudo passa, e o knip não acusa export órfão de `AgentAccounts`.
- [ ] **Step 5: Commit:** `"Settings › Providers is the one place for every account of a provider, Claude subscriptions included, and Agent accounts is gone."`

---

### Task 10: Escolha de conta em New teammate e no painel

**Files:**
- Modify: `ui/src/components/NewTeammate.tsx`:
  - o campo "Account" usa `AccountPicker` com `accountsFor(list, {backendId, modelId})`, só com `choices.length >= 2`;
  - no Hotline Agent, o campo fica sob o modelo, e trocar o modelo para outro provedor volta a escolha ao padrão;
  - `adds.subscription` abre o `AddAccountForm` e o login inline (CLI) ou o `ConnectProvider` com login (Hotline Agent); `adds.apiKey` abre o `ConnectProvider` com chave; a conta nova fica selecionada;
  - ficar no padrão ou em "This computer" não grava `accountId`.
- Modify: `ui/src/components/teammate/AccountRow.tsx`. Vale para os dois backends:
  - título `"{provider name} account"` no Hotline Agent (`providers.list`) e `"{agent} account"` em CLI;
  - rádio com `inUse`;
  - linhas de adicionar no fim;
  - pendente, Keep, Stop and switch now e retenções, como hoje;
  - nota Fresh do Hotline Agent: `"On {account} · Fresh: a new session on this account, handed this conversation."`;
  - o link do setup só para contas de CLI.
- Modify: `ui/src/components/Teammate.tsx:251`: renderiza `AccountRow` também com `hotline`, passando `session.currentModelId ?? persona.modelId`.
- Test: `ui/tests/new-teammate-account.test.tsx` e `ui/tests/teammate-agent-account.test.tsx` (renomeie para `teammate-account.test.tsx`).

**Interfaces:**
- Consumes: `accountsFor`, `inUse`, `AccountPicker` (Task 8); `persona.account` para o Hotline Agent (Task 4).

- [ ] **Step 1: Escrever os testes que falham:**
  - `new-teammate-account.test.tsx`:
    - Claude Code só com "This computer": sem campo;
    - com uma conta: o campo aparece e começa em "This computer";
    - Hotline Agent com duas chaves anthropic: o campo aparece sob o modelo e começa no padrão; escolher B envia `draft.accountId: B`;
    - trocar o modelo para `openai/…` esconde o campo e tira `accountId`;
    - "+ Add API key" abre o formulário e seleciona a chave nova.
  - `teammate-account.test.tsx`:
    - Hotline Agent com duas chaves: rádio "Anthropic account", "In use" no padrão; escolher B chama `persona.account {accountId: B}`; com a troca pendente mostra "Switches to B when its work ends.", "Keep A" e "Stop and switch now";
    - uma chave só: sem seção;
    - os casos de CLI de antes continuam.
- [ ] **Step 2: Rodar e ver falhar:** `cd ui && bun test tests/new-teammate-account.test.tsx tests/teammate-account.test.tsx`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar:** `cd ui && bun run typecheck && bun run lint && bun run knip && bun run test`.
- [ ] **Step 5: Commit:** `"A teammate offers an account choice only when more than one account fits it, for Hotline Agent as for Claude Code."`

---

### Task 11: Cartão de limite com um seletor

**Files:**
- Modify: `ui/src/components/accounts/AccountFailure.tsx`:
  - em `usage_limit` (CLI e ChatGPT) e em `quota` com `error.accountId`, quando há ao menos uma outra conta que serve: a linha "Continue on" [`AccountPicker`] [Continue] [Try again later];
  - o seletor começa em `suggested(choices, failedOn)`;
  - no CLI, só contas "Signed in" (o cartão pede `accounts.status` como hoje) e "This computer";
  - no Hotline Agent, o provedor vem da credencial `error.accountId` em `list.credentials`;
  - "Continue" chama `persona.account` e mostra o corpo pós-troca de hoje;
  - "Try again later" fecha a oferta;
  - sem outra conta, o cartão fica como hoje.
- Modify: `ui/src/components/ErrorCard.tsx`:
  - `aboutAccount` inclui `quota` e `auth` com `accountId`. Num `auth` de uma conta escolhida do Hotline Agent, `AccountFailure` mostra o título e o resumo do notice, mais "Choose account" (`onChooseAccount`);
  - o "Try again" do `ErrorCard` fica escondido enquanto a oferta está aberta, por um `onOffer(open: boolean)` que `AccountFailure` chama.
- Test: `ui/tests/error-card-accounts.test.tsx`.

**Interfaces:**
- Consumes: `accountsFor`, `suggested`, `AccountPicker` (Task 8).

- [ ] **Step 1: Escrever os testes que falham:**
  - Claude Code: `usage_limit` em "Work" com "Personal" assinada e "Agency" assinada: o seletor começa em "Personal"; "Continue" chama `persona.account {accountId: personal}`; depois aparece "Now on Personal: …" com "Try again".
  - Antes de "Continue": "Try again" não aparece; "Try again later" fecha a oferta e o "Try again" volta.
  - Com 30 contas, o campo de busca existe e o cartão não muda de altura ao abrir o seletor.
  - xAI no Hotline Agent: `quota` na chave A, com a assinatura S e a chave B: a sugestão é S, e B mostra "pay per use".
  - `quota` sem outra conta: sem seletor e sem "Try again", como hoje.
  - `usage_limit` de "This computer" sem nenhuma conta: como hoje, com "Try again".
  - `auth` com `accountId` de uma credencial: "Choose account" chama `onChooseAccount`.
- [ ] **Step 2: Rodar e ver falhar:** `cd ui && bun test tests/error-card-accounts.test.tsx`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar:** `cd ui && bun run typecheck && bun run lint && bun run knip && bun run test`.
- [ ] **Step 5: Commit:** `"A usage limit offers every other account that fits the teammate in one picker, starting on a subscription, and never grows its card."`

---

### Task 12: Documentação, matriz de segurança e a lista completa de checks

**Files:**
- Modify: `docs/design.md`. O parágrafo de contas de agente (`:480-497`) vira "Accounts":
  - o vínculo único;
  - a regra de §6.2;
  - a conta padrão e o fim de "first created wins" para quem escolhe;
  - os campos aditivos (`accountId` no lugar de `agentAccountId`, `accountId` em notices do Hotline Agent quando há escolha).

  E em `:692`, "Claude subscription access stays with Claude Code through ACP" continua, citando a Q1.
- Modify: `docs/security.md`:
  - a seção "Agent accounts" (`:1160`) vira "Accounts", com o texto da spec §12;
  - linhas novas na matriz de regressão, citando os testes das Tasks 2 a 7 pelos nomes exatos;
  - `PYTHONUTF8=1 python scripts/security-citations` tem de passar.
- Modify: `docs/wire.md`: `accounts.list` (credentials), `credential.update`, `credential.login {label, id}`, `credential.connect_local {label}`, a recusa de `credential.delete`, e `persona.account` e `persona.create` no Hotline Agent.
- Modify: `docs/sessions.md`: a conta do Hotline Agent aplicada por turno, e a troca.
- Modify: `docs-site/src/content/docs/setup/providers.md`: absorve `setup/agent-accounts.md` (contas de um provedor, escolher, trocar, limite). Apague `agent-accounts.md` e atualize os links (`grep -rn "agent-accounts" docs-site`).
- Modify: `CHANGELOG.md`: uma entrada em Unreleased.

- [ ] **Step 1: Escrever os docs.** Cada afirmação aponta para o código ou para um teste que existe.
- [ ] **Step 2: Rodar a lista completa, nesta ordem:**
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --locked -j 4 -- -D warnings`
  - `cargo build --workspace --bins -j 4`
  - `cargo test --workspace --locked -j 4`
  - `cd ui && bun run typecheck && bun run lint && bun run knip && bun run test`
  - `PYTHONUTF8=1 python scripts/security-citations`
  - `python -m unittest discover -s scripts -p 'test_*.py'`: `test_githooks` falha neste Windows por ambiente (privilégio de symlink, exec de shebang, objetos git somente leitura). Isso é limitação registrada; informe, não desligue.
  - `PYTHONUTF8=1 bash scripts/precheck fmt lockfile citations contract clippy ui`
  - `cargo test -p hotline-contract --locked`, sem filtro, por último

  Esperado: tudo verde, exceto a limitação conhecida de `test_githooks`, informada com a saída.
- [ ] **Step 3: Commit:** `"Every account claim in the docs names the test that proves it."`

---

## Depois das tarefas

- Revisão do branch inteiro (subagent-driven-development).
- **Validação real com o operador**, num `HOTLINE_DATA_DIR` isolado, com `bun run --cwd ui dev` e `cargo run -p hotline-app`:
  - duas contas reais de um provedor do Hotline Agent;
  - duas assinaturas Claude;
  - a troca de conta;
  - o cartão de limite, se um limite real acontecer.

  Pare e pergunte ao operador antes de qualquer passo que precise de conta real.
- O relatório final, em português, diz o que foi validado de verdade e o que só tem fixture.
