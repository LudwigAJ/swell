# Master Product + Architecture Specification: Autonomous Coding Engine

---

## 1. Executive summary

**This specification defines an autonomous coding engine — an agentic IDE that plans, writes, tests, and delivers production-quality code with calibrated human oversight.** The system combines a hierarchical multi-agent orchestrator, a hybrid memory and knowledge graph layer, MCP-based tool integration, sandbox-isolated execution, and a multi-stage validation pipeline into a coherent product that operates across the full software development lifecycle.

The product enters a market projected to grow from **$7–8B in 2025 to $24–30B by 2030**, where 85% of developers already use AI tools and 22% of merged code is AI-authored. Yet critical gaps remain: current tools solve only **23% of enterprise-level tasks** (SWE-bench Pro), produce code with **1.7× more issues** than human-written code, and lack reliable autonomous operation for anything beyond well-scoped, single-file changes. No existing product adequately combines deep codebase understanding, safe autonomous execution, cross-project learning, and calibrated human oversight into a single coherent system.

What makes this product different is a specific architectural bet: **separating the generation, evaluation, and learning loops into independently improvable subsystems**, connected through a stateful orchestrator with formal safety controls. Where Cursor optimizes the human-in-the-loop editing experience and Devin optimizes for fully autonomous task execution, this engine occupies the middle ground — providing autonomous capability with graduated oversight levels, persistent cross-session memory, and a validation pipeline that treats all agent-generated code as untrusted by default.

---

## 2. Product thesis

### Core hypothesis

An autonomous coding engine that combines hierarchical multi-agent orchestration, persistent memory with knowledge graphs, and defense-in-depth validation can reliably execute complex, multi-file software engineering tasks at a quality level that passes human code review **>80% of the time** — while providing transparent oversight and cost-predictable operation.

### Target users

**Primary:** Professional software engineers at mid-size to large engineering organizations (50–500 engineers) who need to scale output without scaling headcount. These teams maintain complex, multi-service codebases and struggle with the gap between current AI tools' demo capabilities and real-world reliability.

**Secondary:** Engineering leads and tech leads who need to delegate well-scoped implementation work (bug fixes, feature implementations from specs, test coverage improvements, refactoring) while retaining architectural control.

**Tertiary:** Solo developers and small teams building products who want an agent that learns their codebase, conventions, and patterns over time rather than starting from zero context each session.

### Value proposition

The engine delivers three things no current product combines effectively:

1. **Reliable autonomous execution with safety guarantees.** Tasks complete with validated output and full audit trails, not "it compiled so I guess it works." Every change passes through deterministic quality gates before any human sees it.

2. **Persistent, accumulating intelligence.** The system learns your codebase, conventions, failure patterns, and architectural decisions across sessions and projects. Session 100 is dramatically more effective than session 1.

3. **Graduated autonomy with cost predictability.** Users choose oversight levels from "pair programming" to "fully autonomous with review," and costs are predictable per-task rather than opaque credit-burning.

### Competitive positioning

| Dimension | Cursor | Devin | Claude Code | This Engine |
|-----------|--------|-------|-------------|-------------|
| Primary mode | IDE-integrated copilot | Fully autonomous agent | Terminal agent | Graduated autonomy agent |
| Context depth | Session-based, file-level | Session-based, task-scoped | Session-based, grep/search | Persistent memory + knowledge graph |
| Validation | User runs tests manually | Agent runs tests | Agent runs tests | Multi-stage pipeline with mutation testing |
| Learning | .cursorrules (manual) | Per-session only | CLAUDE.md (manual) | Automatic skill extraction + cross-session memory |
| Oversight model | Always-in-the-loop | Fully delegated (review PR) | Semi-autonomous | Configurable per-task |
| Safety | User responsible | Sandboxed cloud VM | Optional sandbox | Default-on sandbox + permission tiers |
| Multi-repo | Limited | Single repo per task | Single repo | Knowledge graph spanning repos |
| Pricing model | Credits (unpredictable) | ACUs (unpredictable) | Rate limits | Task-based (predictable) |

**Key gap this product fills:** No existing tool combines persistent cross-session learning, multi-stage autonomous validation, graduated oversight, and knowledge-graph-powered codebase understanding. Cursor is the best IDE experience but lacks autonomy and memory. Devin has autonomy but lacks persistent learning and validated quality. Claude Code has the strongest reasoning model but lacks memory, structured validation, and IDE integration. This engine synthesizes these capabilities into a coherent system.

---

## 3. UX model

### Interaction paradigms

The engine supports three interaction modes, selectable per-task:

**Pair Mode (lowest autonomy).** The engine works alongside the developer in real-time, similar to Cursor's composer but with memory-augmented context. The agent proposes changes, the human approves or redirects, and the agent learns from corrections. Every file modification requires explicit approval. This mode exists primarily for architectural exploration and teaching the system new conventions.

**Sprint Mode (medium autonomy).** The developer provides a task specification (from a GitHub issue, natural language description, or structured spec). The engine generates a plan, presents it for approval, then executes autonomously within the approved scope. The developer receives progress notifications and can interrupt or redirect. Output is a PR with validated tests. **This is the default and expected primary mode.**

**Autonomous Mode (highest autonomy).** The engine picks up work from configured sources (issue trackers, CI failures, code quality reports), plans and executes without pre-approval, and submits PRs for human review. Scoped to well-defined task types with established patterns. Requires explicit enablement and bounded permissions.

### Oversight levels

Each mode maps to a permission tier controlling what the agent can do without asking:

| Action | Pair Mode | Sprint Mode | Autonomous Mode |
|--------|-----------|-------------|-----------------|
| Read files, search, grep | Auto | Auto | Auto |
| Run tests, lint, type-check | Auto | Auto | Auto |
| Edit/create files | Ask | Auto (within plan scope) | Auto (within task type scope) |
| Install packages | Ask | Ask | Deny |
| Git commit | Ask | Auto (to feature branch) | Auto (to feature branch) |
| Git push / open PR | Ask | Auto | Auto |
| Run arbitrary shell commands | Ask | Ask (flagged as high-risk) | Deny |
| Access external APIs | Ask | Ask | Deny |
| Database operations | Ask | Deny | Deny |

### Feedback loops

The engine implements three feedback loop types:

**Immediate feedback** operates within a single task execution. The agent writes code, runs validation gates (lint → type-check → test → security scan), observes results, and iterates. This is fully automated and invisible to the user unless it fails after N retries.

**Session feedback** occurs during a task's lifecycle. The user reviews the agent's plan, provides corrections, reviews the PR, and requests changes. Each interaction updates the agent's understanding of the task and the user's preferences.

**Persistent feedback** operates across sessions. PR review outcomes (accept, reject with reason, request changes), build/deploy results, and production metrics feed back into the memory system. The agent learns which patterns work for this codebase and which don't. Successful task trajectories are extracted as reusable skills.

### Dashboard and monitoring

The monitoring interface provides three views:

**Task View** shows active and queued tasks with status (planning, executing, validating, awaiting review), current agent activity, token consumption, estimated completion, and a live activity log. Users can pause, cancel, redirect, or change oversight level on any running task.

**Quality View** shows aggregate metrics: task completion rate, PR acceptance rate, test pass rate, security finding density, average iteration count, and cost per task. Trends over time demonstrate learning effectiveness.

**Memory View** shows the current state of the agent's learned knowledge: extracted skills, codebase understanding quality (indexed files, KG coverage), active conventions, and recent memory updates. Users can edit, approve, or reject specific memories.

---

## 4. Orchestrator model

### Architecture overview

The orchestrator follows a **hierarchical architecture with sprint-based execution**, validated as the most effective pattern by Anthropic's three-agent harness research (March 2026), Stripe's Minions, and LangChain's Open SWE. Three core subsystems interact:

```
┌──────────────────────────────────────────────────────────────────┐
│                        ORCHESTRATOR                               │
│                                                                    │
│  ┌─────────────┐    ┌──────────────┐    ┌───────────────────┐    │
│  │   PLANNER   │───▶│  GENERATOR   │───▶│    EVALUATOR      │    │
│  │  (read-only │    │  (read-write │    │  (read-only,      │    │
│  │   tools)    │    │   tools,     │    │   test execution, │    │
│  │             │    │   sandbox)   │    │   security scan)  │    │
│  └─────────────┘    └──────────────┘    └───────────────────┘    │
│         │                  │                      │               │
│         ▼                  ▼                      ▼               │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │              STATE MANAGEMENT LAYER                        │    │
│  │  Working Context │ Session Checkpoint │ Long-term Memory  │    │
│  └──────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │              SAFETY LAYER                                  │    │
│  │  CostGuard │ Loop Detection │ Permission Gates │ Rollback │    │
│  └──────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────┘
```

### Planning engine

The Planner agent receives a task description and produces a structured execution plan. It operates with **read-only tool access** — it can search files, read code, query the knowledge graph, and examine test suites, but cannot modify anything. This constraint is enforced by excluding write tools from its schema entirely, not by prompting.

The planning process proceeds in three phases:

**Phase 1: Exploration.** The Planner uses codebase search tools (grep, glob, symbol search, KG traversal) to understand the current state of relevant code. It retrieves relevant memories and skills from the memory system.

**Phase 2: Analysis.** The Planner identifies affected files, dependencies, test coverage gaps, and potential risks. It produces a dependency graph of required changes.

**Phase 3: Plan generation.** The Planner outputs a structured plan document containing: task decomposition into ordered steps, affected files per step, expected test changes, risk assessment per step, and estimated token budget.

**Planning algorithm:** The engine uses **Plan-and-Execute** for high-level decomposition (keeping plans intentionally abstract to avoid specification errors cascading into implementation), **ReAct** within each execution step, and **Reflexion** for error recovery when tests fail. Tree-of-Thoughts / LATS is reserved for particularly difficult sub-problems where the Generator has failed twice on the same step.

**Critical design decision:** Plans are kept deliberately high-level. Research from Anthropic's harness work demonstrates that detailed upfront specifications create cascading errors — a wrong assumption in the plan propagates through all downstream implementation. Plans specify *what* to change and *why*, not the exact code to write.

### Task decomposition

Tasks are decomposed into **steps** with explicit dependencies. Each step targets a coherent unit of change (typically 1–3 files). Steps without mutual dependencies can execute in parallel using isolated git worktrees.

```
Task: "Add rate limiting to the /api/users endpoint"
  ├── Step 1: Add rate limiter middleware (src/middleware/)
  │     Files: rate_limiter.ts, rate_limiter.test.ts
  │     Dependencies: none
  ├── Step 2: Integrate middleware into users route (src/routes/)
  │     Files: users.ts, users.test.ts
  │     Dependencies: Step 1
  ├── Step 3: Add configuration options (src/config/)
  │     Files: config.ts, config.test.ts
  │     Dependencies: none
  └── Step 4: Update API documentation
        Files: docs/api.md
        Dependencies: Steps 1-3
```

Steps 1 and 3 execute in parallel. Step 2 waits for Step 1. Step 4 waits for all.

### Multi-agent coordination

The orchestrator coordinates agents using **artifact-based communication** — agents communicate via files rather than direct messaging. This is more reliable than free-form dialogue (MetaGPT's research demonstrates that structured outputs reduce hallucination cascading) and creates a natural audit trail.

Each agent runs in an isolated context. The Planner's output is a structured JSON plan document. The Generator receives individual steps from the plan and produces code changes. The Evaluator receives the changed files plus the original task description and evaluates whether the changes satisfy the requirements.

**Sprint contracts** (from Anthropic's harness design): Before each step, the Generator and Evaluator negotiate a "sprint contract" — a concrete description of what "done" means for this step, including specific tests that must pass and quality criteria that must be met. This prevents scope creep and provides clear pass/fail evaluation criteria.

### Work generation

In Autonomous Mode, the engine generates work from configured sources:

- **GitHub Issues**: Watches labeled issues (e.g., `agent-eligible`), filters by complexity heuristics, auto-assigns
- **CI Failures**: Monitors CI pipeline, identifies broken tests, generates fix tasks
- **Code Quality**: Periodic static analysis identifies high-priority improvements (security findings, complexity hotspots, dead code)
- **Coverage Gaps**: Identifies functions/modules below coverage thresholds, generates test-writing tasks
- **Dependency Updates**: Monitors for security advisories, generates update+test tasks

Work generation uses **priority scoring**: `priority = severity × confidence × estimated_effort_inverse`. Tasks below a confidence threshold are surfaced for human triage rather than auto-started.

### Execution flow and state management

The execution loop for each step follows the **Agent Harness Loop** pattern:

1. Load step context (plan, relevant files, memories, conventions)
2. Generator produces code changes
3. Run deterministic validation gates (lint, type-check, fast tests)
4. If gates fail → Generator receives error output, iterates (max 5 retries)
5. Run full validation pipeline (all tests, security scan, code review agent)
6. If pipeline fails → Evaluator reviews, determines if the approach needs pivoting
7. If pipeline passes → Step complete, merge to task branch, notify next dependent steps

**State checkpointing** occurs after every side effect (file write, test execution, git operation). The state backend uses PostgreSQL (via LangGraph's checkpoint system) for durability and recoverability. If the engine crashes mid-task, it resumes from the last checkpoint rather than restarting.

**Doom-loop detection:** The safety layer monitors for agents making identical tool calls repeatedly, iteration counts exceeding thresholds, or token consumption exceeding budget. When detected, the engine breaks the loop, triggers a context reset (clearing the context window and carrying only structured state), and attempts a fresh approach. After two context resets on the same step, the task escalates to human intervention.

**Cost management:** A **CostGuard** module tracks token consumption per task in real-time. Each task has a budget derived from its estimated complexity. At 75% budget consumption, the engine generates a status report for the user. At 100%, the task pauses and requests authorization to continue.

---

## 5. Memory and learning model

### Architecture overview

The memory system implements a **four-layer hierarchy** inspired by MemGPT/Letta's architecture, adapted for coding-specific concerns:

```
┌─────────────────────────────────────────────────────────┐
│  Layer 1: Working Memory (Context Window)                │
│  ┌──────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │ System   │  │ Memory       │  │ Message Buffer   │  │
│  │ Prompt   │  │ Blocks       │  │ (recent turns)   │  │
│  │          │  │ (project,    │  │                   │  │
│  │          │  │  user, task) │  │                   │  │
│  └──────────┘  └──────────────┘  └──────────────────┘  │
├─────────────────────────────────────────────────────────┤
│  Layer 2: Session State (Checkpoints)                    │
│  PostgreSQL-backed, per-task, full state at each step    │
├─────────────────────────────────────────────────────────┤
│  Layer 3: Long-term Memory (Persistent)                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Recall Memory│  │ Semantic     │  │ Skills        │  │
│  │ (conv logs,  │  │ Memory       │  │ Library       │  │
│  │  searchable) │  │ (vector +    │  │ (reusable     │  │
│  │              │  │  KG store)   │  │  procedures)  │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
├─────────────────────────────────────────────────────────┤
│  Layer 4: Code Intelligence (Structural)                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ AST Index    │  │ Knowledge    │  │ Dependency    │  │
│  │ (tree-sitter │  │ Graph        │  │ Graph         │  │
│  │  chunks)     │  │ (Neo4j/      │  │ (packages,    │  │
│  │              │  │  KùzuDB)     │  │  services)    │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Short-term context management

Working memory is managed through three mechanisms:

**Memory blocks** are discrete, labeled, editable context units (following Letta's design). Each block has a label, description, value, and character limit. Standard blocks include: `project` (architecture, conventions, tech stack), `user` (preferences, review patterns), and `task` (current objectives, constraints). Blocks are loaded into the context window at the start of each agent invocation.

**Message buffer** contains the recent conversation history. When the buffer approaches 75% of the context window (the empirically-validated sweet spot from Anthropic's research), the engine triggers **compaction** — summarizing older messages into a condensed form. For long-running tasks exceeding one context window, the engine performs a **context reset**: clearing the window entirely and carrying forward only the structured state (memory blocks, plan progress, file snapshots).

**Tool output management:** Large tool outputs (file contents, test output, search results) are written to temporary files and referenced by path rather than included in the context window. This prevents context pollution from verbose output — a pattern proven effective by LangChain's Deep Agents library.

### Long-term knowledge

**Recall memory** stores searchable conversation logs and task traces. Every agent interaction is logged with timestamps, tool calls, outcomes, and user feedback. Retrieval uses BM25 keyword search over structured logs, enabling queries like "how did we handle authentication in the last task?"

**Semantic memory** combines vector search and knowledge graph querying:

- **Vector store:** Code chunks (generated via tree-sitter AST-based chunking) are embedded using **Voyage Code-3** (commercial, best-in-class at 97.3% MRR) or **CodeXEmbed-2B** (open-source alternative). Stored in **pgvector** (simplest operational model for teams already on PostgreSQL) or **Qdrant** (for dedicated vector workloads needing complex metadata filtering). Incremental re-indexing tracks file hashes to avoid re-embedding unchanged code.

- **Knowledge graph:** Built automatically from AST analysis (tree-sitter) plus LSP-derived semantic information (go-to-definition, find-references, type hierarchy). Nodes represent files, classes, functions, methods, and types. Edges represent CALLS, INHERITS_FROM, IMPORTS, DEPENDS_ON, CONTAINS, and HAS_TYPE relationships. Stored in **KùzuDB** for MVP (embedded, zero-config) with a migration path to **Neo4j** for enterprise scale.

**Hybrid retrieval** is non-negotiable based on research findings. Vector search alone cannot answer structural queries ("what calls this function?" "what would break if I change this interface?"). The retrieval pipeline routes queries to vector search, graph traversal, or both, then fuses results through a cross-encoder re-ranker (BAAI/bge-reranker-v2-m3).

### Pattern learning and skill extraction

The engine learns from successful task executions through three mechanisms:

**Skill extraction** (adapted from Letta Code's approach): When a task completes successfully (PR approved and merged), the engine analyzes the task trajectory and extracts reusable procedures. A skill captures: the task pattern it applies to, the steps taken, the tools used, and any project-specific conventions demonstrated. Skills are stored as structured files in a `.skills` directory and version-controlled via git.

**Convention learning:** The engine observes recurring patterns in human feedback (code review comments, rejected PRs) and extracts conventions. For example, if three consecutive PRs receive feedback about import ordering, the engine updates its project memory block with the inferred convention. Users can review and approve extracted conventions through the Memory View.

**Failure pattern recognition:** Failed task attempts are analyzed to extract anti-patterns — approaches that don't work for this codebase. These are stored as negative constraints in the memory system, preventing the engine from repeating known mistakes.

### Cross-project intelligence

Each project maintains an isolated memory namespace (separate vector collection, separate KG, separate memory blocks). Cross-project learning is limited to **anonymized, abstracted patterns** — the engine can recognize that "projects using Express.js typically structure middleware like X" without transferring raw code between projects. This preserves privacy while enabling transfer learning.

**Resolution of a key tension:** The prior design documents appear to envision deep cross-project intelligence, but production experience (Sourcegraph's move away from shared embeddings, Cursor's path obfuscation) demonstrates that privacy concerns dominate. The recommended approach for MVP is **strict project isolation** with an opt-in "pattern library" for teams who explicitly want cross-project learning within their organization.

### Operational memory

Build results, test outcomes, deployment events, and production errors feed into the operational memory store. This enables:

- **Regression awareness:** The engine knows which files have historically been fragile and applies extra caution
- **Performance baselines:** The engine tracks typical build times and test durations, flagging anomalies
- **Deployment correlation:** When a deployment fails after an agent-generated change, the engine logs the correlation and adjusts future confidence for similar changes

---

## 6. Tools and runtime model

### Tool registry

Tools are defined as **typed JSON schemas** registered in a central tool registry. Each tool definition includes: name, description (used by the LLM for selection), input schema, output schema, risk level (read/write/destructive), and permission requirements.

The registry maintains two tool sets:

**Core tools** (always available):
- File I/O: `file_read`, `file_write`, `file_edit` (with diff-based modifications)
- Search: `glob`, `grep`, `symbol_search`, `kg_query`
- Execution: `shell_exec` (sandboxed), `test_run`, `lint_run`, `typecheck_run`
- Git: `git_status`, `git_diff`, `git_log`, `git_commit`, `git_branch`, `git_push`
- Memory: `memory_read`, `memory_write`, `skill_search`, `recall_search`

**Extended tools** (loaded on demand via MCP):
- Browser automation (Playwright MCP server)
- External API access (per-project MCP servers)
- Database querying (read-only, project-specific MCP servers)
- Project management integration (Jira, Linear, GitHub Issues MCP servers)

**Critical design principle:** Dedicated file tools (`file_read`, `file_edit`, `file_write`) are preferred over shell commands for file operations. Research from Claude Code's tool design demonstrates that dedicated tools produce better results than `cat`/`sed`/`awk` via bash, because the tool can enforce edit constraints (show diffs, require confirmation) and provide structured error messages.

### MCP integration

The engine adopts **Model Context Protocol (MCP)** as its tool integration layer. MCP is the de facto standard with 97M+ SDK downloads, support from Anthropic, OpenAI, Google, and Microsoft, and governance now under the Linux Foundation's Agentic AI Foundation.

**Implementation approach:**
- The engine acts as an MCP **Host**, maintaining Client connections to multiple MCP Servers
- Core tools are implemented as a local MCP server (stdio transport) for fast access
- External integrations connect via remote MCP servers (Streamable HTTP transport)
- OAuth 2.1 authentication per the June 2025 spec update for remote servers
- **Async Tasks** (November 2025 spec) enable long-running operations without blocking the agent loop

**MCP server management:**
- Project configuration files specify which MCP servers to connect
- Tool descriptions from MCP servers are filtered using RAG (retrieving only relevant tools per step) rather than loading all tools into context — this produces a **3× accuracy improvement** per LangChain's research
- The engine maintains an internal MCP server registry for governance, tracking which servers are approved for use

**A2A consideration:** The Agent-to-Agent (A2A) protocol is complementary to MCP — MCP connects agents to tools, A2A connects agents to agents. For MVP, the engine handles multi-agent coordination internally. A2A integration is deferred to V2 for external agent interoperability.

### Git and branching strategy

**One branch per task per agent** is the foundational principle, enforced by the orchestrator:

- Each task creates a feature branch: `agent/<task-id>-<short-description>`
- Parallel steps within a task use **git worktrees** for filesystem isolation
- All commits include provenance metadata: agent version, model used, task ID, step ID
- **No auto-merge**: Every task produces a PR for human review. The engine never pushes to main/trunk
- Merge conflicts between parallel steps are resolved by a dedicated Merger step that understands both changes semantically

**Commit strategy:** The engine produces atomic, well-described commits — one commit per logical change, with a message explaining *what* changed and *why*. This is not a single squash commit per task; it preserves meaningful history for human reviewers.

### Sandbox environments

**All agent code execution runs in sandboxed environments.** This is a default-on safety requirement, not an opt-in feature. The sandbox decision is informed by real incidents: Claude Code wiped a user's home directory via `rm -rf`; Cursor deleted 70 files despite explicit constraints; Claude Code found `/proc/self/root/usr/bin/npx` to bypass restrictions.

**MVP sandbox: Firecracker microVMs.** Firecracker provides the strongest isolation boundary (dedicated kernel via KVM) with acceptable startup time (~125ms) and minimal memory overhead (<5 MiB per VM). Each task gets an ephemeral microVM that is destroyed after completion.

**Sandbox configuration:**
- **Filesystem:** Only the project directory is mounted (never home directory or host filesystem)
- **Network:** Default deny egress; explicit domain allowlist per project (e.g., npm registry, PyPI, project APIs)
- **Resources:** CPU, memory, and process count caps per task (configurable)
- **Credentials:** Short-lived tokens with automatic expiry; never long-lived API keys inside the sandbox
- **Lifecycle:** Auto-destroy after task completion or timeout (configurable, default 60 minutes)

**Managed sandbox alternative:** For teams preferring managed infrastructure, the engine supports **E2B** (Firecracker-based, ~150ms cold start), **Daytona** (OCI-based, long-running stateful), or **Docker Desktop 4.60+** (microVM per container) as pluggable sandbox backends.

### Validation gates

Every task passes through a **four-layer validation pipeline** before producing a PR:

**Layer 1 — Deterministic checks (< 10 seconds):**
Linting (Ruff/ESLint), formatting, type checking (mypy/tsc). These run after every file modification during the generation loop and serve as immediate feedback for the Generator agent.

**Layer 2 — Test execution (< 5 minutes):**
The full test suite relevant to changed files runs in the sandbox. New tests written by the agent are included. If tests fail, the Generator receives structured error output and iterates.

**Layer 3 — Security and quality scanning (< 3 minutes):**
SAST scanning (CodeQL or Semgrep), dependency scanning (Snyk), and code quality analysis (SonarQube rules). Any critical or high security findings block the PR.

**Layer 4 — AI code review (< 2 minutes):**
A dedicated Evaluator agent reviews the complete changeset against the original task spec, checking for: correctness, style consistency, unnecessary complexity, potential regressions, and adherence to project conventions. The Evaluator produces a confidence score and detailed review comments.

**Post-MVP additions:** Mutation testing (Stryker Mutator) to validate test quality, property-based testing (Hypothesis/fast-check) for invariant verification, and visual regression testing (Playwright screenshots) for UI changes.

### Runtime safety controls

**Permission model (three-tier):**
1. **Default deny:** All operations blocked unless explicitly allowed in the project configuration
2. **Tiered approval:** Low-risk (read) → auto-approve; medium-risk (file edit within plan scope) → log; high-risk (shell exec, dependency install, git push) → require confirmation per oversight level
3. **Just-in-time elevation:** Temporary permissions granted for specific operations, automatically revoked after the operation completes

**Kill switch:** Any running task can be immediately frozen from the dashboard. Freezing preserves all state and logs for review. The task can be resumed, redirected, or cancelled after review.

**Audit logging:** Every agent action is logged with: timestamp, agent ID, model version, action type, resource affected, outcome, authorization context, and session ID. Logs are stored in append-only, tamper-resistant storage. Structured format (OpenTelemetry) enables compliance platform integration.

---

## 7. Technical architecture

### System components

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLIENT LAYER                             │
│  ┌──────────┐  ┌──────────────┐  ┌───────────┐  ┌───────────┐ │
│  │ VS Code  │  │  Web         │  │   CLI     │  │   API     │ │
│  │ Extension│  │  Dashboard   │  │           │  │ (REST +   │ │
│  │          │  │              │  │           │  │  WebSocket)│ │
│  └──────────┘  └──────────────┘  └───────────┘  └───────────┘ │
├─────────────────────────────────────────────────────────────────┤
│                      ORCHESTRATION LAYER                         │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  LangGraph-based Orchestrator                             │   │
│  │  ┌─────────┐  ┌───────────┐  ┌──────────┐  ┌─────────┐ │   │
│  │  │ Work    │  │  Planner  │  │Generator │  │Evaluator│ │   │
│  │  │ Queue   │  │  Agent    │  │  Agent   │  │  Agent  │ │   │
│  │  └─────────┘  └───────────┘  └──────────┘  └─────────┘ │   │
│  │  ┌─────────────────────┐  ┌────────────────────────┐     │   │
│  │  │ Safety Controller   │  │  Cost Guard            │     │   │
│  │  │ (permissions, doom- │  │  (budget tracking,     │     │   │
│  │  │  loop, kill switch) │  │   alerting)            │     │   │
│  │  └─────────────────────┘  └────────────────────────┘     │   │
│  └──────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│                        MEMORY LAYER                              │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌──────────────┐  │
│  │ pgvector │  │ KùzuDB/  │  │ Recall    │  │ Skills       │  │
│  │ (code    │  │ Neo4j    │  │ Store     │  │ Library      │  │
│  │ embedds) │  │ (code KG)│  │ (Postgres)│  │ (git-backed) │  │
│  └──────────┘  └──────────┘  └───────────┘  └──────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│                      EXECUTION LAYER                             │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Sandbox Manager (Firecracker / E2B / Docker microVM)    │   │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐   │   │
│  │  │ Task 1  │  │ Task 2  │  │ Task 3  │  │ Task N  │   │   │
│  │  │ microVM │  │ microVM │  │ microVM │  │ microVM │   │   │
│  │  └─────────┘  └─────────┘  └─────────┘  └─────────┘   │   │
│  └──────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│                    INTEGRATION LAYER                              │
│  ┌─────────┐  ┌─────────┐  ┌──────────┐  ┌─────────────────┐  │
│  │ MCP     │  │ GitHub  │  │ CI/CD    │  │ Issue Tracker   │  │
│  │ Servers │  │ API     │  │ Pipeline │  │ (Jira/Linear/GH)│  │
│  └─────────┘  └─────────┘  └──────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Data flows

**Task execution flow:**
1. Task enters Work Queue (from user, issue tracker, or work generator)
2. Orchestrator assigns to Planner Agent with project memory blocks
3. Planner queries Memory Layer (vector search + KG traversal + recall search)
4. Planner produces structured plan → user approval (Sprint/Autonomous mode)
5. Orchestrator creates sandbox microVM, checks out repo, creates feature branch
6. Generator Agent executes steps, using core tools inside sandbox
7. After each step: validation gates run inside sandbox
8. Evaluator Agent reviews complete changeset
9. On pass: git push feature branch, create PR via GitHub API
10. PR triggers external CI/CD pipeline for additional validation
11. On merge: operational feedback stored in memory; skill extraction triggered

**Memory update flow:**
1. Task completion triggers skill extraction analysis
2. PR review feedback (human comments, accept/reject) stored as episodic memory
3. Convention patterns extracted from accumulated feedback
4. Code changes trigger incremental re-indexing (changed chunks re-embedded)
5. KG updated with structural changes (new functions, modified call graph)
6. Sleep-time processing consolidates and refines memories during idle periods

### Infrastructure requirements

**Compute:**
- Orchestrator service: 2–4 vCPUs, 8 GB RAM (stateless, horizontally scalable)
- Sandbox host: KVM-capable instances (Firecracker requires bare metal or nested virtualization). Recommended: AWS i3/i4 instances or equivalent. Scale based on concurrent task count.
- Embedding service: GPU-accelerated for indexing large repos (can use batch API for non-real-time embedding). CPU sufficient for query-time embedding.

**Storage:**
- PostgreSQL (primary): Orchestrator state, checkpoints, recall memory, operational metrics
- pgvector extension: Code embeddings (or standalone Qdrant)
- KùzuDB (embedded) / Neo4j (standalone): Code knowledge graph
- Object storage (S3/GCS): Sandbox snapshots, large artifacts, audit logs
- Git: Code repositories, skills library, memory version history

**Network:**
- Internal: Service mesh between orchestrator, memory, and sandbox manager
- External: GitHub API, MCP server connections, LLM API endpoints
- Sandbox network: Isolated VLAN with egress filtering via allowlist proxy

### Technology stack recommendations

| Component | MVP Choice | Rationale | Migration Path |
|-----------|-----------|-----------|----------------|
| Orchestration framework | LangGraph (Python) | Best checkpointing, state management, production maturity. Powers Open SWE. | Stable; v1.0 released |
| LLM (planning/evaluation) | Claude Opus 4.5+ or GPT-5 | Strongest reasoning for complex planning | Model-agnostic via LLM abstraction layer |
| LLM (code generation) | Claude Sonnet 4.5+ | Best cost/performance for coding | Model-agnostic |
| LLM (fast tasks) | Claude Haiku 4.5 or GPT-5-mini | Cost-efficient for simple operations | Model-agnostic |
| Embedding model | Voyage Code-3 | 97.3% MRR, code-specialized | CodeXEmbed (OSS fallback) |
| Vector store | pgvector | Operational simplicity, same DB as state | Qdrant for dedicated workloads |
| Graph database | KùzuDB (embedded) | Zero-config, fast for single-node | Neo4j for enterprise scale |
| AST parser | tree-sitter | Language-agnostic, proven standard | Stable |
| Sandbox | Firecracker microVMs | Strongest isolation, fast startup | E2B (managed alternative) |
| Primary database | PostgreSQL 16+ | State, checkpoints, recall, pgvector | Stable |
| API framework | FastAPI (Python) | Async, WebSocket support, OpenAPI | Stable |
| Client protocol | MCP (tools), REST + WebSocket (clients) | Industry standard | A2A for V2 agent interop |
| CI/CD integration | GitHub Actions | Largest ecosystem, agentic workflow support | GitLab CI as alternative |
| Observability | OpenTelemetry → Grafana stack | Standard, vendor-neutral | Stable |

**Model abstraction layer:** The engine must not hard-code any specific LLM provider. All model interactions go through an abstraction layer that supports swapping models per agent role, enabling cost optimization (expensive model for planning, cheap model for routine tasks) and provider resilience.

---

## 8. MVP definition

### Core capabilities (in scope)

The MVP delivers **Sprint Mode** for single-repository, single-language (Python or TypeScript) projects:

1. **Task intake:** Accept task descriptions via CLI or web interface. Parse GitHub issues as task input.
2. **Planning:** Planner agent explores codebase, produces structured plan, presents for user approval.
3. **Execution:** Generator agent implements plan steps sequentially (parallel execution deferred to V1), in a sandboxed environment.
4. **Validation:** Four-layer validation pipeline (lint, test, security scan, AI review) with automated iteration on failures.
5. **Output:** Feature branch with atomic commits, PR with description and review-ready diff.
6. **Memory:** Session-based memory blocks (project conventions, task context). Persistent recall memory across sessions. Basic vector search over codebase. No knowledge graph in MVP.
7. **Safety:** Firecracker sandbox, three-tier permission model, CostGuard, doom-loop detection, kill switch.
8. **Monitoring:** Basic task view (status, activity log, cost tracking).

### Out of scope for MVP

- Pair Mode and Autonomous Mode (Sprint Mode only)
- Multi-language projects (one language per project)
- Knowledge graph construction and traversal
- Skill extraction and procedural memory
- Cross-project learning
- Parallel step execution (sequential only)
- Web dashboard (CLI + basic web status page only)
- MCP server marketplace / dynamic tool loading
- A2A protocol integration
- Mutation testing, property-based testing
- Visual regression testing
- IDE extension (VS Code plugin deferred to V1)
- Work generation from issue trackers and CI failures

### Success criteria

The MVP succeeds if it demonstrates:

- **Task completion rate ≥ 60%** on a curated set of 50 real-world tasks (bug fixes, feature implementations, refactoring) from open-source Python and TypeScript repositories
- **PR acceptance rate ≥ 50%** when reviewed by the task requester (without additional manual fixes)
- **Zero safety incidents** (no unintended file deletions, no secrets exposure, no sandbox escapes) across 500 task executions
- **Cost per task ≤ $5** average for tasks that would take a developer 30–60 minutes
- **Time to completion ≤ 15 minutes** average for well-scoped tasks
- **Memory persistence demonstrated:** Task 10 on a project measurably outperforms task 1 (measured by iteration count and PR acceptance rate)

---

## 9. Phased roadmap

### Phase 1: MVP (Months 1–4)

**Milestone 1.1 (Month 1): Core orchestration loop**
- LangGraph-based orchestrator with Planner → Generator → Evaluator pipeline
- Sequential step execution with basic state checkpointing
- Shell-based tool execution (no sandbox yet)
- Single model support (Claude Sonnet)

**Milestone 1.2 (Month 2): Sandbox and safety**
- Firecracker microVM integration
- Three-tier permission model
- CostGuard budget tracking
- Doom-loop detection
- Basic audit logging

**Milestone 1.3 (Month 3): Memory and retrieval**
- Memory blocks (project, user, task)
- Vector store with tree-sitter chunking and Voyage Code-3 embeddings
- Recall memory (session logs)
- Hybrid search (vector + BM25)
- Incremental re-indexing

**Milestone 1.4 (Month 4): Validation pipeline and polish**
- Four-layer validation gates
- Git workflow (branch per task, PR creation)
- CLI interface
- Basic web status page
- End-to-end testing against benchmark tasks

**Dependencies:** Months 1–2 are parallel-trackable (orchestration and sandbox can develop independently). Month 3 depends on orchestration. Month 4 depends on all prior milestones.

### Phase 2: V1 (Months 5–8)

**Key additions:**
- **Pair Mode** with real-time interaction and streaming diffs
- **VS Code extension** (primary IDE integration)
- **Knowledge graph** construction from AST + LSP analysis
- **Parallel step execution** using git worktrees
- **Skill extraction** from successful task trajectories
- **Convention learning** from PR review feedback
- **Multi-language support** (Python, TypeScript, Go, Java)
- **MCP server integration** for external tools
- **Web dashboard** with Task View, Quality View, Memory View
- **Mutation testing** integration (Stryker Mutator)
- **GitHub Issues integration** for task intake

**Key milestone:** V1 launch targets **PR acceptance rate ≥ 70%** and **task completion rate ≥ 75%** on the expanded benchmark suite.

### Phase 3: V2 (Months 9–14)

**Key additions:**
- **Autonomous Mode** with work generation
- **Cross-project learning** (opt-in, within organization)
- **A2A protocol** integration for external agent interoperability
- **Property-based testing** and **formal verification** integration
- **Visual regression testing** (Playwright screenshots)
- **CI/CD pipeline integration** (automatic deployment behind feature flags)
- **Multi-model optimization** (automatic model selection per task/step)
- **Team features** (shared memories, role-based access, org-level analytics)
- **Enterprise features** (SSO, VPC deployment, compliance exports)
- **JetBrains IDE extension**

**Key milestone:** V2 launch targets **PR acceptance rate ≥ 80%** and **autonomous work generation** producing valuable PRs with ≥ 60% merge rate.

### Dependency map

```
MVP                          V1                          V2
────────────────────────    ────────────────────────    ────────────────────
Core Orchestrator ──────▶   Parallel Execution ─────▶   Autonomous Mode
Sandbox ────────────────▶   MCP Integration ────────▶   A2A Integration
Vector Search ──────────▶   Knowledge Graph ────────▶   Cross-Project Learning
Memory Blocks ──────────▶   Skill Extraction ───────▶   Convention Auto-Learning
CLI Interface ──────────▶   VS Code Extension ──────▶   JetBrains Extension
Validation Pipeline ────▶   Mutation Testing ───────▶   Formal Verification
Git Workflow ───────────▶   GH Issues Integration ──▶   Work Generation
```

---

## 10. Risks and mitigations

### Technical risks

**Risk: LLM capability ceiling.** Even the best models solve only 23% of enterprise-level tasks (SWE-bench Pro). The engine's effectiveness is bounded by underlying model capability.
**Mitigation:** Design for model-agnostic operation. The orchestration, memory, and validation layers provide value independent of any specific model. As models improve, the engine improves automatically. The harness-based architecture (separating generation from evaluation) extracts more from existing models than raw model performance suggests.

**Risk: Context window degradation on long tasks.** Models lose coherence as context fills. "Context anxiety" causes premature task termination.
**Mitigation:** Aggressive context management — compaction at 75% utilization, context resets for tasks exceeding one window, file-based memory for large artifacts, subagent isolation to prevent context pollution.

**Risk: Sandbox escape.** Agent-generated code finds ways to bypass sandbox restrictions (proven possible with Claude Code's `/proc/self/root` bypass).
**Mitigation:** Firecracker microVMs provide hardware-level isolation (dedicated kernel). Network egress filtering via allowlist proxy. Regular security audits of sandbox configuration. Bug bounty for sandbox escape reports.

**Risk: Knowledge graph staleness.** KG becomes out of date as code evolves faster than the graph updates.
**Mitigation:** Incremental KG updates triggered by file change events (git hooks or file watchers). Background reindexing during idle periods. Staleness metadata on KG nodes (age since last verification).

### Product risks

**Risk: Developer trust.** Developers may not trust autonomous code generation for production codebases, limiting adoption to toy projects.
**Mitigation:** Graduated autonomy model (start with Pair Mode, earn trust). Transparent audit trails. Detailed PR descriptions explaining every change. Validation pipeline runs identical checks to what a human developer would face. No auto-merge — humans always approve.

**Risk: Cost unpredictability.** Developer backlash against credit-based pricing is a top complaint across Cursor, Claude Code, and Devin.
**Mitigation:** Task-based pricing with upfront cost estimates. Hard budget limits (CostGuard) prevent runaway costs. Transparent token consumption tracking in the dashboard. Tiered model selection to optimize cost per task.

**Risk: Competitive moat erosion.** Major players (GitHub Copilot, Claude Code, Cursor) are aggressively expanding capabilities. Features that are differentiators today may be table stakes in 12 months.
**Mitigation:** The persistent memory and learning system is the primary moat — it compounds in value over time and is difficult for competitors to replicate without architectural changes. Focus investment on memory, learning, and cross-session intelligence as the durable differentiator.

### Safety risks

**Risk: Insecure code generation.** Only 55% of AI-generated code is secure (Veracode 2025). AI-generated code shows **322% more privilege escalation paths** in enterprise settings.
**Mitigation:** Mandatory SAST scanning (CodeQL/Semgrep) as a validation gate. No critical/high findings allowed. Security-specific conventions in project memory. Dependency scanning to prevent malicious package installation.

**Risk: Data exfiltration via tool composition.** MCP security research (April 2025) identified risks of agents combining tools to exfiltrate sensitive data.
**Mitigation:** Tool permission model restricts which tools can be composed. Network egress filtering prevents unauthorized outbound connections. Audit logging captures all tool invocations for post-hoc analysis. Read-only tokens for all integrations where possible.

**Risk: Agent-generated technical debt.** AI-authored code shows 4× growth in code clones and creates maintenance burden.
**Mitigation:** Code quality metrics in the validation pipeline (duplication detection, complexity thresholds). The Evaluator agent specifically checks for unnecessary complexity. Convention learning reduces drift over time.

### Adoption risks

**Risk: Onboarding friction.** Developers need to configure sandbox, set up memory, write conventions — too much setup before value.
**Mitigation:** Zero-config defaults that work for common project structures. CLI wizard for initial setup. Pre-configured templates for popular frameworks (Next.js, FastAPI, Spring Boot).

**Risk: Windows support.** No major coding agent can run on Windows containers natively (SWE-bench Windows finding).
**Mitigation:** MVP targets Linux/macOS only. Windows support via WSL2 as a documented workaround. Native Windows sandbox support deferred to V2.

---

## 11. Open decisions

### Architectural choices requiring empirical validation

**Decision 1: Embedded vs. standalone graph database.**
KùzuDB (embedded, zero-config) vs. Neo4j (standalone, richer ecosystem). KùzuDB is recommended for MVP to minimize operational complexity, but if graph query patterns become complex or multi-tenant is needed, Neo4j may be necessary. **Decision criteria:** If KG queries take > 100ms at p95 or if multi-user concurrent access is required, migrate to Neo4j.

**Decision 2: Context reset vs. compaction strategy.**
Research shows context resets work better for long tasks (especially with models exhibiting "context anxiety"), while compaction preserves more continuity. **Recommended approach:** Empirically test both on the benchmark suite and measure PR acceptance rate. The architecture supports both — make it configurable per model.

**Decision 3: Planning granularity level.**
Anthropic's research advocates for "intentionally high-level" plans to avoid cascading specification errors. But SWE-AF's topological sorting requires more detailed dependency information. **Recommended approach:** Start with high-level plans (what/why, not how), measure failure rate from plan ambiguity, and add detail incrementally only where needed.

**Decision 4: Number of agent roles.**
Two competing approaches: few powerful agents (Planner + Generator + Evaluator) vs. many specialized agents (Product Manager + Architect + Programmer + Reviewer + Tester, as in MetaGPT). Research suggests that **fewer agents with richer tool access** outperform many specialized agents — communication overhead and cascading errors dominate with more agents. **Recommended approach:** Start with three roles; add specialization only if measurable quality improvements justify the complexity.

**Decision 5: Self-hosted vs. cloud-hosted sandboxes.**
Self-hosted Firecracker gives maximum control but requires KVM-capable infrastructure. Cloud sandbox providers (E2B, Daytona) reduce operational burden but add latency and cost. **Decision criteria:** Depends on deployment model — self-hosted for enterprise, cloud sandboxes for SaaS offering. Support both as pluggable backends.

### Trade-offs that should be deferred

**Model selection strategy.** The optimal tiered model strategy (which model for which role) changes every 3–6 months as new models release. Hard-coding model choices would be premature. The model abstraction layer enables runtime experimentation.

**Vector database scaling.** pgvector is sufficient for MVP (handles 50M+ vectors at 471 QPS). If query latency or scale requirements demand it, migrating to Qdrant or Turbopuffer is a well-understood operation. Don't over-engineer storage for projected scale.

**Pricing model details.** Task-based pricing is directionally correct, but specific price points depend on empirical cost data from MVP operation. Collect detailed cost telemetry during MVP; set prices based on actual distribution of task costs.

### Contradictions between design domains that require resolution

**Contradiction 1: Memory persistence scope.** The memory/learning design envisions rich cross-project intelligence, but the tools/safety design emphasizes strict project isolation for security. **Resolution:** Default to strict project isolation. Cross-project learning is opt-in at the organization level, using anonymized pattern abstractions rather than raw code transfer. This preserves the security posture while leaving the door open for teams who want the capability.

**Contradiction 2: Autonomy vs. safety.** The product vision includes fully autonomous work generation and execution, but the safety design requires human approval for most write operations. **Resolution:** Autonomous Mode requires explicit enablement per project and per task type. It operates within a bounded permission set (cannot install packages, access databases, or modify infrastructure). Work generation creates PRs, never merges them. This is bounded autonomy, not unbounded.

**Contradiction 3: Validation thoroughness vs. speed.** The validation pipeline (lint + test + security scan + AI review + mutation testing) could take 20+ minutes per step, making the agent too slow for practical use. **Resolution:** Tiered validation — fast checks (lint, type-check) run after every edit during generation. Full tests run after each step. Security scan and AI review run once per task, not per step. Mutation testing is post-integration only. Target: < 2 minutes for per-step validation, < 10 minutes for full task validation.

---

## 12. Recommended implementation direction

### First 30 days: Foundation sprint

**Week 1–2: Orchestrator skeleton.**
Build the LangGraph-based orchestration loop with a minimal Planner → Generator → Evaluator pipeline. Use Claude Sonnet via Anthropic API. Implement the ReAct loop for the Generator with basic file tools (read, write, edit, grep). Test against 10 simple Python tasks (function implementation, bug fix).

**Week 3–4: Sandbox integration.**
Integrate Firecracker microVM (or E2B as faster alternative) as the execution environment. Implement basic permission model (auto-approve reads, ask for writes). Add git workflow (branch creation, commit, PR via GitHub API). Implement CostGuard with hard budget limits. Test against 10 tasks requiring shell execution.

**Deliverable at Day 30:** A working prototype that accepts a task description, plans it, executes it in a sandbox, runs tests, and produces a PR. No memory persistence, no knowledge graph, no IDE integration. Terminal-only interface.

### Days 31–60: Memory and validation

**Week 5–6: Memory layer.**
Implement memory blocks (project, user, task). Set up pgvector with tree-sitter AST chunking and Voyage Code-3 embeddings. Implement hybrid search (vector + BM25). Add recall memory (searchable session logs). Build incremental re-indexing triggered by file changes.

**Week 7–8: Validation pipeline.**
Implement four-layer validation gates. Add Semgrep/CodeQL integration for security scanning. Build the Evaluator agent with structured review output and confidence scoring. Implement doom-loop detection and context reset recovery. Test against 30 tasks of increasing complexity.

**Deliverable at Day 60:** The engine now remembers project context across sessions, retrieves relevant code context during planning, and validates all output through deterministic and AI-powered quality gates.

### Technology choices for MVP

| Decision | Choice | Reasoning |
|----------|--------|-----------|
| Language | Python 3.12+ | LangGraph native, Anthropic SDK, rich ML ecosystem |
| Orchestration | LangGraph v1.x | Proven checkpointing, Deep Agents library for coding primitives |
| Primary LLM | Claude Sonnet 4.5 (generation), Claude Opus 4.5 (planning) | Best coding performance; Haiku 4.5 for cost-sensitive operations |
| Database | PostgreSQL 16 with pgvector | Single database for state, checkpoints, embeddings, recall |
| AST parsing | tree-sitter (Python bindings) | Industry standard, multi-language |
| Sandbox | E2B (managed Firecracker) for MVP → self-hosted Firecracker for production | Faster to integrate; migrate when operational maturity allows |
| Git integration | PyGitHub + subprocess git | Simple, proven, direct |
| API | FastAPI with WebSocket support | Async, fast, good OpenAPI generation |
| Observability | OpenTelemetry → local Grafana stack | Standard, extensible |

### Team structure recommendation

**Minimum viable team (4 engineers):**
- **1 Orchestration Engineer:** LangGraph pipeline, agent coordination, state management, planning algorithms
- **1 Infrastructure Engineer:** Sandbox integration, security controls, git workflow, CI/CD integration, deployment
- **1 Memory/Retrieval Engineer:** Vector store, AST indexing, memory management, hybrid search, (later) knowledge graph
- **1 Product/Validation Engineer:** Validation pipeline, CLI interface, web dashboard, benchmarking, quality metrics

**Week 1 kickoff priorities:**
1. Set up monorepo with shared types and interfaces between subsystems
2. Define tool schema format and permission model data structures
3. Create benchmark task suite (50 tasks across Python and TypeScript)
4. Establish CI pipeline for the engine itself (dogfooding opportunity)
5. Deploy PostgreSQL with pgvector, configure LangGraph checkpointing
6. Get a basic ReAct loop running against Claude Sonnet by end of Week 1

**Scaling to V1 (add 2–3 engineers):**
- IDE/Extension Engineer for VS Code integration
- KG/ML Engineer for knowledge graph construction and skill extraction
- Frontend Engineer for web dashboard

### Critical path items

The highest-risk, longest-lead-time items that should start immediately:

1. **Sandbox reliability.** Firecracker/E2B integration is the foundation of safety. If the sandbox is unreliable, nothing else matters. Prioritize this alongside the orchestrator.

2. **Benchmark suite curation.** The 50-task benchmark suite drives all quality decisions. Curate tasks from real open-source repos spanning bug fixes, feature implementations, refactoring, and test writing. Include tasks that current tools fail on (SWE-bench Pro difficulty level).

3. **Model abstraction layer.** Build the LLM abstraction from Day 1, not as a retrofit. Every model call goes through this layer. This enables cost optimization, model swapping, and A/B testing of models per role — all of which are critical for iterating on quality.

4. **Validation pipeline correctness.** A validation pipeline that produces false positives (blocking good code) or false negatives (passing bad code) undermines trust. Invest in calibrating each gate against the benchmark suite. Track false positive and false negative rates as first-class metrics.

---

*This specification represents a synthesis of current state-of-the-art research and competitive analysis as of April 2026. All technology choices should be re-evaluated against the competitive landscape at each phase gate. The LLM ecosystem evolves on 3–6 month cycles; architectural decisions should be time-boxed accordingly.*