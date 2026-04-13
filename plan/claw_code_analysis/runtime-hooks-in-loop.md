# Runtime Hooks in the Turn Loop

Hook placement inside the turn loop is where runtime control flow gets its most expressive bend: pre-tool hooks can inspect and reshape tool input before permission is evaluated, and post-tool hooks can append structured feedback into the transcript. This document describes how `HookRunner` and `ConversationRuntime` cooperate to make hooks a first-class part of turn execution, not a separate extension surface.

## Hook Events and Execution Points

Hooks fire at three points inside a tool-use iteration in `conversation.rs`:

- **`PreToolUse`** — fires before the permission check, before the tool executes. The hook receives `HOOK_TOOL_NAME`, `HOOK_TOOL_INPUT`, and the full JSON payload on stdin.
- **`PostToolUse`** — fires after a successful tool execution, receiving `HOOK_TOOL_OUTPUT` in addition to the standard inputs.
- **`PostToolUseFailure`** — fires when the tool executor returns an error, receiving `HOOK_TOOL_ERROR` instead of output.

Each event maps to a `HookEvent` enum variant in `hooks.rs`. The `HookRunner::run_pre_tool_use_with_context()` family of methods drive these calls. When no hook commands are configured, `HookRunResult::allow()` is returned immediately with an empty messages vector, so the runtime avoids any overhead.

## HookRunResult as a Control Vector

A hook returns a `HookRunResult` carrying four independent signals:

| Field | Effect on the loop |
|---|---|
| `denied` | Short-circuits to `PermissionOutcome::Deny` without invoking the permission policy |
| `failed` | Same as `denied` — the tool is treated as denied with the hook's failure message |
| `cancelled` | Same as `denied` — the loop treats it as a hard abort on this tool |
| `updated_input` | Replaces `effective_input` before permission evaluation and tool dispatch |
| `permission_override` | One of `PermissionOverride::Allow`, `Deny`, or `Ask` that is passed into `PermissionContext` |
| `permission_reason` | Human-readable reason attached to the override, surfaced in denial messages |
| `messages` | Text appended to the tool-result output via `merge_hook_feedback()` |

This design means a single pre-tool hook can both **inspect** the invocation (via stdin payload) and **reshape** it (via `updated_input`) or **block** it outright (via deny/fail/cancel), without the runtime needing any special-case logic for individual tools.

## Hook Influence on Permission and Tool Flow

The permission evaluation path in `ConversationRuntime::run_turn()` shows exactly how hooks participate:

```rust
// Pre hook runs first
let pre_hook_result = self.run_pre_tool_use_hook(&tool_name, &input);

// Hook can rewrite the input
let effective_input = pre_hook_result
    .updated_input()
    .map_or_else(|| input.clone(), ToOwned::to_owned);

// Hook can set permission override and reason
let permission_context = PermissionContext::new(
    pre_hook_result.permission_override(),
    pre_hook_result.permission_reason().map(ToOwned::to_owned),
);

// Hook denial short-circuits permission evaluation
let permission_outcome = if pre_hook_result.is_cancelled() || pre_hook_result.is_failed() || pre_hook_result.is_denied() {
    PermissionOutcome::Deny { reason: format_hook_message(&pre_hook_result, ...) }
} else {
    self.permission_policy.authorize_with_context(&tool_name, &effective_input, &permission_context, ...)
};
```

`authorize_with_context()` in `permissions.rs` evaluates `PermissionOverride` before it evaluates any static mode or rule:

```rust
match context.override_decision() {
    Some(PermissionOverride::Deny) => return PermissionOutcome::Deny { reason: ... },
    Some(PermissionOverride::Ask) => return Self::prompt_or_deny(...),
    Some(PermissionOverride::Allow) => {
        // still respects ask_rules, then allows
    }
    None => { /* normal policy evaluation */ }
}
```

So a hook can force a deny even when the active mode would allow it, or force an interactive prompt even when the tool's required mode is met. Hook guidance takes precedence over the static policy.

When the tool executes, post-tool hooks receive the actual output and their feedback is merged into the tool-result message that goes back into the transcript:

```rust
let (mut output, mut is_error) = match self.tool_executor.execute(&tool_name, &effective_input) { ... };

// Post hook runs on both success and error paths
let post_hook_result = if is_error {
    self.run_post_tool_use_failure_hook(&tool_name, &effective_input, &output)
} else {
    self.run_post_tool_use_hook(&tool_name, &effective_input, &output, false)
};

// Hook feedback appended to the output
output = merge_hook_feedback(post_hook_result.messages(), output, ...);

ConversationMessage::tool_result(tool_use_id, tool_name, output, is_error)
```

`merge_hook_feedback()` (in `conversation.rs`) appends hook messages as a labeled section below the tool's natural output, so the transcript preserves both the tool result and any hook commentary for the next model turn.

## Abort Signal and Progress Reporting

`HookRunner::run_pre_tool_use_with_context()` accepts an optional `HookAbortSignal` and `dyn HookProgressReporter`. If the abort signal is set during a long-running hook, the hook subprocess is killed and `HookRunResult::is_cancelled()` returns true, causing the tool to be denied rather than left in a zombie state. The reporter surfaces `HookProgressEvent::Started`, `HookProgressEvent::Completed`, and `HookProgressEvent::Cancelled` events so callers (like the runtime or UI layer) can observe hook lifecycle without introspecting hook internals.

## Hook Configuration via RuntimeFeatureConfig

`HookRunner::from_feature_config()` constructs itself from `RuntimeFeatureConfig::hooks()`, which holds three separate command vectors — `pre_tool_use`, `post_tool_use`, and `post_tool_use_failure` — defined in `config.rs` as `RuntimeHookConfig`. This configuration is part of the merged runtime config, so hook commands can be scoped to a project or machine via the config precedence chain.

## Builder Lesson: Hooks as a Mediation Layer, Not a Side Channel

The key architectural insight is that hooks in this runtime are not fire-and-forget notifications. They sit **between** the model's decision to use a tool and the tool's actual execution, and they produce a structured result that the runtime reads and acts on. The `updated_input`, `permission_override`, and `messages` fields are all first-class signals that the loop interprets — not just stdout strings that a hypothetical listener might log.

This makes hook behavior deterministic and testable: the runtime's hook-related test cases in `conversation.rs` (e.g., `denies_tool_use_when_pre_tool_hook_blocks`, `denies_tool_use_when_pre_tool_hook_fails`, `appends_post_tool_hook_feedback_to_tool_result`) verify the loop's response to every meaningful hook outcome. A builder implementing a similar pattern should ensure that hook results carry typed fields for every behavior the runtime needs to branch on, rather than encoding everything in exit codes or stdout prose.
