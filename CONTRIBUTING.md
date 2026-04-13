# Contributing to SWELL

Thank you for contributing to SWELL! This guide covers our workflow for pull requests, commit conventions, and code review process.

## PR Workflow

### Branch Naming

All branches must follow this naming convention:

```
<type>/<short-description>
```

**Types:**
- `feat/` - New features
- `fix/` - Bug fixes
- `docs/` - Documentation changes
- `refactor/` - Code refactoring (no functional change)
- `test/` - Adding or updating tests
- `chore/` - Build system, dependencies, tooling
- `scrutiny/` - Validation/scrutiny rounds
- `user-testing/` - User testing rounds

**Examples:**
```bash
feat/user-testing-auth
fix/memory-leak-in-sqlite
docs/update-readme
refactor/llm-streaming
test/add-mcp-integration-tests
```

### Pull Request Process

1. **Create Branch**: Create a feature branch from `main`
   ```bash
   git checkout main
   git pull origin main
   git checkout -b feat/my-feature
   ```

2. **Make Changes**: Implement your changes following the commit conventions below

3. **Run Validation**: Before submitting, run crate-scoped validation:
   ```bash
   cargo check -p <crate>
   cargo test -p <crate> -- --test-threads=4
   cargo clippy -p <crate> -- -D warnings
   ```

4. **Submit PR**: Push your branch and create a pull request against `main`
   ```bash
   git push origin feat/my-feature
   ```

5. **PR Description**: Include:
   - Summary of changes
   - Reference to relevant spec document (e.g., `plan/research_documents/Autonomous Coding Engine.md`)
   - Testing performed
   - Any breaking changes

### Review Requirements

- **All PRs require at least one approval** before merge
- **CI must pass** (check, test, clippy)
- **No merge conflicts** with target branch
- **Test coverage should not decrease** unless explicitly justified

### Merge Strategy

We use **squash merge** for all PRs:
- All commits in a PR are squashed into a single commit on `main`
- The squash commit message follows conventional commit format (see below)
- This keeps `git log` clean and attributable

## Commit Conventions

We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification.

### Format

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | A new feature |
| `fix` | A bug fix |
| `docs` | Documentation only changes |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `test` | Adding missing tests or correcting existing tests |
| `chore` | Changes to build process, dependencies, or tooling |
| `perf` | Performance improvements |
| `ci` | CI configuration changes |

### Scope (Optional)

The scope indicates which crate or area is affected:

```
feat(swell-llm): add SSE streaming support
fix(swell-tools): correct path canonicalization
docs(swell-core): update error handling docs
```

### Examples

```bash
# Feature
feat(swell-orchestrator): add turn summary capture

# Bug fix
fix(swell-memory): prevent sqlite connection leak

# Documentation
docs(readme): add troubleshooting section
docs(contributing): add PR workflow guide

# Refactor
refactor(swell-validation): simplify gate evaluation

# Test
test(swell-llm): add streaming parser tests

# Chore
chore: update rust toolchain to 1.94
```

### Rules

1. **Subject line** must be ≤ 72 characters
2. **Subject line** uses imperative mood ("add" not "added" or "adds")
3. **Subject line** does not end with a period
4. **Body** explains *what* and *why*, not *how*
5. **Footer** references issues: `Closes #123` or `Refs #456`

## Code Review Process

### For Authors

1. **Self-review first**: Review your own diff before requesting review
2. **Keep PRs focused**: One feature/fix per PR
3. **Respond to feedback**: Address all comments before re-requesting review
4. **Don't force-push** after review has started (creates review context loss)

### For Reviewers

1. **Be constructive**: Explain *why* changes are needed
2. **Be timely**: Review within 24 hours of request
3. **Be specific**: Suggest concrete improvements
4. **Approve when ready**: Don't block on nits; use "Request changes" sparingly

### Review Checklist

#### Correctness
- [ ] Code does what the description says
- [ ] Edge cases are handled
- [ ] No logic errors or bugs introduced
- [ ] Error handling is appropriate

#### Testing
- [ ] Tests cover the new functionality
- [ ] Tests pass (`cargo test -p <crate>`)
- [ ] Test coverage did not decrease
- [ ] Edge cases have test coverage

#### Code Quality
- [ ] Clippy passes (`cargo clippy -p <crate> -- -D warnings`)
- [ ] Code is formatted (`cargo fmt --all`)
- [ ] No unnecessary clones or allocations
- [ ] Variables/functions are appropriately named
- [ ] Comments explain *why*, not *what*

#### Integration
- [ ] Changes follow crate boundaries correctly
- [ ] Public API changes are documented
- [ ] Dependencies are appropriate (no circular deps)
- [ ] Feature flags used if applicable

#### Security (for external integrations)
- [ ] No hardcoded secrets or API keys
- [ ] Input validation on external data
- [ ] Permission checks are appropriate

## Crate-Scoped vs Workspace Validation

### When to Use Crate-Scoped Validation

For changes affecting a single crate:

```bash
cargo check -p <crate>
cargo build -p <crate>
cargo test -p <crate> -- --test-threads=4
cargo clippy -p <crate> -- -D warnings
```

### When to Use Workspace-Wide Validation

For:
- Cross-crate changes
- Dependency updates
- Final release gates
- Changes to workspace configuration

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Commit Message Format for Squash Merges

When squash merging, use this format for the final commit:

```
<type>(<scope>): <description>

<detailed explanation of changes>

Refs: #[issue-number]
```

**Example:**
```
feat(swell-llm): add prompt caching headers to AnthropicBackend

Add anthropic-beta: prompt-caching-2024-07-31 header and cache_control
blocks on system messages to enable prompt caching. This reduces
token costs for repeated system prompts.

Refs: #42
```

## Getting Help

- **Questions**: Open a Discussion on GitHub
- **Bugs**: Open an Issue with bug label
- **Features**: Open an Issue with feature request label
- **Security**: See [SECURITY.md](SECURITY.md) for vulnerability reporting
