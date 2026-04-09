# Product definition and UX strategy: the autonomous engineering system

**The core problem is simple: today's AI coding tools make developers faster at writing code, but no tool makes engineering teams faster at shipping software.** Current tools operate at the wrong altitude — they assist with keystrokes when the bottleneck is coordination, planning, and sustained execution against complex goals. This document defines a product that operates at the engineering-system level: ingesting goals, decomposing work, orchestrating parallel agent execution, and delivering verified results — with full human oversight.

The market has evolved through three generations — autocomplete-first (Copilot), chat-first (ChatGPT), and agent-first (Devin, Cursor agent mode, Claude Code) — but the fourth generation, **orchestration-first**, remains unbuilt. No existing tool combines spec-driven planning, persistent work graphs, parallel multi-agent execution, operator-grade observability, and configurable autonomy in a single coherent product. This is that product.

---

## 1. Product thesis

### The problem: AI coding tools don't compound

Individual AI coding tools have reached impressive capability thresholds. Claude Code can sustain **30+ hours** of autonomous coding. Cursor runs **8 parallel agents** via git worktrees. Devin merges **67% of its PRs** autonomously. SWE-bench scores exceed **77%** for top systems. Yet engineering teams report paradoxical results: Google's 2025 DORA Report found 90% AI adoption increase correlating with **9% more bugs**, **91% more code review time**, and **154% larger PRs**. METR's study revealed experienced developers believed AI made them 20% faster while objective measurement showed they were **19% slower** — a 39-percentage-point perception gap.

The root cause is architectural. Every current tool treats AI assistance as a session-scoped, single-task phenomenon. A developer opens a chat, asks for help, gets code, and manually integrates it. Even the most autonomous tools (Devin, Codex Cloud) produce isolated pull requests disconnected from any broader engineering plan. There is no persistent work graph. No connection between a PRD and the tasks derived from it. No system that tracks which agent is working on which part of a feature, what's blocked, what's validated, and what remains.

### Why current tools fall short

Current tools cluster into three categories, each with a structural limitation:

**IDE-native agents** (Cursor, Windsurf, Cline, Augment) embed agents inside code editors. They excel at interactive, session-length work but lack orchestration. There is no work graph, no task decomposition from specs, no operator view. When Cursor runs 8 parallel background agents, the user cannot see how those agents' work connects, whether they'll create merge conflicts, or how their output maps to a feature spec.

**Autonomous cloud agents** (Devin, Factory, Codex Cloud) fire-and-forget to produce PRs. They handle long-running work but operate as black boxes. Devin's own assessment calls it "senior-level at codebase understanding but junior at execution." Users report that sessions run "too slow to watch in real time, yet fast enough to interrupt deeper focus work" — an awkward temporal middle ground with no purpose-built UX.

**App generators** (Replit Agent, Lovable, Bolt, v0) produce entire applications from prompts but generate prototype-grade code unsuitable for production. They have no concept of an existing codebase, no spec-driven workflow, and no validation beyond visual preview.

**The missing product sits above all three:** an orchestration layer that ingests engineering goals, decomposes work into a visible task graph, dispatches specialized agents (which may themselves use Cursor, Claude Code, or other execution engines), tracks progress against the source spec, handles merge coordination, and surfaces results for human review — all with configurable autonomy and full audit trails.

### What this product does differently

This product treats engineering as an **operations problem**, not an editing problem. The primary metaphor is not "editor with AI" but "engineering control room." The user defines intent through structured specifications. The system derives work, assigns agents, manages parallel execution, validates output, and reports progress. The user's role shifts from writer to **operator** — setting goals, calibrating autonomy, reviewing results, and intervening at critical junctures.

Three architectural decisions distinguish it:

1. **Spec-first, not prompt-first.** Following the pattern pioneered by AWS Kiro, the system ingests structured goals (PRDs, feature specs, architecture notes) and derives `requirements.md → design.md → tasks.md` before any code is written. The specification is the primary artifact, not a chat message.

2. **Work graph as source of truth.** Every derived task exists in a visible, interactive DAG with dependencies, assignments, status, and verification criteria. This work graph persists across sessions, connects to the source spec, and updates as agents complete (or fail) tasks.

3. **Parallel execution with merge orchestration.** Multiple agents work simultaneously in isolated git worktrees with automated conflict detection, sequential merge coordination, and per-worktree environment isolation.

### Why "agent-first" matters

The distinction is not semantic. **Autocomplete-first** tools predict your next tokens — the AI has no agency. **Chat-first** tools are advisory — they suggest changes you manually apply. **Agent-first** tools take goals and autonomously execute them — reading files, writing code, running commands, testing output, and iterating on failures. The human shifts from writer to reviewer.

But even agent-first tools (Generation 3) remain session-scoped and single-threaded. **Orchestration-first** (Generation 4) means the system coordinates multiple agents working in parallel against a shared plan, with persistent state, cross-session context, and operator-grade observability. This is the leap from "AI pair programmer" to "AI engineering team."

---

## 2. Primary user profiles

### The technical operator (primary)

A lead engineer, architect, or engineering manager who oversees a codebase and team. They define what needs to be built (or improved), want to delegate execution to agents, and need visibility into progress and quality. They think in terms of features, not files. Their workflow is: define goal → dispatch work → monitor progress → review results → merge. They currently use a combination of Linear/Jira for planning, Cursor/Claude Code for coding, and GitHub for review — and the coordination overhead between these tools is their primary bottleneck. **This user needs an operator dashboard more than a code editor.**

### The solo founder / indie hacker

A technical-enough builder shipping a product alone or with a tiny team. They have a clear product vision, can write (or review) a PRD, and want to turn specs into working software with minimal hands-on coding. They value speed and breadth — building auth, billing, API, frontend, and deployment simultaneously rather than sequentially. Their risk tolerance for autonomy is high; their patience for per-action approval is low. **They need fire-and-forget execution with clear checkpoints and easy rollback.**

### The AI-native development operator

A new role emerging in 2025-2026: someone whose primary skill is directing AI agents rather than writing code themselves. They are expert prompt engineers, understand software architecture conceptually, and can review code for correctness but prefer not to write it. They may be a product manager, designer, or technical writer who has learned to "program through specification." **They need a spec-driven interface with progressive technical disclosure.**

### The research engineer

Works on experimental, open-ended problems where the goal is exploration rather than production deployment. They need agents that can try multiple approaches in parallel (best-of-N), maintain detailed logs of what was tried and why it failed, and provide rich comparison tools. They value reproducibility, audit trails, and the ability to branch experiments. **They need parallel exploration with comprehensive provenance tracking.**

### The platform / infrastructure engineer

Maintains large codebases where the work is often mechanical but high-stakes: dependency upgrades, security patches, migration tasks, test expansion, and infrastructure-as-code changes across many services. They need agents that can apply consistent transformations across hundreds of files with validation at each step. **They need batch execution with regression safety guarantees.**

---

## 3. Primary use cases

### Build from PRD or feature brief

The user pastes or links a product requirements document. The system generates structured requirements with acceptance criteria (using EARS format), derives a technical design, and decomposes it into a sequenced task graph. The user reviews and approves (or modifies) the plan before any code is written. Agents then execute tasks, with the work graph updating in real time. This is the flagship use case — the full pipeline from intent to verified implementation.

**Key design decision:** The planning phase must be decoupled from execution. Users report that agents starting to code before the plan is reviewed and approved leads to wasted work and trust erosion. Kiro's three-phase workflow (requirements → design → tasks) is the right model.

### Continue coding autonomously for hours

The user dispatches work and walks away — for lunch, overnight, or over a weekend. The system executes, handles errors through self-healing loops, checkpoints progress through git commits, and pauses at configurable approval gates. When the user returns, they see a timeline of everything that happened, organized as a session replay with decision points, test results, and code changes highlighted. This requires robust context management (automatic compaction when approaching window limits), doom-loop detection (forcing strategy changes when stuck), and clean session handoff (generating structured summaries for the next session).

**Critical constraint:** METR research shows task failure rate quadruples as task duration doubles. Every agent experiences performance degradation after approximately **35 minutes of equivalent human time**. The system must therefore decompose long-running work into sub-tasks of manageable duration, using fresh agent contexts for each sub-task to prevent drift.

### Branch/worktree-based parallel execution

Multiple agents work simultaneously on different tasks in isolated git worktrees. The system manages worktree creation, dependency installation, port allocation, and environment setup. A conflict detection layer (inspired by the Clash tool) continuously monitors worktree pairs for potential merge conflicts, warning before they accumulate. When agents complete tasks, a sequential merge orchestrator integrates changes one at a time, rebasing remaining branches after each merge.

**Key tradeoff:** Parallelization works well for additive, independent tasks (new endpoints, new tests, new components) but poorly for tasks that touch shared files (route registries, configuration, database schemas). The system must analyze task file overlap before parallelizing and route overlapping tasks to sequential execution.

### Repo improvement from a goal spec

The user provides a high-level improvement goal: "Increase test coverage to 80%," "Migrate from Express to Fastify," "Add OpenTelemetry instrumentation to all services," or "Fix all P1 security vulnerabilities." The system scans the codebase, derives specific tasks, estimates effort, and executes. This use case is particularly well-suited to the platform/infrastructure engineer profile and benefits from batch execution across many files with consistent transformations.

### Ongoing product construction from a roadmap

The system ingests a product roadmap (a Markdown file, a Linear board, or a structured document) and treats it as a queue of features to build. As each feature is completed and merged, the system picks up the next one, deriving a fresh plan. This is the "overnight factory" use case — continuous, autonomous product development with daily human review checkpoints.

### Autonomous bugfix, refactor, and maintenance

Triggered from external events: a CI failure, a Sentry alert, a security scan, or a GitHub issue assignment. The system picks up the signal, diagnoses the problem, generates a fix, validates it, and opens a PR. Factory's Droids already demonstrate this pattern — the system acts as a responsive, always-available maintenance agent. The key requirement is reliable validation: the fix must not introduce regressions.

---

## 4. User experience model

### How it should feel to use

The product should feel like **operating a well-instrumented factory**, not like writing code. The primary sensation is one of directed oversight — you see work flowing through a pipeline, intervene at decision points, and approve output. The closest analogy in developer experience is a CI/CD dashboard (Buildkite, GitHub Actions) combined with a project management tool (Linear) and a code review interface (GitHub PR review), but unified and AI-native.

The temporal experience is critical. Most interaction should be **asynchronous check-ins**: 30-second glances to assess status, decide whether intervention is needed, and either approve results or redirect. The system must be optimized for this "check-in" workflow — glanceable status indicators, quick diffs, one-click approvals, and clear escalation signals. Devin's favicon status dots (green = working, orange = waiting) are a micro but essential UX innovation to adopt.

### How an operator sets goals

Goals enter the system through three channels, listed in order of preference:

1. **Structured specs**: Markdown documents following a requirements template with user stories, acceptance criteria, and constraints. This is the "gold path."
2. **External triggers**: GitHub issues, Linear tickets, Sentry alerts, CI failures — ingested and automatically converted into structured specs by a planning agent.
3. **Natural language prompts**: Free-form text describing what the user wants. The system generates a structured spec from the prompt and presents it for review before proceeding.

In all cases, the system produces a reviewable plan (requirements → design → tasks) before execution begins. The user reviews, modifies, and approves the plan. This approval is a hard gate — no code is written until the plan is accepted.

### How work is visualized

The **work graph** is the central visualization. It is an interactive DAG where:
- Nodes represent tasks (derived from the spec)
- Edges represent dependencies
- Color indicates status (queued, assigned, in progress, validating, done, failed, blocked)
- Node size or weight indicates estimated complexity
- Each node links to its agent session, code changes, test results, and the spec section it fulfills

This is not a flat task list. It is a dependency-aware graph that makes blocked paths, critical chains, and parallel opportunities visible at a glance. It borrows from Airflow's Graph View (logical DAG structure) and Grid View (runs-over-time matrix) while being purpose-built for agent-driven software development.

### How autonomous progress is observed

Three levels of observation, matching three levels of urgency:

**Glanceable (status bar / notifications)**: A persistent element showing "3 agents working, 1 waiting for approval, 12 tasks completed, 4 remaining." Status beacons (colored dots) for each active agent. Push notifications for completion, failures, or approval requests. Available on mobile.

**Scannable (command center)**: A dashboard showing all active work as cards with agent status, current task, elapsed time, cost, and last action. Modeled after Airflow's home page. One-minute assessment of overall project health.

**Deep (session timeline)**: A Gantt-style timeline for a specific agent session showing phases (planning, reading, writing, testing, fixing), with expandable decision logs, file diffs, and test output at each step. Replay capability for understanding agent reasoning.

### How humans intervene when needed

Intervention happens at three granularities:

- **Redirect**: Change the agent's current task or approach mid-stream via natural language instruction. The agent re-plans within the task's scope.
- **Pause/Resume**: Halt an agent's execution, review its current state, and either resume or abort. All file changes are preserved in the worktree.
- **Approve/Reject**: At configured gates (plan approval, pre-merge review, risky operations), the system pauses and presents a clear decision interface with context.

Anthropic's autonomy research reveals a key insight: experienced users don't approve less — they **auto-approve more but also interrupt more** (5% → 9% of turns). The system should support this pattern: high baseline autonomy with easy, low-friction interruption.

### How trust is built through transparency and control

Trust calibration is the central UX challenge. Only **29% of developers trust AI tool output** (Stack Overflow 2025), down 11 points from 2024. The system builds trust through five mechanisms:

1. **Plan visibility**: Show the full plan before execution. Let users modify it. This prevents the "what is it doing?" anxiety.
2. **Reasoning traces**: Every agent decision includes an expandable reasoning log. Not hidden in debug tools — surfaced in the primary UI.
3. **Agent-initiated uncertainty**: Configure agents to pause and ask when uncertain rather than proceeding confidently. Anthropic's data shows Claude Code pauses for clarification **more often than humans interrupt it**, especially on hard tasks. This is a trust-building behavior.
4. **Progressive autonomy**: New users start with full approval gates. As agents demonstrate competence on a project, the system suggests relaxing gates: "You've approved 200 git operations without issues. Auto-approve git commands?"
5. **The 'recently denied' dashboard**: Show what the system blocked, with one-click retry. Transparency about safety boundaries builds confidence that guardrails work.

---

## 5. Core surfaces and views

### Source goal / spec view

The entry point for all work. Displays the source document (PRD, feature spec, architecture note) alongside the system's derived requirements, design, and task list. A bi-directional traceability matrix maps each requirement to its derived tasks and their current status. The spec has a lifecycle (draft → approved → in progress → implemented → verified) that progresses as agents complete work.

**Key feature:** When an agent's implementation deviates from the spec — or when the agent discovers implied work not in the spec — the system highlights the divergence and prompts for resolution. This prevents spec-code drift, which is currently an unsolved problem across all tools.

### Task graph / work graph

The interactive DAG described in Section 4. Supports zoom, filter (by status, agent, priority, time), and drill-down. Nodes are clickable and expand to show the task's spec section, assigned agent, session timeline, code changes, and test results. TaskGroups (borrowed from Airflow) organize related tasks visually. The graph updates in real time as agents report progress.

**Design recommendation:** Default to a simplified "status board" view (Kanban-like columns: queued, in progress, reviewing, done) with a toggle to the full DAG view. Most users will want the board; architects and operators will want the graph.

### Active agent sessions

A list or grid of currently running agents, each showing: task name, status beacon, current action (one-line summary), elapsed time, tokens consumed, cost, and context utilization percentage. Click to expand into the session timeline. Support for the parallel grid view (Cursor 3's Agents Window pattern) for monitoring multiple agents simultaneously.

**Critical metric to surface:** Context utilization (what percentage of the context window is consumed). Claude Code's performance degrades at **50-75% window fill**. When an agent approaches this threshold, the system should either trigger automatic compaction or alert the operator.

### Branch / worktree map

A visual representation of the repository's branch topology showing: main branch, active worktree branches (one per agent), their divergence from main, and potential merge conflicts. Color-coded conflict indicators (green = clean merge, yellow = minor conflicts, red = major conflicts) updated continuously using `git merge-tree` simulation (the Clash approach).

**Merge queue view:** When agents complete tasks, their branches enter a merge queue. The system merges sequentially: merge branch A → rebase remaining branches on updated main → merge branch B → repeat. The merge queue shows order, estimated merge complexity, and any required conflict resolution.

### Validation / testing view

Shows the current validation state of each task and the overall project: test suite status, linting results, type-check output, build status, and any custom validation rules defined in the spec. For each agent's work, shows a validation pyramid: deterministic validators (linters, type checkers) → automated test results → LLM-based evaluation → human review status.

**Design recommendation:** Integrate validation directly into the task graph (each node shows a green/red/yellow validation badge) rather than having a separate testing view. The separate view serves as a drill-down for detailed test output.

### Memory / knowledge view

Displays the system's accumulated knowledge about the codebase and project: architecture patterns, coding conventions, dependency relationships, API contracts, and learned preferences. This is the persistent context that prevents the "50 First Dates problem" — agents starting fresh each session with no memory of past work.

Inspired by Windsurf's Memories (auto-learned conventions), Devin's Repo Notes/Wiki, and Claude Code's CLAUDE.md files, but structured as a queryable knowledge graph rather than flat text. Includes an "autoDream" background process (inspired by Claude Code's leaked architecture) that consolidates learnings between sessions, resolves contradictions, and rewrites the memory index.

### Diff / merge / review view

A purpose-built code review interface for agent-generated code. Unlike standard GitHub PR review, it includes:
- **Per-hunk accept/reject** (not just per-file)
- **Agent reasoning annotations** explaining why specific changes were made
- **Automated quality indicators** — highlighted areas of concern (complexity increases, pattern violations, potential security issues)
- **Spec traceability** — each change linked to the requirement it fulfills
- **Comparison mode** for best-of-N parallel execution: view diffs from multiple agents side by side

**Design principle:** Treat agent output like code from a capable but imperfect junior developer. The review interface should facilitate efficient, high-throughput review — not force "accept all or reject all."

### Autonomous run timeline / audit trail

A chronological record of every action taken by every agent, structured as an event stream. Each event captures: timestamp, action type, tool used, input/output summaries, risk classification, approval status, cost, and model ID. Events are hash-chained for tamper evidence. Supports filtering by agent, time range, action type, and risk level.

**Regulatory readiness:** Designed to satisfy EU AI Act Article 12 (automatic recording), Article 19 (6-month log retention), and SOC 2 Trust Services Criteria. While compliance is not a v1 feature, the data model must be extensible to support it. The IETF's 2026 draft "Agent Audit Trail" standard provides a useful schema starting point.

### Configurable autonomy / policy controls

A hierarchical settings system following Claude Code's model: Enterprise Policies > Organization > Team > Project > User. Higher levels override lower. Specific controls include:

- **Allowed/disallowed tools** with glob patterns (e.g., allow `Bash(git *)`, deny `Bash(rm -rf *)`)
- **File system boundaries** (workspace jail, protected paths)
- **Network access controls** (allowlisted domains for agent HTTP requests)
- **Cost budgets** per session, per task, and per day — with auto-pause at thresholds
- **Autonomy presets**: "Supervised" (approve everything), "Guided" (approve plans and risky ops), "Autonomous" (approve only merges), "Full Auto" (sandboxed environments only)
- **Hooks**: PreToolUse, PostToolUse, PermissionDenied — custom scripts that can allow, deny, or modify tool requests at runtime

**Open question:** Should autonomy be configured per-agent-type (planner vs. coder vs. reviewer) or per-task-type (bugfix vs. new feature vs. refactor)? The answer is likely both, with task-type defaults that can be overridden per-agent.

---

## 6. Product boundaries and non-goals

### What should NOT be in v1

**Full IDE replacement.** The product should not attempt to replace Cursor, VS Code, or JetBrains as a code editor. The editing experience is a commodity; the orchestration experience is the differentiator. V1 should include a capable embedded editor (Monaco-based) for review and minor edits, but developers should be able to use their preferred editor alongside the product. The product's value is above the editor — in planning, dispatch, monitoring, and review.

**App generation from scratch.** The product is not Lovable, Bolt, or Replit Agent. It is designed for existing codebases and ongoing software engineering, not greenfield "describe an app and get it built." While it should support new projects, the primary value is in sustained, multi-session work on real codebases.

**Hosting and deployment.** The product orchestrates code generation and validation, not infrastructure management. Deployment remains the responsibility of existing CI/CD systems. The product should integrate with deployment pipelines (via MCP) but not replace them.

**Custom model training or fine-tuning.** The product is model-agnostic and should work with any capable frontier model via API. It should not require or offer custom model training. The moat is in the orchestration harness, not the model — as the Claude Code architecture demonstrates.

**Natural language programming for non-developers.** While the AI-native operator profile includes people who don't write code, v1 should assume users can read code, understand git, and evaluate technical decisions. A non-technical interface is a v2+ opportunity.

### What should remain outside the product initially

- **Real-time collaborative editing** (Google Docs-style multi-cursor for human + agent). Complexity is too high for v1; use git-based workflows instead.
- **Cross-repository orchestration** (coordinating agents across a microservices architecture). Focus v1 on single-repo workflows.
- **Automated production deployment** of agent-generated code. The merge-to-main gate should remain manual in v1.
- **Agent marketplace** or plugin ecosystem. Standardize on MCP for tool integration; defer a custom marketplace.

### What kinds of "full autonomy" claims are unsafe or premature

Based on current capability thresholds and documented failure modes, the following claims would be irresponsible:

- **"Ship to production without review."** Agent-authored PRs have **15-40% lower acceptance rates** when properly reviewed. The quality gap is real. Claiming autonomous production deployment is premature.
- **"Replaces your engineering team."** Multi-step software evolution (spanning many files over time) achieves only **21% success** even with GPT-5 on the SWE-EVO benchmark. Agents remain "junior at execution."
- **"Works on any codebase."** Agents struggle with brittle environments (flaky tests, complex service orchestration, legacy codebases with poor documentation). Performance varies dramatically by codebase quality.
- **"Cost-effective at scale."** Early Cursor 3 adopters reported **$2,000+ in 2 days** running parallel cloud agents. Cost predictability remains terrible — only 15% of organizations can forecast AI costs within ±10%.

The product should position autonomy as a spectrum with clearly communicated capabilities and limitations, not as a binary "autonomous" claim.

---

## 7. Success criteria

### What would make this genuinely better than a standard AI editor

The product succeeds if it achieves outcomes that are structurally impossible with current tools:

**Sustained multi-session execution.** A user defines a feature via spec on Monday morning, approves a plan, and returns Tuesday to find 80%+ of the implementation complete, tested, and ready for review — with a clear audit trail of everything that happened. No current tool delivers this reliably.

**Parallel agent throughput.** Three to five agents working simultaneously on independent tasks within a single repo, with automated conflict detection and merge coordination, completing in aggregate what would take a single developer 2-3x as long sequentially.

**Spec-to-code traceability.** Every line of generated code traces back to a specific requirement in the source spec. Every requirement shows its implementation status. The spec and the code stay synchronized as both evolve.

**Operator efficiency.** A technical lead can effectively oversee 5-10 agents working across multiple features, spending less than 20% of their time on oversight and the remainder on high-judgment work (architecture decisions, spec refinement, stakeholder communication).

### What user outcomes indicate it's working

- **Time-to-PR decreases by 3-5x** for well-specified features compared to manual implementation
- **PR merge rate exceeds 70%** for agent-generated code (matching Devin's best-case and exceeding its 67% average)
- **Agent utilization exceeds 6 hours/day** of autonomous productive work per user (indicating trust and sustained execution)
- **Check-in frequency stabilizes at 3-5x/day** rather than continuous monitoring (indicating appropriate trust calibration)
- **Spec completion rate exceeds 85%** — features defined in specs are fully implemented without requiring the user to fall back to manual coding
- **Users return daily for 30+ days** — sustained engagement indicating the product is core to their workflow, not a novelty

---

## 8. Evolution roadmap

### V1: The orchestrated agent workbench (months 1-6)

**Core capabilities:**
- Spec ingestion and structured planning (requirements → design → tasks)
- Single-agent task execution with configurable autonomy
- Work graph visualization (task board + dependency graph)
- Session timeline with decision logs and diffs
- Git worktree-based isolation for parallel execution (2-3 agents)
- Basic conflict detection and sequential merge coordination
- MCP integration for tool extensibility
- CLAUDE.md / AGENTS.md support for project context
- Checkpoint/rollback per task
- Cost tracking and budget limits

**Primary user:** Solo founder and technical operator working on a single repo.

**Tradeoffs:**
- Single-repo only — cross-repo orchestration deferred
- Embedded editor is minimal — users expected to use external editors for deep coding
- Planning agent quality depends on frontier model capabilities — system adds structure but cannot compensate for weak model reasoning
- Parallel execution limited to independent tasks — dependency-aware scheduling is simplified

**Open questions for v1:**
- Should the product be a standalone desktop app, a VS Code extension, or a web application? Each has tradeoffs: desktop enables deep OS integration and worktree management; VS Code extension leverages ecosystem but constrains UI; web enables mobile check-ins but limits local file access. **Recommendation:** Desktop app with web-based command center for remote monitoring.
- What is the minimum viable planning experience? Can the system generate good plans from unstructured prompts, or must users provide structured specs? **Recommendation:** Support both, but guide users toward structure. Generate structured specs from prompts and present for review.
- How should the product handle model selection? Fixed model, user-selected, or task-routed? **Recommendation:** Default to task-routed (frontier model for planning, optimized model for execution) with user override.

### V2: Multi-agent orchestration and team features (months 6-12)

**Added capabilities:**
- Multi-agent coordination with specialized roles (planner, coder, reviewer, tester)
- Automatic task-to-agent routing based on task type and agent capabilities
- Best-of-N parallel execution for critical tasks
- Cross-session memory consolidation (autoDream-style background process)
- Team dashboards with fleet management (multiple users monitoring shared agent pool)
- External trigger integration (GitHub issues, Linear tickets, Sentry alerts, CI failures)
- Enhanced merge orchestration with AI-assisted conflict resolution
- Validation pyramid integration (linting → tests → LLM evaluation → human review)
- Policy-as-code for enterprise governance
- Audit trail export for compliance

**Primary user expansion:** Engineering teams with 3-10 developers sharing agent resources.

**Tradeoffs:**
- Multi-agent coordination introduces communication overhead — agent teams beyond 3-7 members show diminishing returns and require hierarchical structures
- Team features require authentication, authorization, and multi-tenancy infrastructure
- Cross-session memory creates a cold-start problem for new projects and a stale-memory problem for fast-evolving codebases

**Open questions for v2:**
- How should agents communicate with each other? MCP handles tool access, but agent-to-agent coordination (Google's A2A protocol, IBM's ACP) is still emerging. **Recommendation:** Start with a coordinator pattern (one orchestrator agent dispatches to workers via structured messages) rather than peer-to-peer agent communication.
- How do you price a multi-agent product fairly? Per-agent-hour? Per-task? Per-seat with usage caps? Current market pricing (Cursor's credits, Devin's ACUs, Codex's token-based) all create "token anxiety." **Recommendation:** Capacity-based pricing (X concurrent agents for $Y/month) with clear cost dashboards.
- What is the right boundary between the product's orchestration and existing project management tools? Should the product replace Linear/Jira for agent-managed work, or integrate as a "backend executor"? **Recommendation:** Integrate, don't replace. Sync task status bidirectionally with existing PM tools.

### V3 and beyond: the autonomous engineering platform (12+ months)

**Capabilities to explore:**
- Cross-repository orchestration for microservice architectures (coordinating changes across 5+ repos with API contract validation)
- Continuous autonomous improvement — the system proactively identifies improvement opportunities (test gaps, performance issues, security vulnerabilities, dependency updates) and proposes/executes them
- External side-effect tracking and rollback (API calls, database changes, infrastructure modifications — currently an unsolved problem across all tools)
- Non-developer operator interface — a spec-driven, visual workflow for product managers and designers
- Agent skill marketplace — reusable, community-shared task patterns and agent configurations
- Compliance-ready audit and reporting (EU AI Act, SOC 2, HIPAA)
- Self-improving orchestration — the system learns from past plan success/failure rates to improve future task decomposition and agent routing

**Open questions for v3+:**
- Will foundation model providers (Anthropic, OpenAI, Google) build orchestration into their own products, commoditizing this layer? Claude Code and Codex are already adding sub-agent coordination, background execution, and worktree support. **Mitigation:** The moat is in the UX, the work graph, and the spec-code traceability — not in raw agent execution.
- How does the product handle the transition from "AI-assisted" to "AI-primary" engineering? At what point does the operator role itself need to be redefined? This is a fundamental product vision question that will depend on model capability trajectories.
- Can the product credibly extend to non-software engineering domains (data pipelines, infrastructure-as-code, documentation, design systems)? Each domain has different validation requirements and toolchains. **Recommendation:** Stay focused on software engineering until the core product is mature.

---

## Conclusion: the strategic opportunity

The autonomous engineering system described here occupies a specific and currently **vacant position** in the market: above the code editor, below the project management tool, and across the agent execution layer. Every current tool either edits code with AI assistance or executes isolated tasks autonomously. None provides the connective tissue — the work graph, the spec traceability, the merge orchestration, the operator dashboard — that transforms individual agent capabilities into compounding engineering throughput.

The technical foundations exist. MCP provides a universal tool integration layer with **97M+ monthly SDK downloads** and **10,000+ published servers**. Git worktrees enable parallel agent isolation. Frontier models can sustain multi-hour autonomous coding sessions. Spec-driven planning (Kiro's requirements → design → tasks) provides structured goal decomposition. The pieces are available; the integration is missing.

Three strategic risks deserve explicit acknowledgment. First, **model capability ceilings** — the 35-minute degradation problem and 21% success rate on multi-step evolution tasks mean autonomous engineering remains bounded. The product must deliver value within current capability thresholds while being positioned to benefit as models improve. Second, **platform risk** — Anthropic, OpenAI, and Google are all adding orchestration features to their coding tools. The product must build defensible value in the UX and workflow layer, not the execution layer. Third, **the trust gap** — developer trust in AI output is declining (29%, down 11 points). The product must earn trust through transparency, not claim it through marketing.

The core bet is that **engineering is becoming an operations discipline** — that the highest-leverage work for a skilled engineer is defining intent, reviewing output, and directing autonomous systems, not writing code line by line. If that bet is correct, the product that best serves the operator — with visibility, control, and trust — wins the next generation of developer tools.