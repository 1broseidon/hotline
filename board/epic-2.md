---
id: epic-2
title: Research Jev alternatives to heuristic agent operations
type: epic
column: todo
position: 3
tags:
  - improvement
  - bro-78
createdAt: "2026-10-11T21:04:50.164Z"
description: |-
  ## Reopened 2026-10-01: decisions as a provider capability

  Since this was cancelled, more than one model speaks the same typed-decision API (state + typed questions in, probabilities out): Jev on OpenRouter (`/api/alpha/decisions`, `/api/v1/systemone`), Together AI's open Tev1 (4B and 0.8B, local on Ollama `/v1/systemone`, about 2k-token context), and Kev 4B. OpenAI announced a Decisions API in limited preview on 2026-09-29. One adapter covers the shared contract, and Ollama lets a server desk decide without its transcripts leaving the machine. Public benchmark accuracy: Jev 76%, Tev1 4B 73%, Tev1 0.8B 64%. Good enough to skip routine work; never the only check on something risky.

  **Shape:** following "capabilities, not screens", "decisions" comes from providers already connected (OpenRouter key → Jev, local Ollama → Tev1, OpenAI when GA). It gets its own resolver beside the images and speech resolvers and a `CapabilityOptions` field. With no provider, today's behaviour stays exactly as it is. Every automatic decision is visible in the conversation. This is a deliberate departure from quiet/narration's "kind, never content" rule, so it stays opt-in.

  **Uses, in order (sub-issues):**

  1. Notification loudness: needs you now vs for your information.
  2. Answer routine permission prompts: allow once when the call is in scope and reversible, otherwise a card; never auto-deny.
  3. Chapter source selection: the existing lead below, Jev only because of context size.

  Later candidates: voice-call teammate routing (faster than a generative tool call); detecting a stuck long-running teammate. Not worth doing: failure classification/retries (fixed rules are right), first-contact approval (too rare), per-turn model routing (OpenRouter's Jev Router already covers it as a model).

  ---

  Continue researching where TypeSafe’s Jev can improve Hotline’s mechanical agent operations that currently depend on fixed heuristics. Jev supplies typed judgments (Choice/Noul/Score); application code owns the resulting policy, and the generative LLM continues to write responses and handoff notes.

  The goal is better decisions and preservation of context. This is a research backlog item; the existing experiments have not changed Hotline’s application behavior.

  ## Candidate areas

  | Area | Current limitation / hypothesis | Next evidence needed |
  | -- | -- | -- |
  | **Chapter source selection — strongest current lead** | Fixed first-8k/last-48k truncation can discard constraints, corrections, decisions and unfinished work in the middle. Jev can rank source blocks within the same budget. | Blind-label real sources; compare evidence retained against both current truncation and a cheap heuristic; then evaluate paired generated handoff notes if selection improves. |
  | **Adaptive effort, potentially model routing** | Static effort settings and simple complexity/failure rules may miss changing task difficulty. Context-informed judgments could choose an appropriate effort or defer to the existing setting. Model routing is a later extension, not yet tested. | Judge real recommendations, then compare actual task outcomes under fixed and adaptive policies. Classification alone cannot establish better accuracy or lower total latency. |
  | **Recovery and risk triage** | Fixed retry counts, generic escalation rules, or keyword-based risk checks can miss whether a failure is understood, a blocker remains, or an action has uncertain effects. | Build bounded replay cases for retry/replan/escalate recommendations and false escalations. Keep deterministic permissions and explicit user authorization authoritative. |

  Skill/MCP relevance selection remains a secondary comparison: the generative agent may already choose effectively with a good harness. Earlier server selection was conservative (reported precision 1.00 on six positive selections, recall 0.33 against observed tool use); observed agent behavior is not a reliable label for actual need.

  ## Evidence so far

  **Chapter experiment,** `jev-1.13.0`**:**

  | Selector | Synthetic important facts retained |
  | -- | -- |
  | Current head/tail | 28/37 |
  | Cheap user-first / failure / recency control | 30/37 |
  | Jev with the same deterministic anchors as the cheap control | 37/37 |

  * Six long synthetic position-stress cases plus one short control. These are authored tests, not independent evidence of real-world accuracy.
  * Five approved oversized real chapters (408,050 serialized, redacted characters) completed successfully. All three selectors stayed within the baseline’s rendered character ceiling. Real quality is **still unlabeled**.
  * 27 real batch calls, zero errors; HTTP p50/p95 about 524/608 ms per batch; 4.89 seconds for the full five-chapter run with three workers. Batch latency is not chapter latency.
  * 371,246 reported real input tokens: approximately **$0.0156 total** at the quoted $0.042/MTok input and free output. Cost is a minor consideration at this scale.
  * Key failure mode to examine: keeping middle passages displaces other evidence. Whole-block packing can also lose extra text, especially for a chapter barely over budget.
  * All arms retain existing per-message (2,000 characters) and tool-output (240 characters) limits. Jev cannot recover text removed at those earlier steps.
  * Real chapters reconstruct currently persisted, compacted tapes, rather than exact historical closure-time inputs. Generated chapter notes were excluded from inputs and labels.
  * Ten chapter-harness tests pass.

  **Effort experiment:**

  * Forty real requests compared with and without prior context, plus 16 authored probes.
  * Context changed 25/40 recommendations; 15/16 synthetic probes and all six pairwise checks passed.
  * 27/40 context-informed choices had confidence below 0.5. Confidence describes distribution concentration, not the probability of choosing the best effort.
  * No counterfactual task outcomes exist yet, so improved accuracy and compute savings remain unproven.

  ## Continuation plan

  - [ ] Blind-label the five real chapter sources before revealing selector identities. Assess constraints/corrections, decisions/rationale, open work, and uncertain execution; record misleading omissions as well as retained quotes.
  - [ ] Compare Jev against the production baseline **and** the cheap control. Inspect displaced tail evidence and barely oversized chapters.
  - [ ] If source selection improves, run paired handoff generation with the same model, prompt and budget, then assess continuation quality.
  - [ ] Identify concrete heuristic seams in `app/` / `hotline-core` for effort and recovery/risk triage; prioritize by likely accuracy benefit and evaluate against simple deterministic alternatives.
  - [ ] Define per-category success and regression criteria before further tuning; retain held-out cases, latency, failure/fallback behavior, and cost measurements.
  - [ ] Produce a go/no-go recommendation per category. Any later product integration should be optional, off by default, backed by TypeSafe provider configuration, and enabled only for validated categories.

  ## Research artifacts

  Paths are relative to the local Hotline workspace; the experiment files are currently local/uncommitted:

  * `lab/hack/jev-chapters/harness.py`, `README.md`, `FINDINGS.md`
  * `lab/hack/jev-chapters/results/real/review.json` — blind review; keep `review-key.json` closed until labeling is complete
  * `lab/hack/jev-effort/harness.py`, `FINDINGS.md`
  * `lab/hack/jev-relevance/harness.py` — earlier relevance experiment

  Raw transcripts, requests, responses and review sources remain private in gitignored local results directories; this issue records aggregate findings only.

  [TypeSafe documentation index](<https://docs.typesafe.ai/llms.txt>) · [Score primitive](<https://docs.typesafe.ai/primitives/score>) · [Speculative fan-out](<https://docs.typesafe.ai/patterns/fan-out>)

  Migrated from Linear BRO-78: https://linear.app/1broseidon/issue/BRO-78/research-jev-alternatives-to-heuristic-agent-operations (2026-10-11).
updatedAt: "2026-10-11T21:20:14.962Z"
---
