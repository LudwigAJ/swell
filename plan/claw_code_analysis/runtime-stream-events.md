# Runtime Stream Events: An Event-Rich Design for Model Output

> **Evidence:** `references/claw-code/rust/crates/runtime/src/conversation.rs` (lines 26–57), `references/claw-code/rust/crates/runtime/src/usage.rs`

The ClawCode runtime is structured around a typed event stream emitted by the model client during a single assistant turn. Rather than collapsing the stream into a single structured response, the runtime processes individual events as they arrive, building up the full assistant message incrementally and feeding telemetry back into the session tracker. This design makes the turn loop explicit, testable, and observable at every phase.

## The AssistantEvent type

The `ApiClient` trait requires implementors to return a `Vec<AssistantEvent>` from a single `stream` call:

```rust
pub enum AssistantEvent {
    TextDelta(String),   // incremental text from the model
    ToolUse { id, name, input },  // a requested tool invocation
    Usage(TokenUsage),  // token consumption telemetry from the provider
    PromptCache(PromptCacheEvent),  // prompt-cache diagnostic event
    MessageStop,  // terminal marker indicating the stream ended cleanly
}
```

These five variants cover every behavioral class the runtime cares about. They are exhaustive — there is no escape hatch for additional event types without adding a new variant.

### TextDelta

A `TextDelta` carries a string fragment that concatenates with all preceding deltas to form the complete assistant text block. The runtime appends each delta to the running text buffer and flushes it as a `ContentBlock::Text` when the stream ends or when a different event type intervenes. This means streaming UI can display text as it arrives without waiting for `MessageStop`.

### ToolUse

A `ToolUse` event contains the tool identifier (`id`), the canonical tool name (`name`), and the serialized JSON input (`input`). The runtime extracts all pending tool uses from an assistant message by scanning for `ContentBlock::ToolUse` blocks after the stream resolves. A single turn can emit multiple `ToolUse` events before `MessageStop`, and the runtime handles each one in sequence.

### Usage

A `Usage` event carries a `TokenUsage` struct containing four distinct counters:

```rust
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}
```

These four dimensions are kept separate rather than being collapsed into a single token total. `input_tokens` + `output_tokens` track raw throughput; `cache_creation_input_tokens` and `cache_read_input_tokens` track prompt-cache efficiency. The runtime records each `Usage` event into the `UsageTracker`, which maintains both the latest-turn usage and the cumulative session totals.

### PromptCache

A `PromptCache` event surfaces diagnostic information when the provider detects anomalous prompt-cache behavior:

```rust
pub struct PromptCacheEvent {
    pub unexpected: bool,
    pub reason: String,
    pub previous_cache_read_input_tokens: u32,
    pub current_cache_read_input_tokens: u32,
    pub token_drop: u32,
}
```

`unexpected: true` signals that cache-read efficiency degraded in a way the runtime should know about. The `reason` field gives a human-readable explanation, and the before/after token counts document the magnitude. These events are accumulated in `TurnSummary::prompt_cache_events` and are available for logging, parity testing, or user-facing telemetry.

### MessageStop

`MessageStop` is the terminal event. It tells the runtime that the stream concluded normally. The `build_assistant_message` function treats a missing `MessageStop` as a runtime error:

```rust
if !finished {
    return Err(RuntimeError::new(
        "assistant stream ended without a message stop event",
    ));
}
```

This enforced marker keeps the contract between provider and runtime explicit. If a provider stream closes without sending `MessageStop`, the turn fails rather than silently returning a partial message.

## TurnSummary: the output of a turn

After the event stream is consumed and all tool calls are resolved, the runtime returns a `TurnSummary`:

```rust
pub struct TurnSummary {
    pub assistant_messages: Vec<ConversationMessage>,
    pub tool_results: Vec<ConversationMessage>,
    pub prompt_cache_events: Vec<PromptCacheEvent>,
    pub iterations: usize,
    pub usage: TokenUsage,
    pub auto_compaction: Option<AutoCompactionEvent>,
}
```

- `assistant_messages` — the assistant messages produced in this turn (there may be more than one when the loop iterates)
- `tool_results` — the tool-result messages written into the transcript as feedback to the model
- `prompt_cache_events` — all `PromptCache` events captured during the turn
- `iterations` — how many model→tool cycles ran in this turn
- `usage` — **cumulative** usage for the session, not just this turn (via `usage_tracker.cumulative_usage()`)
- `auto_compaction` — if the runtime triggered automatic compaction after this turn, the `removed_message_count` is recorded here

The `usage` field in `TurnSummary` is the cumulative total, not the per-turn count. Callers who need the current-turn usage specifically can read it from `usage_tracker.current_turn_usage()`.

## Why event richness matters

An event-typed stream rather than a single monolithic response gives the runtime several advantages:

1. **Observable loop**: Every phase of the turn is represented as a typed event. Tests can assert on exact event sequences (see the `ScriptedApiClient` in conversation.rs which validates event ordering in `runs_user_to_tool_to_result_loop_end_to_end_and_tracks_usage`).

2. **Incremental processing**: `TextDelta` allows streaming output without buffering the full response. UI layers can display text as it arrives.

3. **Explicit termination**: `MessageStop` makes missing terminal markers a hard failure rather than a silent partial. This keeps providercontract errors from propagating as corrupted session state.

4. **Telemetry without interception**: `Usage` and `PromptCache` events are first-class citizens in the event stream, not intercepted metadata that requires adapter code. The runtime routes them directly into `UsageTracker` and `prompt_cache_events`.

5. **Testable contract**: The `AssistantEvent` enum is a closed set. Adding a new event type requires a code change in the enum, forcing the developer to handle it in `build_assistant_message` rather than silently ignoring it.

## Builder lesson: typed events over raw stream proxies

A common mistake in AI runtime design is treating the model stream as an opaque byte pipe and reconstructing semantics in a separate layer. ClawCode's approach encodes every semantically meaningful phase as a distinct enum variant. This makes the runtime easier to test, easier to extend, and easier to observe.

When designing a similar loop, consider:
- Which events does your runtime need to act on vs. pass through?
- Can missing events be treated as errors rather than ignored?
- Are usage and telemetry surfaced as first-class events or hidden in metadata?

Typed events force these questions to be answered explicitly at the type level, rather than being deferred to runtime behavior.
