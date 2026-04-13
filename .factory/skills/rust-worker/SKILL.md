---
name: rust-worker
description: Rust implementation worker for new features, refactors, wiring, and tests. Uses TDD (red-green) and verifies through cargo test/check/clippy.
---

# Rust Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Features that involve writing or modifying Rust code: new types, traits, functions, wiring existing components together, refactoring, adding tests, fixing bugs.

## Required Skills

Invoke the `Rust` skill (via Skill tool) at the start of your session for idiomatic Rust patterns and conventions.

## Work Procedure

1. **Read the feature description** carefully. Note expectedBehavior and verificationSteps.
2. **Investigate the existing code**:
   - Read the relevant crate's source files
   - Understand the existing types, traits, and patterns
   - Check AGENTS.md (root and per-crate if it exists) for conventions
   - Read `.factory/library/architecture.md` for system overview
   - Read `.factory/library/minimax-integration.md` if working on LLM backends (for live test patterns)
3. **Write failing tests FIRST (Red)**:
   - Add test cases that verify the expectedBehavior
   - Run `cargo test -p <crate> -- --test-threads=4` and confirm tests FAIL
   - Tests should cover happy path, error cases, and edge cases from the feature description
4. **Implement the feature (Green)**:
   - Write the minimum code to make tests pass
   - Follow existing patterns in the crate (check how similar things are done)
   - Match the crate's error handling style (thiserror for library crates, anyhow for binaries)
   - Never hardcode secrets or API keys — always use environment variables
5. **Run full crate validation**:
   - `cargo test -p <crate> -- --test-threads=4` — all tests pass
   - `cargo clippy -p <crate> -- -D warnings` — zero warnings
   - `cargo check -p <crate>` — compiles clean
6. **Check for regressions**:
   - If the change touches swell-core or other foundational crates, run downstream crate tests too
   - Run `cargo build --workspace` to ensure no cross-crate breakage
7. **Manual verification** where applicable:
   - For daemon changes: verify the daemon binary still builds (`cargo build -p swell-daemon`)
   - For CLI changes: verify the CLI binary still builds (`cargo build --bin swell`)

**SECURITY RULES for LLM/API features:**
- NEVER hardcode API keys in source code or test code
- Always load credentials from environment variables (std::env::var)
- Gate live API tests with #[ignore] so they don't run in CI
- The MiniMax API key is in MINIMAX_API_KEY env var — never read from plan/minimax-docs/

## Example Handoff

```json
{
  "salientSummary": "Implemented SSE streaming for AnthropicBackend with content_block_delta, message_delta, and message_stop event parsing. Added 6 tests covering text streaming, tool_use accumulation, and error handling. All tests pass, clippy clean.",
  "whatWasImplemented": "Added stream() method to AnthropicBackend that parses SSE events via reqwest's bytes_stream(). Implemented SseParser that handles content_block_start, content_block_delta (text and input_json_delta), message_delta (usage + stop_reason), and message_stop events. Tool use JSON is accumulated across multiple input_json_delta events and parsed on content_block_stop.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      { "command": "cargo test -p swell-llm -- --test-threads=4", "exitCode": 0, "observation": "26 tests passed (6 new)" },
      { "command": "cargo clippy -p swell-llm -- -D warnings", "exitCode": 0, "observation": "No warnings" },
      { "command": "cargo build --workspace", "exitCode": 0, "observation": "All crates compile" }
    ],
    "interactiveChecks": []
  },
  "tests": {
    "added": [
      {
        "file": "crates/swell-llm/src/anthropic.rs",
        "cases": [
          { "name": "test_stream_text_deltas", "verifies": "SSE text content_block_delta events are accumulated into complete text" },
          { "name": "test_stream_tool_use_accumulation", "verifies": "Fragmented tool_use JSON across 3 input_json_delta events reconstructs complete tool call" },
          { "name": "test_stream_message_stop", "verifies": "Stream terminates cleanly on message_stop event" },
          { "name": "test_stream_usage_extraction", "verifies": "Token usage (input, output, cache_creation, cache_read) extracted from message_delta" },
          { "name": "test_stream_error_event", "verifies": "Server error events produce SwellError" },
          { "name": "test_stream_empty_response", "verifies": "Empty stream produces appropriate error" }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- Feature depends on types/traits that don't exist yet in another crate
- Requirements are ambiguous — multiple valid implementations possible
- Existing tests break in ways unrelated to the current feature
- Cross-crate changes needed that exceed the feature's scope
- Build/test infrastructure is broken (cargo test failures in unrelated code)
