# Contas por provedor — design

- **Data:** 2026-10-22
- **Estado:** aprovada pelo operador em 2026-10-22, depois do builder check e da resposta sobre a escolha por provedor (§6.2). Próximo passo: plano de implementação.
- **Base de código:** branch `feature/agent-accounts` em `870ae957` (PR #202, rascunho), que este trabalho reescreve.
- **Pedido do mantenedor** (issue #199): "make all account types oauth or API key multi-capable as a primitive, so we can have multiple of anything - then at the teammate level we could choose an account from a new account dropdown when multiple are detected for a model/provider type". Mais tarde, trocar o backend de um teammate (Claude → Codex, ou Claude → Claude) com continuidade por capítulos e transcrições.
- **Proposta aprovada pelo operador:** `docs/superpowers/plans/agent-accounts/proposal/accounts-proposal.html` e as telas `1-providers.png` … `5-usage-limit.png` (fora do repositório, no checkout principal), e o comentário publicado na issue #199.
- **Spec anterior:** `docs/superpowers/specs/2026-10-21-agent-accounts-design.md` (contas de agente do #202). Tudo o que ela decide e este documento não muda continua valendo: pastas, `EnvPlan`, status, login, setup, recusas, privacidade.

A interface do produto é em inglês; os textos de UI citados aqui são literais. O resto está em português.

---

## 1. Intenção

Hoje o Hotline tem dois sistemas de conta que não se conhecem:

- **Credenciais de provedor** do Hotline Agent (chaves de API e os logins ChatGPT, Copilot, xAI e OpenRouter). Um provedor já pode guardar várias, mas só a primeira criada é usada (`Vault::connections_from`, `vault.rs:394-412`), e um teammate escolhe só o modelo.
- **Contas de agente** do #202 para harnesses: um login de CLI numa pasta, hoje só do Claude Code, gerenciado numa página própria, Settings › Agent accounts.

O operador quer, e o mantenedor pediu, **uma única ideia de conta**: qualquer jeito de entrar num provedor pode existir várias vezes, cada um com nome, e cada teammate escolhe um quando há mais de um que sirva para ele. Exemplos:

- Developer e Reviewer em duas assinaturas Claude diferentes, rodando ao mesmo tempo (já existe no #202).
- Researcher numa chave Anthropic de produção e Builder numa chave pessoal, os dois no Hotline Agent.
- Dois logins do ChatGPT ou do Copilot, cada um para um teammate.
- Quando uma conta atinge o limite, continuar em outra do mesmo provedor sem sair da conversa.

O caso de uso é da comunidade, não de uma instalação: distribuir trabalho entre assinaturas e chaves, separar gastos e continuar trabalhando quando uma conta esgota.

## 2. O que foi decidido (aprovado pelo operador)

1. **Um lugar só: Settings › Providers.** O layout atual continua: Add provider, as linhas de provedor, Use for, Live calls, Budgets. A linha de um provedor diz o que ele guarda, por exemplo "2 subscriptions · 2 API keys · All models".
2. **A página de um provedor lista todos os jeitos de entrar nele**, em duas seções:
   - **Subscriptions:** os logins de CLI (as assinaturas Claude do Claude Code) e a linha embutida "This computer", que é o System default de hoje.
   - **API keys:** quantas houver.

   Cada linha tem nome, status, "Used by …", Rename e Remove. "+ Add subscription" e "+ Add API key" são a primeira linha da sua seção. "Models in the picker" continua. A página Settings › Agent accounts some; o conteúdo dela vem para cá.
3. **A escolha de conta no teammate** aparece só quando existe mais de uma conta que serve para ele, em New teammate e no painel do teammate:
   - um teammate do Hotline Agent escolhe entre as contas do provedor do seu modelo: chaves de API e logins como ChatGPT, Copilot, xAI e OpenRouter;
   - um teammate do Claude Code escolhe entre as assinaturas Claude.

   A lista termina com "+ Add subscription" e "+ Add API key", que abrem o formulário inline já usado no #202 e selecionam a conta nova sem sair do formulário. Sem escolha, o comportamento é o de hoje.
4. **Vínculo.** O teammate guarda o id da conta ao lado do id do modelo, nunca dentro de `modelId`, para que o `session.set_model` de um celular não consiga trocar de conta. A regra "a primeira credencial criada vence" dá lugar à conta escolhida, e continua valendo quando não há escolha.
5. **Troca.** Mudar de conta usa a troca do #202: espera o teammate ficar ocioso, revoga o lease e começa uma sessão Fresh com a conversa; os checkpoints pertencem a (backend, conta). Os dados ficam prontos para uma troca de backend futura (Claude → Codex) reaproveitar o mesmo mecanismo. **A UI de troca de backend não é construída.**
6. **Cartão de limite de uso.** Um seletor único substitui um botão por conta. A conta sugerida já vem selecionada (assinaturas primeiro, chaves marcadas "pay per use"), seguida de "Continue" e "Try again later". O cartão continua compacto: o seletor abre por cima, sem aumentar o cartão, e tem busca quando a lista é longa.

## 3. Objetivos

1. **Várias contas de qualquer tipo por provedor**, cada uma com nome que o operador escolhe e pode mudar.
2. **Uma escolha por teammate**, persistida, que manda em todos os turnos dele, e que só aparece quando há o que escolher.
3. **Settings › Providers como o único lugar** para ver, criar, renomear, entrar de novo e remover contas, inclusive as assinaturas de CLI.
4. **Troca segura** pelo motor do #202, também para o Hotline Agent.
5. **Continuar no limite** com um seletor que funciona com 3 contas ou com 30.
6. **Compatibilidade:** salas existentes migram sem mudar de sentido. Ausente quer dizer o comportamento de hoje.

## 4. Não objetivos (nesta entrega)

- **Chaves de API ou URLs base para agentes de CLI** (pergunta Q1 ao mantenedor, §15). Um teammate do Claude Code vê só assinaturas. O modelo de dados aceita acrescentar chaves depois sem mudar o vínculo (§6.3).
- **Voz, imagens, notas de capítulo e completions avulsas** seguindo a conta do teammate (pergunta Q2, §15). Continuam com a conta padrão do provedor, como hoje. (A sonda que confere um login depois de um turno recusado já usa o mapa do próprio turno e passa a seguir a conta, §7.2.)
- **UI de troca de backend.** `persona.update {backendId}` continua como está.
- **Lista de modelos por conta.** O seletor de modelos e "Models in the picker" continuam vindo da conta padrão do provedor (§6.4).
- **Contas de outros agentes de CLI** (Codex, opencode). O modelo aceita; só `claude-acp` tem uma linha de suporte.
- **Rotação automática** de contas ao atingir o limite.
- **Escolher a conta padrão** de um provedor. Continua sendo a primeira criada.
- **Escolha de conta pelo celular.** O celular continua sem ver nem atribuir contas.

## 5. Anti-objetivos (fracasso mesmo que "funcione")

- Um teammate passa a usar outra conta sem o operador escolher, inclusive porque a escolhida sumiu, saiu ou atingiu o limite.
- Um celular, um colega ou o próprio modelo troca a conta de um teammate.
- Qualquer parte de um segredo (chave, token, fragmento de chave) chega a stream, tape, log, wire ou contrato.
- Uma sala que não usa contas muda de comportamento ou de bytes na tape.
- A escolha aparece para quem tem uma conta só, enchendo formulários de quem não precisa.
- Remover uma conta em uso deixa um teammate num estado indefinido.

## 6. O modelo

### 6.1 O que é uma conta

Uma **conta** é um jeito de entrar num provedor, com nome. Ela tem:

- `id`;
- `providerId` (o provedor cuja página a lista);
- uma **seção**: `subscription`, `api_key` ou `server`;
- `name`;
- quem a usa.

O armazenamento continua onde cada segredo mora hoje, porque as regras de segurança de cada um são diferentes e já estão provadas:

| Conta | Onde fica | Seção |
| --- | --- | --- |
| Login de CLI de um agente (#202) | setting `agentAccounts` + pasta `<data>/agent-accounts/<id>`; o segredo é da CLI | `subscription` |
| Login OAuth do Hotline Agent que é plano: ChatGPT (`openai-codex`), Copilot (`github-copilot`), xAI | evento `credential` + vault | `subscription` |
| Chave de API do Hotline Agent | evento `credential` + vault | `api_key` |
| Login do OpenRouter | evento `credential` + vault; o login gera uma chave paga por uso | `api_key` |
| Servidor Ollama local | evento `credential` com `baseUrl` | `server` |

O primitivo pedido pelo mantenedor não é um terceiro armazenamento. É **um vínculo** (`Persona.accountId`, §6.3), **uma regra de quais contas servem para um teammate** (§6.2), **uma leitura** que lista todas as contas com quem as usa (§9), **um motor de troca** (§8) e **uma página** (§10). Juntar os dois armazenamentos misturaria a pasta sem segredo do #202 com o vault, que guardam coisas diferentes sob regras diferentes de `docs/security.md`, sem que nada fique mais simples para quem usa.

Conexões personalizadas (`openai-compatible`, guardadas como `custom-<uuid>`) já são um provedor cada uma. Elas continuam como estão: uma linha e uma página de edição cada, sem seções.

### 6.2 Quais contas servem para um teammate

Uma única regra no core, `fits(teammate)`, também espelhada na janela para desenhar os seletores:

- **Agente de CLI com linha de suporte** (hoje `claude-acp`): as contas de agente daquele backend, mais "This computer". Até a Q1, nenhuma chave.
- **Agente de CLI sem linha de suporte:** nenhuma; não há seletor.
- **Hotline Agent com modelo `P/m`:** as credenciais vivas (não revogadas) do provedor `P`. Uma conexão personalizada tem uma só; não há seletor.

O seletor aparece quando `fits` tem duas ou mais contas. Para o Claude Code isso quer dizer pelo menos uma assinatura além de "This computer".

**A escolha nunca atravessa provedores** (decisão do operador, 2026-10-22). ChatGPT (`openai-codex/…`) e OpenAI (`openai/…`) são provedores diferentes, com ids de modelo diferentes: quem roda no ChatGPT escolhe entre logins do ChatGPT, e quem roda numa chave OpenAI escolhe entre chaves OpenAI. A tela 4 do protótipo, que punha "OpenAI key" e "ChatGPT Pro" lado a lado, fica assim. Continuar numa conta de outro provedor (uma assinatura que acabou indo para uma chave de outro provedor) é trocar o modelo junto, e fica para a troca de backend e modelo que o mantenedor tem no backlog. Por isso, no cartão de limite (§10.5), a seção "API keys · pay per use" só aparece onde o mesmo provedor tem os dois tipos: xAI e OpenRouter hoje, e a Anthropic para o Claude Code se a Q1 for sim.

A linha de suporte de um agente ganha `provider_id` (`"anthropic"` para `claude-acp`): é por ela que as assinaturas Claude aparecem na página Anthropic. Um Codex futuro apontaria para `openai-codex`, a página do ChatGPT.

### 6.3 O vínculo do teammate

- `Persona.agentAccountId` passa a se chamar **`accountId`**, e o mesmo em `PersonaDraft`. Ele guarda:
  - num agente de CLI, o id de uma conta de agente; ausente é "This computer";
  - no Hotline Agent, o id de uma credencial; ausente é a conta padrão do provedor do modelo, a primeira credencial viva criada (a regra de hoje).
- **Um id que pertence a outro backend ou a outro provedor fica guardado e é ignorado**, e o teammate usa o padrão daquele provedor ou agente. Ele volta a valer se o teammate voltar. Isso estende o "guardado mas ignorado" do #202 ao caso de o modelo mudar de provedor, que é o que um `session.set_model` pode fazer.
- **Um id que não existe em lugar nenhum** recusa a partida, como a conta desconhecida do #202: "Pick a {provider or agent} account for {teammate}: the one it was set to is not on this desk." Nunca há fallback silencioso.
- Só `persona.account` e o rascunho de `persona.create` gravam o campo; `persona.update` o recusa, como no #202.
- O nome muda porque o mesmo campo passa a guardar uma credencial do Hotline Agent, que não é uma "conta de agente". O #202 nunca foi publicado: um registro gravado por um build do #202 perde a escolha (o teammate volta a "This computer"), o que só afeta diretórios de validação.

### 6.4 A conta padrão do Hotline Agent

- O padrão de um provedor é a regra de hoje (`Vault::connections_from`): a primeira credencial na ordem do stream que não está revogada e ainda tem o seu segredo ou a sua pasta de login. A página a marca como "Default", e `usedBy` e "Default" usam essa mesma regra, de um lugar só.
- Teammates sem escolha seguem o padrão a cada turno, como hoje: se o padrão for removido, eles passam ao próximo.
- Uma atualização no lugar (§11.2) mantém o id e a posição no stream, então nunca muda qual conta é o padrão.
- **O que continua vindo do padrão** (Q2 e §4): voz, imagens, notas de capítulo, `Agents::complete`, a lista de modelos e a descoberta de modelos.

## 7. Resolução no core

### 7.1 Agente de CLI

Como no #202 (`accounts::resolve`, checagem da pasta, `EnvPlan`), lendo `accountId`.

### 7.2 Hotline Agent

- **Na construção do driver** (`DeskAgents::agent`), o driver recebe o id escolhido (ou o `from` de uma troca pendente, como no #202) e o guarda pela vida da sessão. O modelo ainda não está decidido nesse ponto (`InProcess::start` o escolhe, e `set_model` o muda sem reconstruir), então aqui só se confere que o id existe: um id que não é credencial nem conta de agente recusa a partida (§6.3), com o kind `account_unavailable` e `code: "unknown_account"` que o #202 já usa.
- **A cada turno**, o driver aplica o id guardado se ele for uma credencial do provedor do modelo daquele turno, e o ignora se não for. Um teammate que vai para outro provedor e volta reencontra a sua conta. O mapa de autenticação do turno é o de hoje, com a entrada do provedor da conta substituída por ela:
  - uma conta viva entra com a sua autenticação;
  - uma conta revogada ou que sumiu faz o turno falhar como `auth`, com "Choose account" no cartão;
  - em nenhum caso entra outra credencial do mesmo provedor.
- Quem paga e o orçamento (`ChatMeter`), a sonda que confere um login depois de um 401/403 e o `recovery::Origin` usam esse mesmo mapa, então seguem a conta que serviu o turno. O veredito que a sonda guarda por 60 segundos passa a ser por credencial, não por provedor, para dois logins do mesmo provedor não dividirem a resposta.
- **Notices de falha** de um turno do Hotline Agent levam `accountId` (a credencial que serviu o turno) só quando há escolha: o teammate tem uma conta escolhida ou o provedor tem mais de uma conta. Uma sala de uma conta só escreve os mesmos bytes de hoje.
- **O nome de uma conta é privado, como no #202.** Notices vão para a tape, e o celular lê tapes: elas levam só o id, e o cartão da janela do desk ou do dono procura o nome.

### 7.3 Limite de uso no Hotline Agent

- `quota` (crédito esgotado numa chave) passa a oferecer o seletor de contas no cartão (§10.5), como `usage_limit`.
- **Limite do ChatGPT:** um 429 do cliente ChatGPT com `error.type == "usage_limit_reached"` vira `usage_limit`. Hoje ele é `rate_limit` e é tentado três vezes à toa. O formato vem do Codex CLI (`openai/codex`, `codex-rs/codex-api/src/api_bridge.rs`: `UsageErrorBody { type, plan_type, resets_at }`, com `resets_at` em segundos desde a época). O resumo do cartão diz quando o limite volta a partir de `resets_at`, quando há; o corpo bruto fica só nos detalhes. Um teste fixa o corpo citado.

## 8. Troca de conta

O motor do #202 (`session/accounts.rs`) passa a valer para todo teammate que usa contas: um agente de CLI com linha de suporte e o Hotline Agent.

- `persona.account { personaId, accountId | null }` valida pela regra de §6.2 e grava, sob `policy_update_lock`. No Hotline Agent, a credencial tem de estar viva e ser do provedor do modelo atual.
- **A troca aplica quando o teammate está ocioso**, revogando o lease e reconstruindo a sessão. No Hotline Agent a sessão nova é o driver em processo semeado com a conversa do capítulo, que é o que já acontece quando o desk reinicia. Não há checkpoints a apagar.
- **"Keep {current}"** e **"Stop and switch now"** como no #202.
- **Escolher a conta em que a sessão já roda** (o mesmo id) cancela uma troca pendente sem reconstruir nada, como no #202. Passar de "sem escolha" para a escolha explícita da conta padrão é uma troca como outra qualquer: a sessão é reconstruída quando o teammate ficar ocioso, para o driver passar a guardar o id e não seguir mais o padrão.
- **Retenções** ("not signed in", "signing in") valem só para contas de CLI. Uma credencial revogada não serve e não pode ser escolhida.
- **Mudar o modelo para outro provedor** não é uma troca de conta: vale no próximo turno, como hoje, e o teammate usa o padrão do novo provedor (§6.3).
- **Pronto para a troca de backend:** os dados que a troca precisa são o trio (backend, modelo, conta) no registro e o par (backend, conta) nos checkpoints, capítulos e `link`. Uma troca de backend futura grava os três juntos e usa este mesmo motor. Nada disso é construído agora.

## 9. Wire

| Comando | Mudança |
| --- | --- |
| `accounts.list` | Passa a ser a leitura única das contas. Responde também `credentials`: cada credencial com `usedBy` (os teammates que ela serve, inclusive os que seguem o padrão), `default` e a sua seção (§6.1). `AccountAgent` ganha `providerId`. Só Desk e Owner, como hoje. |
| `credential.create` | O nome é o que o operador digitou (§11.1). |
| `credential.login` | Ganha `label?` (sem ele, um nome livre, §11.1) e `id?`: com `id`, entra de novo naquela mesma conta (§11.2). |
| `credential.connect_local` | Ganha `label?`. |
| `credential.update` (novo) | `{ id, label?, secret?, baseUrl? }`: renomeia, troca a chave ou troca a URL de um servidor no lugar (§11.2). Só Desk e Owner. |
| `credential.delete` | Recusado enquanto um teammate local tem essa conta escolhida, sob `policy_update_lock`: "{name} can't be removed while {teammates} use it." O "Sign out" de um login continua sendo este comando. |
| `persona.account` | Vale também para o Hotline Agent (§8). |
| `persona.create` | O rascunho leva `accountId`, validado pela mesma regra. |

Os comandos `accounts.*` do #202 continuam para as contas de CLI.

O seat Phone continua sem `credential.*`, `accounts.*` e `persona.account` (a permissão vem só do seat; `lean` muda só a forma das respostas). O roster do seat Phone e de qualquer socket `lean` apaga `accountId` e `session.account`, como apagava `agentAccountId`.

## 10. Interface

O protótipo aprovado (telas 1 a 5) é a referência visual, com os ajustes de §16.

### 10.1 Settings › Providers (tela 1)

- **Uma linha por provedor que guarda alguma conta:** credenciais, vivas ou revogadas, ou contas de agente cujo agente aponta para ele. Uma conexão personalizada continua sendo uma linha.
- **Detalhe da linha:** as contagens por seção ("2 subscriptions · 2 API keys"), mais "All models" ou "N models" quando o provedor tem uma credencial do Hotline Agent; "Signed out" quando só tem contas revogadas. As tags (Chat · Images) continuam.
- **"Add provider"** lista os provedores que ainda não estão na lista. Escolher um com mais de um jeito de entrar pergunta qual, como o OpenRouter já faz. A Anthropic ganha "Claude subscription" quando o Claude Code pode rodar neste desk.
- A navegação de Settings perde "Agent accounts".

### 10.2 Página do provedor (tela 2)

- **Cabeçalho:** inicial, nome e contagens.
- **Subscriptions:**
  - "+ Add subscription", quando o provedor oferece login de plano ou tem um agente de CLI que aponta para ele.
  - **Conta de CLI:** ponto de status, nome, "status · email · Used by …", Rename e ›. A seta abre a página da conta do #202 (login, Check, Used by, setup, pasta, Remove), cujo "voltar" leva à página do provedor.
  - **Login OAuth:** nome, "Signed in · Used by …", Rename, "Sign in again" (no lugar, §11.2) e "Sign out".
  - **"This computer" (Built in):** quando um agente de CLI aponta para o provedor: "{agent}'s own sign-in, as before · Used by …".
- **API keys:** "+ Add API key"; cada linha com nome, "API key · pay per use · Used by …" ("Signed in with OpenRouter · pay per use" para o login do OpenRouter), Rename, "Change key" (no lugar, §11.2) e "Remove key".
- **Servers** (Ollama local): "+ Add server"; nome, URL, Used by, Rename, "Change server" (no lugar) e "Disconnect".
- **Uma credencial revogada** (só `credential.revoke` produz uma, e a janela não o envia) aparece com "Signed out" e só "Remove".
- **A conta padrão** do Hotline Agent leva "Default" no detalhe quando o provedor tem mais de uma.
- **Dica abaixo das seções:** "A teammate on {provider} picks one of these when it has more than one to choose from."
- **"Models in the picker"** como hoje, mostrado só quando o provedor tem uma credencial viva do Hotline Agent.
- **Adicionar** abre inline o formulário que já existe: `ConnectProvider` com um campo Name para chave, login e servidor, e `AddAccountForm` do #202 para assinatura de CLI. Um nome em branco vira um nome livre: "{provider}", "{provider} 2", e assim por diante; para assinatura de CLI, "Account N", como no #202.
### 10.3 New teammate (tela 3)

- **Claude Code:** campo "Account" com o seletor sobreposto, quando há ao menos uma assinatura. Seção "Subscriptions" (assinaturas com status e "This computer"); rodapé "+ Add subscription", que abre o `AddAccountForm` e o login inline e seleciona a conta nova. Começa em "This computer".
- **Hotline Agent:** sob o modelo, quando o provedor do modelo tem duas ou mais contas. Seções por tipo, rodapé com "+ Add subscription" e/ou "+ Add API key" conforme o provedor oferece. Começa no padrão; ficar nele não grava `accountId`.
- Trocar o modelo para outro provedor volta a escolha para o padrão.
- A dica diz "Rename or remove accounts later in Settings › Providers."

### 10.4 Painel do teammate (tela 4)

- **Seção "{provider} account"** (Hotline Agent) ou **"{agent} account"** (CLI), em rádio, pela mesma regra de §6.2. "In use" marca a conta que serve a sessão, ou o padrão quando não há escolha.
- Termina com as linhas de adicionar.
- Bloco de troca pendente, retenções e nota Restored/Fresh como no #202. A nota Fresh do Hotline Agent: "On {account} · Fresh: a new session on this account, handed this conversation."
- O link "{account}'s setup" continua só para contas de CLI.

### 10.5 Cartão de limite de uso (tela 5)

- **Vale para** `usage_limit` (CLI e ChatGPT, §7.3) e `quota` (Hotline Agent), quando o teammate tem ao menos uma outra conta que serve.
- **Linha:** "Continue on" [seletor] [Continue] [Try again later].
- **O seletor:**
  - abre por cima, ancorado no botão, sem aumentar o cartão;
  - tem busca quando há mais de 8 contas;
  - seções "Subscriptions" e "API keys · pay per use";
  - tira a conta que falhou;
  - nas contas de CLI, só as que estão "Signed in" (o cartão pede o status ao abrir, como no #202) e "This computer".
- **A sugestão** é a primeira assinatura disponível na ordem da lista, senão "This computer", senão a primeira chave.
- **"Continue"** chama `persona.account`, que aplica na hora porque o teammate está ocioso. O cartão passa a dizer, como no #202: "Now on {account}: a fresh session that is handed this conversation. The failed turn may have done part of its work, so Try again tells it to check before repeating anything.", com "Try again".
- **"Try again later"** fecha a oferta e não envia nada; o cartão fica como registro.
- **Sem outra conta,** o cartão fica exatamente como hoje: `usage_limit` com "Try again", `quota` sem ele.

## 11. Nomes, substituição e remoção

### 11.1 Nomes

- O rótulo de uma credencial passa a ser o nome que o operador vê. `credential.rename` muda só ele.
- **Únicos por provedor e seção, sem diferenciar maiúsculas,** com 1 a 64 caracteres, contados em caracteres. A criação e o rename recusam um nome repetido. Duplicados antigos ficam até alguém renomear.
- Credenciais existentes mantêm o rótulo que têm: o nome do provedor para chaves e logins, "Ollama Local" para servidores, o nome digitado para conexões personalizadas. Duas chaves antigas da Anthropic aparecem como "Anthropic", "Anthropic" até serem renomeadas; o detalhe da linha as distingue pela data ("Added Oct 21").

### 11.2 Atualização no lugar

"Change key", "Change server" e "Sign in again" mudam a conta **no lugar**: mesmo id, mesmo nome, mesmos teammates, mesma posição no stream (o log dobra eventos pelo id e mantém o lugar do primeiro), então a conta padrão nunca muda por causa disso.

- `credential.update { id, secret }` troca a chave guardada no vault sob o mesmo id.
- `credential.update { id, baseUrl }` troca a URL de um servidor e descarta a lista de modelos descoberta para aquele id, que era do servidor antigo.
- `credential.login { providerId, id }` faz o login de novo na pasta daquela credencial. Os tokens novos só substituem os antigos quando o login termina com sucesso; um login cancelado ou que falhou deixa a conta como estava.
- Nenhuma delas reconstrói sessões: o turno seguinte usa a autenticação nova, como já acontece hoje quando uma chave muda. Uma resposta guardada que o provedor não aceite mais cai na recuperação de `replay` que já existe.

Isso substitui o "adiciona e depois apaga" que a janela faz hoje (`Providers.tsx:102-113`). Com várias contas por provedor, aquilo mudaria o padrão em silêncio (a conta nova vai para o fim do stream) e esbarraria na remoção recusada.

### 11.3 Remoção

- Uma conta escolhida por algum teammate não pode ser removida (§9), nem uma credencial nem uma conta de CLI (#202).
- A conta padrão pode ser removida quando ninguém a escolheu explicitamente: os teammates sem escolha passam ao próximo, como hoje.

## 12. Segurança

Atualiza a seção "Agent accounts" de `docs/security.md`, que passa a se chamar "Accounts" e cobre os dois tipos.

- **Default:** nenhum teammate tem conta. Sem `accountId`, um agente de CLI é "This computer" (ambiente herdado, nada removido nem acrescentado) e o Hotline Agent usa a primeira credencial viva do provedor, como hoje.
- **Origem da concessão:** o operador, por `persona.account`, `persona.create`, `credential.*` e `accounts.*`, nos seats Desk e Owner, sob `policy_update_lock`.
- **Enforcement,** no core, fora do modelo:
  - a resolução na partida e o mapa de autenticação de cada turno, que nunca põe outra credencial no lugar da escolhida;
  - `persona.account` e `persona.create` validam pela regra de §6.2;
  - `persona.update` recusa `accountId`;
  - `credential.delete` é recusado enquanto a conta está escolhida;
  - a allowlist do seat Phone não tem `credential.*`, `accounts.*` nem `persona.account`, e `session.set_model` não leva conta;
  - a troca aplica como mudança de política, revogando o lease antigo;
  - tudo o que o #202 já faz para as contas de CLI.
- **Dados privados:**
  - nenhum fragmento de chave vai para a janela; o protótipo mostrava "····a1b2" e isso foi retirado (§16);
  - o roster do seat Phone e de sockets `lean` apaga `accountId` e `session.account`;
  - notices e recusas públicas levam só o id da conta, nunca o nome (§7.2).
- **Testes:** as linhas novas da matriz (§13).
- **Revogabilidade:** uma escolha nova aplica quando o teammate fica ocioso, com o lease antigo revogado. A remoção é recusada enquanto a conta está em uso.
- **Risco residual:**
  - um `session.set_model` do celular para outro provedor leva o teammate ao padrão daquele provedor, como hoje;
  - voz, imagens, notas de capítulo, completions avulsas e a lista de modelos ainda usam o padrão (Q2);
  - o login do OpenRouter continua com preço zero e fora do orçamento de Chat (`pricing.rs`), o que já acontece hoje;
  - os riscos do #202 para contas de CLI continuam.

## 13. Testes e critérios de aceite

| Critério | Prova |
| --- | --- |
| A conta escolhida serve os turnos, e a sem escolha usa a primeira | harness em `crates/hotline-core/tests/`: dois servidores Ollama locais como duas contas do provedor `ollama`; um teammate na segunda conta faz o pedido chegar ao segundo servidor, e um sem escolha, ao primeiro |
| Nunca há fallback para outra credencial | unit do mapa por turno: conta revogada ou sumida falha como `auth` sem usar a outra credencial; o id é aplicado só no provedor do modelo do turno e volta a valer quando o modelo volta; harness: remover a escolhida é recusado |
| A troca no Hotline Agent espera o teammate ficar ocioso e começa Fresh | unit de sessão (teste do #202 estendido): troca durante um turno fica pendente; Keep cancela; o driver reconstruído guarda o id novo |
| A atualização no lugar não muda o padrão | unit do vault: trocar a chave ou a URL da primeira credencial mantém o id e o padrão; um login de novo cancelado deixa os tokens antigos |
| Celular não troca conta | `wire/tests/seats.rs` e roster: o celular não tem os comandos; `session.set_model` não muda `accountId`; o roster do celular, com e sem `lean`, não traz o campo |
| Escolha validada | wire: credencial de outro provedor, revogada ou desconhecida é recusada em `persona.account` e `persona.create`; `persona.update` recusa `accountId` |
| Renomear, atualizar, remover | wire: rename único por provedor e seção; `credential.update` e `credential.login {id}` mantêm id, nome e teammates; delete recusado nomeando os teammates |
| `accounts.list` | wire: credenciais com `usedBy` (escolha explícita e padrão), `default` e seção; agentes com `providerId`; só Desk e Owner |
| Limite de uso | unit `Failure::classify`: o corpo citado do ChatGPT vira `usage_limit`; UI: o seletor do cartão sugere a primeira assinatura, marca chaves "pay per use", tem busca acima de 8 e chama `persona.account` |
| Compatibilidade | unit de fold: sala sem `accountId` e sem `agentAccounts` se comporta como hoje; notice do Hotline Agent numa sala de uma conta só não ganha `accountId` |
| UI | testes em `ui/tests/`: Providers (linhas por provedor e contagens), página do provedor (seções, adicionar, renomear, remover recusado, padrão), New teammate (seletor só com 2+, adicionar seleciona), painel (rádio, pendente, Keep), cartão (seletor, Continue, Try again later) |
| Sem regressão do #202 | todos os testes do #202 continuam passando, com `agentAccountId` renomeado |

**Validação real** com o operador no fim, num `HOTLINE_DATA_DIR` isolado: duas contas reais de um provedor do Hotline Agent e duas assinaturas Claude. Tudo o que precisa de conta real para antes e pergunta ao operador.

## 14. Documentação

- `docs/design.md`: o parágrafo de contas de agente vira "Accounts", com o vínculo único, a regra de §6.2 e o fim de "first created wins" para quem escolhe.
- `docs/security.md`: a seção "Accounts" (§12) e linhas novas na matriz.
- `docs/wire.md`: os comandos de §9.
- `docs/sessions.md`: a conta do Hotline Agent e a troca.
- Docs de uso: `docs-site/src/content/docs/setup/agent-accounts.md` vira parte de `setup/providers.md` (contas de um provedor, escolher, trocar, limite), com o link de `setup/teammates.md` atualizado.
- `CHANGELOG.md`: uma entrada.

## 15. Em aberto com o mantenedor (não decidir aqui)

- **Q1:** o Hotline pode guardar e injetar chaves de API ou URLs base em agentes de CLI? Até a resposta, agentes de CLI só têm assinaturas. Se for sim, é uma linha a mais na regra de §6.2 (as chaves do provedor da linha de suporte) e um `EnvPlan` que define a chave em vez de removê-la, como mudança de capacidade em `docs/security.md`.
- **Q2:** voz, imagens e notas de capítulo seguem a conta do teammate ou uma escolha do desk? Até a resposta, continuam com o padrão. Nada aqui impede as duas respostas: a conta de um teammate é lida de um lugar só, e o padrão também.

Uma resposta que chegar na issue #199 vai primeiro ao operador.

## 16. Decisões minhas (para conferir)

1. **Dois armazenamentos atrás de um vínculo, uma regra, uma leitura e uma página** (§6.1), em vez de um armazenamento novo.
2. **`agentAccountId` renomeado para `accountId`** (§6.3), sem ler o nome antigo.
3. **Uma conta de outro backend ou provedor é ignorada, não recusada** (§6.3). O #202 recusava uma conta de outro backend.
4. **Remover uma conta escolhida é recusado também para credenciais**, e "Change key", "Change server" e "Sign in again" atualizam a conta no lugar (§11.2), sem reconstruir sessões. "Sign out" continua apagando o login.
5. **Sem fragmento de chave na tela.** O protótipo mostrava "····a1b2"; mostrar isso exigiria guardar parte da chave no stream.
6. **O login do OpenRouter fica em "API keys",** porque gera uma chave paga por uso; os logins de ChatGPT, Copilot e xAI ficam em "Subscriptions".
7. **Ollama local ganha uma seção "Servers"**, a terceira, porque um servidor não é assinatura nem chave.
8. **O seletor do cartão vale também para `quota` e para o limite do ChatGPT** no Hotline Agent (§7.3).
9. **"Try again later" só fecha a oferta.** "Try again" volta depois de "Continue", como no #202.
10. **"Default" marcado na conta padrão** quando o provedor tem mais de uma, e sem ação para mudá-la.
11. **Nomes únicos por provedor e seção** na criação e no rename; duplicados antigos ficam.
12. **`accountId` nas notices do Hotline Agent só quando há escolha** (§7.2), para não mudar os bytes de quem tem uma conta só.
13. **O nome de uma credencial é privado**, como o de uma conta de CLI: notices levam só o id (§7.2).
14. **Passar de "sem escolha" para a conta padrão explícita reconstrói a sessão** (§8), uma vez, em vez de manter um driver que segue o padrão com um registro que diz outra coisa.

## 17. O que fica para o construtor

- Os nomes exatos dos tipos novos de contrato e a divisão de arquivos dentro do orçamento.
- O texto exato da UI que este documento não cita, seguindo `ui/design.md` e o protótipo.
- O mecanismo do mapa por turno (um método novo em `ProviderKeys` ou um embrulho no driver), desde que o mapa venha de um lugar só.
