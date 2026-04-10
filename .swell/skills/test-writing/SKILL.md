---
name: test-writing
description: Write comprehensive tests for Rust code. Use when adding tests, improving test coverage, fixing failing tests, writing unit tests, integration tests, async tests with tokio::test, mocking external dependencies, or property-based testing. Keywords: test, testing, mock, async test, tokio::test, unit test, integration test, property-based, proptest.
---

# Test Writing Skill

## Test Types

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_functionality() {
        assert_eq!(add(2, 3), 5);
    }
}
```

### Async Tests
```rust
#[tokio::test]
async fn test_async_operation() {
    let result = fetch_data().await.unwrap();
    assert!(!result.is_empty());
}
```

### Integration Tests
```rust
// tests/integration_test.rs
use swell_core::prelude::*;

#[tokio::main]
async fn main() {
    // Test public API
}
```

## Mocking
```rust
#[derive(Default)]
struct MockClient {
    calls: RefCell<Vec<String>>,
}

impl ApiClient for MockClient {
    async fn fetch(&self, url: &str) -> Result<String> {
        self.calls.borrow_mut().push(url.to_string());
        Ok("mock response".to_string())
    }
}
```

## Best Practices
- Test behavior, not implementation
- Use descriptive test names: `test_<function>_<scenario>`
- Cover happy path and error cases
- Aim for meaningful assertions, not just `assert!(true)`
