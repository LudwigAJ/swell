---
name: docs-worker
description: Documentation writer for README, AGENTS.md, CONTRIBUTING.md, and per-crate docs. Writes accurate, comprehensive documentation for both human and AI agent audiences.
---

# Documentation Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Features that create or update documentation files (.md files). This includes README.md, AGENTS.md (root and per-crate), CONTRIBUTING.md, and any other markdown documentation.

## Required Skills

None.

## Work Procedure

1. **Read the feature description** to understand exactly what documentation is needed.
2. **Investigate the codebase** to gather accurate information:
   - Read relevant source files to understand module structure, public API, key types
   - Check Cargo.toml for dependencies
   - Run `cargo test -p <crate>` to understand test patterns
   - Read existing docs to maintain consistency
3. **Write documentation** following these rules:
   - Be accurate — every claim must be verifiable from the code
   - Be specific — include actual type names, function signatures, module paths
   - Be actionable — a reader should be able to build, test, and use the code from your docs
   - Follow the existing doc style (check root AGENTS.md and README.md for conventions)
   - For per-crate AGENTS.md: include responsibility, public API, testing guide, patterns, integration points
4. **Verify documentation accuracy**:
   - Every build/test command mentioned actually works (run it)
   - Every file path mentioned exists
   - Every type/function name mentioned exists in the code
   - Cross-references between docs are consistent
5. **Run validation**:
   - `cargo check --workspace` (ensure no compile errors introduced)
   - If doc tests exist, run them

## Example Handoff

```json
{
  "salientSummary": "Created per-crate AGENTS.md for swell-llm covering responsibility (LLM backends), public API (AnthropicBackend, OpenAIBackend, ModelRouter, MockLlm), testing guide (cargo test -p swell-llm, uses MockLlm), patterns (reqwest HTTP, serde for API types), and integration points (swell-core::LlmBackend trait, swell-orchestrator agents).",
  "whatWasImplemented": "Created crates/swell-llm/AGENTS.md with 5 required sections: responsibility, public API surface listing all exported types, testing guide with example commands, patterns (HTTP client, streaming, error handling), and integration points with swell-core and swell-orchestrator.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      { "command": "cargo check --workspace", "exitCode": 0, "observation": "All crates compile" },
      { "command": "ls crates/swell-llm/AGENTS.md", "exitCode": 0, "observation": "File exists, 2.1KB" }
    ],
    "interactiveChecks": []
  },
  "tests": { "added": [] },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- Source code contradicts existing documentation (which is authoritative?)
- Module structure is unclear and crate has no tests to verify behavior
- Feature asks for documentation of functionality that doesn't exist yet
