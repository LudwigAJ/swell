# Swell Audit — 2026-04-13

This directory contains a gap analysis comparing the current Swell implementation against:

1. The original design documents in `plan/Autonomous Coding Engine.md` and `plan/research_documents/`
2. Lessons learned from an architectural analysis of a competing agentic coding tool ("Claw Code"), documented in `plan/claw_code_analysis/`

## Methodology

Three parallel sub-agents read all source material:
- **Plan agent**: read all 7 plan/research documents in full
- **Claw Code agent**: read all 53 claw_code_analysis documents
- **Implementation agent**: surveyed the actual Rust codebase to assess what is real vs. stubbed

## Overall Assessment

Swell has strong architectural abstractions — the type system, memory layer, validation framework design, and orchestration state machine are solid. However, the **critical execution path is entirely mocked**: agent reasoning, real LLM calls, tool invocations, git operations, and sandbox enforcement are all stubs or not wired together. No code can actually be written, committed, or validated by a running Swell instance today.

The Claw Code analysis reveals additional design patterns around runtime reliability, permission semantics, and operational transparency that Swell currently lacks.

## Audit Documents

| File | Topic | Source |
|------|-------|--------|
| [01_critical_execution_path.md](01_critical_execution_path.md) | Wiring the real agent→tool→code→PR pipeline | Plan + Implementation survey |
| [02_git_integration.md](02_git_integration.md) | Actual git operations: branches, worktrees, commits, PRs | Plan (Phase 0-1) |
| [03_runtime_turn_loop_and_hooks.md](03_runtime_turn_loop_and_hooks.md) | Structured turn loop, pre/post-tool hooks, post-turn compaction | Claw Code analysis |
| [04_permission_system.md](04_permission_system.md) | Five permission modes, per-tool specs, three rule layers, bash classification | Claw Code analysis |
| [05_session_config_cli.md](05_session_config_cli.md) | Workspace-scoped sessions, five-layer config precedence, CLI slash commands | Claw Code analysis |
| [06_mcp_plugins_lsp.md](06_mcp_plugins_lsp.md) | MCP degraded startup, plugin lifecycle state machine, LSP registry | Claw Code analysis + Plan |
| [07_llm_providers_and_routing.md](07_llm_providers_and_routing.md) | Real LLM backends, credential shapes, model routing, cost optimization | Plan + Claw Code |
| [08_observability.md](08_observability.md) | Four-dimensional usage tracking, prompt cache events, config audit trail | Claw Code analysis + Plan |
| [09_operator_surfaces_ux.md](09_operator_surfaces_ux.md) | VS Code extension, dashboard views, three interaction modes | Plan (Phase 1-3) |
| [10_memory_storage_backends.md](10_memory_storage_backends.md) | LanceDB, Voyage embeddings, tiered storage, graph DB backends | Plan (research docs) |

## Audit Documents — Second Pass (2026-04-13)

A second pass covered the remaining claw code analysis files and deeper sections of the research docs.

| File | Topic | Source |
|------|-------|--------|
| [11_tool_system_and_skills.md](11_tool_system_and_skills.md) | Tool registry layering, normalization, structured results, skills dispatch | Claw Code analysis |
| [12_task_lifecycle_and_coordination.md](12_task_lifecycle_and_coordination.md) | Task spec validation, team/cron registry, worker boot lifecycle, typed events, recovery recipes | Claw Code analysis |
| [13_prompt_architecture_and_testing.md](13_prompt_architecture_and_testing.md) | SystemPromptBuilder, instruction file discovery, scenario harness, mutation testing | Claw Code + Plan |
| [14_orchestrator_gaps_and_trust.md](14_orchestrator_gaps_and_trust.md) | Trust resolution, config merge semantics, session autosave, knowledge graph impl, ACP/IPC daemon | Plan research docs |

## Priority Order

The documents are roughly ordered by impact:

1. **01** and **02** are blockers — nothing useful can happen without a real execution path and git.
2. **07** unblocks **01** — real LLM calls are needed before agent reasoning can work.
3. **03** and **04** harden the runtime once execution works.
4. **11** and **13** — tool dispatch correctness and prompt architecture are execution-quality blockers.
5. **12** and **14** — task lifecycle, trust, config, and session persistence are reliability blockers.
6. **05**, **06**, **08** improve operational quality and debuggability.
7. **09** and **10** expand the product surface and knowledge depth.
