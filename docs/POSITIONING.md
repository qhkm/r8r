# r8r Positioning & USP

> **Date:** 2026-08-15
> **Status:** Working strategy — evolves with user feedback
> **Companion docs:** [MARKET_RESEARCH.md](MARKET_RESEARCH.md) (market validation), [TODO.md](TODO.md) (engineering roadmap)

## 1. The One-Liner

> **r8r is the trust layer for autonomous agents. Agents propose. r8r disposes — deterministically, safely, on budget.**

Positioning alternatives (shorter, for landing page):

- "Let your agent act. r8r keeps it honest."
- "Your agent's safety net."
- "Durable workflows, safe by default, for AI agents."

## 2. Why the current pitch is not enough

r8r's existing README pitch — *lightweight agent-native workflow engine, YAML-first, cloud to edge, 15MB, 50ms cold start* — is a **feature list, not a USP**:

- Every serious player in the category now claims "AI workflows" (n8n 200K★, Inngest, Hatchet, Kestra, Windmill, Activepieces).
- Light + fast + YAML is *necessary* but *insufficient*. Nobody adopts a durable execution engine because it is 15MB.

A USP must own a **problem nobody owns**, aimed at a **sharper buyer**.

## 3. The wedge: trust (control) over durability

Competitors answer the question **"will it finish?"** (durability: Temporal, Hatchet, Inngest).
Nobody answers **"can I trust it?"** (control: budgets, permissions, approvals, audit).

Evidence the problem is real and unpaid:

- Claude Code post-launch backlash: the #1 recurring sentiment is *"great, but I only run it in plan mode — I don't trust it with execute."*
- Runaway token bills; agents modifying files / doing "too much without asking."
- Platforms bolt on half-measures (approval UIs, hooks) — none make control a first-class durable system.

**The unclaimed combination (nobody cleanly owns all five):**

1. **Trust-first**: permission matrix, per-turn/per-session cost caps, human-in-the-loop approvals, arm/disarm, audit trail — as the core product.
2. **Durable execution**: retries, exactly-once, state persistence, deterministic failure routing.
3. **YAML-first + LLM-generatable**: agents author their own workflows ("prompt → safe workflow"), validated against the permission matrix before execution.
4. **MCP-native**: agents invoke governed workflows as MCP tools.
5. **Lightweight self-host**: single Rust binary (~15MB, ~50ms cold start), dev laptop to edge.

## 4. The core loop (what makes it click)

```
1. Agent proposes a workflow (YAML) — or calls a role
2. r8r validates against permission matrix + budget policy
3. r8r executes deterministically: retries, idempotency, state, audit
4. Dangerous/non-approved steps → human approval gate
5. Agent gets the result; operator gets the kill switch + full log
```

The YAML is LLM-generatable **because** the guardrail system makes that safe. Freedom for the agent, control for you.

## 5. Competitive matrix

| Competitor | Sells | Missing vs r8r |
|---|---|---|
| Temporal | Durable execution | Heavy (cluster + service), no guardrail model, not YAML-first |
| Hatchet | Durability for AI background tasks | No trust/cost/approval layer as product, not YAML-first |
| n8n / Activepieces | Human workflow tools + agent APIs | Human-first (visual, heavy), not a safety layer |
| Windmill | Scripts → workflows + UIs | Heavier platform, not agent-native (no MCP-first story) |
| LangGraph / CrewAI | Agent *frameworks* | No runtime engine; they need exactly what r8r is |
| Inngest | Durable step functions / AI workflows | Cloud-first, not self-host-light, no guardrail layer |

## 6. How we win (execution playbook)

### 6.1 Beachhead
Target **people who already trust agents but fear the consequences**:
- Solo founders / indie devs running Claude Code, Codex, Cursor with real side effects.
- AI agencies automating client operations.
- NOT enterprises (long sales cycles, feature requests we don't want to chase yet).

**Dogfooding is the proof.** ZeptoClaw + r8r runs the founder's real operations (AISAR automation, Threads ops, client workflows). Publish it: *"We run our business on r8r."*

### 6.2 Hero demo (the 30-second hook)
The marketing asset: *an agent tries something scary (spend, delete, bulk email) → r8r blocks it / asks a human / stops on budget.*
That demo IS the positioning. Everything else supports it.

### 6.3 Distribution
1. Make the repo public (AGPL-3.0 already set) with the trust-first README story.
2. Show HN launch ("I built a safety net for my agents because Claude Code scared me").
3. List in MCP server registries (agents discover r8r as a tool).
4. Comparison pages: "vs Temporal", "vs Hatchet", "vs Claude Code hooks" — the proven play, every peer does it.

### 6.4 Moat
- **Trust layer lock-in**: once your ops depend on budgets/approvals/audit, switching costs are real.
- **YAML schema purpose-built for LLM generation** → format + community workflow templates lock-in.
- **Rust footprint** → JVM/Node competitors cannot follow to edge/dev-laptop usage.
- **Operational depth** from real dogfooding (ZeptoClaw).

## 7. Honest risks

- n8n has approvals; Claude Code has hooks. Incumbents *could* bolt on guardrails.
- Window = category-creation credibility: ship the trust layer as a first-class product *publicly*, faster than they assemble it.
- Zero public users today; demand for *r8r specifically* is still unproven (see MARKET_RESEARCH.md).

## 8. Plan B wedge (for contrast — pick ONE headline)

**Edge/embedded agent automation**: Pi / Jetson / industrial gateways / offline Ollama. Real and largely uncontested, but a **different buyer** (embedded/industrial vs developer). Keep it as supporting proof in the README; do not split the headline.

## 9. Next 30 days (actionable)

1. **Rewrite README + landing** around the trust wedge. Light/fast/YAML become supporting proof, not the headline.
2. **Build the hero demo** (agent tries a scary action → r8r blocks/approves/stops). Record it.
3. **Sharpen guardrail features to product quality**: permission matrix, cost caps, approvals, arm/disarm, audit — these are THE product now.
4. **Recruit 3–5 beta users** from the beachhead segment (founders running agents, AI agencies). Learn from their workflows before adding features.
5. **Public launch sequence**: public repo → Show HN → MCP registry → comparison pages → dogfood case study.

---

*This doc is strategy, not marketing copy. It should evolve after every meaningful user conversation.*
