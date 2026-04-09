# Rust Worker Skill

## Overview

This skill defines how worker agents implement features for the SWELL Rust codebase.

## Procedure

1. **Understand the feature**
   - Read the feature description in `features.json`
   - Check the validation assertions in `validation-contract.md`
   - Read relevant existing code to understand patterns

2. **Implement the feature**
   - Follow Rust conventions (async_trait, thiserror, tracing)
   - Use `#[tokio::test]` for async tests
   - Import from `swell_core` using `../swell-core`

3. **Test the implementation**
   - Run `cargo test -p <crate>` for the affected crate
   - Run `cargo clippy --workspace` to catch issues
   - Fix any warnings/errors

4. **Verify against contract**
   - Check that your implementation fulfills the assertion IDs
   - Ensure no breaking changes to other features

## Handoff Format

When completing, return:

```
Feature ID: <id>
Status: completed
Tests: <test results>
Fulfills: <assertion IDs>
Notes: <any issues or observations>
```
