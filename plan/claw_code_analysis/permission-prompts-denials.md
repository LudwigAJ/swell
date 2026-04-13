# Permission Prompts and Denials

This document explains how ClawCode's runtime constructs approval request payloads, distinguishes between ask-rule triggering and mode-escalation prompting, and surfaces denied operations as transcript-level tool-result failures rather than silent drops.

## Approval Request Payloads

When the permission policy decides a tool invocation needs interactive approval, it constructs a [`PermissionRequest`](references/claw-code/rust/crates/runtime/src/permissions.rs) — a structured payload that carries everything the prompter UI needs to render a meaningful choice:

```rust
pub struct PermissionRequest {
    pub tool_name: String,
    pub input: String,
    pub current_mode: PermissionMode,
    pub required_mode: PermissionMode,
    pub reason: Option<String>,
}
```

- **tool_name**: Which tool is requesting execution (e.g., `"bash"`, `"write_file"`).
- **input**: The JSON input blob the tool would receive. The policy inspects this to extract a human-readable subject (e.g., the `command` field from a bash tool call) for the prompt UI.
- **current_mode**: The session's active permission mode at the moment of the call.
- **required_mode**: The minimum mode the tool declares as required.
- **reason**: An optional human-readable explanation for why approval is needed. This is populated differently depending on what triggered the prompt (see below).

The `PermissionPrompter` trait defines the decision interface:

```rust
pub trait PermissionPrompter {
    fn decide(&mut self, request: &PermissionRequest) -> PermissionPromptDecision;
}

pub enum PermissionPromptDecision {
    Allow,
    Deny { reason: String },
}
```

A `None` prompter (no interactive flow available) causes the policy to immediately return a denial rather than silently allowing the operation — prompting is never bypassed when a prompter is absent and the policy requires one.

## Ask-Rule Versus Mode-Escalation Triggering

There are two structurally distinct paths that lead to a permission prompt. The runtime does not collapse them into a single "needs approval" signal because each path carries different semantics about _why_ prompting is happening.

### Ask-Rule Triggering

Ask rules are explicit policy directives configured at the session or project level. They are declared in `RuntimePermissionRuleConfig` as structured strings like `bash(git:*)` or `bash(rm -rf:*)` and parsed into [`PermissionRule`](references/claw-code/rust/crates/runtime/src/permissions.rs) objects that match tool name plus an optional input subject pattern.

When an ask rule matches, the policy calls `prompt_or_deny` and populates `reason` with:

```
tool '{tool_name}' requires approval due to ask rule '{rule.raw}'
```

The ask rule fires **regardless of the active mode** — a session running in `DangerFullAccess` still prompts if an ask rule matches, because the rule expresses an explicit policy override that outranks the current mode. This is confirmed in the test `ask_rules_force_prompt_even_when_mode_allows`.

### Mode-Escalation Triggering

Mode-escalation prompting fires when the active mode is insufficient for the tool's declared requirement and no allow-rule or higher-mode clause covers it. The canonical case is a session in `WorkspaceWrite` that attempts a tool requiring `DangerFullAccess`:

```rust
if current_mode == PermissionMode::Prompt
    || (current_mode == PermissionMode::WorkspaceWrite
        && required_mode == PermissionMode::DangerFullAccess)
{
    let reason = Some(format!(
        "tool '{tool_name}' requires approval to escalate from {} to {}",
        current_mode.as_str(),
        required_mode.as_str()
    ));
    return Self::prompt_or_deny(...);
}
```

Here the `reason` explains the current-to-required mode transition rather than citing a named rule.

### Hook Override Short-Circuit

Hooks introduce a third path that can force prompting even when the static policy would allow. A `PreToolUse` hook that returns `PermissionOverride::Ask` bypasses the normal mode/rule evaluation and routes directly to `prompt_or_deny` with a reason derived from the hook's own guidance. Similarly, `PermissionOverride::Deny` short-circuits immediately without any prompting.

## Denied Operations as Transcript-Level Tool-Result Failures

A denied tool invocation does not silently disappear from the conversation. Instead, it is recorded in the session transcript as a tool-result message with `is_error: true`. This matters for several reasons:

1. **Visibility**: The agent sees the denial on the next iteration and can reason about what happened rather than assuming the tool succeeded or was never called.
2. **Auditability**: Every tool invocation — allowed or denied — appears in the session transcript, making it possible to reconstruct what the session attempted.
3. **Loop integrity**: The turn loop continues to completion even when individual tools are denied. A denied tool produces a result, the loop checks for remaining pending tool uses, and the turn concludes normally with a final assistant message.

The denial path in `run_turn` is explicit:

```rust
PermissionOutcome::Deny { reason } => ConversationMessage::tool_result(
    tool_use_id,
    tool_name,
    merge_hook_feedback(pre_hook_result.messages(), reason, true),
    true,  // is_error = true
),
```

The `is_error: true` flag marks the message as a failure in the transcript schema. This is verified in the test `records_denied_tool_results_when_prompt_rejects`, which asserts that a denied tool produces a `ToolResult` block where `output == "not now"` and `is_error == true`.

## Parity Harness Evidence

The [`mock_parity_harness.rs`](references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs) scenarios `bash_permission_prompt_approved` and `bash_permission_prompt_denied` exercise the full prompting flow end-to-end:

- `bash_permission_prompt_denied` asserts that denied prompts produce `tool_results[0].is_error == true` and that `tool_results[0].output` contains `"denied by user approval prompt"`.
- The runtime's iteration count remains 2 in both approved and denied cases, confirming the turn completes normally after the denial.

The `write_file_denied` scenario exercises permission denial without prompting (read-only mode attempts write_file) and similarly asserts transcript-level error output and a 2-iteration turn.

## Builder Lessons

- **Prompt payloads should be rich enough to support a meaningful human decision.** Including both current and required mode plus a structured reason is deliberate — it lets the prompter explain _what would change_ and _why_ without requiring the UI to reverse-engineer the policy state.
- **Distinguishing trigger paths matters for UX and for diagnostics.** If a builder collapses ask-rule and mode-escalation prompting, the denial reason shown to users loses the information about which policy layer caused the prompt. ClawCode keeps these paths separate so that "requires approval due to ask rule" and "requires approval to escalate from workspace-write to danger-full-access" are distinct messages.
- **Denied operations are not exceptional in the loop — they are normal transcript entries.** The design decision to surface denied tools as error tool-results rather than dropping them or surfacing them out-of-band keeps the conversation transcript coherent and makes it possible for the agent to observe and reason about every tool invocation, not just the successful ones. A turn can have zero, one, or many denied tool results and still produce a valid final assistant message.
