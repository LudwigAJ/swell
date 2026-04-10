---
name: rust-coding
description: Write idiomatic Rust code following best practices. Use when implementing features, fixing bugs, or modifying Rust code. Covers ownership, lifetimes, error handling with anyhow/thiserror, async with Tokio, and workspace management.
---

# Rust Coding Skill

## Core Principles

1. **Ownership over garbage collection**: Never use Rc/RefCell unless necessary
2. **Error handling**: Use `Result<T, E>` for recoverable errors, `panic!` only for bugs
3. **Async/Await**: Use Tokio for async runtime, prefer `#[tokio::main]` or explicit runtime
4. **Traits for polymorphism**: Define traits in `swell-core`, implement in concrete crates

## Common Patterns

### Error Handling
```rust
use anyhow::{Context, Result};

fn example() -> Result<()> {
    let data = read_file("config.json")
        .context("Failed to read config")?;
    Ok(data)
}
```

### Async Functions
```rust
#[tokio::main]
async fn main() -> Result<()> {
    let result = do_work().await?;
    Ok(())
}
```

### Builder Pattern
```rust
impl Config {
    pub fn new() -> Self {
        Self { ..Default::default() }
    }
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}
```

## Testing
- Unit tests in `#[cfg(test)]` modules at bottom of source files
- Integration tests in `tests/` directory
- Use `#[tokio::test]` for async tests
- Mock external dependencies with trait objects

## Workspace Structure
- Core types in `swell-core`
- Domain-specific code in feature crates
- CLI client in `clients/swell-cli`
- Tests verify behavior, not implementation
