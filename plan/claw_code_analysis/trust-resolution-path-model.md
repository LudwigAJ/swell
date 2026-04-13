# Trust Resolution Path Model

## What This Is

ClawCode uses a path-based trust resolution system to decide whether a worker automatically clears the "do you trust the files in this folder" prompt that Claude sends when entering a new working directory. The resolution logic lives in `trust_resolver.rs` and is wired into the worker boot state machine in `worker_boot.rs`. The system produces structured trust events that recovery recipes consume when a trust prompt goes unresolved.

## Why It Exists

When a worker boots inside a git worktree or project directory, Claude Desktop asks an interactive trust question before allowing file operations. In an autonomous multi-worker context, this question can block a fleet of workers waiting for a human to click "Yes." The trust resolver automates answers for known-safe paths so workers never stall on the prompt.

The design is also a safety boundary: paths explicitly denied are rejected outright rather than presented for approval, preventing accidental trust escalation for sensitive directories.

## Core Data Structures

### TrustPolicy

```rust
pub enum TrustPolicy {
    AutoTrust,       // answer yes without prompting
    RequireApproval, // pass through to human
    Deny,            // reject outright
}
```

Three outcomes, not two. The `RequireApproval` outcome still surfaces the trust prompt to a human; it just does not auto-answer. This distinction matters for observability: a worker that lands in an untracked path still shows `TrustRequired` in its status, making it easy to audit where manual intervention was needed.

### TrustEvent

```rust
pub enum TrustEvent {
    TrustRequired { cwd: String },
    TrustResolved { cwd: String, policy: TrustPolicy },
    TrustDenied { cwd: String, reason: String },
}
```

Every resolution emits a sequence of events. The sequence always opens with `TrustRequired`, then adds either `TrustResolved` or `TrustDenied` depending on the outcome. These events are the integration surface that lane-event consumers and recovery recipes consume downstream.

### TrustConfig

```rust
pub struct TrustConfig {
    allowlisted: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}
```

The caller (typically the runtime config layer) builds a `TrustConfig` by calling `.with_allowlisted(path)` and `.with_denied(path)` in any order. The resolver evaluates denied roots first, then allowlisted roots, then falls through to `RequireApproval`.

## Resolution Algorithm

`TrustResolver::resolve` evaluates three branches in order:

```
1. No trust prompt detected in screen_text
   → return TrustDecision::NotRequired

2. cwd matches a denied root
   → return TrustDecision::Required { policy: Deny, events: [TrustRequired, TrustDenied] }

3. cwd matches an allowlisted root
   → return TrustDecision::Required { policy: AutoTrust, events: [TrustRequired, TrustResolved] }

4. Neither match
   → return TrustDecision::Required { policy: RequireApproval, events: [TrustRequired] }
```

Denial takes precedence over allowlisting. A path that appears on both lists resolves to `Deny`. This is intentional: explicit denial is a stronger safety signal than explicit allowlisting.

### Path Matching

```rust
fn path_matches(candidate: &str, root: &Path) -> bool {
    let candidate = normalize_path(Path::new(candidate));
    let root = normalize_path(root);
    candidate == root || candidate.starts_with(&root)
}
```

Matching is prefix-based: `/tmp/worktrees` matches `/tmp/worktrees/repo-a` because the canonicalized candidate path starts with the canonicalized root. Symlinks are collapsed during canonicalization, so `/tmp/worktrees` and `/tmp/./worktrees` produce the same canonical path and match identically.

Sibling prefix confusion is guarded against. `/tmp/worktrees-other` does not match `/tmp/worktrees` because `worktrees-other` does not start with `worktrees/` (the trailing separator is required for prefix matching).

## Prompt Detection

```rust
const TRUST_PROMPT_CUES: &[&str] = &[
    "do you trust the files in this folder",
    "trust the files in this folder",
    "trust this folder",
    "allow and continue",
    "yes, proceed",
];
```

A simple case-insensitive substring scan over the worker's screen text. If none of the cues match, the resolver returns `NotRequired` without consulting the path config at all. This keeps the hot path fast for normal conversation turns.

## Integration with Worker Boot

`WorkerRegistry::observe` calls into the trust resolver when processing screen text from a booting worker. The flow in `worker_boot.rs` is:

```rust
if !worker.trust_gate_cleared && detect_trust_prompt(&lowered) {
    worker.status = WorkerStatus::TrustRequired;
    // ... record failure ...

    if worker.trust_auto_resolve {
        worker.trust_gate_cleared = true;
        worker.status = WorkerStatus::Spawning;
        // auto-progress: TrustRequired → Spawning
    }
}
```

`trust_auto_resolve` is set during worker creation based on whether the worker's `cwd` matches any `trusted_roots` from the runtime config. The resolver itself is consulted independently for explicit deny/allow decisions via `TrustResolver::resolve`.

The two surfaces are separate but complementary:

- `worker_boot.rs` `trust_auto_resolve` flag: a boolean gate pre-computed at worker creation time from config paths. Used for silent auto-advance when an allowlisted worker boots.
- `trust_resolver.rs` `TrustResolver::resolve`: a full decision tree consulted when a trust prompt actually appears in the worker's output, producing structured events.

`WorkerRegistry::resolve_trust` provides a manual resolution path for `RequireApproval` cases. This is the escape hatch for operators: when a worker lands in an untracked directory and a human approves the prompt, the registry records `ManualApproval` as the resolution and clears the trust gate.

## Trust Events in Recovery

`recovery_recipes.rs` defines `FailureScenario::TrustPromptUnresolved` as a first-class failure scenario with its own recipe:

```rust
FailureScenario::TrustPromptUnresolved => RecoveryRecipe {
    scenario: *scenario,
    steps: vec![RecoveryStep::AcceptTrustPrompt],
    max_attempts: 1,
    escalation_policy: EscalationPolicy::AlertHuman,
},
```

The recipe maps `WorkerFailureKind::TrustGate` to `TrustPromptUnresolved` via `FailureScenario::from_worker_failure_kind`. One automatic recovery attempt is made (accepting the prompt), and if it fails, the system escalates to a human. This ties the structured trust events from `trust_resolver.rs` directly into the recovery policy loop.

## Configuration Source

`trustedRoots` is loaded through the runtime config merge chain from `settings.json` and `settings.local.json`. The field is a flat string array of absolute paths. No wildcards, no glob patterns — only exact or prefix matches on canonicalized paths. See `config.rs` `parse_optional_trusted_roots` and the test `parses_trusted_roots_from_settings` for the loading sequence.

## Builder Lessons

### Deny-first ordering

Checking denied roots before allowlisted ones is a deliberate safety ordering. If a sensitive directory appears on both lists, denial wins. When designing any two-list permission system (allow/deny, grant/revoke, whitelist/blacklist), decide which list wins on conflict and enforce that order consistently.

### Events over booleans

Storing resolution outcomes as structured events rather than a single boolean enables richer observability: lane event consumers can distinguish auto-resolved from manually approved resolutions without peeking into opaque state. The `TrustEvent::TrustDenied { reason }` variant carries a human-readable reason string that explains exactly why a path was denied.

### Prompt detection is a separate concern

The prompt detection cue list is a string-based scan separate from the path resolution logic. This separation means adding a new Claude prompt variant (e.g., a new localized prompt) requires only updating `TRUST_PROMPT_CUES`, not the resolver itself. The two concerns are orthogonal and should remain so.

### Prefix matching with canonicalization

Using `Path::starts_with` after canonicalization handles symlink traversal correctly without requiring special symlink-handling logic. The canonicalization step is fallible — if `canonicalize` fails (e.g., path does not exist yet), it falls back to the raw path. Workers in non-existent directories (created just-in-time) still participate in trust resolution.

### Explicit over implicit for safety

`Deny` as an explicit policy, rather than treating "not on allowlist" as implicit denial, makes the safety boundary visible and auditable. An operator can inspect the trust decision log and see that a path was denied rather than simply never approved.

## Evidence Sources

- `references/claw-code/rust/crates/runtime/src/trust_resolver.rs` — core `TrustResolver`, `TrustConfig`, `TrustPolicy`, `TrustEvent`, and path-matching implementation with tests
- `references/claw-code/rust/crates/runtime/src/worker_boot.rs` — `trust_auto_resolve` flag, `WorkerStatus::TrustRequired`, `WorkerRegistry::resolve_trust`, and integration tests `allowlisted_trust_prompt_auto_resolves_then_reaches_ready_state`, `trust_prompt_blocks_non_allowlisted_worker_until_resolved`
- `references/claw-code/rust/crates/runtime/src/recovery_recipes.rs` — `FailureScenario::TrustPromptUnresolved` recipe and `RecoveryStep::AcceptTrustPrompt`
- `references/claw-code/rust/crates/runtime/src/config.rs` — `trustedRoots` field, `parse_optional_trusted_roots`, and config tests `parses_trusted_roots_from_settings`, `trusted_roots_default_is_empty_when_unset`
