# Compaction Trigger, Tail Preservation, and Tool-Pair Boundary

## What This Document Covers

This document explains how ClawCode's runtime triggers session compaction, which messages are preserved in the recent tail, and how the compactor avoids splitting assistant/tool pairs at the preservation boundary. It is scoped to the trigger mechanics and preservation strategy — for synthetic continuation messages and summary shape, see `analysis/compaction-resume-packet.md`.

**Evidence sources:**
- `references/claw-code/rust/crates/runtime/src/compact.rs` — compaction logic
- `references/claw-code/rust/crates/runtime/src/conversation.rs` — auto-compaction in the turn loop

---

## The Compaction Trigger

Compaction is a runtime response to token-budget pressure. When the conversation transcript grows large enough that continuing would risk exceeding the model's context window, the runtime summarizes older messages into a compact marker and preserves only the most recent turns verbatim.

The mechanism lives in two layers:

### Layer 1 — Per-turn `should_compact()` check

`compact.rs` defines `CompactionConfig`:

```rust
pub struct CompactionConfig {
    pub preserve_recent_messages: usize,  // default: 4
    pub max_estimated_tokens: usize,       // default: 10_000
}
```

`should_compact(session, config)` returns `true` when:
1. The session has more than `preserve_recent_messages` compactable messages (excluding any already-compacted summary prefix), **and**
2. The estimated token count of the compactable region exceeds `max_estimated_tokens`.

```rust
pub fn should_compact(session: &Session, config: CompactionConfig) -> bool {
    let start = compacted_summary_prefix_len(session);
    let compactable = &session.messages[start..];

    compactable.len() > config.preserve_recent_messages
        && compactable
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>()
            >= config.max_estimated_tokens
}
```

Token estimation is rough but fast — each message block contributes `(text_or_input_length / 4) + 1` tokens. This avoids a live `count_tokens` round-trip on every turn.

### Layer 2 — Automatic post-turn compaction

`conversation.rs` calls `maybe_auto_compact()` at the end of each turn:

```rust
fn maybe_auto_compact(&mut self) -> Option<AutoCompactionEvent> {
    if self.usage_tracker.cumulative_usage().input_tokens
        < self.auto_compaction_input_tokens_threshold
    {
        return None;
    }
    // ... compact with max_estimated_tokens=0 to let preserve_recent_messages drive
}
```

The default threshold is **100,000 cumulative input tokens** (`DEFAULT_AUTO_COMPACTION_INPUT_TOKENS_THRESHOLD`). It is configurable via the `CLAUDE_CODE_AUTO_COMPACT_INPUT_TOKENS` environment variable — a higher value delays compaction, a lower value triggers it earlier.

`TurnSummary::auto_compaction` carries the `AutoCompactionEvent { removed_message_count }` so callers (and tests) can observe that a turn resulted in compaction.

---

## Preserved Tail Strategy

When `compact_session()` is called, it produces a `CompactionResult` with:

- A **synthetic system message** containing a formatted summary of the removed history
- The **recent tail** preserved verbatim as the first N messages after the system message

The number of tail messages preserved is `config.preserve_recent_messages` — default 4. These recent messages are not summarized; they survive intact so the agent retains context of what just happened.

The trade-off is deliberate: summarizing preserves breadth (everything that happened), while the tail preserves recency (exactly what was said). Together they give the model enough historical grounding to continue coherently without re-reading everything.

### What "recent" means in practice

The `preserve_recent_messages` count is measured from the **end** of the message list. If compaction removes 12 messages and `preserve_recent_messages` is 4, the last 4 messages survive unchanged and the model sees them directly in the restored session.

The synthetic system message that replaces the removed block says:

```
This session is being continued from a previous conversation that ran out of
context. The summary below covers the earlier portion of the conversation.

[formatted summary here]

Recent messages are preserved verbatim.

Continue the conversation from where it left off without asking the user any
further questions. Resume directly — do not acknowledge the summary, do not
recap what was happening, and do not preface with continuation text.
```

This is the "resume directly" instruction that tells the model not to recap.

---

## Tool-Pair Boundary Preservation

The most subtle part of the compaction boundary logic is the guard against splitting an assistant `ToolUse` and its corresponding `ToolResult` across the preservation cut.

### Why it matters

On the OpenAI-compatible API path, a `tool` role message must be preceded by an assistant message that contains `tool_calls`. If the compaction boundary falls between a `ToolUse` assistant turn and its `ToolResult` turn, the preserved message sequence starts with an orphaned `ToolResult` — no preceding `tool_calls` — and the provider rejects it with a 400 error.

The comment in the source describes this precisely:

> If the first preserved message is a user message whose first block is a ToolResult, the assistant message with the matching ToolUse was slated for removal — that produces an orphaned tool role message on the OpenAI-compat path (400: tool message must follow assistant with tool_calls).

### The guard

`compact_session()` walks the boundary back when needed:

```rust
let keep_from = {
    let mut k = raw_keep_from;
    loop {
        if k == 0 || k <= compacted_prefix_len {
            break;
        }
        let first_preserved = &session.messages[k];
        let starts_with_tool_result = first_preserved
            .blocks
            .first()
            .is_some_and(|b| matches!(b, ContentBlock::ToolResult { .. }));
        if !starts_with_tool_result {
            break;
        }
        // Check the message just before the current boundary.
        let preceding = &session.messages[k - 1];
        let preceding_has_tool_use = preceding
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        if preceding_has_tool_use {
            // Pair is intact — walk back one more to include the assistant turn.
            k = k.saturating_sub(1);
            break;
        }
        // Preceding message has no ToolUse but we have a ToolResult —
        // this is already an orphaned pair; walk back to try to fix it.
        k = k.saturating_sub(1);
    }
    k
};
```

The loop terminates when:
- The boundary reaches the start of the message list, or
- The first preserved message is not a `ToolResult`, or
- The pair is intact (preceding message has a `ToolUse` and we walk back one more to include it)

The net effect: the compaction boundary never falls between a `ToolUse` and its `ToolResult`. Either both are preserved or both are summarized. This is tested by `compaction_does_not_split_tool_use_tool_result_pair` in `compact.rs`.

---

## Connection to Context-Window Continuity

Compaction exists to keep a session alive across turns when the raw transcript would otherwise exceed the model's context window. The design choices reflect this:

- **Token-budget trigger** — compaction fires before the window is exhausted, not after. The threshold is a safety margin, not a hard limit.
- **Preserved tail** — the most recent turns are kept verbatim so the model sees the actual state of the conversation, not just a summary of it.
- **Tool-pair integrity** — compaction cannot introduce malformed tool sequences, because splitting a tool pair would cause the model to generate invalid API requests on the next turn.

The post-compaction health probe in `conversation.rs` is the runtime's way of confirming that compaction succeeded and the tool surface is still usable — if `glob_search` fails after compaction, the turn fails fast rather than continuing with a broken session.

---

## Builder Lessons

1. **Rough token estimates beat round-trip counting for trigger decisions.** Using `text.len() / 4 + 1` per block is fast enough to call on every turn without adding latency. Accuracy matters less than consistency — you calibrate the threshold, not the formula.

2. **Preserve the tail, summarize the middle.** Recent messages are high-signal for the model's next action. Older messages contribute context that degrades gracefully when summarized. This is the right split for a coding agent.

3. **Boundary logic must understand protocol invariants, not just message counts.** The tool-pair guard exists because the OpenAI-compatible API has a structural requirement (`tool` must follow `assistant` with `tool_calls`). The compactor cannot be oblivious to this — it must walk back to preserve the invariant.

4. **Auto-compaction at the turn level decouples it from user-visible latency.** Triggering compaction inside `run_turn()` means it happens in the background after the turn completes, not as a synchronous gate that slows down the user's first response. The `TurnSummary::auto_compaction` field surfaces the event to callers.

5. **Environment-variable configuration lets operators tune without code changes.** `CLAUDE_CODE_AUTO_COMPACT_INPUT_TOKENS` is read at startup, so a dev working with a 200K-context model can raise the threshold without changing `CompactionConfig::default()`.
