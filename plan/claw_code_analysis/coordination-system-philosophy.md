# Coordination System Philosophy

## The Code Is Evidence; The Coordination System Is the Lesson

`PHILOSOPHY.md` is explicit about the repo's core framing:

> "The code is evidence. The coordination system is the product lesson." — [`PHILOSOPHY.md` lines 83–84](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/PHILOSOPHY.md)

The Python rewrite was a byproduct. The Rust rewrite was also a byproduct. The real artifact worth studying is the **system that produced them**: a coordination loop where humans give direction and autonomous claws perform the labor. The generated files are traces of that system running — not the thing itself.

This distinction matters for builders because:

- **End-user documentation** describes what the CLI does for a human sitting at a terminal.
- **Builder-oriented engineering documentation** describes the coordination architecture underneath — the state machines, event flows, trust gates, and recovery recipes that make the output possible.
- **This repo** treats the Rust implementation as the canonical active surface (`rust/`), with `src/` and `tests/` as secondary reference surfaces — `references/claw-code/README.md` makes this hierarchy explicit.

The coordination-system framing means that understanding `WorkerStatus`, `LaneEvent`, `RecoveryRecipe`, or `TrustResolver` is more valuable to a builder than memorizing CLI flags. The mechanisms *are* the design lesson.

## Human Role: Direction-Setting, Not Micromanagement

`PHILOSOPHY.md` names the human interface as Discord — not a terminal:

> "The real human interface is a Discord channel. A person can type a sentence from a phone, walk away, sleep, or do something else. The claws read the directive, break it into tasks, assign roles, write code, run tests, argue over failures, recover, and push when the work passes." — [`PHILOSOPHY.md` lines 17–20](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/PHILOSOPHY.md)

The philosophy statement is direct:

> "humans set direction; claws perform the labor." — [`PHILOSOPHY.md` line 25](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/PHILOSOPHY.md)

This framing is architecturally encoded in the system in several concrete ways:

1. **Task decomposition is automated** — `TaskRegistry` (`references/claw-code/rust/crates/runtime/src/task_registry.rs`) tracks task lifecycle, not a human manually routing outputs.
2. **Trust resolution is path-based** — `TrustResolver` (`references/claw-code/rust/crates/runtime/src/trust_resolver.rs`) produces `AutoTrust`, `RequireApproval`, or `Deny` outcomes without human intervention for known paths.
3. **Lane events are typed** — `LaneEvent` (`references/claw-code/rust/crates/runtime/src/lane_events.rs`) exposes coordination state as structured data, not as scraped tmux output.
4. **Recovery recipes act before escalation** — `RecoveryRecipe` (`references/claw-code/rust/crates/runtime/src/recovery_recipes.rs`) attempts one automatic recovery step before surfacing a failure to the human.

The human's durable contributions are **direction, judgment, decomposition, and taste** — not terminal micromanagement of every tool call. `PHILOSOPHY.md` articulates this as:

> "The bottleneck is no longer typing speed. When agent systems can rebuild a codebase in hours, the scarce resource becomes: architectural clarity, task decomposition, judgment, taste, conviction about what is worth building."

## The Three-Part Coordination System

`PHILOSOPHY.md` identifies three layers that compose the coordination system:

### 1. OmX (oh-my-codex) — Workflow Layer

[oh-my-codex](https://github.com/Yeachan-Heo/oh-my-codex) turns short directives into structured execution: planning keywords, execution modes, persistent verification loops, and parallel multi-agent workflows. This is the layer that converts a sentence into a repeatable work protocol.

### 2. clawhip — Event Router

[clawhip](https://github.com/Yeachan-Heo/clawhip) watches git commits, tmux sessions, GitHub issues and PRs, agent lifecycle events, and channel delivery. Its job is keeping monitoring and notification routing **outside** the coding agent's context window so agents stay focused on implementation.

### 3. OmO (oh-my-openagent) — Multi-Agent Coordination

[oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent) handles planning, handoffs, disagreement resolution, and verification loops across agents. When Architect, Executor, and Reviewer disagree, OmO provides the structure for that loop to converge instead of collapse.

## What Changes: The Bottleneck Shift

`PHILOSOPHY.md` describes the bottleneck shift as the core strategic insight:

> "A fast agent team does not remove the need for thinking. It makes clear thinking even more valuable." — [`PHILOSOPHY.md` lines 42–43](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/PHILOSOPHY.md)

In this framing, the scarce resource is no longer typing speed. What matters more:

- **Architectural clarity** — what the system should look like
- **Task decomposition** — how to split work for parallel execution
- **Judgment** — which approach is right for the problem at hand
- **Taste** — what is worth building at all
- **Operational stability** — knowing what can fail and building recovery for it

## Clawable Harness Principles Inform the Implementation

The ROADMAP.md defines what "clawable" means in concrete engineering terms. These principles shape how the coordination system is built:

1. **State machine first** — every worker has explicit lifecycle states (`Spawning`, `TrustRequired`, `ReadyForPrompt`, `Running`, `Finished`, `Failed` per `WorkerStatus` in `references/claw-code/rust/crates/runtime/src/worker_boot.rs`).
2. **Events over scraped prose** — channel output is derived from typed `LaneEvent` schemas rather than tmux pane scraping.
3. **Recovery before escalation** — `RecoveryRecipe` attempts one automatic recovery before surfacing the failure to a human.
4. **Terminal is transport, not truth** — orchestration state lives above terminal/tmux implementation details; `.claw/worker-state.json` and `claw state` are the shipped observability surfaces.

These principles are visible in the Rust implementation: `WorkerStatus` is a finite state machine, `LaneEvent` is a typed schema, and `RecoveryRecipe` encodes named failure scenarios with one auto-recovery attempt before escalation.

## Source-of-Truth Hierarchy

`references/claw-code/README.md` establishes the canonical source hierarchy:

> "`rust/` — canonical Rust workspace and the `claw` CLI binary" as the primary surface, with `src/` and `tests/` as secondary reference surfaces that should stay consistent with generated guidance but are not the primary runtime.

This matters for the coordination philosophy because the *coordination mechanisms* (`WorkerStatus`, `TaskRegistry`, `LaneEvent`, `TrustResolver`, `RecoveryRecipe`) live in `rust/crates/runtime/src/`. These are the concrete surfaces builders should trace their lessons from.

## Builder Lessons

A builder takingaway from this repo should understand:

1. **The CLI is a delivery mechanism, not the lesson.** Claw Code demonstrates that a repository can be autonomously built in public, coordinated by agents rather than human pair-programming alone, and operated through a chat interface. The `claw` binary is one possible client for that coordination system.

2. **Coordination state should be explicit and typed.** `WorkerStatus` state machine and `LaneEvent` typed schemas make coordination observable and auditable in a way that scraped tmux output cannot.

3. **Recovery recipes encode failure intelligence.** Rather than surfacing every failure to a human immediately, the system attempts one structured auto-recovery step. This is what makes "humans set direction" scalable.

4. **Trust is path-based and explicit.** `TrustResolver` produces named outcomes (`AutoTrust`, `RequireApproval`, `Deny`) rather than implicit or binary-only authorization. This makes the human's trust-setting role concrete and reviewable.

5. **The coordination loop is the product, not the output files.** `PHILOSOPHY.md` frames this directly: the code is evidence, the coordination system is the product lesson. A builder building a similar system should study `rust/crates/runtime/src/` — not just the CLI help text.

## References

- [`PHILOSOPHY.md`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/PHILOSOPHY.md) — primary philosophy framing
- [`references/claw-code/README.md`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/README.md) — source-of-truth hierarchy
- [`references/claw-code/rust/README.md`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/README.md) — Rust workspace canonical surface
- [`references/claw-code/ROADMAP.md`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/ROADMAP.md) — clawable harness principles (lines 63–68)
- [`references/claw-code/rust/crates/runtime/src/worker_boot.rs`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/crates/runtime/src/worker_boot.rs) — WorkerStatus state machine
- [`references/claw-code/rust/crates/runtime/src/lane_events.rs`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/crates/runtime/src/lane_events.rs) — typed lane event schemas
- [`references/claw-code/rust/crates/runtime/src/recovery_recipes.rs`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/crates/runtime/src/recovery_recipes.rs) — recovery recipe encoding
- [`references/claw-code/rust/crates/runtime/src/trust_resolver.rs`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/crates/runtime/src/trust_resolver.rs) — path-based trust resolution
- [`references/claw-code/rust/crates/runtime/src/task_registry.rs`](file:///Users/ludwigjonsson/Projects/claw_code_learned/references/claw-code/rust/crates/runtime/src/task_registry.rs) — task lifecycle tracking
