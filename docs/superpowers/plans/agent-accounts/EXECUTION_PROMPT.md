Implemente a funcionalidade "contas de agente por teammate" no Hotline seguindo o plano aprovado, usando a skill superpowers:subagent-driven-development.

Repositório: C:\Users\kelviny\stellar\hotline (clone de https://github.com/1broseidon/hotline, base e05716c4 / Release 0.41.0). Máquina Windows 11, shell Git Bash, sem `make`.

LEIA ANTES DE COMEÇAR
- AGENTS.md, docs/standards.md, docs/design.md, docs/security.md, docs/development.md
- Spec aprovada: docs/superpowers/specs/2026-10-21-agent-accounts-design.md
- Plano aprovado (13 tarefas): docs/superpowers/plans/2026-10-21-agent-accounts.md
- Nomes e decisões vinculantes: docs/superpowers/plans/agent-accounts/CONTRACT_SHEET.md e docs/superpowers/plans/agent-accounts/RESOLUTIONS.md (em conflito, vale RESOLUTIONS)
- Textos de UI aprovados: docs/superpowers/plans/agent-accounts/prototype/*.dc.html

EXECUÇÃO
1. Crie o branch `feature/agent-accounts` a partir do `main` atual. O primeiro commit adiciona `docs/superpowers/` (spec, plano e anexos), que ainda não estão versionados.
2. Antes do primeiro build: `python scripts/fetch-speech-engine` e `cd ui && bun install --frozen-lockfile`.
3. Execute as tarefas na ordem 1 → 13 com subagent-driven-development:
   - um implementador novo por tarefa;
   - TDD: teste falhando, depois implementação mínima, depois teste passando;
   - um revisor independente aprova cada tarefa antes de a próxima começar;
   - uma revisão do branch inteiro no final.
4. Siga o plano à risca. Se o código real contradisser um passo, faça o menor ajuste possível e registre o desvio numa seção "Desvios" no fim do plano. Não mude decisões da spec sem me perguntar.

REGRAS QUE NÃO PODEM SER QUEBRADAS
- O Hotline instalado pode estar rodando (talvez até esta conversa). Use um `HOTLINE_DATA_DIR` isolado, por exemplo `.hotline-dev` no checkout. Não toque nos dados reais (`%APPDATA%\Hotline`), não substitua a instalação e não publique release.
- Não encerre processos por nome ou porta; só os PIDs que você mesmo iniciou.
- Nunca use `std::env::set_var`. Tokens, segredos e o JSON bruto do CLI nunca vão para log, tape, stream ou wire.
- Rode `cargo test -p hotline-contract` sem filtro, como última execução antes de commitar `ui/src/generated/contract.ts`.
- Não desabilite checks nem testes para ficar verde. Um teste que depende de conta real fica `#[ignore = "needs …"]`.
- Commits de uma linha no estilo do repositório, declarando o invariante. Não faça push nem PR sem eu pedir.

CHECKS NO FIM (a Task 13 lista os comandos e o resultado esperado de cada um)
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test -p hotline-contract --locked`
- `bun run typecheck && bun run lint && bun run knip && bun run test` em `ui/`
- `PYTHONUTF8=1 python scripts/security-citations`
- `python -m unittest discover -s scripts -p 'test_*.py'`
- `PYTHONUTF8=1 bash scripts/precheck fmt lockfile citations contract clippy ui`

Se algum check não puder rodar aqui (por exemplo cargo-machete/cargo-deny ou as pernas de precheck específicas de Mac), informe o comando, a causa e a limitação.

VALIDAÇÃO REAL
Ela precisa de mim para fazer login em duas contas Claude. Quando chegar nela (protocolo na Task 13), pare e me peça, num `HOTLINE_DATA_DIR` isolado. Não declare validação real se só os testes com fixtures rodaram.

FORA DE ESCOPO
- Rotação automática de contas.
- Levar a sessão completa do Claude Code de uma conta para outra (fica para um teste separado depois).

ENTREGA FINAL, EM PORTUGUÊS
- Como cadastrar e autenticar duas contas.
- Como atribuí-las a teammates diferentes.
- Como a troca de conta afeta uma sessão.
- Quais testes passaram (com os comandos) e quais validações reais foram feitas.
- Quais limitações e riscos ficaram.
- Os desvios em relação ao plano e à spec.
