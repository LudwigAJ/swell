# Environment

Environment variables, external dependencies, and setup notes.

**What belongs here:** Required env vars, external API keys/services, dependency quirks, platform-specific notes.
**What does NOT belong here:** Service ports/commands (use `.factory/services.yaml`).

---

## Required Environment

- **Rust**: 1.94+ (edition 2021)
- **SQLite**: Embedded via sqlx (no external setup)
- **Git**: System git CLI (used by swell-tools for shell-based git operations)

## Optional Environment Variables

- `ANTHROPIC_API_KEY` - Required for real Anthropic LLM backend (tests use MockLlm)
- `OPENAI_API_KEY` - Required for real OpenAI LLM backend (tests use MockLlm)
- `VOYAGE_API_KEY` - Required for real Voyage embedding client (tests use mock HTTP)
- `MINIMAX_API_KEY` - Required for live integration tests against MiniMax API (tests gated with `#[ignore]`)

## Live Integration Testing (MiniMax)

MiniMax provides OpenAI-compatible (`https://api.minimax.io/v1`) and Anthropic-compatible (`https://api.minimax.io/anthropic`) endpoints. Set `MINIMAX_API_KEY` env var to run live integration tests with model `MiniMax-M2.7`. See `.factory/library/minimax-integration.md` for full details.

**SECURITY: The API key MUST be loaded from env var only. NEVER hardcode it or read from files in committed code. The key file at `plan/minimax-docs/minimax-api-key.md` is gitignored.**

## Key Dependencies

- `tokio` - Async runtime (multi-threaded)
- `sqlx` - SQLite database access (compile-time checked queries)
- `reqwest` - HTTP client for LLM backends
- `serde` / `serde_json` - Serialization
- `uuid` - Unique identifiers
- `thiserror` - Error types in library crates
- `anyhow` - Error handling in binary crates

## Platform Notes

- Unix sockets used for daemon IPC (Linux/macOS only)
- File operations are async via tokio
- All git operations shell out to `git` CLI (no git2/gix library)
