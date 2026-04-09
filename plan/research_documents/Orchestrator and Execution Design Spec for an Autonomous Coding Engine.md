# Orchestrator and Execution Design Spec for an Autonomous Coding Engine

**An autonomous coding engine must solve three problems simultaneously: decompose ambiguous specifications into executable tasks, coordinate multiple agents that write and validate code in parallel, and remain safe and inspectable across hours or days of unsupervised operation.** This spec defines the architecture for a system that accepts source documents, plans work, delegates to specialist agents, validates outcomes, and autonomously generates follow-up tasks — all while staying resumable, auditable, and under operator control. It draws on production patterns from OpenAI Codex (built on Temporal), Claude Code (single-loop with sub-agents), Cursor (parallel worktree agents), Devin (fleet-of-VMs), OpenHands (event-sourced), and the broader ecosystem of durable execution frameworks and multi-agent research from 2025–2026.

---

## 1. Responsibilities of the orchestrator

The orchestrator is the deterministic control plane that owns the lifecycle of all work. It does not write code. It does not call LLMs for unbounded generation. Its judgment is narrow and auditable: given a plan, a set of agent results, and a policy configuration, it decides what happens next.

### Planning and decomposition

The orchestrator accepts source documents — PRDs, design specs, GitHub issues, or natural-language descriptions — and delegates them to a **Planner agent** for decomposition into a task graph. The Planner produces a structured plan: a list of tasks with `id`, `type`, `description`, `acceptance_criteria`, `dependencies[]`, `estimated_files[]`, and `priority`. The orchestrator validates this plan for circular dependencies (topological sort), rejects plans that exceed a configurable maximum task count (default: 50 per planning cycle), and presents the plan for operator approval before any execution begins. This two-phase approach — plan then execute — is the single most important architectural decision. Cursor's scaling experiments with hundreds of concurrent agents found that **static pre-planned task decomposition consistently outperformed dynamic agent coordination**, and OpenAI Codex's 25-hour autonomous sessions succeeded precisely because the spec was "frozen" into markdown files the agent revisited repeatedly.

### Prioritization

Tasks are priority-ordered by **reverse-dependency count** (tasks that unblock the most downstream work execute first), weighted by operator-assigned priority overrides. A task with zero dependencies and high reverse-dependency count is the highest-priority candidate. The orchestrator maintains a ready queue of tasks whose dependencies are all satisfied and assigns them to available workers in priority order.

### Delegation

The orchestrator assigns tasks to typed specialist agents: **Coder**, **Test Writer**, **Reviewer**, **Refactorer**, **Documentation Writer**. Each agent receives a narrowly scoped prompt containing the task description, acceptance criteria, relevant file paths, and architectural constraints from a project-level configuration file (analogous to `CLAUDE.md` or `AGENTS.md`). The orchestrator never sends an agent the full plan or full codebase context — only what is needed for the specific task. This follows the production insight from both LangChain and Cognition that **context engineering is the #1 challenge** in multi-agent systems: vague instructions cause sub-agents to misinterpret tasks or duplicate work.

### Progress tracking

The orchestrator maintains a **task board** — a persistent data structure recording every task's current state, assigned agent, start time, elapsed time, token consumption, iteration count, and validation results. This board is the single source of truth for system state and is persisted durably (see Section 7). Progress is reported via structured events, not free-text summaries. The orchestrator polls or subscribes to agent completion events rather than waiting synchronously.

### Validation routing

When an agent reports task completion, the orchestrator does not accept the claim. It routes the result to the **Validation Pipeline** (see Section 6), which runs deterministic checks (build, lint, typecheck, test suite) and optionally delegates to a Reviewer agent for semantic review. Only when validation passes does the orchestrator mark the task as accepted.

### Task regeneration

When validation fails, the orchestrator decides whether to retry (send the failure context back to the original agent), reassign (give the task to a different agent or model), or decompose (break the task into smaller sub-tasks). Retry is the default for the first two failures. After three consecutive failures on the same task, the orchestrator escalates to the operator with a structured failure report including the task description, attempted approaches, validation output, and a recommended next action.

### Escalation to human

The orchestrator escalates in four situations: (1) a task has failed three times, (2) a policy gate requires approval (see Section 5), (3) the system detects potential spec drift (see Section 8), or (4) the autonomous work generator proposes work that exceeds a configurable novelty threshold. Escalations are durable — the system pauses the affected task (not the entire run) and persists state until the operator responds, which may be hours or days later.

### Stopping conditions

The orchestrator enforces layered stopping conditions. A run terminates when: all tasks in the plan are accepted, the operator issues a stop command, or any hard limit is breached. **Hard limits are non-negotiable**: maximum wall-clock time per run (default: 8 hours), maximum total token spend (configurable per run), maximum task count (prevents unbounded proliferation), and maximum consecutive failures across all agents (default: 10). Soft limits trigger warnings: elapsed time thresholds, cost thresholds, no-progress detection (no task accepted in the last N minutes).

---

## 2. Execution model

### Single orchestrator with hierarchical delegation

The system uses a **single durable orchestrator** that delegates to specialist agents. For large projects, the orchestrator can spawn **Feature Lead sub-orchestrators**, each managing a sub-graph of tasks for a specific feature or module. Feature Leads have their own task queues but report completion back to the root orchestrator. This two-level hierarchy is the sweet spot: production evidence from Claude Code's team architecture and IBM research shows that hierarchical structures become necessary beyond 5 agents, but more than two levels of hierarchy introduces coordination overhead that exceeds its benefits for most codebases.

```
┌─────────────────────────────────────────────────────────────┐
│                    ROOT ORCHESTRATOR                        │
│  Plan approval · Task queue · Dependency graph · Policies   │
│  Stopping conditions · Escalation · Progress tracking       │
├─────────────┬──────────────┬──────────────┬─────────────────┤
│ Feature Lead│ Feature Lead │ Feature Lead │ (optional)      │
│ Auth Module │ API Module   │ UI Module    │                 │
├─────┬───────┼──────┬───────┼──────┬───────┤                 │
│Coder│Tester │Coder │Tester │Coder │Tester │                 │
│     │Review │      │Review │      │Review │                 │
└─────┴───────┴──────┴───────┴──────┴───────┘
         ▲              ▲              ▲
         │              │              │
    Git Worktree A  Worktree B    Worktree C
```

### Workers, specialists, and sub-agents

**Specialist agents** are typed by role: Planner, Coder, Test Writer, Reviewer, Refactorer, Doc Writer. Each specialist is a stateless function: it receives a task payload, performs work using tools (file read/write, shell execution, web search), and returns a structured result. Specialists do not maintain state between tasks — all continuity comes from the orchestrator's task board and the repository itself. This statelessness is critical for fault tolerance: a crashed specialist can be replaced without losing progress.

**Sub-agent spawning** is permitted within a specialist's execution. A Coder agent may spawn a sub-agent to research an API or generate a helper function. Sub-agents inherit their parent's worktree and policy constraints but have independent iteration budgets (default: 5 iterations). The parent agent is responsible for aggregating sub-agent results. Sub-agent depth is limited to 2 levels to prevent unbounded delegation chains.

### Isolated execution contexts

Every agent execution occurs in an **isolated context** — either a git worktree (for local execution) or a sandboxed container/microVM (for cloud execution). Worktrees share the same `.git` database but provide complete filesystem isolation, preventing agents from interfering with each other's uncommitted changes. Each worktree gets its own branch, named deterministically (`agent/<task-id>`). Known worktree limitations to handle: port conflicts between parallel dev servers, missing `node_modules` or `.env` files (solved by a shared setup script), and disk space growth (~5x repo size per active worktree).

For higher isolation guarantees, use sandboxed containers (Docker with gVisor, Firecracker microVMs, or E2B-style cloud sandboxes). OpenAI Codex uses per-task microVMs with **internet disabled by default**; Devin uses full cloud VMs per session. The isolation level should be configurable per deployment: worktrees for trusted internal use, containers for untrusted or production-adjacent work.

### Concurrency model

The orchestrator runs a **bounded worker pool** with configurable concurrency (default: 3–5 parallel agents). Tasks are pulled from the ready queue by available workers. The pool is bounded by a semaphore to prevent resource exhaustion. **Three to five concurrent agents is the empirically validated sweet spot** — below 3, a single agent suffices; above 7, coordination complexity and compound error rates outweigh throughput gains unless hierarchical structures are used. Cursor's production experience found that scaling to hundreds of agents required static planning and periodic fresh starts to remain productive.

Independent tasks (no shared file dependencies) execute in parallel. Tasks that modify the same files are serialized — the orchestrator detects file-level conflicts from `estimated_files[]` in the task definition and enforces ordering. A file-locking mechanism (logical locks in the orchestrator, not filesystem locks) prevents concurrent edits to the same file.

### Task queues and dependency graphs

The task graph is a directed acyclic graph stored in the orchestrator's durable state. Each task tracks its dependencies (upstream tasks that must complete first) and dependents (downstream tasks it unblocks). When a task is accepted, the orchestrator walks its dependents and moves any newly-unblocked tasks to the ready queue. The graph supports dynamic modification: new tasks can be inserted, and dependency edges can be added, as the run progresses.

### Retry and failure handling

Retries follow a structured escalation policy per task:

- **Attempt 1–2**: Retry with the same agent, appending the failure context (error messages, test output, validation results) to the prompt. Include a forced reflection: "What specifically failed? What single change would fix it?"
- **Attempt 3**: Reassign to a different model or agent type (e.g., switch from a fast model to a stronger reasoning model).
- **Attempt 4+**: Escalate to operator. The task is paused (not the run), and the orchestrator continues with other available tasks.

Retries use **exponential backoff with jitter** for rate-limited or infrastructure failures. Non-retryable errors (authentication failures, missing repository access) are immediately escalated.

### Pausing and resuming runs

The orchestrator supports pause and resume at any point. Pausing persists the full task graph state, all pending task payloads, and the current ready queue to durable storage. Resuming reconstructs the orchestrator state from the persisted checkpoint and continues from where it left off. Individual tasks can be paused independently — an escalated task waiting for human approval does not block other tasks.

### Long-running autonomous sessions

For multi-hour sessions, the orchestrator implements **session hygiene**: every 60 minutes (configurable), it performs a "session checkpoint" that logs cumulative progress (tasks completed, tasks remaining, tokens consumed, cost incurred, failures encountered) and evaluates whether continued execution is likely to produce value. If the ratio of accepted tasks to attempted tasks drops below a threshold (default: 20% over the last hour), the orchestrator pauses and notifies the operator rather than continuing to burn tokens unproductively. OpenAI Codex's 25-hour sessions and Claude Code's 45-minute turn durations demonstrate that long-running autonomy is achievable, but only with explicit progress tracking and durable project memory (markdown files the agent revisits to maintain alignment).

---

## 3. Task lifecycle

Every task passes through a defined state machine with **8 states**. Transitions are event-driven, logged, and auditable.

```
CREATED → ENRICHED → READY → ASSIGNED → EXECUTING → VALIDATING → ACCEPTED
                                  ↑           │                      │
                                  │           ▼                      │
                                  ├──── REJECTED ◄───────────────────┘
                                  │        │          (validation fail)
                                  │        ▼
                                  │   FAILED / ESCALATED
                                  │
                                  └── FOLLOW_UP_GENERATED
```

### Task creation

Tasks originate from three sources: (1) **plan decomposition** — the Planner agent breaks a spec into tasks, (2) **failure-driven generation** — a failed validation produces a follow-up task targeting the specific failure, (3) **gap discovery** — the autonomous work generator identifies missing work (see Section 4). Every task is created with a unique ID, a source reference (which spec, failure, or gap triggered it), and a creation timestamp.

### Task enrichment

Before a task enters the ready queue, the orchestrator enriches it with context: relevant file paths (discovered via codebase indexing or AST analysis), related test files, architectural constraints from the project configuration, and any prior attempt history. Enrichment is deterministic and fast — no LLM calls. The enriched task payload is what the assigned agent receives.

### Assignment

The orchestrator assigns tasks from the ready queue to available workers based on worker type (a testing task goes to a Test Writer agent, not a Coder) and current load. Assignment is logged with a timestamp and worker ID.

### Execution

The agent executes the task within its isolated context (worktree or sandbox). Execution is bounded by per-task limits: maximum iterations (default: 15), maximum tokens (configurable), and maximum wall-clock time (default: 30 minutes). The agent produces a structured result containing: files modified (with diffs), commands executed (with output), tests run (with results), and a completion claim with supporting evidence.

### Validation

The orchestrator routes the agent's result through the Validation Pipeline (Section 6). Validation is a deterministic, repeatable process — no agent judgment involved in the core checks. The pipeline produces a pass/fail verdict with structured evidence.

### Acceptance or rejection

If validation passes, the task moves to ACCEPTED. The orchestrator commits the agent's changes (if not already committed) and updates the dependency graph, unblocking downstream tasks. If validation fails, the task moves to REJECTED with the failure evidence attached. The orchestrator then applies the retry/reassign/escalate policy.

### Task closure

An ACCEPTED task is closed and its changes are staged for integration (merge to the target branch). Closure triggers downstream dependency resolution and may trigger follow-up task generation.

### Generation of implied follow-up tasks

After a task is accepted, the orchestrator runs a lightweight **follow-up check**: does this change imply additional work? For example: a new API endpoint may need documentation, a new model may need migration scripts, a new feature may need integration tests beyond the unit tests already written. Follow-up tasks are generated by querying a Follow-Up Generator agent with the diff and the original spec. Generated follow-ups are subject to the same plan-approval gate as initial tasks — they are proposed, not automatically executed. This prevents unbounded task proliferation.

---

## 4. Autonomous work generation

This is the most dangerous capability in the system. Unbounded autonomous work generation leads to **meaningless task proliferation, spec drift, and wasted resources**. Every mechanism described here includes explicit constraints.

### How the system determines the "next best thing to do"

The orchestrator maintains a **work backlog** — an ordered list of potential tasks. The backlog is populated from four sources, in priority order:

1. **Plan tasks**: Tasks from the approved plan that are not yet complete. These always take priority.
2. **Failure-derived tasks**: When a task fails validation, the specific failure is converted into a new task (e.g., "Fix type error in `auth.ts` line 42"). These are auto-approved because they directly support a plan task.
3. **Spec-gap tasks**: After the initial plan is executed, a Gap Analyzer agent compares the original spec against the current repository state. It identifies missing requirements (features described in the spec but not implemented), missing test coverage, and missing documentation. Gap tasks require operator approval.
4. **Improvement tasks**: Optional, lowest priority. The system can identify code quality issues (dead code, missing error handling, inconsistent patterns). These are never auto-approved and are only generated if explicitly enabled by the operator.

### Discovering missing work from source documents

The Gap Analyzer works by re-reading the original spec and producing a checklist of requirements, then querying the codebase (via grep, AST analysis, and test coverage reports) to determine which requirements have been implemented. Requirements without corresponding code or tests become gap-task candidates. This is re-run after each major milestone (configurable: every N accepted tasks, or after all plan tasks complete).

### Identifying repo gaps

Beyond spec compliance, the system can scan for structural gaps: exported functions without tests, API endpoints without documentation, error paths without handling, environment variables referenced but not documented. These scans use deterministic tooling (coverage reports, linters, static analysis) rather than LLM judgment, making them reliable and fast.

### Turning failures into new tasks

When a task fails, the orchestrator extracts the **specific failure signal** — compiler error, test failure, lint violation — and wraps it in a new task. The new task has a narrower scope than the original (fix this specific error, not re-implement the feature) and inherits the original task's priority. Failure-derived tasks are capped at **3 per original task** to prevent retry storms from metastasizing into task proliferation.

### Preventing meaningless or low-value task proliferation

Five constraints prevent runaway generation:

- **Task budget**: A hard cap on total tasks per run (default: 100). The orchestrator refuses to create tasks beyond this limit.
- **Novelty check**: Before creating a task, the orchestrator checks whether a substantially similar task already exists (by description similarity and file overlap). Duplicates are rejected.
- **Value scoring**: Each proposed task is scored on a 1–5 scale based on: alignment with the original spec (is it mentioned?), blocking impact (does it unblock other tasks?), and estimated complexity. Tasks scoring below 2 are discarded.
- **Approval gates**: All spec-gap and improvement tasks require operator approval. Only plan tasks and failure-derived tasks are auto-approved.
- **Decay function**: As the run progresses, the threshold for auto-approving new work increases. Early in a run, failure-derived tasks are freely created. After 80% of the plan is complete, even failure-derived tasks require approval if they involve files outside the original plan scope.

### Staying aligned with the original goal

The orchestrator maintains a **frozen copy of the original spec** that is never modified during execution. Every proposed task is checked against this frozen spec for relevance. Additionally, every 30 minutes, the orchestrator generates a structured progress summary that compares completed work against spec requirements, making drift visible. The pattern from OpenAI Codex's long-horizon experiments is instructive: the spec was written to markdown files that the agent revisited repeatedly, and this stable reference point prevented the agent from "building something impressive but wrong."

---

## 5. Control and safety model

### Autonomy levels

The system supports **four operator-configurable autonomy levels**, inspired by Anthropic's empirical research on Claude Code usage patterns (where experienced users auto-approve >40% of actions but interrupt more frequently for targeted intervention):

| Level | Name | Behavior |
|-------|------|----------|
| L1 | **Supervised** | Every task result requires operator approval before acceptance. Agents cannot execute shell commands without approval. Plan changes require approval. |
| L2 | **Guided** | Plan requires approval. Task results are auto-accepted if validation passes. Shell commands in an allowlist execute without approval; others require approval. |
| L3 | **Autonomous** | Plan requires approval. Tasks execute and validate autonomously. Operator notified of progress at configurable intervals. Escalation only on failure or policy violation. |
| L4 | **Full Auto** | Plan is auto-approved if it passes structural validation (no circular deps, within task budget). Fully autonomous execution within policy constraints. Operator notified on completion or failure. **Only for isolated environments with no production access.** |

The default is **L2 (Guided)**, which matches the empirical finding that the most productive configuration is high auto-approval with targeted human intervention at decision points.

### Policy gates

Policy gates are **deterministic checkpoints** evaluated by the orchestrator (not by an LLM) before, during, or after agent actions. They are defined in a YAML policy file versioned alongside the project:

```yaml
policies:
  pre_execution:
    - gate: plan_approval
      requires: operator  # at L1-L3
    - gate: task_budget
      max_tasks: 100

  during_execution:
    - gate: file_scope
      deny: ["*.env", "*.secret", "docker-compose.prod.*", ".github/workflows/*"]
    - gate: command_allowlist
      allow: ["npm test", "npm run build", "npx tsc", "pytest", "go test"]
      deny: ["rm -rf", "git push --force", "curl", "wget", "DROP TABLE"]
    - gate: cost_limit
      max_tokens_per_task: 500000
      max_cost_per_run: 50.00

  post_execution:
    - gate: validation_required
      checks: [build, lint, typecheck, test]
    - gate: merge_approval
      requires: operator  # always
```

Gates are evaluated in order: **deny rules always override allow rules**, matching Claude Code's deny-first evaluation model. This ensures that a misconfigured allow rule cannot bypass a safety constraint.

### Operator approval points

The operator is consulted at these mandatory checkpoints regardless of autonomy level: (1) **initial plan approval** (except L4), (2) **merge to target branch** (always — the engineer is the final approval gate), (3) **tasks exceeding scope** (modifying files not in the original plan's file list), (4) **escalated failures** (3+ consecutive failures). At L1, the operator is additionally consulted for every task acceptance and every non-allowlisted command.

### Safe default behaviors

The system ships with conservative defaults that an operator must explicitly relax:

- **Read-only by default**: Agents start with read permissions only; write permissions are granted per-task by the orchestrator.
- **No network access**: Agents cannot make outbound HTTP requests unless explicitly allowlisted per-project. This prevents data exfiltration and dependency confusion attacks.
- **No production access**: Agents have no credentials for production databases, APIs, or deployment systems. Ever.
- **Minimum necessary scope**: Agents only receive context for files relevant to their task, not the full repository.
- **Bias toward reversibility**: When multiple approaches exist, prefer the one that can be undone (create a new file rather than modify an existing one; add a migration rather than alter a table directly).
- **Ask on uncertainty**: If an agent's confidence is below a threshold, it pauses and generates a clarification request rather than proceeding with a guess.

### Branch and worktree isolation

All agent work happens on **feature branches**, never on main/trunk. The branch naming convention is deterministic: `auto/<run-id>/<task-id>`. Each parallel agent gets its own git worktree. Worktrees are provisioned from the orchestrator and cleaned up after task acceptance or run completion. A worktree farm (pool of pre-provisioned worktrees) reduces setup latency for parallel execution.

### Merge rules

Agent branches are **never auto-merged to main**. The orchestrator creates a pull request with: the diff, the original task description, validation results (build/test/lint output), and the agent's completion evidence. Merge requires operator approval. For L3–L4 autonomy, the orchestrator may auto-merge to a **staging branch** (`auto/staging/<run-id>`) that aggregates all accepted task branches, but merging staging to main always requires human review. Branch protection rules (required status checks, required reviews) are enforced at the repository level, not by the orchestrator.

### Destructive action controls

The Kiro incident (December 2025), where Amazon's AI deleted and recreated an entire production AWS environment causing a 13-hour outage, and the Claude Code `rm -rf ~/` incident demonstrate that destructive action controls must be **defense-in-depth**, not single-layer:

1. **Agent reasoning layer**: System prompt explicitly lists prohibited actions. Agents are instructed never to use `--force`, `--no-verify`, or destructive shortcuts.
2. **Command filter layer**: The orchestrator's policy engine pattern-matches every shell command against deny lists before execution. Commands containing `rm -rf`, `DROP`, `TRUNCATE`, `--force`, `reset --hard` are blocked regardless of agent intent.
3. **Filesystem layer**: Write permissions are scoped to the worktree directory. Writes to `.git/`, home directory, system directories, and any path outside the worktree are blocked by OS-level sandboxing (Bubblewrap on Linux, Seatbelt on macOS).
4. **Infrastructure layer**: IAM roles/permissions for agent execution contexts have no write access to production resources, deployment pipelines, or secret stores. Even if all other layers fail, the agent cannot reach production.

---

## 6. Validation model

Validation is the mechanism by which the system distinguishes real progress from **hallucinated progress** — the most insidious failure mode in autonomous coding, accounting for 12% of failures in production autonomous sessions. The validation model is built on one principle: **require observable evidence, never accept confidence claims**.

### Test generation and execution

When a Coder agent completes a task, the orchestrator dispatches a **Test Writer agent** to generate tests for the new code if the task did not already include test writing. Tests are generated against the acceptance criteria in the task definition, not against the implementation — this is critical to avoid the documented failure mode where AI-generated tests "validate implemented behavior rather than specified behavior," effectively protecting bugs rather than catching them.

Tests are executed in the agent's isolated context. The test runner captures: pass/fail status, execution time, coverage delta, and stdout/stderr. Only deterministic test results are accepted — flaky tests (different results on re-run) are flagged and excluded from the validation verdict.

### Lint, type, and build checks

Every task result is validated against three deterministic checks, run in sequence:

1. **Build**: The project compiles/builds without errors. Build output is captured.
2. **Type check**: Static type checking passes (TypeScript `tsc --noEmit`, Python `mypy`, etc.). Type errors are captured with file and line references.
3. **Lint**: Project linter runs clean (or with no new violations compared to the base branch). Lint violations are captured.

These checks are **non-negotiable** — a task cannot be accepted if any of them fail. They run before more expensive validation (tests, review) to fail fast on obvious issues.

### Task-specific acceptance criteria

Every task carries explicit acceptance criteria defined at creation time. After deterministic checks pass, a **Verifier agent** evaluates whether the acceptance criteria are met by examining the diff, test results, and build output. The Verifier produces a structured verdict: for each criterion, it states whether it is met, not met, or unclear, with specific evidence (file paths, line numbers, test names). Criteria marked "unclear" trigger operator review.

The Verifier is instructed to watch for **hedging language** in its own output — words like "should," "probably," "seems to," "I believe," and "looks correct" trigger automatic re-verification. Each such word indicates the Verifier is reasoning rather than observing, which is precisely the behavior that leads to phantom verification.

### Spec-completion checks

At milestone boundaries (configurable: every N tasks, or after all tasks in a feature group complete), the orchestrator runs a **spec-completion check**. The Gap Analyzer re-reads the frozen spec and evaluates which requirements are now implemented, producing a completion percentage and a list of remaining gaps. This check is separate from task-level validation — it catches the case where all individual tasks pass but the aggregate result doesn't satisfy the spec (the "all green but wrong" failure mode).

### Regression avoidance

The system implements three layers of regression protection:

1. **Pre-existing test suite**: Before any agent executes, the orchestrator runs the full existing test suite and records the baseline pass count. After each task acceptance, the full suite runs again. Any decrease in passing tests (PASS_TO_PASS failures) is a regression and blocks acceptance. Research from TDAD (Test-Driven Agentic Development, 2026) found that vanilla coding agents cause **an average of 6.5 broken existing tests per patch** — this check is essential.

2. **Dependency-aware testing**: The orchestrator maps which tests cover which source files (via coverage data or AST analysis). When a task modifies files, only the relevant tests are run for fast feedback. But full-suite regression runs happen before merge.

3. **Change-scope validation**: If an agent modifies files outside its task's `estimated_files[]` list, the orchestrator flags the change as out-of-scope and requires additional validation (full test suite run + operator review).

### Evidence required before accepting work

Inspired by the Evidence Gate protocol, the following evidence is required before any task can move to ACCEPTED:

| Evidence Type | Required Artifact |
|---|---|
| Code compiles | Build output showing zero errors |
| Types check | Type checker output showing zero errors |
| Lint clean | Linter output showing no new violations |
| Tests pass | Actual test runner output (not agent claims) showing pass count and zero failures |
| No regressions | Full test suite results showing pass count ≥ baseline |
| Acceptance criteria met | Verifier verdict with per-criterion evidence |
| Diff is minimal | Changed files match task scope; no unrelated modifications |

If any evidence is missing or inconclusive, the task returns to REJECTED for another attempt.

---

## 7. Durability and robustness requirements

An autonomous coding engine that runs for hours must survive crashes, restarts, and infrastructure failures without losing progress. The durability model is informed by production patterns from Temporal (used by OpenAI Codex and Replit Agent), Restate, DBOS, and OpenHands' event-sourced architecture.

### Checkpointing

The orchestrator checkpoints state at every significant transition: task state changes, agent assignments, validation results, dependency graph updates, and follow-up task generation. Checkpoints are written to a durable store (PostgreSQL for single-node deployments, or a dedicated Temporal/Restate service for distributed deployments). **The checkpoint interval is the state transition, not a time interval** — every state change is immediately persisted, ensuring zero data loss on crash.

Agent-level checkpointing operates differently. Agents checkpoint their conversation state and file modifications after each tool call. If an agent crashes mid-task, it can resume from the last tool call rather than restarting the task from scratch. Claude Code's checkpoint system (automatic state save before each change, rewind via `/rewind`) and OpenHands' event-sourced model (deterministic replay from event log) are the reference implementations.

### Resumability

The system supports three levels of resume:

1. **Orchestrator restart**: The orchestrator process crashes and restarts. It reads the last checkpoint, reconstructs the task graph, identifies in-progress tasks (now stale — their agents are gone), and re-queues them. Accepted tasks are not re-executed. The run continues from where it left off.

2. **Agent restart**: A single agent crashes. The orchestrator detects the failure (via heartbeat timeout), marks the task as available, and assigns it to a new agent. The new agent receives the full task payload plus any partial progress from the previous agent's checkpoint.

3. **Full system restart**: Everything crashes (infra failure, deployment). On restart, the orchestrator loads the last checkpoint, resets all in-progress tasks to READY, and begins re-execution. Accepted work is preserved. This is the "continue-as-new" pattern from Temporal, adapted for the coding domain.

### Durable state

The following state must survive any single failure:

- **Task graph**: All tasks, their states, dependencies, and history.
- **Plan**: The approved plan, frozen spec, and any plan amendments.
- **Agent results**: Completed task results, validation verdicts, and evidence artifacts.
- **Run metadata**: Start time, configuration, policy settings, cost tracking, operator decisions.
- **Git state**: Branch names, commit SHAs, worktree locations. (Git itself is durable; the orchestrator tracks references.)

State is stored in a **single PostgreSQL database** for simplicity in v1. The schema uses an append-only event log for the task graph (enabling replay and audit) with materialized views for current state (enabling fast queries). This follows the ESAA (Event Sourcing for Autonomous Agents) pattern validated with 4 concurrent heterogeneous LLM agents across 50 tasks.

### Idempotent actions

All orchestrator actions must be idempotent — safe to execute more than once:

- **Task assignment**: Assigning an already-assigned task is a no-op.
- **Validation**: Running validation on already-validated results returns the cached verdict.
- **Git operations**: Creating a branch that already exists is handled gracefully. Committing when there are no changes is a no-op.
- **Agent tool calls**: Agents use idempotency keys for external API calls. File writes are idempotent (writing the same content is a no-op).

### Recovery after tool/process failure

Tool failures (git command fails, test runner crashes, LLM API returns 500) are handled by the retry policy. The orchestrator distinguishes between **transient failures** (network timeouts, rate limits — retry with backoff) and **permanent failures** (authentication error, missing file — escalate immediately). Agents report failure type in their structured results, and the orchestrator applies the appropriate policy.

LLM API failures receive special handling: the orchestrator maintains a **model fallback chain** (e.g., Claude Sonnet → GPT-4o → Claude Haiku). If the primary model is unavailable, tasks are retried with the next model in the chain. Model fallback is logged and reported to the operator.

### Auditability

Every action in the system is logged as a structured event in the append-only event log:

- Task state transitions (with before/after states)
- Agent assignments (with agent type and model)
- Tool calls (with arguments and results)
- Policy gate evaluations (with gate name, input, and verdict)
- Operator decisions (with timestamp and decision)
- Validation results (with full evidence)
- Cost events (tokens consumed, API calls made)

The event log supports **deterministic replay**: given the event log and the initial state, the system can reconstruct the exact state at any point in time. This enables post-hoc debugging, incident investigation, and compliance auditing.

### Observability

The orchestrator exposes real-time observability via:

- **Dashboard**: Task board showing current state of all tasks, active agents, cost accumulator, progress percentage, and recent events.
- **Structured logs**: JSON-formatted logs with correlation IDs linking events across tasks and agents.
- **Metrics**: Task completion rate, validation pass rate, retry rate, cost per task, agent utilization, time-to-completion per task type.
- **Alerts**: Configurable alerts for: run cost exceeding threshold, no progress in N minutes, consecutive failures, policy violations.

---

## 8. Failure modes and mitigations

Production data from Columbia DAPLab's analysis of hundreds of failures across 15+ applications, the MAST study of 1,642 multi-agent execution traces (41–87% failure rates), and documented incidents inform this section. Each failure mode includes its observed frequency, detection mechanism, and mitigation.

### Infinite loops

**Observed frequency**: The single most common agent failure. A Claude Code sub-agent ran `npm install` 300+ times over 4.6 hours consuming 27M tokens. A LangGraph agent processed 2,847 iterations at $400+ for a $5 task.

**Detection**: The orchestrator tracks per-task iteration count and per-tool-call deduplication. If an agent calls the same tool with the same arguments more than twice, a "stuck" flag is raised. No-progress detection identifies iterations that produce no new file changes or test results.

**Mitigation**: Hard iteration cap per task (default: 15). Hard token cap per task. Tool+arguments deduplication — the third identical call triggers a forced reflection prompt ("What specifically is failing? What different approach could you try?"). After 5 iterations with no progress (no new file changes, no new passing tests), the task is killed and escalated. Microsoft Magentic-One's **dual-loop pattern** is adopted: an outer loop resets the entire agent strategy when the inner loop stalls.

### Poor task generation

**Observed frequency**: Common in autonomous work generation. Agents propose vague tasks ("improve code quality"), redundant tasks (duplicates of existing tasks), or tasks unrelated to the spec.

**Detection**: Novelty checking (similarity against existing tasks), value scoring (alignment with spec, blocking impact), and scope checking (do proposed files overlap with the spec?).

**Mitigation**: The task budget (max 100 tasks per run) is the hard ceiling. All autonomously generated tasks except failure-derived tasks require operator approval. The decay function raises the approval threshold as the run progresses. Tasks scoring below 2/5 on value are silently discarded with a log entry.

### Spec drift

**Observed frequency**: SWE-bench Pro analysis found that **35.9% of Claude Opus 4.1's failures were syntactically valid patches that completely missed the actual bug**. The agent had the skill but lost the target.

**Detection**: The 30-minute spec-completion check compares progress against the frozen spec. A drift detector compares the set of files being modified against the set expected from the plan. If >30% of modifications target files not in the plan, drift is flagged.

**Mitigation**: The frozen spec is never modified. Every task's acceptance criteria reference specific spec requirements by ID. The Verifier agent re-reads the relevant spec section before evaluating acceptance criteria. Periodic fresh starts (clearing agent context and re-loading from the spec) combat the reasoning drift that occurs over long sessions — Cursor found this essential for multi-day runs.

### Repeated invalid retries

**Observed frequency**: Agents often retry the exact same approach after failure, expecting different results.

**Detection**: The orchestrator compares the diff produced by each retry against previous attempts. If the diff is >90% similar to a previous failed attempt, it is flagged as a non-novel retry.

**Mitigation**: After a non-novel retry, the orchestrator forces a **strategy change**: either switch to a different model, provide additional context (e.g., relevant documentation or examples), or decompose the task into smaller pieces. The forced reflection prompt ("What failed? What specific change would fix it?") is mandatory before every retry. After 3 retries, the task is escalated regardless.

### Over-delegation

**Observed frequency**: The MAST study found coordination overhead plateaus beyond 4 agents, and multi-agent systems produce "politeness loops" where agents confirm and re-confirm rather than making progress.

**Detection**: The orchestrator monitors the ratio of coordination events (inter-agent messages, delegation calls) to productive events (file changes, test executions). A ratio above 3:1 indicates over-delegation.

**Mitigation**: Sub-agent depth is limited to 2 levels. The orchestrator caps concurrent agents at the configured pool size. Feature Lead sub-orchestrators are only spawned for plans with >15 tasks. For smaller plans, the root orchestrator manages all tasks directly. Cursor's finding is adopted: removing an "integrator" role that was supposed to coordinate quality control actually improved outcomes because it created more bottlenecks than it solved.

### Branch chaos

**Observed frequency**: Without structured workflow, agents create branches that accumulate without review or cleanup. One developer using agents submitted as many PRs in 3 days as they had in 3 years.

**Detection**: The orchestrator tracks all branches it creates and their states (active, merged, abandoned, stale).

**Mitigation**: Deterministic branch naming (`auto/<run-id>/<task-id>`) enables automated cleanup. Branches for accepted tasks are deleted after merge. Branches for failed tasks are preserved for 7 days then cleaned up. A hard limit of 20 active branches per run prevents accumulation. The orchestrator refuses to create new branches above this limit until existing ones are resolved.

### Hallucinated progress

**Observed frequency**: 12% of autonomous session failures in production (Crosley, 500+ sessions). An agent reports "all tests pass" without running the test runner.

**Detection**: The Validation Pipeline requires **actual tool output**, not agent claims. The orchestrator verifies that the test runner was actually executed by checking for the test command in the agent's tool call log and matching the reported results against the captured stdout. If the agent claims tests pass but the tool call log shows no test execution, the claim is rejected.

**Mitigation**: Evidence gates (Section 6) require observable artifacts for every acceptance criterion. The Verifier agent is explicitly instructed to flag hedging language. Agent claims without corresponding tool output are treated as failures, not partial successes. The system never trusts "I believe the tests would pass" — only "pytest output: 42 passed, 0 failed."

### False-positive validation

**Observed frequency**: University of Alberta research confirmed that LLM-generated tests frequently validate implemented behavior rather than specified behavior — they protect bugs rather than catch them.

**Detection**: Spec-completion checks catch cases where all tests pass but the spec is not satisfied. Coverage analysis identifies code paths that are tested but produce wrong results (high coverage, wrong behavior).

**Mitigation**: Test generation is decoupled from implementation — the Test Writer agent generates tests from the acceptance criteria and spec, not from the code. Tests are reviewed by the Reviewer agent before being trusted. The full pre-existing test suite (written by humans) serves as the ground truth; agent-generated tests supplement but never replace it. When the pre-existing test suite is thin, the operator is warned that validation confidence is lower.

---

## 9. Recommended execution architecture

### What to build in v1

**v1 is a single-machine orchestrator with worktree-isolated agents, PostgreSQL-backed durability, and a CLI/web dashboard.** It is ambitious enough to run overnight on a real codebase but simple enough to debug when things go wrong.

```
┌─────────────────────────────────────────────────────────────────┐
│                        OPERATOR (CLI / Web UI)                  │
│  Start run · Approve plan · Review PRs · Handle escalations     │
└──────────────────────────────┬──────────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────────┐
│                     ORCHESTRATOR PROCESS                        │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │Task Graph │  │Policy    │  │Scheduler │  │Event Logger   │  │
│  │(DAG)     │  │Engine    │  │& Queue   │  │(append-only)  │  │
│  └──────────┘  └──────────┘  └──────────┘  └───────────────┘  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │Checkpoint│  │Progress  │  │Drift     │  │Cost           │  │
│  │Manager   │  │Tracker   │  │Detector  │  │Accumulator    │  │
│  └──────────┘  └──────────┘  └──────────┘  └───────────────┘  │
└──────┬──────────────┬──────────────┬──────────────┬────────────┘
       │              │              │              │
  ┌────▼────┐   ┌─────▼────┐  ┌─────▼────┐  ┌─────▼────┐
  │Worker 1 │   │Worker 2  │  │Worker 3  │  │Validator │
  │(Coder)  │   │(Coder)   │  │(Tester)  │  │(det.)    │
  │Worktree │   │Worktree  │  │Worktree  │  │          │
  │   A     │   │   B      │  │   C      │  │          │
  └─────────┘   └──────────┘  └──────────┘  └──────────┘
       │              │              │
       ▼              ▼              ▼
  ┌─────────────────────────────────────┐
  │           Git Repository            │
  │  main ← staging ← agent branches   │
  └─────────────────────────────────────┘
       │
  ┌────▼──────────────────┐
  │   PostgreSQL           │
  │  Event log · Task state│
  │  Checkpoints · Metrics │
  └────────────────────────┘
```

**v1 components**:

- **Orchestrator process**: A single long-running process (Python or TypeScript) implementing the task graph, policy engine, scheduler, and checkpoint manager. Uses PostgreSQL for all durable state via an event-sourced schema (append-only event table + materialized task state view). Communicates with agents via subprocess spawning or HTTP calls to a local agent server.

- **Worker pool**: 3 concurrent workers (configurable), each running in its own git worktree. Workers are stateless agent invocations — the orchestrator spawns a worker process, passes the task payload, and collects the structured result. Workers use LLM APIs (Anthropic, OpenAI, or local models) with a model fallback chain.

- **Validation pipeline**: A deterministic pipeline that runs build, typecheck, lint, and test suite as shell commands in the agent's worktree. Results are captured as structured data. Optionally invokes a Reviewer agent for semantic review.

- **Policy engine**: Evaluates YAML-defined policies against every agent action. Deny-first evaluation. Command allowlists/denylists. File scope restrictions. Cost limits.

- **CLI/Web dashboard**: Start runs, approve plans, review escalations, monitor progress, inspect event logs, view cost tracking. The CLI is the primary interface; the web dashboard is a read-only view for monitoring.

- **Agent implementations**: v1 ships with three agent types: Planner (decomposes specs into task graphs), Coder (implements tasks), and Reviewer (validates results). Each is a prompt template + tool configuration, not a separate codebase. Agents use file read/write, shell execution, and search tools. Model-agnostic — any LLM API that supports tool use.

**v1 capabilities**: Accept a spec document, decompose into tasks, execute tasks in parallel with worktree isolation, validate with build/test/lint, retry on failure, escalate after 3 failures, generate failure-derived follow-up tasks, checkpoint all state to PostgreSQL, resume after crash, enforce cost and time limits, create PRs for review.

**v1 limitations**: Single-machine only (no distributed workers). No container/microVM isolation (worktrees only). No cloud execution. No Feature Lead sub-orchestrators. No Gap Analyzer. No improvement task generation. Limited to 5 concurrent agents.

### What to build in v2

- **Distributed workers**: Agent execution on remote machines or cloud sandboxes (Firecracker, E2B). Enables scaling beyond single-machine resource limits and provides stronger isolation.
- **Feature Lead sub-orchestrators**: Hierarchical delegation for large projects (>15 tasks). Each Feature Lead manages a sub-graph independently.
- **Gap Analyzer**: Post-plan spec-completion checking with autonomous gap-task generation (operator-approved).
- **Temporal/Restate integration**: Replace the custom PostgreSQL-based durability layer with a production durable execution framework for stronger guarantees, built-in retry policies, and better observability.
- **MCP tool integration**: Agents use Model Context Protocol for standardized tool access, enabling plug-and-play tool ecosystems.
- **Multi-repository support**: Orchestrate tasks across multiple repositories with cross-repo dependency tracking.

### What to defer to v3+

- **Self-improving agents**: Agents that create their own tools at runtime (as in Live-SWE-agent). High capability but high risk — requires robust containment.
- **Dynamic re-planning**: Mid-execution plan modification based on discovered complexity. v1–v2 use static plans with failure-driven amendments only.
- **Production deployment integration**: Agents that deploy to staging and run integration tests against live services. Requires significantly stronger safety controls.
- **Multi-model ensemble**: Running the same task on multiple models simultaneously and selecting the best result. Expensive but effective for high-stakes tasks.

### Open questions and decision points

1. **Orchestrator runtime**: Build custom on PostgreSQL (simpler, fewer dependencies) vs. adopt Temporal (proven at scale by Codex/Replit, but adds operational complexity)? **Recommendation**: Start with PostgreSQL in v1 for rapid iteration; migrate to Temporal in v2 when distributed execution is needed.

2. **Agent context management**: How much context to carry between tasks? Stateless agents (fresh context per task) are simpler and avoid drift, but lose valuable information. Persistent agents (carry context across tasks) are more efficient but accumulate stale context. **Recommendation**: Stateless with explicit context injection — the orchestrator maintains a "project memory" file (similar to `CLAUDE.md`) that is included in every agent prompt, updated after each accepted task.

3. **Test generation strategy**: Generate tests before implementation (TDD-style, higher spec fidelity) vs. after implementation (simpler, but risks validating bugs)? **Recommendation**: Generate acceptance tests before implementation from the spec, then generate edge-case tests after implementation. Both test sets must pass.

4. **Concurrency ceiling**: How many parallel agents before compound errors outweigh throughput? **Recommendation**: Default to 3, allow up to 8 with explicit operator opt-in. Monitor the validation-pass-rate-to-attempt-rate ratio and automatically reduce concurrency if it drops below 50%.

5. **Cost model**: Per-task cost limits vs. per-run cost limits vs. both? **Recommendation**: Both. Per-task limits (default: $2) prevent single tasks from consuming the budget. Per-run limits (operator-configured) cap total spend. Cost tracking is real-time and visible in the dashboard.

6. **When to stop generating follow-up work**: How does the system know the spec is "done enough"? **Recommendation**: The spec-completion check produces a percentage. When it reaches a configurable threshold (default: 90%) and all generated tasks are either accepted or explicitly deferred, the run completes. The remaining gaps are reported to the operator as a punch list.

7. **Trust bootstrapping**: How does a new operator learn to trust the system? **Recommendation**: Start at L1 (supervised) for the first run. After each successful run, suggest increasing to the next autonomy level. Track the ratio of operator overrides to auto-accepted results — if the operator approves >90% without changes over 3 runs, recommend upgrading.

---

## Conclusion

The architecture described here is not a theoretical exercise. Every pattern is drawn from production systems shipping in 2025–2026: Temporal's durable execution powering OpenAI Codex, Claude Code's checkpoint-and-rewind model, Cursor's worktree-isolated parallel agents, OpenHands' event-sourced state, and the hard-won lessons from incidents where agents deleted production databases and wiped home directories.

**Three design principles above all others determine whether this system works overnight or wastes the night.** First, the plan is static and frozen — agents execute against a stable target, not a moving one. Second, validation requires observable evidence, never agent confidence — "tests pass" means captured pytest output, not an LLM's belief. Third, every state transition is durable — a crash at 3 AM means losing at most one in-flight task, not eight hours of work.

The most important insight from the research is counterintuitive: **the constraint that makes the system most useful is not more autonomy but better stopping conditions.** An agent that runs all night and produces 50 accepted, validated, spec-compliant tasks is transformative. An agent that runs all night and produces 200 tasks of which 30 are useful and 170 require cleanup is worse than doing nothing. The task budget, the value scoring, the decay function, the spec-completion threshold — these are not safety features bolted on. They are the core product.

Build v1 with PostgreSQL, worktrees, 3 agents, and conservative defaults. Let it prove itself on real codebases at L2 autonomy. Then scale.