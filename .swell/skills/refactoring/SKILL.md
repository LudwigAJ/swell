---
name: refactoring
description: Refactor existing Rust code while preserving behavior. Use when improving code structure, extracting functions, reducing duplication, introducing builders, or applying Martin Fowler patterns. Strangler fig for migrations. Keywords: refactor, extract, restructure, builder pattern, Martin Fowler, migration.
---

# Refactoring Skill

## Core Principles

1. **Preserve behavior**: Tests must pass before and after
2. **Small steps**: Make one change at a time
3. **Test-driven**: Write failing test first, then refactor

## Common Refactorings

### Extract Function
```rust
// Before
fn process(data: &Data) -> Result<Output> {
    let validated = validate(data)?;
    let transformed = transform(validated)?;
    let output = finalize(transformed)?;
    Ok(output)
}

// After
fn process(data: &Data) -> Result<Output> {
    let validated = validate(data)?;
    process_internal(validated)
}
```

### Introduce Builder
```rust
// Before
let config = Config {
    timeout: 100,
    retries: 3,
    debug: false,
};

// After
let config = Config::builder()
    .with_timeout(100)
    .with_retries(3)
    .build();
```

### Replace Magic Numbers
```rust
const MAX_RETRIES: u32 = 3;
const DEFAULT_TIMEOUT_MS: u64 = 5000;
```

## Strangler Fig Pattern
For large migrations:
1. Create thin wrapper around old code
2. Implement new code alongside
3. Gradually shift traffic to new code
4. Remove old code once stable

## Verification
```bash
cargo test --workspace
cargo clippy --workspace
git diff --stat  # Should show minimal changes
```
