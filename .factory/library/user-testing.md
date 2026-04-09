# User Testing Guide

## Validation Surface

The SWELL MVP exposes the following surfaces for testing:

### CLI Commands (Terminal)

All testing is done via CLI commands in a terminal. No browser required.

**Setup:**
1. Start daemon: `cargo run --bin swell-daemon` (in one terminal)
2. Run CLI: `cargo run --bin swell <command>` (in another terminal)

**Test Commands:**
```bash
# Create a task
swell task "implement hello world function"

# List all tasks
swell list

# Watch task progress
swell watch <task-id>

# Approve task plan
swell approve <task-id>

# Cancel task
swell cancel <task-id>
```

### Expected Behavior

- Task creation returns a UUID
- `swell list` shows tasks with their states
- `swell watch` streams progress updates
- Daemon responds to SIGTERM cleanly

## Validation Concurrency

For MVP, sequential testing is sufficient. No parallel validators needed.

- Single CLI process at a time
- Single daemon instance
- SQLite database for state (single writer)

## Resource Cost

- **Memory**: ~100MB for daemon, ~50MB per CLI invocation
- **CPU**: Minimal during idle, spikes during LLM calls
- **Disk**: SQLite database grows with task history (~1MB per 100 tasks)

## Manual Testing Checklist

1. **Build & Test**
   - [ ] `cargo build --workspace` succeeds
   - [ ] `cargo test --workspace` passes

2. **Daemon Lifecycle**
   - [ ] Start daemon: `cargo run --bin swell-daemon`
   - [ ] Stop daemon: `pkill swell-daemon`
   - [ ] Daemon handles SIGTERM gracefully

3. **CLI Commands**
   - [ ] `swell task "test"` creates task
   - [ ] `swell list` shows created task
   - [ ] `swell watch <id>` shows progress
   - [ ] `swell approve <id>` approves task
   - [ ] `swell cancel <id>` cancels task

4. **Full Pipeline** (if LLM configured)
   - [ ] Create real task
   - [ ] Watch planning execute
   - [ ] Watch generation execute
   - [ ] Watch validation execute
   - [ ] See final result

## Environment Variables

- `ANTHROPIC_API_KEY` - Required for real LLM calls
- `SWELL_SOCKET` - Unix socket path (default: `/tmp/swell-daemon.sock`)

## Known Issues

### Daemon Socket Reading Bug (CRITICAL)

**File:** `crates/swell-daemon/src/server.rs`, line 54

**Problem:** The buffer initialization `let mut buf = vec![0u8; 4096];` creates a Vec with 4096 bytes already used. Tokio's `read_buf()` appends to this buffer, but cannot because `length == capacity` (buffer appears "full"). The daemon reads 0 bytes and fails to parse an empty string as JSON, returning: `"Invalid command: expected value at line 1 column 1"`.

**Impact:** ALL CLI commands fail because the daemon cannot read any socket data.

**Quick Fix:** Change `let mut buf = vec![0u8; 4096];` to `let mut buf = Vec::with_capacity(4096);`

**Verification:** Even sending valid JSON like `{"type":"TaskList"}` from a Python socket client returns the same error, confirming the issue is in the daemon's read implementation.
