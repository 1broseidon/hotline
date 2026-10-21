# Contas de agente por teammate — design

- **Data:** 2026-10-21 (revisada após builder check e revisões de código, segurança e protótipo)
- **Estado:** aprovada pelo operador em 2026-10-21 (protótipo e spec); próximo passo: plano de implementação.
- **Protótipo aprovado:** https://claude.ai/artifact/2nixmDsgN3esfjkh594aza (telas 1–7)
- **Base de código:** `1broseidon/hotline` em `e05716c4` (Release 0.41.0)

A interface do produto é em inglês; os textos de UI citados aqui são literais. O restante está em português. "Agente" aqui é o harness (backend id); "conta" é uma conta de agente.

---

## 1. Intenção

Hoje, todos os teammates Claude Code de um desk usam o mesmo login: o do usuário do sistema (`~/.claude`). O operador quer, por exemplo:

- Developer → Account 1
- Reviewer → Account 2
- Researcher → Account 3

Cada teammate usa o login da conta escolhida, sem afetar os outros, e todos rodam ao mesmo tempo. O caso concreto é distribuir trabalho entre assinaturas e continuar trabalhando quando uma conta atinge o limite de uso, trocando de conta manualmente.

O Claude Code isola configuração, histórico e login por diretório através de `CLAUDE_CONFIG_DIR`. A funcionalidade dá a cada conta um diretório próprio, gerenciado pelo Hotline, e aplica esse diretório apenas ao processo ACP do teammate e aos subprocessos dele.

**O modelo é genérico por agente (requisito do operador).** Cada conta pertence a um agente. Hoje só o Claude Code (`claude-acp`) suporta contas. Outra CLI ganha suporte com uma linha numa tabela de dados (§9.1), sem tela, comando ou campo novo.

## 2. Objetivos

1. **Cadastro de contas.** Criar, renomear, verificar o login e remover contas, e ver quais teammates usam cada uma.
2. **Criação no ponto de uso.** "Add account" dentro de New teammate › Thinks with e no painel do teammate. Settings › Agent accounts serve para gerenciar.
3. **Escolha por teammate,** com "System default", persistida entre reinícios.
4. **Isolamento por processo.** A pasta da conta vale só no processo ACP do teammate e nos seus subprocessos. Credenciais e configuração de provedor herdadas do ambiente não substituem a conta escolhida.
5. **Login funcional no Windows** pelo CLI oficial, com estados claros e verificação real antes de reportar sucesso.
6. **Troca explícita de conta** que não interrompe trabalho em silêncio, mantém a conversa do Hotline e nunca retoma a sessão do agente de outra conta.
7. **Troca manual no limite de uso.** O cartão de limite oferece continuar em outra conta.
8. **Compatibilidade total.** Dados antigos e "System default" se comportam exatamente como hoje, inclusive o "Try again" no cartão de limite.
9. **Setup por conta, configurável no Hotline** (decisão do operador, 2026-10-21):
   - **Ao criar:** a conta começa com uma cópia do setup do System default, sem login, chaves nem histórico, ou vazia. O operador escolhe o que é copiado.
   - **Depois:** cada conta é editada por conta própria. Cada item do setup (CLAUDE.md, settings.json, skills, plugins, MCP) tem "Open", que abre no editor ou no gerenciador de arquivos do sistema. Não há editor embutido nem validação do conteúdo (decisão do operador).
   - **A qualquer momento:** dá para copiar de novo do System default, item por item.

## 3. Não objetivos (nesta entrega)

- Rotação automática de contas por limite de uso.
- Contas para outros agentes. O modelo já aceita; só `claude-acp` tem uma linha na tabela.
- Logout pelo Hotline. Remover uma conta não desloga; a confirmação diz como deslogar pelo CLI.
- Importar cookies, tokens ou perfis do Claude Desktop.
- Sincronizar contas ou credenciais entre desks.
- **Levar a sessão interna do agente para outra conta.** A troca é sempre Fresh com o histórico da conversa (decisão do operador, 2026-10-21). Fica como esforço separado e posterior, depois da validação real com duas contas (§14):
  - **Spike:** copiar `projects/<slug>/<session>.jsonl` (e o que a sessão referenciar: memória, subagentes) da pasta da conta A para a B e retomar com `session/load` em B.
  - **Passa se:** (1) o contexto está presente; (2) o uso é cobrado em B e é o limite de B que diminui; (3) a sessão escrita sob a identidade de A carrega em B sem erro de autenticação.
  - **Se passar,** vira a opção explícita "Continue on {account} with full context" no cartão de limite e na troca do painel. Nunca automática; Fresh continua o padrão.
- Login pelo celular companion. Um dispositivo pareado como dono (seat Owner) tem a mesma autoridade do desk, como já acontece com `agent.auth.*` (§11).

## 4. Anti-objetivos (fracasso mesmo que "funcione")

- Um teammate passa a usar outra conta sem que o operador tenha escolhido, inclusive por fallback quando a conta está indisponível.
- Uma credencial ou um provedor herdado do ambiente do Hotline responde no lugar da conta escolhida.
- Token, cookie, senha ou JSON de status bruto aparecem em stream, log, tape, contrato ou resposta de wire.
- Trocar de conta ou entrar de novo numa conta interrompe um turno em andamento sem o operador pedir.
- "Signed in" aparece sem que o próprio CLI do agente tenha confirmado o login, ou depois de uma tentativa abandonada.
- Uma conta removida leva junto a pasta ou o login.
- Um teammate (o modelo) consegue mudar a própria conta por um caminho que o Hotline controla.
- O ambiente global do Hotline ou do Windows é alterado (`std::env::set_var` é proibido).
- Um caminho de pasta ou o nome de uma conta chega a colega, desk vinculado ou celular companion.

## 5. Restrições

- **Regras do repositório** (`AGENTS.md`, `docs/standards.md`):
  - Um escritor por stream.
  - Todo tipo exportado fica em `hotline-contract`, e `cargo test -p hotline-contract` roda sem filtro.
  - As dependências entre crates só descem.
  - Delete, don't disable.
  - Nenhum trait com uma única implementação.
  - Orçamento de tamanho por arquivo.
  - Harness que exercita o core real pelo wire.
- **Segurança.** É uma mudança de capacidade e segue o método de `docs/security.md` (§13): default, origem da concessão, pontos de enforcement, testes e risco residual.
- **Tapes e linhas `link` compatíveis byte a byte** com a árvore de referência. Campos novos são opcionais e aditivos, registrados em `docs/design.md`, com o importador verificado.
- **Hotline em uso.** O desenvolvimento usa `HOTLINE_DATA_DIR` isolado, nada é encerrado por nome ou porta, e a instalação real não é tocada.
- **Windows.** Não há `make`; os checks são rodados à mão (§15). Caminhos com espaços e Unicode funcionam, sem string de shell.
- **Política da Anthropic.** A documentação do Agent SDK proíbe que terceiros ofereçam login claude.ai sem aprovação. Aqui é o login oficial do Claude Code na máquina do operador; fica registrado como risco se a funcionalidade for distribuída.

## 6. Fatos verificados (spike de 2026-10-20, Windows 11)

Adaptador `@agentclientprotocol/claude-agent-acp` 0.78.0 (o fixado em `registry.rs`) e Claude Code 2.1.300.

- **O adaptador respeita `CLAUDE_CONFIG_DIR`.** Ele cria `.claude.json`, `backups/` e `sessions/` no diretório, inclusive num caminho com espaço e "ção". A documentação confirma que settings, CLAUDE.md, skills, plugins, histórico, projetos e login (`.credentials.json` no Windows e no Linux, entrada do Keychain por diretório no macOS) ficam sob esse diretório.
- **`session/new` funciona sem login.** O primeiro `session/prompt` falha com `-32000 "Authentication required"`, que `Failure::classify` já trata como `AgentAuth`.
- **Métodos de login.** Com `auth.terminal = false` (Windows hoje) o adaptador não anuncia nenhum. Com `true`, anuncia `claude-ai-login` (`--cli auth login --claudeai`), sem `env`, e não implementa `authenticate` para ele: o cliente roda o comando.
- **Status.** `<launch> --cli auth status --json` leva ~6 s via npx. Sem login, sai com código 1 e ainda imprime JSON válido. Campos: `loggedIn`, `authMethod`, `apiProvider`, `apiKeySource`, `email`, `orgId`, `orgName`, `subscriptionType`, `configDirectory`.
- **Herança de credenciais.** `ANTHROPIC_API_KEY` ou `CLAUDE_CODE_OAUTH_TOKEN` herdados fazem um diretório vazio aparecer `loggedIn: true`. `CLAUDE_CODE_USE_BEDROCK=1` vira `third_party`. `ANTHROPIC_AUTH_TOKEN` aparece `loggedIn: false`, mas tem a segunda maior precedência.
- **Precedência oficial:**
  1. `CLAUDE_CODE_USE_BEDROCK/VERTEX/FOUNDRY`
  2. `ANTHROPIC_AUTH_TOKEN`
  3. `ANTHROPIC_API_KEY`
  4. `apiKeyHelper`
  5. `CLAUDE_CODE_OAUTH_TOKEN`
  6. perfil ou federação Anthropic
  7. login OAuth de assinatura
- **Limite de uso.** O SDK emite uma mensagem sintética que começa com um dos `USAGE_LIMIT_ERROR_PREFIXES` ("You've hit your", "You've reached your", "You're out of usage credits", "Your org is out of usage · …", "You're out of extra usage" e as variantes de tipo de assento). O adaptador falha o prompt como `internalError` (-32603), com o texto bruto como detalhe. Hoje essas falhas chegam à UI como `acp`, que oferece "Try again".
- **O processo vivo não troca de conta.** Trocar é reiniciar o filho.

## 7. Experiência (o protótipo é a referência visual)

### 7.1 Settings › Agent accounts (tela 1)

Seção própria, logo depois de Providers. Três níveis, no padrão de Providers:

1. **Lista de agentes.**
   - Agentes com contas abrem a sua página, por exemplo "Claude Code · 4 accounts · 1 not signed in".
   - Os demais agentes instalados aparecem como linha simples: "Uses its own sign-in".
2. **Página do agente.**
   - "Add account" no topo: nome, email opcional (só se o agente aceita) e "Add and sign in".
   - Linha "System default · Built in", com quem a usa.
   - Uma linha por conta: ponto de status, nome e "status · email · Used by …". A linha abre a página da conta. Uma tentativa de login em andamento mostra o painel inline (§7.3).
3. **Página da conta.**
   - Rename.
   - Linha de login com "Check".
   - Used by.
   - **Setup** (§7.6): Instructions, Settings, e skills, plugins e servidores MCP, cada um com "Open".
   - Folder, com "Show folder".
   - Rodapé: "Remove account" (perigo, discreto) e "Sign in" ou "Sign in again".

**Estados de status visíveis:**
- Not checked (sem ponto, com "Check")
- Checking… (âmbar)
- Signed in (verde)
- Not signed in (cinza)
- Status unavailable (cinza, com "Check")

### 7.2 Criação e escolha inline (telas 2 e 3)

**New teammate › Thinks with,** quando o agente suporta contas:
- A lista "Account" mostra System default e as contas desse agente, com status.
- "Add account" cria a conta inline e começa o login. A conta nova fica selecionada.
- Se o operador cancelar o login, a conta fica "Not signed in" e o teammate pode ser criado mesmo assim.

**Painel do teammate,** seção "{agent} account" (só para agentes com contas):
- Rádio com "In use" e "Add account" inline.
- Uma conta adicionada ali passa a ser a escolha do teammate assim que o login for confirmado; antes disso, nada muda.
- Escolher uma conta "Not signed in" grava a escolha, mas a troca fica retida até a conta ser assinada: "{account} is not signed in. {teammate} keeps {current} until it is."
- Estado pendente: "Switches to {account} when its work ends", com "Keep {current}" e "Stop and switch now".
- Nota da seção: "On {account} · Restored …" ou "On {account} · Fresh …", conforme `contextRestored`.

**Nome em branco** em qualquer formulário vira o placeholder "Account N", aplicado pelo cliente.

**Ponto de partida do setup em todo formulário de adicionar** (protótipo, tela 7):
- **Agent accounts:** o formulário mostra "Start with": "My {agent} setup" (padrão) ou "Empty". "Customize what is copied…" abre as categorias com checkbox, todas marcadas por padrão.
- **Formulários inline** (New teammate e painel do teammate): mostram uma linha, "Starts with your {agent} setup · Change or customize", que abre as mesmas escolhas.
- **Painel do teammate:** na seção da conta, o link "{account}'s setup" leva à tela de setup da conta (§7.6) e avisa que o setup é compartilhado por todos os teammates daquela conta.

### 7.3 Login (tela 5)

1. "Add and sign in", "Sign in" ou "Sign in again".
2. **Windows:** o Hotline abre uma janela de console própria rodando o login oficial do agente com a pasta da conta. Isso só acontece a partir do seat desk; numa janela remota (seat Owner), a linha mostra de antemão "Sign in from the desk itself". **macOS e Linux:** terminal embutido no Hotline, o mesmo do login ACP, com entrada de texto para colar o código quando o callback do navegador não chega.
3. O navegador abre a página do Claude; o operador entra e autoriza. Se o navegador não abrir, a janela mostra o link.
4. Quando o processo de login termina, o Hotline confere sozinho. Não há botão "Check sign-in" durante a tentativa: o painel inline mostra só "Cancel" (e "Sign in later" no New teammate, que cancela).
5. **Sucesso** exige as três coisas (§10.3): o processo saiu com código 0, o status confirma o login na pasta da conta, e o login é de assinatura de primeira parte.
6. **Resultado da tentativa e status da conta são camadas separadas.** Uma conta pode continuar "Signed in" (o login antigo) enquanto a última tentativa mostra "Sign-in did not finish". Uma tentativa abandonada nunca conta como sucesso.
7. **Dica fixa:** o navegador reaproveita a sessão claude.ai aberta. Use "Switch account" na página ou cole o link da janela no perfil de navegador certo.

### 7.4 Troca de conta (tela 2)

- **Aplica quando o teammate não tem trabalho em andamento** (§10.4): nenhum turno na DM e nenhuma side thread, job, subagente ou par no meio de um turno. Se ele está ocioso, aplica na hora.
- **Com algo rodando,** fica pendente com "Switches to {account} when its work ends".
- **"Stop and switch now"** para todo o trabalho do teammate (o mesmo que Stop) e aplica a troca na hora: o painel mostra "In use" na conta nova e a nota Fresh.
- **"Keep {current}"** cancela a troca pendente sem reconstruir nada e sem apagar checkpoints.
- **Depois de aplicar,** a sessão nova é Fresh com o histórico da conversa. A tape não ganha linha, porque troca de provedor não narra.
- **Limite de uso:** o cartão "{account} hit its usage limit" mostra o texto do próprio agente (que inclui o horário de reset) e oferece "Continue on {conta assinada}", "Continue on System default", "Try again" e "Details". Depois da troca, "Try again" reenvia a mensagem na conta nova, e o texto avisa que o turno que falhou pode ter feito parte do trabalho.

### 7.5 Erros acionáveis (tela 4); nunca há troca silenciosa

| Situação | Onde aparece | Ação |
| --- | --- | --- |
| Conta sem login | cartão na conversa (`AgentAuth` do primeiro prompt) | "Sign in {account}", "Choose account" |
| Conta desconhecida (import, registro editado) | cartão de recusa ao iniciar | "Choose account" |
| Teammate de um desk vinculado cuja conta não existe lá | cartão de recusa no desk dele | "Choose account" abre o painel do teammate no desk dele (a janela segue o teammate, como em "Linked desks") |
| Pasta inacessível ou que não é mais a pasta da conta | cartão de recusa (caminho e tipo de erro só para desk e dono) | "Show folder", "Choose account" |
| Login não concluído | linha da conta | "Try again" |
| Limite de uso | cartão na conversa | "Continue on …", "Try again" |
| Entrou de novo (qualquer identidade) | aviso na linha da conta | nenhuma; os teammates dela recomeçam Fresh quando ficarem ociosos |

### 7.6 Setup da conta (tela 6)

A tela é alcançada pela página da conta e pelo link do painel do teammate. Uma coluna, no padrão das páginas de Settings:

- **Setup:**
  - A origem: "Started from your {agent} setup on {data}" ou "Started empty".
  - Uma linha por item, cada uma com "Open": **Instructions** (CLAUDE.md e `rules/`), **Settings** (settings.json), **Skills** (contagem; `skills/`, `agents/`, `commands/`), **Plugins** (contagem) e **MCP servers** (contagem; `.claude.json`).
  - "Open" chama um comando novo do shell desktop, `open_account_setup(path)`. Ele só aceita um caminho dentro de `<diretório de dados>/agent-accounts/` (checagem canônica), cria vazio um CLAUDE.md ou settings.json que falte (settings.json como `{}`) e abre com o app padrão do sistema; uma pasta abre no gerenciador de arquivos. O opener da janela hoje só revela arquivos, por isso o comando novo. Só o seat Desk mostra "Open".
  - Dica: "Opens in your own editor or file manager on this computer. Changes apply from the next session of the teammates on this account."
- **"Copy again from System default":** "Choose…" abre as categorias com checkbox e "Copy N items". O aviso diz que os itens marcados substituem os da conta, inclusive as suas mudanças neles, e que o login não é tocado.
- **Sem editor embutido e sem validação** (decisão do operador): o Hotline não lê nem verifica o que você escreve. Se o settings.json da conta substituir o login (por exemplo com `apiKeyHelper`), a conta deixa de aparecer "Signed in", porque o status exige o login de assinatura dela (§10.2).

## 8. Modelo de dados

### 8.1 `AgentAccount` (em `hotline-contract`, exportado)

```rust
pub struct AgentAccount {
    pub id: String,            // uuid v4 (Uuid::parse_str), estável; nome da pasta
    pub backend_id: String,    // "claude-acp" hoje
    pub name: String,          // rótulo do operador
    pub created_at: i64,
    pub login_at: Option<i64>, // época do último login confirmado (§10.3)
    pub setup: Option<AccountSetupOrigin>, // §8.6; ausente em contas sem registro
}

pub struct AccountSetupOrigin {
    pub from: SetupFrom,           // system_default | empty
    pub at: i64,
    pub categories: Vec<String>,   // ids de categoria copiados
}
```

- **Armazenamento.** Ficam no setting de sala `agentAccounts` (array), com default `[]`.
- **Leitura tolerante, entrada por entrada,** com um filtro próprio no padrão de `normalize_servers`, e não por `normalize_setting`, que mantém a lista inteira quando há erro. Entradas descartadas: id que não é uuid, id duplicado, backend vazio, nome fora de 1–64 caracteres ou duplicado no mesmo backend (sem diferenciar maiúsculas).
- **Escrita.** `settings.update` recusa a chave (como `teamLead`). Só os comandos `accounts.*` escrevem, com as mesmas validações aplicadas antes de gravar e sob `policy_update_lock`.
- **Nenhum segredo e nenhum caminho** fica guardado: a pasta é derivada.

### 8.2 Pasta da conta

- **Local:** `paths::agent_account_dir(root, id)` → `<data>/agent-accounts/<id>`, validando o uuid com `Uuid::parse_str`. Fica fora de workspaces e repositórios. `checked_folders` já recusa concessões dentro do diretório de dados.
- **Criação** com o helper da casa para diretórios de credenciais (`make_private_directory`, levado do vault para `hotline-support`): DACL protegida no Windows desde o primeiro instante e 0700 em Unix. Isso vale mesmo quando `HOTLINE_DATA_DIR` aponta para fora do perfil do usuário.
- **Antes de cada uso** (spawn, status, login), o caminho é canonizado com `dunce`, que nunca devolve `\\?\`. O uso é recusado se o caminho não for um diretório real (reparse point, junction ou symlink) ou se não resolver mais para `<data>/agent-accounts/<id>`, como já acontece com as pastas concedidas.

### 8.3 Teammate

- `Persona.agent_account_id: Option<String>`, ausente quando não há conta. Ausente significa System default.
- `PersonaDraft.agent_account_id` para `persona.create`, validado sob `policy_update_lock`. `MobileDraft` não ganha o campo.
- `persona.update` recusa a chave no patch (como `avatar`); a atribuição só acontece por `persona.account`.
- **Teammate que muda para um agente sem suporte a contas:** o campo fica guardado e é ignorado, e a linha some da UI. Se ele voltar para Claude Code, a conta volta a valer.
- **Para o seat Phone, com ou sem `lean`,** o roster apaga o campo e `SessionInfo.account`, usando o mesmo predicado dos snapshots de tape (`seat == Phone || lean`).

### 8.4 Checkpoints e vínculos de sessão

- `SessionCheckpoint.account_id: Option<String>`, aditivo. A busca usa o par `(backend_id, account_id)` em `open_session`, `thread/store.rs` (`dm()`), `room::checkpoint_session` e `room::clear_checkpoint`. `None` significa System default, então os checkpoints antigos continuam valendo para quem fica no System default.
- **Linhas `link`** (`hotline-store/src/link.rs`: `Link::event`, `read_link`, `read_marker`, `AgentBinding`) ganham `accountId` opcional, e o espelho tipado `TranscriptEvent::Link` também; o marcador legado `TranscriptEvent::Side` também. O `SessionCheckpoint` sintético montado a partir do vínculo para retomar threads (`session/agent.rs`) leva o `accountId`. `resumable()` recusa quando a conta é diferente.
- `TranscriptEvent::Chapter` ganha `accountId` opcional; a retomada de capítulo recusa conta diferente e abre Fresh. Isso é registrado em `docs/design.md`, e o importador é verificado.
- **Ao aplicar uma troca de conta** (não ao gravar a escolha), os checkpoints do teammate para aquele backend são apagados, de todas as contas. Sem isso, voltar para a Account 1 depois retomaria uma sessão sem o trecho conversado na Account 2, e o Hotline diria "Restored" sem que fosse verdade.
- **Gravações do registro do teammate** (checkpoint de fim de turno, limpeza e `persona.account`) não podem perder uma atualização concorrente:
  - O read-modify-write de `with_checkpoints` passa por um lock curto, compartilhado com `persona.account`.
  - Um checkpoint não é gravado quando a `(conta, login_at)` da sessão difere do registro.

### 8.5 Estado de login e tentativas (só em memória)

- **`AccountStatus`** (por conta): `state` (`not_checked | checking | signed_in | signed_out | unavailable`), `checked_at`, `email?` e `plan?`.
- **`LoginAttempt`** (por conta, no máximo um): `id`, `state` (`running | succeeded | failed`), `error?` e o dono (o socket).
- Os dois são devolvidos só a seats desk e dono. Nunca são persistidos, logados nem escritos em tape. O JSON bruto do CLI é descartado depois do parse.

### 8.6 Setup da conta (decisão do operador, 2026-10-21)

Uma pasta de conta guarda também o setup de usuário do agente: o que o Claude Code lê de `~/.claude` quando não há conta. Sem isso, um teammate com conta perderia CLAUDE.md, rules, skills, plugins, MCP de usuário, permissões, deny e hooks.

- **Origem ("System default"):** a pasta de configuração que o próprio Hotline enxerga, ou seja, o `CLAUDE_CONFIG_DIR` do ambiente do Hotline se existir, senão `~/.claude` e `~/.claude.json`. Ela só é lida, nunca escrita.
- **Categorias** (para `claude-acp`; cada agente declara as suas, §9.1):

  | Categoria | O que copia |
  | --- | --- |
  | Instructions and rules | `CLAUDE.md`, `rules/` |
  | Skills, agents and commands | `skills/`, `agents/`, `commands/`, `output-styles/` |
  | Plugins | `plugins/` e, no settings.json da conta, as chaves `enabledPlugins` e `extraKnownMarketplaces` |
  | MCP servers | só a chave `mcpServers` de `~/.claude.json`, gravada na `.claude.json` da conta |
  | Settings | `settings.json` sem as chaves de autenticação (`apiKeyHelper`, `awsAuthRefresh`, `awsCredentialExport` e as chaves de `env` da lista de scrub, §9.2), que são retiradas e listadas no resultado. Só a cópia retira essas chaves; o que o operador edita depois não é verificado |

- **Nunca copiado nem mostrado:** `.credentials.json`, entradas de Keychain, `history.jsonl`, `projects/`, `sessions/`, `todos/`, `statsig/`, `shell-snapshots/`, `ide/`, `backups/`, e qualquer outra chave de `.claude.json` (`oauthAccount`, projetos, onboarding).
- **Links na origem** (symlink, junction): não são seguidos. Um link é pulado e contado no resultado ("2 links skipped").
- **Semântica de cópia:** cada categoria marcada substitui a da conta. Uma pasta é espelhada, removendo o que só existia na conta dentro daquela categoria; um arquivo é sobrescrito; chaves de JSON são mescladas por chave. A escrita é atômica por arquivo (temporário + rename), dentro da pasta da conta, que passa pela checagem de §8.2.
- **Registro:** `AgentAccount.setup = { from: "system_default" | "empty", at, categories }`, gravado na criação e em cada "Copy again". Ele dá a linha de origem.
- **Edição:** fora do Hotline, no editor do sistema ("Open"). O Hotline não lê, não grava e não verifica o conteúdo editado.
- **Quando vale:** setup editado ou copiado vale a partir da próxima sessão do agente naquela conta. Nada é reiniciado.

## 9. Suporte por agente e ambiente do processo

### 9.1 Tabela de suporte

Uma tabela de dados no core, com uma entrada (`claude-acp`). É um `match` sobre `backend_id`, sem trait:

```rust
struct AccountSupport {
    env_var: &'static str,                    // "CLAUDE_CONFIG_DIR"
    login_args: &'static [&'static str],      // ["--cli","auth","login","--claudeai"]
    email_flag: Option<&'static str>,         // Some("--email")
    status_args: &'static [&'static str],     // ["--cli","auth","status","--json"]
    parse_status: fn(&[u8], &Path) -> AccountStatus, // valida contra a pasta (§10.2)
    scrub: &'static [&'static str],           // §9.2
    usage_limit_prefixes: &'static [&'static str], // §10.5, citando a versão do SDK
    setup: &'static [SetupCategory],          // §8.6: o que é copiado e aberto
    settings_auth_keys: &'static [&'static str], // retiradas do settings.json copiado
}
struct SetupCategory {
    id: &'static str,                         // "instructions", "skills", ...
    paths: &'static [&'static str],           // relativos à origem e à pasta da conta
    json_keys: &'static [(&'static str, &'static [&'static str])], // (arquivo, chaves mescladas)
    open: &'static str,                       // o que "Open" abre: um arquivo ou uma pasta
}
fn account_support(backend_id: &str) -> Option<&'static AccountSupport>
```

A origem do setup (§8.6) também vem da linha: para `claude-acp`, o `CLAUDE_CONFIG_DIR` do Hotline ou `~/.claude` e `~/.claude.json`.

### 9.1.1 Reaproveitamento por outros agentes (requisito do operador)

Tudo o que não é uma linha dessa tabela é genérico por agente. Nada fora da tabela e dos testes dela cita `claude-acp`, `CLAUDE_CONFIG_DIR` ou "Claude":

- **Modelo de dados:** `AgentAccount.backend_id`, `Persona.agent_account_id` e `SessionCheckpoint.account_id`.
- **Wire:** `accounts.*` recebe `backendId` onde cria e devolve os agentes com `supportsAccounts`; `persona.account` vale para qualquer agente com uma linha na tabela.
- **Mesmo código para todo agente:** resolução, ambiente, status, login, troca, remoção e recusas, parametrizados pela linha.
- **UI:** textos montados com o nome do agente vindo do registry ("{agent} account", "A {agent} window opened…", "Pick a {agent} account"). O rótulo do email só aparece com `email_flag`.
- **Limite de uso:** os prefixos vêm da linha do agente e são passados do `ChildAgent` (que conhece o backend) até a classificação.
- **Setup:** origem, categorias, o que cada "Open" abre e as chaves de autenticação retiradas na cópia vêm da linha. A tela de setup e "Copy again" servem a qualquer agente.

Dar contas a outra CLI depois (por exemplo Codex com `CODEX_HOME`) é uma linha na tabela, o parser de status dela e os testes dessa linha.

### 9.2 Ambiente de um processo ligado a uma conta

- **Uma única função pura** monta um `EnvPlan { remove: Vec<OsString>, set: Vec<(OsString, OsString)> }` a partir do launch, da conta resolvida e, quando houver, do `env` anunciado pelo método de login.
- **Dois aplicadores:** um para `tokio::process::Command` (`env_remove`/`env`) e outro para `portable_pty::CommandBuilder` (`env_remove`/`env`). Testes verificam o `EnvPlan`, e cada aplicador é testado uma vez.
- **Ordem:** catálogo (`launch.env`) → `env` do método de login → scrub → `env_var = pasta`. O scrub vem depois de tudo que pode reintroduzir uma variável removida (o catálogo pode carregar chaves), e a pasta vem por último.
- **Scrub de `claude-acp`:**
  - `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_CUSTOM_HEADERS`
  - `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`, `CLAUDE_CODE_OAUTH_SCOPES`
  - `ANTHROPIC_BASE_URL`
  - `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY`
  - `ANTHROPIC_PROFILE`, `ANTHROPIC_FEDERATION_RULE_ID`, `ANTHROPIC_ORGANIZATION_ID`
  - `CLAUDE_SECURESTORAGE_CONFIG_DIR`, `CLAUDE_CODE_SIMPLE`
  - `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`
  - `CLAUDE_CONFIG_DIR` (substituído)
- **A lista de remoção não basta sozinha.** O status confirma quem responde (§10.2), o que pega uma variável nova que versões futuras do CLI tragam.
- **System default** não remove nada e não acrescenta nada: o comportamento legado é preservado exatamente.
- **Os mesmos caminhos usam o `EnvPlan`:**
  - spawn do filho ACP e `restart_after_failure`;
  - status e login de conta;
  - login ACP de terminal em Unix, só para teammate sem conta (§10.3).
- **Nunca `set_var`.** Argumentos sempre por `Command::args`, sem string de shell.
- **Email.** Em Windows o launch costuma ser `npx.cmd`, e o std do Rust escapa ou recusa argumentos perigosos para `.cmd`. O email é validado em formato estrito antes de virar argumento; se for recusado, o login segue sem `--email`. Nenhuma frase de erro reconstrói a linha de comando (o email fica fora de `launch.line()`).
- **O que o scrub não alcança** (risco residual, §13):
  - Settings de projeto (`<workspace>/.claude/settings*.json`) com `env` ou `apiKeyHelper`. Isso já vale hoje para System default.
  - O `settings.json` da própria pasta da conta.
  - Settings gerenciados do sistema (política HKLM/MDM).

### 9.3 Ponto de aplicação

- **`DeskAgents::agent`** (`session.rs`, o único `ChildAgent::new`) resolve a conta uma vez e passa `with_account(ResolvedAccount { id, dir, support, login_at })`. Isso cobre DM, side thread, job, run, par, reattach e restart.
- **`ChildAgent::start`** repete a checagem de §8.2 antes de cada spawn ou restart.
- **`agent.auth.*`** é recusado para teammate com conta, com "Sign in {account} from its account instead", e o cartão aponta para `accounts.login`. Assim o login do harness nunca escreve na pasta do System default por engano.

## 10. Comportamento

### 10.1 Resolução e recusas

`resolve(persona)` devolve:

- **`Ok(None)`** sem conta, ou quando o agente não suporta contas.
- **`Ok(Some(ResolvedAccount))`** quando a conta existe, é do mesmo backend e a pasta passa em §8.2.
- **`Err(AccountRefusal)`**, uma recusa estruturada antes de qualquer spawn.

`AccountRefusal` tem `kind` (`unknown_account | folder_unavailable`), `account_id`, um nome e um caminho para quem tem direito de ver, e o tipo de erro de IO. Ela vira um `Failure` de kind novo (`account_unavailable`) com `accountId` e `code` (o `kind` da recusa) no JSON do notice, para o `ErrorCard` desenhar o título e as ações. Duas formas da frase:

- **Privada** (desk e dono: banda da conversa, `accounts.status`, ErrorCard):
  - "Pick a {agent} account for {teammate}: the one it was set to is not on this desk."
  - "Can't open {account}'s folder ({path}): {kind}. Fix the folder or pick another account."
- **Pública** (notice de thread, resposta de troca para um colega, link, companion, `info.error` no roster): "{teammate} can't start: its {agent} account needs attention at the desk." Nunca leva caminho, nome de conta nem email.

**Conta sem login** não é verificada no caminho quente: aparece no primeiro prompt como `AgentAuth`. Para teammate com conta, o cartão troca os métodos do harness por "Sign in {account}" e "Choose account". Quando o cache já diz `signed_out`, a UI avisa antes, e a troca para essa conta fica retida (§10.4).

### 10.2 Status

`accounts.status { accountId }`, também disparado em segundo plano (veja o fim desta seção):

- **Execução:** roda `launch + status_args` com o `EnvPlan` da conta e stdin nulo. No Windows usa `prepare` com `Job::attach` (um timeout mata a árvore npx → node → claude), e em Unix um grupo de processos. Timeout: o `StartBounds` conhecido ou o de primeira vez, conforme `registry::started_before`.
- **Concorrência:** no máximo uma execução por conta; pedidos repetidos esperam a mesma.
- **Fora do laço do socket:** roda em tarefa própria (padrão `LinkPreview`/`DesksLink`), porque `accounts.*` não tem lane.
- **Parse:** do stdout como JSON qualquer que seja o código de saída. `signed_in` exige ao mesmo tempo:
  - `loggedIn: true`;
  - `apiProvider == "firstParty"`;
  - `authMethod == "claude.ai"`;
  - nenhum `apiKeySource`;
  - `configDirectory` igual à pasta canônica da conta.

  `loggedIn: false` resulta em `signed_out`. Qualquer outra combinação, falha ao rodar, timeout ou JSON inválido resulta em `unavailable`, com uma frase fixa e nunca a saída bruta.
- **Atualização automática:**
  - ao abrir o desk, para as contas em uso;
  - em `accounts.list`, para contas já verificadas cujo cache tem mais de 10 minutos (uma conta nunca verificada fica "Not checked" até o "Check" ou um login);
  - depois de cada tentativa de login;
  - quando o cartão de limite abre (para listar as contas assinadas).

### 10.3 Login

**Comandos:** `accounts.login.start { accountId, email? }` → `{ id }`, e depois `.poll`, `.input` (Unix) e `.cancel`.

- **Tentativa presa ao socket que a iniciou**, copiando o padrão de `agent.auth.*`:
  - mapa de dono no `Outbox`;
  - poll, input e cancel só pelo mesmo socket;
  - `commands::run` recusa esses comandos fora do socket;
  - desconectar cancela a tentativa.
- **Uma tentativa por conta,** mantida num mapa no core. Uma segunda é recusada.
- **A espera roda em tarefa própria,** nunca no laço do socket. Timeout de 10 minutos.

**Windows (só o seat desk):**

- Spawn visível com `CREATE_NEW_CONSOLE | CREATE_SUSPENDED` mais um Job. É um helper novo, `process_windows::prepare_visible`, porque `CREATE_NO_WINDOW` é incompatível.
- **O Job do login não usa `KILL_ON_JOB_CLOSE`.** Cancel e timeout chamam `TerminateJobObject`. Numa saída normal o handle só é fechado, para não matar um navegador que o login tenha aberto como descendente.
- O seat Owner recebe a recusa "Sign in from the desk itself", e `accounts.list` já informa `canSignInHere`.
- Comando: `launch + login_args [+ email_flag email]`, com o `EnvPlan` e o cwd na pasta da conta.

**macOS e Linux:**

- `driver::auth::terminal` é generalizado para receber `(Launch, cwd, args, EnvPlan)`.
- O xterm da UI é reaproveitado, extraindo um `SignInTerminal` exportado e ligado a um "job" de conta.

**Resultado.** Depois que o processo sai, o Hotline roda o status.

- **`succeeded`** exige que o processo tenha saído com código 0 **e** que o status diga `signed_in`. Então:
  1. `login_at` vai para agora, gravado no setting.
  2. Os checkpoints dessa conta em todos os teammates são apagados, sob o lock de §8.4.
  3. As sessões ligadas a ela que começaram antes do novo `login_at` são reconstruídas Fresh quando o teammate ficar ocioso (§10.4).
- **Qualquer outro resultado** (fechada à mão, código diferente de 0, cancel, timeout) é `failed` e não mexe em `login_at`, checkpoints nem sessões. O status da conta é atualizado à parte: um login antigo continua "Signed in".

**`agent.auth.*`** continua existindo só para teammates sem conta (§9.3).

### 10.4 Atribuição e troca

`persona.account { personaId, accountId | null }`, na lane do teammate:

- **Valida:** o teammate é local; o agente suporta contas, ou o valor é `null`; a conta existe e é do mesmo backend.
- **Grava a escolha** sob `policy_update_lock`, sem `invalidate` imediato.
- **Escolher a conta já em uso** cancela uma troca pendente, sem reconstruir nada e sem apagar checkpoints. É o "Keep".
- **`Session` guarda** `account_id` e `login_at` com que começou, ao lado de `backend_id`. `SessionInfo.account = { running: Option<String>, wanted: Option<String>, held: Option<"signing_in"> }` (só ids) mostra "In use", o estado pendente e a retenção.

**Aplicar a troca é uma mudança de política**, igual às que existem hoje (reach, ferramentas, harness): `invalidate` → apaga os checkpoints daquele backend → reattach. Ela só acontece quando:

- o teammate não tem trabalho em andamento: nenhum turno na DM, e nenhuma side thread, job, subagente ou sessão de par no meio de um turno;
- a conta escolhida não está retida (status em cache `signed_out`, ou tentativa de login em andamento).

**Momento de aplicar.** Na hora, se o teammate já está ocioso. Senão, o fim de cada turno em qualquer thread do teammate agenda uma verificação. Ela roda numa tarefa que confere de novo sob `start_gate`, porque `rest()` é síncrono, e aplica a troca quando tudo está ocioso.

**Efeitos ao aplicar** (aceitos, porque são os mesmos de qualquer mudança de política hoje):

- O lease antigo morre, então turnos atrasados não gravam checkpoints.
- Side threads paradas, agentes de job e sessões de par do teammate são liberados e recomeçam na conta nova na próxima vez que forem usados.
- Uma DM parada começa direto na conta nova.

**"Stop and switch now"** = `session.stop` (para todo o trabalho do teammate, o que o rótulo diz), seguido da troca e de `session.start` na conta nova.

**A sessão nova** é `session/new` com o briefing Fresh que já existe: o histórico do capítulo aberto e "Earlier execution may be incomplete…".

### 10.5 Limite de uso

- **Kind novo:** `Failure::classify` ganha `Kind::UsageLimit` (e não `Quota`) para os prefixos da linha do agente na fase `acp*`. Os prefixos chegam pelo `ChildAgent`. O título é "{agent} usage limit reached" e o resumo é o texto do próprio agente, que traz o horário de reset. A reescrita para `AgentAuth` não se aplica a esse kind.
- **"Try again" continua** para todos, inclusive System default, como hoje com `acp`.
- **Cartão:**
  - Para teammate de agente com contas, o `ErrorCard` de `usage_limit` oferece "Continue on {conta}" para cada outra conta com status `signed_in` (o cartão dispara uma atualização de status) e "Continue on System default".
  - O clique chama `persona.account`. Como o teammate fica ocioso depois da falha, a troca vale na hora.
  - O notice leva o `account_id` em que a falha aconteceu, para o cartão continuar correto depois de recarregar.

### 10.6 Cadastro

- **`accounts.create { backendId, name, startWith: "system_default" | "empty", categories? }`:** valida, gera o uuid, cria a pasta privada e, com `systemDefault`, copia as categorias pedidas (todas, se omitidas) como em §8.6. Depois grava o setting com `setup` e devolve a conta mais o resultado da cópia (itens copiados, chaves retiradas, links pulados). Um `path` mandado pelo cliente é ignorado. Se a cópia falhar no meio, a conta não é gravada e a pasta fica para inspeção, com a frase do erro.
- **`accounts.rename { accountId, name }`:** muda só o nome.
- **`accounts.remove { accountId }`:**
  - É recusada enquanto algum teammate local referenciar a conta ("{account} can't be removed while {nomes} use it."), sob `policy_update_lock`. `persona.create` com conta também pega esse lock.
  - Senão, retira do setting e nunca apaga a pasta.
  - A confirmação diz que o login continua em `<pasta>` e que dá para deslogar pelo CLI do agente.
- **`accounts.list {}`:** devolve:
  - os agentes com `supportsAccounts` e o nome de exibição;
  - as contas com o status em cache e a tentativa em curso;
  - `usedBy` (ids de teammates) e `folder` (só para desk e dono);
  - `canSignInHere` por seat e plataforma.

### 10.7 Setup

Todos estes comandos rodam fora do laço do socket, como o status.

- **`accounts.setup.list { accountId }`:** devolve `setup` (a origem) e, por categoria, `id` e a contagem de itens. Para os seats Desk e Owner, devolve também o caminho que "Open" abre. Não tem efeito colateral: não cria arquivos. A janela só mostra "Open" no seat Desk e usa `open_account_setup` (§7.6), que cria o arquivo que faltar antes de abrir.
- **`accounts.setup.copy { accountId, categories }`:** copia do System default como em §8.6, atualiza `setup` e devolve o resultado.

## 11. Autorização

- **Comandos novos:** `accounts.*` (inclusive `accounts.setup.*`) e `persona.account` são de operador, para os seats Desk e Owner. O companion é negado porque fica fora da allowlist (`seat.rs` não muda), e Link também é negado.
- **Owner:** um dispositivo pareado como dono (celular ou desktop) gerencia contas, como já faz com `agent.auth.*`. No Windows, o login de conta só começa no seat Desk.
- **Caminhos fechados:**
  - `persona.update` recusa `agentAccountId`.
  - `settings.update` recusa `agentAccounts`.
  - `mobile.persona_create` e `mobile.persona_update` não conseguem expressar o campo.
  - `agent.auth.*` é recusado para teammate com conta.
  - Nenhuma ferramenta de agente toca contas.
  - A visão de colega (`list_teammates`, linhas de desk vinculado) não inclui conta.
- **Privacidade:** o roster do companion apaga conta e `SessionInfo.account`. As recusas públicas não levam caminho nem nome (§10.1).

## 12. Compatibilidade e limites

- **Dados antigos:**
  - sem `agentAccounts`, a lista é vazia;
  - teammate sem `agentAccountId` fica em System default, com o ambiente idêntico ao de hoje;
  - checkpoint, linha `link` ou marcador sem `accountId` vale como System default;
  - o cartão de limite de um System default continua oferecendo "Try again".
- **Downgrade:** um build antigo que reescreve o registro descarta o campo, e o teammate volta ao System default. Isso fica documentado. Um notice `usage_limit` ou `account_unavailable` lido por um cliente antigo aparece como um cartão genérico.
- **Desks vinculados:** contas e pastas nunca atravessam. A recusa pública não diz nada de privado.
- **Mover o diretório de dados:** a pasta é derivada, então continua funcionando.
- **Importador:** a árvore de referência não tem contas, e os campos novos são opcionais.

## 13. Segurança (para a seção nova de `docs/security.md`)

- **Default:** nenhum teammate tem conta; System default é o comportamento atual.
- **Origem da concessão:** o operador, por `accounts.*` e `persona.account`, nos seats Desk ou Owner, sob `policy_update_lock`.
- **Enforcement:**
  - resolução e checagem da pasta antes do spawn;
  - um `EnvPlan` com scrub, aplicado em todos os caminhos;
  - status que confirma provedor, método e pasta;
  - recusas em `persona.update`, `settings.update` e `agent.auth.*`;
  - allowlist do companion;
  - login preso ao socket;
  - troca aplicada como mudança de política (lease antigo revogado);
  - checkpoints com dono `(backend, conta, login_at)`.
- **Testes:** ver §14; cada um é citado na matriz.
- **Risco residual:**
  - O harness confiável pode ler a própria pasta e as de outras contas do mesmo usuário do SO; não é um sandbox.
  - Um teammate Hotline Agent com alcance de máquina inteira pode ler as pastas.
  - Settings de projeto no workspace (`.claude/settings*.json`, vindos de um repositório ou escritos pelo modelo) podem definir `env` ou `apiKeyHelper` que substituem a conta. Isso já vale hoje para System default; o status roda fora do workspace e não detecta. Um aviso no painel quando o workspace tem essas chaves fica como continuação.
  - O setup da conta não é verificado pelo Hotline (decisão do operador). O `settings.json` da conta vale para todos os teammates daquela conta, inclusive hooks que rodam comandos e chaves que substituiriam o login. Neste último caso a conta deixa de aparecer "Signed in", mas a sessão não é bloqueada. A pasta é gravável pelo harness, que roda como o mesmo usuário do SO, então um teammate pode alterar o setup da própria conta.
  - A cópia do setup leva para a conta o que estiver em `~/.claude`: skills, plugins e hooks do operador.
  - Settings gerenciados do sistema injetam ambiente.
  - O navegador pode autorizar a conta errada; a confirmação mostra o email.
  - O email do `--email` aparece na linha de comando do processo de login.
  - A política da Anthropic sobre login claude.ai em terceiros e os termos sobre várias assinaturas.

## 14. Testes e critérios de aceite

| Critério | Prova |
| --- | --- |
| Dois teammates simultâneos recebem pastas diferentes, sem contaminação | harness `crates/hotline-core/tests/agent_accounts.rs`: re-exec com shims `npx`/`claude` (cópias do `hotline-acp-fixture`) à frente do PATH; o fixture ganha `REPORT_ENV` e `CHILD_ENV` (subprocesso); três teammates (Conta A, Conta B, System default) iniciados juntos |
| Login, status e execução resolvem a mesma conta | unit do `EnvPlan` usado pelos três; unit dos dois aplicadores (`Command`, `CommandBuilder`); harness lê o env do status e do filho |
| Credenciais herdadas não substituem a conta | re-exec com `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_CUSTOM_HEADERS` e `CLAUDE_CODE_USE_BEDROCK` definidos: a conta não vê nenhum, o System default vê todos; unit do parse de status recusando `api_key`, `third_party`, `apiKeySource` e `configDirectory` diferente; unit de ordem (catálogo que reintroduz uma variável removida perde) |
| Seleção persiste após reiniciar | harness: atribuir, fechar e reabrir o `Desk` no mesmo diretório; o registro e o próximo spawn usam a conta |
| Dados antigos e System default inalterados | unit de fold com sala sem o setting e com entradas inválidas (filtradas uma a uma); harness com o teammate em System default herdando o ambiente sentinela; unit: limite de uso de System default ainda oferece retry |
| Caminhos Windows com espaço e Unicode | unit e harness com raiz `…/Conta Teste ção/`; unit: junction ou symlink no lugar da pasta é recusado |
| Conta ausente, pasta inacessível, login inválido | unit das recusas estruturadas e das duas formas da frase; harness com conta removida do setting e pasta apagada; unit `AgentAuth` → ação de conta; unit `agent.auth.*` recusado para teammate com conta |
| A troca respeita trabalho em andamento e não retoma a sessão de outra conta | unit (agente roteirizado): o checkpoint da A não é carregado na B (`session/new`); unit de sessão: troca durante turno de DM ou de side fica pendente e aplica quando tudo fica ocioso; "Keep" não apaga checkpoints; checkpoint atrasado do lease antigo não é gravado; vínculo de `link` com outra conta não é retomado |
| Login abandonado não é sucesso | unit: tentativa com saída diferente de 0 ou cancelada sobre uma conta já assinada termina `failed` sem mexer em `login_at` e checkpoints; uma tentativa por conta; status com uma execução por conta |
| Remover conta em uso tem comportamento definido e não apaga credenciais | wire: remoção recusada nomeando os teammates; harness: remoção livre mantém a pasta |
| Autorização do operador | `wire/tests/seats.rs` (Desk/Owner sim, Phone/Link não); `owner_handler.rs` pelo handler real; teste de 4 fases do login preso ao socket (inclui `.input`); login Windows recusado para Owner; `mobile.persona_update` não define o campo; `persona.update` e `settings.update` recusam; visão de colega e roster do Phone (com e sem `lean`) não mostram |
| Limite de uso | unit `Failure::classify` com os prefixos → `UsageLimit` e recusa da reescrita `AgentAuth`; UI: cartão oferece "Continue on …" e "Try again"; o notice leva `account_id` |
| Setup copiado sem segredo | unit com uma origem de fixture que tem `.credentials.json`, `history.jsonl`, `projects/`, `sessions/`, um symlink e um `settings.json` com `apiKeyHelper` e `env.ANTHROPIC_API_KEY`: a conta recebe só as categorias pedidas, sem esses arquivos, chaves e links, e o resultado lista o que foi retirado e pulado; `.claude.json` da conta recebe só `mcpServers` |
| Setup é do operador | `wire/tests/seats.rs` e `owner_handler.rs`: `accounts.setup.*` para Desk e Owner sim, Phone e Link não; caminhos só para Desk e Owner; "Copy again" substitui só as categorias marcadas |
| UI | `ui/tests/agent-accounts.test.tsx` (agentes, página, conta, add com nome em branco, rename, remove recusado e livre, login, estados de status), `teammate-agent-account.test.tsx` (só agentes com contas, seleção, System default = `null`, pendente, Keep, retenção por falta de login, Stop and switch, link do setup), `new-teammate-account.test.tsx` ("Starts with", Change or customize), `agent-account-setup.test.tsx` (lista com contagens, "Open" só no seat Desk, Copy again com categorias) |

**Validação real versus fixtures.**

- Todos os testes acima usam fixtures.
- A validação com contas reais exige que um humano faça login em duas contas Claude, e é feita à mão num `HOTLINE_DATA_DIR` isolado:
  1. criar a Account 1 e a Account 2 e fazer login nas duas;
  2. atribuir cada uma a um teammate e rodar os dois ao mesmo tempo;
  3. confirmar pelo status e pelo uso de cada conta.
- O relatório final diz explicitamente o que foi validado de verdade.
- O spike da sessão completa (§3) usa essas mesmas duas contas e fica fora deste plano.

## 15. Checks (Windows, sem `make`)

```bash
python scripts/fetch-speech-engine
(cd ui && bun install --frozen-lockfile)
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p hotline-contract --locked          # sem filtro, por último
(cd ui && bun run typecheck && bun run lint && bun run knip && bun run test)
PYTHONUTF8=1 python scripts/security-citations
python -m unittest discover -s scripts -p 'test_*.py'
PYTHONUTF8=1 bash scripts/precheck fmt lockfile citations contract clippy ui
```

As pernas específicas de Mac (`windows`, `linux`, `actionlint`, `bare-path`) e o gate `deps` (cargo-machete, cargo-deny) não rodam aqui. Isso é informado com o comando e a causa, sem desabilitar nenhum check.

## 16. Documentação

- `docs/security.md`: seção nova (§13) e linhas na matriz de regressão.
- `docs/sessions.md`: em "An ACP child", contas e ambiente; em "Checkpoints", a chave `(backend, account)`, a limpeza ao aplicar a troca e o login.
- `docs/wire.md`: as linhas de `accounts.*` e `persona.account` e o parágrafo de comportamento.
- `docs/design.md`: um parágrafo da decisão e os campos aditivos de tape e de `link`.
- `docs/development.md`: `agent-accounts/` no layout do diretório de dados.
- Docs de uso: `docs-site/src/content/docs/setup/agent-accounts.md` (cadastrar, setup inicial e edição, entrar, atribuir, trocar, limites, o que a pasta guarda), com link a partir de `setup/teammates.md`.
- `CHANGELOG.md`: uma entrada; não há release nesta entrega.

## 17. O que fica para o construtor

- Nomes exatos e divisão de arquivos dentro do orçamento (sugestão: `crates/hotline-core/src/accounts.rs`, `wire/commands/accounts.rs`, `ui/src/components/settings/AgentAccounts.tsx`, `ui/src/components/teammate/AccountRow.tsx`).
- O lock curto de §8.4 e onde ele mora.
- Verificar num build de release (`windows_subsystem = "windows"`) que o console aberto com `CREATE_NEW_CONSOLE` recebe handles utilizáveis. Se não receber, usar `conhost.exe` com argumentos explícitos ou `CreateProcessW` sem `STARTF_USESTDHANDLES`.
- A cópia exata dos textos da UI, seguindo `ui/design.md`.

## 18. Decisões minhas (para conferir)

1. **Seção própria "Agent accounts"** logo depois de Providers.
2. **Metadados no setting `agentAccounts`,** não no vault: não há segredo do Hotline, e `credential` dispara atualização de modelos.
3. **Status e login pelo mesmo executável da sessão** (`<launch> --cli auth …`).
4. **Sem verificação de login no caminho de início;** conta sem login aparece no primeiro prompt, com a ação da conta.
5. **`ANTHROPIC_BASE_URL` e `ANTHROPIC_CUSTOM_HEADERS` são removidos** para teammates com conta; proxy vai no `settings.json` da conta.
6. **A troca aplica como mudança de política, só com o teammate inteiro ocioso.** Ela libera side threads paradas, agentes de job e pares, como mudar reach faz hoje. Construir um reinício só da DM que mantém threads antigas na conta antiga foi descartado: elas continuariam batendo numa conta esgotada.
7. **Checkpoints apagados ao aplicar** a troca (todas as contas daquele backend), não ao gravar a escolha.
8. **Login com sucesso** = código 0 + status confirmado. Qualquer sucesso avança `login_at` e recomeça Fresh as sessões daquela conta.
9. **Durante a tentativa não há "Check sign-in" manual** nem "Show window": o Hotline confere quando a janela fecha. "Sign in later" cancela. (O protótipo foi alinhado.)
10. **Uma conta "Not signed in" escolhida para um teammate existente fica retida** até ser assinada; o teammate continua na conta atual.
11. **`UsageLimit` como kind novo,** com "Try again" para todos, em vez de reaproveitar `Quota`.
12. **Login pelo console do Windows só no seat Desk.**
13. **Remoção recusada enquanto a conta estiver em uso.**
14. **Conta guardada mas ignorada** se o teammate mudar para um agente sem contas.
15. **Email opcional passado uma vez como `--email`,** validado e nunca guardado.
16. **Setup:**
   - Tudo abre no editor ou no gerenciador de arquivos do sistema, sem editor embutido e sem validação (decisão do operador). Um arquivo ausente é criado vazio antes de abrir.
   - "Copy" substitui a categoria inteira, espelhando pastas e mesclando chaves.
   - Links na origem são pulados, não seguidos.
   - Os servidores MCP são copiados só pela chave `mcpServers`.
   - Uma mudança no setup vale na próxima sessão, sem reiniciar nada.
   - Settings vem desmarcado no "Copy again" do protótipo e marcado na criação, porque recopiar settings apaga as edições de permissões e hooks da conta.
17. **Esta spec fica em `docs/superpowers/specs/`,** como pede a skill. Isso conflita com `AGENTS.md` ("docs/ guarda só o que existe"). Não há commit; posso mover para o board (`.brainfile/`).
