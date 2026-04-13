# User Testing

Testing surface, tools, and validation configuration.

---

## Validation Surface

### Primary: Cargo Test Suite
- **Tool**: cargo commands (test, check, clippy)
- **Coverage**: Unit tests per crate (~2,397 existing), integration tests in tests/
- **Execution**: `cargo test --workspace -- --test-threads=4`
- **Lint**: `cargo clippy --workspace -- -D warnings`

### Secondary: Daemon IPC
- **Tool**: Build and run swell-daemon binary, connect via Unix socket
- **Coverage**: Task creation, state transitions, event streaming
- **Execution**: Start daemon on temp socket, send JSON commands, verify responses

## Validation Concurrency

### Cargo Test Surface
- **Max concurrent validators**: 5
- **Rationale**: 10-core / 16 GB machine. cargo test uses ~500 MB per invocation. 5 concurrent = ~2.5 GB, well within 70% of ~10 GB available headroom.

### Daemon IPC Surface
- **Max concurrent validators**: 3
- **Rationale**: Each daemon instance uses a unique temp socket and ~200 MB. Conservative limit due to filesystem I/O.

## Testing Patterns

- All LLM tests use MockLlm or ScenarioMockLlm (no real API calls)
- Integration tests use temporary directories for filesystem isolation
- Daemon tests use temporary Unix sockets in /tmp/
- Git tests may need temp git repos (initialized per test)
