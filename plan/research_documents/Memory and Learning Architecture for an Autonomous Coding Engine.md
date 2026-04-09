# Memory and Learning Architecture for an Autonomous Coding Engine

**An autonomous coding engine that plans, spawns agents, executes against plans, and continues finding work across long-running sessions needs memory that outlives any single context window.** Without durable, structured, evidence-based memory, every agent session starts from zero — re-discovering environment quirks, re-learning repo conventions, and repeating failed strategies. This specification defines a memory and learning architecture that gives the system continuity, robustness, and compounding intelligence across runs, while avoiding the brittleness and delusion that plague naive learning systems. The design draws from the current state of the art across MemGPT/Letta, Zep/Graphiti, Cognee, Mem0, LangGraph, and production coding agents like Devin and Claude Code, synthesized with academic research from Reflexion, ExpeL, MACLA, and the emerging field of agent memory engineering (2024–2026).

---

## 1. Memory goals: what memory must accomplish

Memory serves four irreducible functions in a long-running autonomous coding engine: **continuity** across sessions and context window boundaries, **efficiency** through avoiding redundant discovery, **robustness** through evidence-based decision-making, and **adaptability** through operational learning that compounds over time.

### Why memory is non-negotiable

A 200K-token context window exhausts in hours during active coding sessions. A 500-line TypeScript file consumes 3,000–5,000 tokens. Without memory that survives compaction, session boundaries, and crashes, the system cannot execute multi-day migrations, cannot learn that a specific test suite requires a 30-second timeout, and cannot avoid the deployment pattern that failed three runs ago. Memory transforms a stateless tool-caller into an agent that improves.

### What kinds of memory matter most

Not all memory is equal. The system must distinguish between **what happened** (episodes), **what is true** (semantic knowledge), **what works** (procedural strategies), and **what is currently relevant** (working context). These four strata serve different purposes, decay at different rates, and require different storage and retrieval mechanisms. Additionally, **operator feedback** — corrections, preferences, and constraints from humans — occupies a privileged position: it is higher-trust than agent-generated knowledge and should be harder to decay.

### How memory supports autonomy

Memory enables autonomy by reducing the need for human re-instruction. An agent that remembers "this repo uses `pnpm` not `npm`" does not need to be told again. An agent that remembers "the `auth` module requires database migrations before tests pass" does not block on a preventable failure. An agent that remembers "Claude tends to hallucinate the `user_id` parameter name for this API — the actual field is `customer_uuid`" avoids a class of systematic errors. Each memory eliminates a future interruption or failure.

### Design principles governing all memory decisions

- **Append-only at the event layer.** Raw observations are never mutated. Derived knowledge is versioned and superseded, never silently overwritten.
- **Evidence-weighted, not vote-counted.** Confidence tracks Bayesian posteriors, not simple counts.
- **Scoped by default.** Every memory is tagged with its origin context (repo, language, environment, task type). Cross-scope generalization requires explicit promotion.
- **Inspectable and editable.** Humans can browse, query, correct, and delete any memory. The system is never a black box.
- **Durable but decayable.** All memory persists to disk. Non-reinforced memory decays. Procedural knowledge decays slowly; environmental observations decay faster.

---

## 2. Recommended memory layers

The architecture defines six memory layers, each with distinct semantics, storage characteristics, and lifecycle rules. This taxonomy draws from cognitive science (Tulving's episodic/semantic distinction), OS design (MemGPT/Letta's virtual memory hierarchy), and production agent systems (Zep's three-subgraph model, Cognee's session/permanent split).

### Layer 0: Working memory (the active context)

Working memory is what the LLM sees right now — the assembled prompt comprising system instructions, active task context, relevant retrieved memories, and recent conversation history. It is **ephemeral by nature** (exists only for the current inference call) but **constructed deliberately** from durable lower layers. Target size: **2,000–5,000 tokens** of injected memory context, within a total context budget managed by the orchestrator.

Working memory is not stored — it is compiled. The orchestrator builds it fresh for each agent invocation by selecting from the layers below. This follows Google ADK's principle: separate storage from presentation, and make the compilation step observable and testable.

### Layer 1: Episodic memory (the event log)

Episodic memory records **what happened** — every agent action, tool invocation, observation, decision, error, and outcome as immutable, timestamped events in an append-only log. This is the system's ground truth, the non-lossy data store from which all derived knowledge traces its provenance.

**Characteristics**: immutable, append-only, complete. Never summarized in-place (summaries are derived artifacts stored elsewhere). Serves as the audit trail, the replay source, and the evidence base for learning. Follows Zep/Graphiti's episodic subgraph pattern — raw episodes exist independently, with edges linking them to the semantic entities they reference.

**Retention**: hot (0–90 days, indexed for fast query), warm (3–12 months, compressed), cold (1+ years, archived to object storage). Episodic memory is the largest layer by volume but the least frequently queried directly.

### Layer 2: Semantic memory (what the system knows)

Semantic memory stores **facts, entities, relationships, and constraints** extracted from episodes and external sources. This is the system's knowledge graph — a structured representation of what is true about codebases, frameworks, environments, and operational patterns.

Examples of semantic memories: "`ProjectAlpha` uses React 18 with Next.js 14," "The `payments` module depends on `stripe-node` v12," "Running `make test` in this repo requires PostgreSQL on port 5433," "Python 3.12 changed `asyncio.Task` cancellation semantics in ways that break this codebase's error handling."

**Structure**: property graph with typed nodes (File, Module, Function, Dependency, Framework, Environment, Pattern, Constraint) and typed edges (CALLS, DEPENDS_ON, REQUIRES, CONFLICTS_WITH). Each node and edge carries confidence, evidence count, temporal validity (`valid_from`, `valid_until`), and provenance links to source episodes. Follows Graphiti's bi-temporal model: tracks both when a fact became true and when the system learned it.

### Layer 3: Procedural memory (what works and what doesn't)

Procedural memory stores **strategies, procedures, and action patterns** — reusable sequences of steps that the system has learned succeed or fail in specific contexts. This is the system's skill library, analogous to Voyager's growing code-based skill set or MACLA's hierarchical procedural memory.

Examples: "When adding a new API route in this repo, also update `routes/index.ts`, add a test in `__tests__/routes/`, and run `pnpm lint`," "For React component refactoring, write tests first, then refactor, then verify — this order succeeds 87% of the time vs 52% for refactor-first," "When `npm install` fails with ERESOLVE, try `--legacy-peer-deps` before investigating dependency conflicts."

**Structure**: each procedure has preconditions (when to apply), action schema (what to do), postconditions (expected outcomes), a Beta posterior tracking success rate (α = successes + 1, β = failures + 1), and scope tags. Following MACLA's architecture, procedures support hierarchical composition — chain primitive procedures into meta-procedures with control policies (skip, repeat, abort).

### Layer 4: Operator feedback memory (what humans have said)

Operator feedback memory stores **corrections, preferences, instructions, and constraints** from human operators. This layer is privileged: it has higher base confidence than agent-generated knowledge, decays more slowly, and can override conflicting procedural or semantic memories.

Sources: `CLAUDE.md` / `AGENTS.md` files (version-controlled project instructions), explicit corrections during sessions ("No, use `yarn` not `npm` in this repo"), approved/rejected suggestions, and configuration. This layer maps to the AGENTS.md convergence pattern observed across Claude Code, Cursor, Windsurf, OpenHands, and Codex — the industry has standardized on file-based operator instructions as a memory primitive.

**Critical constraint**: operator instructions consume instruction-following budget. Frontier LLMs follow approximately 150–200 instructions reliably. The system must keep total active operator instructions under this threshold, prioritizing by recency and relevance.

### Layer 5: Meta-cognitive memory (how the system itself performs)

Meta-cognitive memory stores **self-knowledge about the system's own performance** — which models perform best for which task types, which prompting strategies yield better results, how token budgets affect quality, and which coordination patterns reduce agent conflicts.

Examples: "Claude Sonnet 4 produces better test code than Haiku for this repo's test patterns," "Splitting file-editing tasks across more than 3 parallel agents increases merge conflicts," "Context windows above 70% capacity degrade code quality — compact proactively."

This layer is the least mature in current systems and should be treated as experimental in v1, but its existence as a design concept is important for the system's long-term self-improvement trajectory.

---

## 3. Knowledge representation: ontology, graphs, and evolving certainty

### The case for property graphs over flat stores

An autonomous coding engine's knowledge is inherently relational. Files declare functions. Functions call other functions. Tests validate classes. Dependencies conflict with each other. Strategies work for specific task types in specific repos. **Flat key-value or vector-only stores cannot represent these relationships**, and the inability to traverse relationships means the system cannot answer multi-hop questions like "which functions in the payments module call the Stripe API and lack error handling tests?"

Property graphs (Neo4j, FalkorDB, or lighter embedded options like Kuzu) are the recommended representation. They offer flexible schemas, fast traversal via adjacency lists, inline vector storage for hybrid retrieval, and natural support for confidence/temporal metadata as node and edge properties. **RDF/OWL is not recommended** — its formal semantics add complexity without sufficient benefit for an operational coding agent, and its performance characteristics are poor for the dynamic, high-frequency reads this system requires.

### Three-layer graph schema

The knowledge graph is organized into three layers that mirror the memory taxonomy:

**Code structure layer** (populated by static analysis, not LLM extraction): Nodes for File, Module, Class, Function, Variable, Test, Dependency. Edges for DECLARES, CALLS, INHERITS_FROM, IMPORTS, DEPENDS_ON, TESTS, DATA_FLOWS_TO. This layer is populated deterministically via tree-sitter AST parsing across 66+ languages, ensuring completeness and correctness that LLM extraction cannot match.

**Agent learning layer** (populated from experience): Nodes for Pattern, Strategy, Constraint, Outcome, Hypothesis, FrameworkKnowledge. Edges for OBSERVED_IN, WORKS_FOR, CONFLICTS_WITH, CAUSED_BY, EVIDENCE_FOR, SUPERSEDES. Every edge carries `confidence`, `evidence_count`, `valid_from`, `valid_until`, and `source_episode_id`. This layer grows through the learning pipeline described in Section 4.

**Episodic reference layer** (populated from the event log): Nodes for Episode, ToolCall, ReasoningTrace. Edges for TRIGGERED_BY, RESULTED_IN, PART_OF_SESSION. This layer is immutable and serves as the provenance backbone — every fact in the learning layer traces back through evidence chains to raw episodes.

### Representing evolving certainty

Every learned fact carries a confidence score on a continuous [0.0, 1.0] scale with defined semantics:

- **0.0–0.3 (hypothesis)**: Observed once or twice, unverified. Not injected into agent context by default. Available if explicitly queried.
- **0.3–0.6 (emerging pattern)**: Observed multiple times with no contradictions. Injected as low-priority suggestions ("this pattern has been observed but is not confirmed").
- **0.6–0.8 (established pattern)**: Consistently observed across multiple episodes and contexts. Injected as guidance.
- **0.8–1.0 (confirmed knowledge)**: Validated by outcomes, reinforced by repeated evidence, or sourced from operator feedback. Injected as facts.

Confidence updates follow Bayesian mechanics. Each confirming observation: `confidence = min(1.0, confidence + (1 - confidence) × 0.1)`. Each contradicting observation: `confidence = max(0.0, confidence × 0.7)`. Time-based decay: `decayed = confidence × e^(−decay_rate × idle_hours / 168)`, where `decay_rate` varies by memory type (0.01 for procedural, 1.0 for environmental observations, 5.0 for buffer/working hypotheses).

### Inspectability and editability as first-class requirements

The knowledge graph is the system's beliefs made visible. Every fact can be browsed via direct graph queries (Cypher or equivalent). Every fact links to its evidence chain. Humans can directly edit nodes and edges — mark a pattern as deprecated, adjust confidence, add constraints, or delete incorrect knowledge. The system exposes tools for `memory_search`, `memory_inspect`, `memory_correct`, and `memory_export`. Following Graphiti's principle, the graph itself **is** the audit trail.

---

## 4. Learning model: from observations to durable knowledge

The learning model defines how raw experience becomes useful knowledge without becoming brittle, delusional, or dangerous. This is the system's most sensitive subsystem — the difference between an agent that compounds intelligence and one that compounds errors.

### The observation-hypothesis-knowledge pipeline

Learning follows a staged pipeline with explicit gates between stages, drawing from ExpeL's insight extraction, MACLA's Bayesian procedural memory, and the instinct-mcp production implementation:

**Stage 1 — Observation.** During execution, the system records raw events: tool call results, test outcomes, error messages, timing data, human corrections. These are episodic memories — immutable, uninterpreted, complete. No filtering occurs at this stage. Every observation is tagged with full context (repo, language, environment, agent, task type, timestamp).

**Stage 2 — Pattern detection.** A background process (not the active agent) scans recent observations for recurring patterns using contrastive analysis. When the same approach succeeds in context A but fails in context B, the differences are analyzed to extract candidate rules. When the same error recurs across multiple episodes, a candidate constraint is generated. This follows MACLA's contrastive refiner pattern, which demonstrated that comparing success/failure pairs is the primary mechanism for avoiding false rules.

**Stage 3 — Hypothesis formation.** Candidate rules become hypotheses — graph nodes with `status: hypothesis`, initial confidence 0.1–0.3, and links to source episodes. A hypothesis is a claim about the world: "Running `pytest` with `--no-header` in this repo prevents output truncation." Hypotheses are not injected into agent context by default.

**Stage 4 — Evidence accumulation.** As the system encounters situations where a hypothesis is relevant, it tracks outcomes. Each confirming outcome updates the Beta posterior (α += 1). Each contradicting outcome updates it (β += 1). The posterior mean (α / (α + β)) determines confidence. Following MACLA's research, this Bayesian approach cleanly separates high-quality procedures (mean > 0.7, tight variance) from noise (mean < 0.5, wide variance).

**Stage 5 — Promotion or deprecation.** Hypotheses meeting promotion criteria (≥5 confirming observations, confidence > 0.6, no unresolved contradictions) advance to established patterns. Patterns meeting higher criteria (≥10 confirmations, confidence > 0.8, cross-context validation) become confirmed rules. Hypotheses that accumulate contradicting evidence (confidence < 0.3) are deprecated — marked `status: deprecated` with a `deprecated_reason` and `superseded_by` link. **Deprecated knowledge is never deleted**, preserving the audit trail.

### What the system learns

The learning model targets seven specific knowledge categories, each with distinct scoping and decay characteristics:

- **Language and framework behavior in practice.** Runtime surprises, version-specific breaking changes, API parameter names that differ from documentation, common pitfalls. Scoped to language + framework + version. Moderate decay (frameworks evolve).
- **Repository-specific conventions.** Naming patterns, error handling style, test structure, module boundaries, build commands. Scoped to repo. Low decay (conventions are stable within a repo), but invalidated on major structural changes.
- **Execution patterns that succeed or fail.** Which approaches to refactoring/testing/debugging work in which contexts. Scoped to task type + language. Bayesian tracking via procedural memory.
- **Environment and tooling constraints.** Shell behavior, CI/CD pipeline specifics, package manager quirks, port configurations. Scoped to environment. Moderate decay (environments change).
- **Agent prompting and routing strategies.** Which models perform best for which task types, which prompting patterns yield better code, which coordination patterns reduce conflicts. Scoped to model + task type. High decay (models update frequently).
- **Recurring operational patterns.** Common error-fix pairs, debugging workflows, dependency resolution sequences. Scoped to repo or language. Low decay.
- **Operator preferences and corrections.** Human-provided instructions and corrections. Scoped per operator configuration. Very low decay (operator intent is stable).

### Contrastive learning from failures

Following the ExpeL and MACLA pattern, the system's primary learning mechanism is **contrastive analysis of success vs. failure trajectories**. When a procedure succeeds in one context but fails in another, the system identifies differentiating factors and tightens preconditions. This produces scoped, conditional knowledge ("this refactoring pattern works for stateless components but fails for components with useEffect cleanup") rather than brittle universal rules.

### Safe learning boundaries

All learning operates within explicit boundaries:

- **No learning from a single observation.** Minimum evidence thresholds prevent noise from becoming knowledge.
- **All learned knowledge is scoped.** A pattern observed in repo A is tagged to repo A. Generalization to "all repos" requires evidence from multiple repos and explicit promotion.
- **Operator feedback overrides agent learning.** If a human corrects a learned behavior, the correction takes precedence regardless of the system's accumulated evidence.
- **Golden sample testing.** Before any learned procedure is auto-applied (confidence > 0.9), it is validated against representative test cases to ensure it does not degrade performance.
- **Version control with rollback.** All learned modifications are tracked. If performance degrades after a knowledge update, the system can revert to the prior knowledge state.

---

## 5. Retrieval and use of memory

### How agents query memory

Memory retrieval uses a **triple-stream hybrid pipeline** combining three retrieval strategies in parallel, fused via Reciprocal Rank Fusion (RRF), and optionally refined by a cross-encoder reranker:

1. **Vector similarity search** (semantic entry point): The query is embedded and compared against stored memory embeddings using cosine similarity. Returns semantically related memories regardless of exact wording. Latency: 10–50ms. Best for: "How does error handling work in this repo?"

2. **BM25/keyword search** (precision anchor): Full-text search with stemming and domain-specific synonym expansion ("db" ↔ "database", "perf" ↔ "performance"). Returns exact or near-exact matches. Latency: 5–20ms. Best for: specific identifiers, error codes, function names, file paths.

3. **Graph traversal** (structural context): From entry nodes identified by vector or keyword search, traverse N hops in the knowledge graph to gather connected entities, relationships, and constraints. Latency: 50–150ms. Best for: multi-hop questions requiring relationship reasoning.

**Fusion**: RRF score = Σ 1/(k + rank_i(d)) across strategies, with k=60. This rank-based fusion is robust across strategies with different score distributions. Typical weight balance: 60–70% vector, 30–40% keyword, with graph results used to expand context around top-ranked hits.

**Reranking**: For high-stakes queries (planning, validation), a cross-encoder reranker (BGE-reranker-v2-m3 for self-hosted, 278M parameters) scores the top 50 candidates and returns the top 5–10. This yields **20–50% quality improvement** at a cost of 100–250ms additional latency. Critical caveat: most cross-encoders truncate at 512 tokens — code chunks exceeding this length must be truncated explicitly with logging.

### How the orchestrator uses memory in planning and routing

The orchestrator queries memory at four decision points:

- **Task generation**: Before creating subtasks, retrieve repo-specific conventions, known constraints, and procedural knowledge relevant to the goal. This prevents generating tasks that will fail due to known constraints.
- **Agent routing**: Query meta-cognitive memory to select the best model and prompting strategy for each task type. Retrieve past performance data for similar tasks.
- **Context compilation**: For each agent invocation, compile working memory by selecting from retrieved memories using a token budget (default 2,000–4,000 tokens). Priority order: system instructions > active task context > operator feedback > high-confidence relevant memories > historical patterns.
- **Validation**: After task execution, retrieve expected outcomes and known success criteria. Compare actual results against learned patterns to detect anomalies.

### Memory scoping prevents cross-contamination

Every memory query is scoped by default. The scoping hierarchy, from broadest to narrowest: **organization → workspace → repository → language → framework → environment → task type → session**. A query for "testing patterns" in a Python repo does not return JavaScript testing patterns unless explicitly cross-scoped.

Implementation follows Mem0's validation pattern: at least one scope identifier (repo, workspace, or session) is required for any memory operation. Queries without scope identifiers are rejected. Cross-scope queries require explicit `scope_override` with justification logging.

### Avoiding irrelevant or dangerous memory injection

Five mechanisms prevent bad memory from entering agent context:

- **Confidence thresholds**: Memories below 0.3 confidence are never injected unless explicitly requested. Memories between 0.3–0.6 are injected with uncertainty markers.
- **Staleness detection**: Memories not reinforced within their type-specific decay window are tagged as stale and excluded from default retrieval. MemGuard research showed **55% of pricing facts and 15% of job titles become stale within 90 days** — coding-domain equivalents (API versions, dependency versions, environment configurations) likely decay similarly.
- **Provenance filtering**: Memories without clear source attribution (missing episode links) are excluded from high-confidence injection.
- **Relevance scoring**: Retrieved memories below a similarity threshold are excluded. The threshold is calibrated per retrieval strategy (0.7 for vector, configurable per deployment).
- **Cascading retirement**: Following agentmemory's pattern, deprecated facts are tagged so they never pollute context. The `superseded_by` chain ensures queries always resolve to the most current knowledge.

---

## 6. Storage and operational architecture

### Foundational principle: the append-only event log

The event log is the system's single source of truth. Every agent action, observation, decision, and outcome is recorded as an immutable JSONL event. JSONL was chosen because append operations cost **~0.75ms** (vs ~7 seconds for full JSON file rewrite — a 9,000x improvement), crashes during write risk only the last record, and the format is human-readable, grep-able, and trivially parseable.

**Event schema** (versioned from day one):

```
{
  "v": 1,
  "ts": "2026-04-08T14:30:00.000Z",
  "event_id": "evt_01abc",
  "agent_id": "agent_42",
  "session_id": "sess_789",
  "repo_id": "repo_123",
  "event_type": "tool_invocation | decision | observation | memory_update | error | checkpoint",
  "payload": { /* event-type-specific */ },
  "metadata": {
    "model_id": "claude-sonnet-4-5",
    "token_usage": { "input": 150, "output": 500 },
    "cost_usd": 0.003,
    "parent_event_id": "evt_01aab",
    "correlation_id": "task_456"
  }
}
```

Schema evolution follows the additive-only principle: new fields are added with defaults; old events remain valid. The `v` field enables upcasting at read time when schema changes are unavoidable. Events are never modified — if meaning changes, create a new event type.

### Graph-backed views and indexes

The knowledge graph is a **materialized view** over the event log, not a primary store. It is populated by background processors that extract entities, relationships, and patterns from episodes. If the graph is corrupted or lost, it can be reconstructed from the event log (with LLM extraction costs). This architecture provides both the queryability of a graph and the auditability of an event log.

**Graph backends** (pluggable, not locked to one):

- **Neo4j**: Full-featured property graph with native vector search, Cypher queries, and the neo4j-agent-memory library. Best for production deployments requiring multi-hop traversal and rich querying.
- **FalkorDB**: Lightweight, embeddable graph with Redis-compatible protocol. Best for single-machine deployments.
- **Kuzu**: Embedded graph database (Cognee's default). Best for zero-infrastructure local development.
- **SQLite + FTS5**: Surprisingly capable for single-agent systems. Full-text search, JSON support, embedded. Best for v1 prototyping.

### Retrieval indexes

In addition to the knowledge graph, the system maintains:

- **Vector index**: Embeddings of semantic and procedural memories, stored in pgvector, Qdrant, or an embedded option (LanceDB, sqlite-vec). Indexes are partitioned by scope (repo, language) for faster filtered queries.
- **Full-text index**: BM25-compatible index over memory content, entity names, and episode summaries. SQLite FTS5 for embedded deployments, PostgreSQL tsvector for production.
- **Temporal index**: Sorted index on `valid_from` / `valid_until` for time-range queries. Enables "what was true at time T?" queries critical for debugging.

### Operational vs. historical storage

The system implements a five-tier storage hierarchy inspired by MemGPT's OS metaphor:

| Tier | Purpose | Backing store | Latency | Retention |
|------|---------|---------------|---------|-----------|
| **Tier 0** | Core memory blocks (always in-context) | In-process, persisted to disk | Sub-ms | Permanent (compact) |
| **Tier 1** | Session state, active task context | Redis or in-memory with WAL | <1ms | Session lifetime |
| **Tier 2** | Semantic/procedural knowledge graph | Graph DB + vector index | 10–150ms | Permanent with decay |
| **Tier 3** | Full episode archive (recent) | PostgreSQL or SQLite | 50–500ms | 90 days hot |
| **Tier 4** | Cold episode archive | Object storage (S3/GCS) | 500ms–5s | 1+ years |

**Compaction**: Before summarizing or compacting conversation history, the system **flushes important specifics to Tier 2** (semantic/procedural memory). Summaries lose details; structured facts survive compaction. This is the single most important operational pattern for preventing information loss — observed in both Letta's sleep-time agents and Claude Code's Auto Dream.

### Auditability and replayability

Every agent decision is traceable. The event log supports full replay: load events up to timestamp T, reconstruct agent state, inspect intermediate states. This enables post-hoc debugging ("why did the agent delete that test file?") by replaying the decision chain that led to the action.

Decision traces capture **why**, not just what. Each decision event includes the agent's reasoning, the memories that influenced the decision, the alternatives considered, and the confidence level. This goes beyond simple action logging to provide the explainability needed for trust in autonomous operation.

---

## 7. Failure modes and mitigations

Agent memory systems fail in predictable ways. Understanding these failure modes is essential for building a system that degrades gracefully rather than catastrophically.

### Stale memory silently degrades performance

The most insidious failure mode. An agent encodes a build command, a dependency version, or a team member's role. The real world changes. The agent continues using the stale information with full confidence. A real-world case: an operations agent routed approvals to departed directors for weeks because its memory of the org chart was never invalidated.

**Mitigations**: Time-based confidence decay (5% per month without reinforcement). Active staleness detection for high-impact memories (re-verify before use). Environment-change triggers that invalidate affected memories when `package.json`, `Cargo.toml`, or CI configuration changes.

### False rules from superstitious learning

An agent observes a coincidental correlation (test passed after adding a comment, therefore the comment was necessary) and encodes it as a rule. Reflexion research documented cases where self-reflection hallucinated task specifications, reinforcing incorrect understanding with confident reasoning.

**Mitigations**: Minimum evidence thresholds (never promote from a single observation). Contrastive analysis (require both success and failure examples to bound a rule's applicability). Counter-evidence tracking (every rule tracks disconfirming as well as confirming evidence). MACLA's research showed that **73% of pruned procedures had success rates below 0.5** — Bayesian posteriors cleanly separate signal from noise.

### Overgeneralization across contexts

A pattern learned in a Python/Django repo is incorrectly applied to a Rust/Actix repo. A debugging strategy that works on macOS fails on Linux. The system generalizes beyond the scope of its evidence.

**Mitigations**: Mandatory scope tagging on all learned knowledge. Cross-scope generalization requires evidence from multiple scopes and explicit promotion. MACLA's precondition tightening — when a procedure fails in a new context, contrastive analysis identifies which preconditions were violated and narrows the rule's applicability.

### Memory bloat degrades retrieval quality

Without decay and compaction, memory grows unbounded. Retrieval quality degrades as irrelevant results crowd out relevant ones. Research showed agents exhibit systematic miscalibration — showing green checkmarks while having functionally forgotten **87% of stored knowledge** under complexity.

**Mitigations**: Tiered decay rates by memory type. Weekly compaction that merges semantically similar memories (cosine distance < 0.20). Deduplication at write time (reject new memories within cosine distance 0.15 of existing ones, merge instead). Maximum retrieval budget (inject at most N memories per query).

### Poisoned or adversarial memory

Malicious content injected through processed documents, emails, or external data gets stored in semantic memory and recalled during future operations. Microsoft's red team demonstrated **40–80% success rates** for memory poisoning attacks on email-processing agents. MITRE classifies this as AML.T0080.

**Mitigations**: Provenance tracking on all memories (what source, what agent, what context). Confidence caps on externally-sourced memories. Row-level security for multi-tenant deployments (namespace isolation alone is insufficient). Anomaly detection on incoming memories (flag statistically unusual patterns). Human review gates for high-impact memory additions.

### Self-reinforcing feedback loops

A single hallucination stored in memory poisons subsequent reasoning, which generates further hallucinated evidence, which reinforces the original error. By the time performance degradation becomes visible, corruption has spread across the memory store.

**Mitigations**: Ensemble verification for high-impact memories (confirm via multiple independent observations). Separation of evidence sources (a memory cannot be its own evidence). Golden sample testing — maintain representative test cases and validate that learned changes don't degrade performance. Version control with instant rollback for all learned modifications.

### Conflicting knowledge without resolution

Different agents, sessions, or contexts produce contradictory memories. Without resolution, the system behaves inconsistently depending on which memory is retrieved first.

**Mitigations**: Automatic conflict detection (new memories checked against existing ones in the same scope; cosine distance 0.15–0.40 flags potential conflicts). Resolution rules: more recent evidence supersedes older evidence (with provenance chain); operator feedback overrides agent-generated knowledge; higher-confidence knowledge takes precedence. All superseded knowledge retains `superseded_by` links for audit.

---

## 8. Recommended memory design: the practical architecture

### What should be in v1

The v1 architecture prioritizes **operational value with minimal infrastructure complexity**. It implements the memory layers using the simplest viable storage backends, with clean interfaces that allow backend upgrades without architectural changes.

**V1 storage stack**:

- **Event log**: JSONL files, one per session, with a manifest file tracking all sessions. Schema-versioned from day one. This is non-negotiable — the event log is the foundation for everything else.
- **Semantic/procedural memory**: SQLite with FTS5 for full-text search and sqlite-vec for vector similarity. Single-file database, zero infrastructure, ACID transactions. Adequate for single-machine deployments and prototyping.
- **Operator feedback**: Markdown files (`CLAUDE.md` / `AGENTS.md` pattern) version-controlled alongside code. Loaded at session start, re-read after compaction. This format has been validated across Claude Code, Cursor, Windsurf, OpenHands, and Codex.
- **Working memory compiler**: A deterministic pipeline that reads from the above stores and assembles context for each agent invocation. Explicitly testable — given the same inputs, produces the same context.

**V1 learning pipeline**:

- **Observation recording**: All events logged to JSONL. No filtering.
- **Pattern detection**: Post-session background process scans episodes for recurring patterns using simple frequency analysis + LLM-based contrastive analysis.
- **Hypothesis management**: Hypotheses stored in SQLite semantic memory with confidence tracking. Promotion threshold: ≥5 confirmations, confidence > 0.6. Deprecation threshold: confidence < 0.3.
- **Operator override**: Humans can directly add, edit, or delete any memory via CLI tools or file edits.

**V1 retrieval pipeline**:

- **Dual-stream**: Vector similarity (sqlite-vec) + full-text search (FTS5), fused via RRF. Graph traversal deferred to v2.
- **Scoping**: Mandatory repo scope on all queries. Language and task-type scoping as optional filters.
- **Context budget**: 2,000-token default for injected memories, configurable per task type.

**V1 deliberately excludes**: knowledge graph database (complexity), cross-encoder reranking (latency/infrastructure), meta-cognitive memory (insufficient data), auto-compaction of the knowledge store (manual trigger only), multi-tenant memory isolation (single-user assumption).

### What should be in v2

V2 upgrades storage backends and adds the full knowledge graph:

- **Replace SQLite with PostgreSQL** (pgvector for vectors, native FTS for text, JSONB for flexible schemas). Single database providing ACID, row-level security, and rich querying.
- **Add property graph** (Neo4j or FalkorDB) for the code structure layer and multi-hop retrieval. Populate from tree-sitter AST parsing.
- **Add cross-encoder reranking** (BGE-reranker-v2-m3, self-hosted) for planning and validation queries.
- **Implement the full triple-stream retrieval pipeline** (vector + BM25 + graph traversal).
- **Add sleep-time processing** (following Letta's pattern): background agents that consolidate, deduplicate, and decay memories during idle periods.
- **Add environment-change triggers**: watch `package.json`, `Cargo.toml`, CI configs, and invalidate affected memories on change.

### What should be in v3 and beyond

- **Multi-agent memory consistency protocols**: formal models for concurrent memory access, conflict resolution, and ordering guarantees across parallel agents.
- **Meta-cognitive memory**: systematic tracking of model performance, prompting strategy effectiveness, and coordination pattern outcomes.
- **Cross-repo knowledge transfer**: controlled generalization of patterns from one repository to another within the same organization.
- **Durable execution integration**: Temporal.io or equivalent for crash-safe, long-running workflow orchestration with automatic checkpointing (following OpenAI Codex's architecture).
- **Memory-aware planning**: the planner uses learned procedural knowledge to estimate task duration, predict failure modes, and proactively allocate resources.

### What should be configurable

Configuration should cover tuning parameters without requiring architectural changes:

- **Decay rates** per memory type (default: 0.01 for procedural, 1.0 for environmental, 5.0 for buffer)
- **Promotion thresholds** (default: 5 confirmations for pattern, 10 for rule)
- **Context budget** for injected memories (default: 2,000 tokens)
- **Compaction trigger** (default: 60–70% of context window)
- **Scope hierarchy** (default: org/workspace/repo/language/task_type)
- **Retrieval strategy weights** (default: 60% vector, 40% keyword)
- **Confidence thresholds for injection** (default: 0.3 minimum, 0.6 for unprompted)
- **Storage backends** (SQLite for dev, PostgreSQL for production, with pluggable graph backends)

### Open questions and future research needs

Several questions remain unresolved and should be tracked as active research areas:

- **Optimal evidence thresholds for promotion**: The 5/10 confirmation thresholds are pragmatic defaults. Rigorous calibration against real-world coding agent performance data is needed. The POPPER framework's e-value-based sequential testing may provide a principled alternative.
- **Memory consistency in multi-agent systems**: No formal model exists for cache coherence across concurrent LLM agents. Current approaches (last-writer-wins, orchestrator-mediated) are fragile. This is the largest conceptual gap identified in the research.
- **Balancing stability and plasticity**: How aggressively should the system update its beliefs when new evidence conflicts with established knowledge? Too aggressive leads to instability; too conservative leads to stale knowledge. The optimal balance likely varies by knowledge type and context.
- **Memory-performance feedback loops**: How to detect when learning itself is degrading system performance, especially in cases where the degradation is gradual and distributed across many memories?
- **Cross-modal memory**: Current designs focus on textual/code knowledge. How should memory represent visual information (UI screenshots, architecture diagrams) as multi-modal models become standard?
- **Forgetting as a feature**: The Ebbinghaus-inspired decay models (SuperLocalMemory V3.3) and Cognee's `memify()` pruning suggest that principled forgetting is as important as remembering. The optimal forgetting curves for different knowledge types in coding domains are unknown.
- **Cost-quality tradeoffs in memory operations**: Every LLM-driven memory operation (extraction, consolidation, contrastive analysis) costs tokens. At what point does the cost of memory management exceed the value of the memories produced? This boundary shifts as model costs decrease.

---

## Conclusion: memory as the competitive moat

The autonomous coding engine's memory system is not a feature — it is the architecture's central nervous system. Without it, every session starts cold. With it, the system accumulates institutional knowledge that would otherwise exist only in engineers' heads, and it compounds that knowledge across every run.

The key architectural insight from this research is that **memory must be layered, evidence-gated, and scoped** — not a single store with a single retrieval mechanism. The industry has converged on hybrid architectures (vector + graph + keyword), tiered storage (hot/warm/cold with compaction), and LLM-driven extraction with Bayesian confidence tracking. The most successful production systems (Zep/Graphiti, Mem0, Cognee, Letta) all implement some variant of this pattern.

The most dangerous failure mode is not forgetting — it is **remembering wrong things with high confidence**. Every design decision in this specification, from Bayesian posteriors to mandatory scoping to contrastive learning to golden sample testing, serves the goal of ensuring that what the system learns is actually true, is bounded to the contexts where it applies, and can be corrected when it is not. An autonomous system that learns well is powerful. An autonomous system that learns poorly is dangerous. The architecture must make the difference unmistakable.