# Runtime Turn Loop

## What It Is

The runtime turn loop is the beating heart of ClawCode's conversation engine. It is the control flow that drives a single user turn from raw input through zero or more model→tool→model iterations and finally to a structured turn output. The loop lives in `ConversationRuntime::run_turn()` in `references/claw-code/rust/crates/runtime/src/conversation.rs` and is owned by the runtime crate.

## Runtime Ownership

The runtime crate (`rust/crates/runtime/src/lib.rs`) owns the core conversation loop and session persistence, not just general orchestration. Its public surface exports include:

- `ConversationRuntime`
- `Session`
- `TurnSummary`
- `ApiRequest` / `AssistantEvent`
- Usage tracking, permission policy, and prompt assembly

This ownership statement matters because it clarifies that the loop is not an accidental byproduct of CLI wiring — it is a first-class abstraction with explicit inputs and outputs.

## Iterative Model→Tool→Model Execution

A turn does **not** always produce one final response. The loop in `run_turn()` is explicitly iterative:

```rust
loop {
    iterations += 1;
    if iterations > self.max_iterations {
        return Err(RuntimeError::new("conversation loop exceeded the maximum number of iterations"));
    }

    let request = ApiRequest { system_prompt, messages };
    let events = self.api_client.stream(request)?;
    let (assistant_message, usage, cache_events) = build_assistant_message(events)?;

    self.session.push_message(assistant_message.clone())?;
    let pending_tool_uses = /* extract tool_use blocks */;

    if pending_tool_uses.is_empty() {
        break;  // done — no more tools to run
    }

    for (tool_use_id, tool_name, input) in pending_tool_uses {
        // permission check, execute, record result in transcript
        let result_message = ConversationMessage::tool_result(tool_use_id, tool_name, output, is_error);
        self.session.push_message(result_message.clone())?;
    }
}
```

The loop repeats until the model produces no `ToolUse` blocks. A single turn can therefore execute:

1. Model generates tool call(s)
2. Runtime executes each tool (with permission enforcement)
3. Tool results are appended to the session transcript
4. Loop repeats — model sees results as the next input

## Request Assembly

Outbound requests are assembled into `ApiRequest`:

```rust
pub struct ApiRequest {
    pub system_prompt: Vec<String>,    // from SystemPromptBuilder
    pub messages: Vec<ConversationMessage>,  // full session transcript
}
```

At each iteration the request is rebuilt from the current session state, meaning the model always sees the complete transcript including any tool results from prior iterations within the same turn.

## Tool Results as Transcript Messages

Tool execution outcomes are **not** hidden side effects. They are written into the session as `ConversationMessage::tool_result(...)` and immediately fed back into the transcript for the next loop iteration:

```rust
let result_message = ConversationMessage::tool_result(tool_use_id, tool_name, output, is_error);
self.session.push_message(result_message.clone())?;
tool_results.push(result_message);
```

This transcript-mediated feedback path is why the model can chain tools — each result is a first-class message in the conversation history.

## Iteration Safeguard

The loop has a hard stop via `max_iterations`:

```rust
if iterations > self.max_iterations {
    return Err(RuntimeError::new(
        "conversation loop exceeded the maximum number of iterations",
    ));
}
```

This guard prevents runaway loops from consuming unbounded API calls. The default `max_iterations` is `usize::MAX` (essentially infinite), but callers can set a finite cap via `with_max_iterations()`.

## Turn Summary Output

When the loop terminates normally, `run_turn()` returns a `TurnSummary`:

```rust
pub struct TurnSummary {
    pub assistant_messages: Vec<ConversationMessage>,  // model responses this turn
    pub tool_results: Vec<ConversationMessage>,      // all tool results this turn
    pub prompt_cache_events: Vec<PromptCacheEvent>,  // cache telemetry
    pub iterations: usize,                            // loop iterations this turn
    pub usage: TokenUsage,                            // cumulative usage at turn end
    pub auto_compaction: Option<AutoCompactionEvent>,  // compaction applied after turn
}
```

The summary gives callers everything they need: what the model said, what tools ran, how many iterations, cumulative token usage, and whether post-turn compaction fired.

## Hooks in the Loop

Pre-tool and post-tool hooks participate directly in turn execution:

- `run_pre_tool_use_hook()` — runs before each tool, can modify input, cancel, or deny
- `run_post_tool_use_hook()` — runs after a successful tool, can append feedback to output
- `run_post_tool_use_failure_hook()` — runs when a tool returns an error

Hook results are merged into the tool result message that gets written to the transcript, so hooks can influence downstream model behavior within the same turn.

## Compaction After the Turn

After the loop exits, `maybe_auto_compact()` is called:

```rust
fn maybe_auto_compact(&mut self) -> Option<AutoCompactionEvent> {
    if self.usage_tracker.cumulative_usage().input_tokens
        < self.auto_compaction_input_tokens_threshold
    {
        return None;
    }
    // ... compact session if threshold crossed
}
```

If cumulative input tokens exceed `auto_compaction_input_tokens_threshold` (default 100,000, configurable via `CLAUDE_CODE_AUTO_COMPACT_INPUT_TOKENS`), the session is compacted and an `AutoCompactionEvent` is returned in the summary.

## Session Health Probe After Compaction

A freshly compacted session runs a health probe before the next turn begins:

```rust
fn run_session_health_probe(&mut self) -> Result<(), String> {
    if self.session.messages.is_empty() && self.session.compaction.is_some() {
        return Ok(());  // empty compacted session — normal
    }
    let probe_input = r#"{"pattern": "*.health-check-probe-"}"#;
    match self.tool_executor.execute("glob_search", probe_input) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Tool executor probe failed: {e}")),
    }
}
```

If this probe fails, the turn returns an error rather than proceeding with a potentially broken session.

## Builder Lessons

1. **Transcript is the loop invariant.** Every tool result is written back into the messages list before the next iteration. The model always sees a consistent, growing transcript.

2. **Iteration guards are cheap and necessary.** A simple `iterations > max` check prevents runaway loops from burning API budget or looping forever on a broken tool.

3. **Turn summaries are richer than one response.** A single turn can produce multiple assistant messages, multiple tool results, cache events, and still return one coherent summary.

4. **Hooks belong in the loop, not outside it.** Running hooks inside the loop (pre/post tool) and merging their output into the transcript gives hooks real leverage over subsequent model behavior.

5. **Compaction is a post-iteration concern.** Triggering compaction after the loop (rather than inside it) keeps the loop logic clean and separates concerns.

## Evidence

| Claim | Source |
|-------|--------|
| Runtime crate owns conversation loop | `references/claw-code/rust/crates/runtime/src/lib.rs` — module docstring |
| Iterative model→tool→model loop | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `run_turn()` loop |
| Max-iteration safeguard | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `run_turn()` |
| ApiRequest assembly | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `ApiRequest` struct |
| Tool results as transcript messages | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `push_message(result_message)` |
| TurnSummary structure | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `TurnSummary` struct |
| Session health probe | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `run_session_health_probe()` |
| Hooks in loop | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `run_pre/post_tool_use_hook()` |
| Post-turn auto-compaction | `references/claw-code/rust/crates/runtime/src/conversation.rs` — `maybe_auto_compact()` |
