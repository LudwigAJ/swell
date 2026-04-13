# Structured Tool Results

Tool execution outcomes in ClawCode are not freeform strings dumped into a transcript. They are first-class structured payloads with explicit schema fields that flow through the runtime loop, get serialized into the session transcript, and are fed back to the model as typed input content blocks.

## Tool Result Serialization

The `ContentBlock` enum in `runtime/src/session.rs` defines three variants:

```rust
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: String },
    ToolResult { tool_use_id, tool_name, output, is_error },
}
```

`ConversationMessage::tool_result` (also in `runtime/src/session.rs`) constructs a tool result message:

```rust
pub fn tool_result(
    tool_use_id: impl Into<String>,
    tool_name: impl Into<String>,
    output: impl Into<String>,
    is_error: bool,
) -> Self {
    Self {
        role: MessageRole::Tool,
        blocks: vec![ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            tool_name: tool_name.into(),
            output: output.into(),
            is_error,
        }],
        usage: None,
    }
}
```

The `ContentBlock::to_json` method (in `runtime/src/session.rs`) serializes a `ToolResult` variant into a typed JSON object:

```rust
Self::ToolResult { tool_use_id, tool_name, output, is_error } => {
    object.insert("type".to_string(), JsonValue::String("tool_result".to_string()));
    object.insert("tool_use_id".to_string(), JsonValue::String(tool_use_id.clone()));
    object.insert("tool_name".to_string(), JsonValue::String(tool_name.clone()));
    object.insert("output".to_string(), JsonValue::String(output.clone()));
    object.insert("is_error".to_string(), JsonValue::Bool(*is_error));
}
```

This produces a JSON object like:

```json
{
  "type": "tool_result",
  "tool_use_id": "tool-1",
  "tool_name": "read_file",
  "output": "file contents here...",
  "is_error": false
}
```

When the session transcript is replayed to the model in subsequent turns, `InputContentBlock::ToolResult` (from `api/src/types.rs`) re-materializes these fields:

```rust
ContentBlock::ToolResult { tool_use_id, output, is_error, .. } =>
    InputContentBlock::ToolResult {
        tool_use_id: tool_use_id.clone(),
        content: vec![ToolResultContentBlock::Text { text: output.clone() }],
        is_error: *is_error,
    },
```

`ToolResultContentBlock` itself supports `Text` and `Json` variants, meaning tool outputs can carry structured data that the model can reason over:

```rust
pub enum ToolResultContentBlock {
    Text { text: String },
    Json { value: Value },
}
```

## Multi-Tool Same-Turn Execution

A single turn does not terminate after one tool call. The `run_turn` method in `runtime/src/conversation.rs` collects all pending tool uses from the assistant's response block and processes them iteratively before returning a final response.

The loop structure:

```rust
let pending_tool_uses = assistant_message
    .blocks
    .iter()
    .filter_map(|block| match block {
        ContentBlock::ToolUse { id, name, input } => Some((id.clone(), name.clone(), input.clone())),
        _ => None,
    })
    .collect::<Vec<_>>();

if pending_tool_uses.is_empty() {
    break;
}

for (tool_use_id, tool_name, input) in pending_tool_uses {
    // execute tool, produce ConversationMessage::tool_result
    // push result to session
}
```

Each tool result is appended directly to the session transcript inside the loop. By the time the turn completes, the transcript contains an assistant message with all tool-use blocks followed by one tool-result message per tool call. The next API request to the model includes the full updated transcript, enabling the model to synthesize a final response that reasons over all tool outputs from the same turn.

The `TurnSummary` struct exposes this cleanly:

```rust
pub struct TurnSummary {
    pub assistant_messages: Vec<ConversationMessage>,
    pub tool_results: Vec<ConversationMessage>,  // all results from the turn
    pub iterations: usize,
    pub usage: TokenUsage,
    pub auto_compaction: Option<AutoCompactionEvent>,
}
```

## Parity Evidence for Multi-Tool Turns

The `multi_tool_turn_roundtrip` scenario in the mock parity harness (`rusty-claude-cli/tests/mock_parity_harness.rs`) validates this behavior end-to-end:

```rust
ScenarioCase {
    name: "multi_tool_turn_roundtrip",
    permission_mode: "read-only",
    allowed_tools: Some("read_file,grep_search"),
    prepare: prepare_multi_tool_fixture,
    assert: assert_multi_tool_turn_roundtrip,
    // ...
}
```

The assertion verifies two tool uses and two tool results in a single turn:

```rust
fn assert_multi_tool_turn_roundtrip(_: &HarnessWorkspace, run: &ScenarioRun) {
    assert_eq!(run.response["iterations"], Value::from(2));
    let tool_uses = run.response["tool_uses"].as_array().expect("tool uses array");
    assert_eq!(tool_uses.len(), 2, "expected two tool uses in a single turn");
    assert_eq!(tool_uses[0]["name"], Value::String("read_file".to_string()));
    assert_eq!(tool_uses[1]["name"], Value::String("grep_search".to_string()));
    let tool_results = run.response["tool_results"].as_array().expect("tool results array");
    assert_eq!(tool_results.len(), 2, "expected two tool results in a single turn");
    // ...
}
```

This scenario appears in the milestone 2 coverage list in `references/claw-code/PARITY.md`:

> - [x] Scripted multi-tool turn coverage: `multi_tool_turn_roundtrip`

## Builder Lessons

**Structured payloads over string dumping.** Tool results are not opaque blobs. Every result carries `tool_use_id` (to correlate with the originating call), `tool_name`, `output`, and `is_error`. This makes it possible for the model to reason about which tool produced which output, and for the runtime to replay sessions faithfully.

**Transcript-mediated feedback.** Tool results are inserted into the conversation transcript, not returned as a side channel. The next model request sees the updated transcript with all results already in context. This eliminates the need for out-of-band result passing and keeps the runtime loop stateless with respect to tool execution.

**Batch execution within a turn.** Processing all pending tools before returning to the model lets the model synthesize a coordinated response over multiple tool results. The alternative (one turn per tool) would require the model to produce an additional assistant message after each result, inflating token usage and slowing response time.

**Separation of execution and serialization.** `tool_executor.execute` produces a raw string output. The runtime wraps it into a `ConversationMessage::tool_result` with typed fields before pushing to the transcript. The serialization concern is isolated to a single conversion point, so tool implementations stay simple and the runtime controls the structured output contract.
