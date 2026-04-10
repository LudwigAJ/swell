---
name: code-review
description: Review Rust code for correctness, style, performance, and security. Use when reviewing PRs, performing code reviews, or pre-commit checks. Covers clippy lints, Rust idioms, common pitfalls, and security considerations.
---

# Code Review Skill

## Checklist

### Correctness
- [ ] Does the code do what the PR claims?
- [ ] Are edge cases handled?
- [ ] Are errors propagated correctly?
- [ ] Is there race condition potential?

### Rust Idioms
- [ ] Uses `Result` for error handling?
- [ ] Appropriate use of `Arc<RwLock<T>>` vs `Mutex<T>`?
- [ ] No unnecessary cloning?
- [ ] Uses iterators over loops where appropriate?

### Performance
- [ ] Avoids unnecessary allocations?
- [ ] Uses appropriate data structures?
- [ ] No blocking in async code?

### Security
- [ ] No hardcoded secrets?
- [ ] Input validation?
- [ ] No `unsafe` blocks (unless necessary)?
- [ ] Follows principle of least privilege?

## Common Clippy Warnings
- `let_Unit_value` - unnecessary assignment
- `redundant_field_names` - duplicate field names
- `unused_variables` - dead code
- `clone_on_copy` - prefer Copy if small

## Review Commands
```bash
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```
