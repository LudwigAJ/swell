# Compaction Resume Packet — Synthetic Continuation, Summary Shape, and Repeated Merge

## What This Document Covers

This document explains the structure and behavior of the synthetic message injected after session compaction — the "resume packet" that carries prior summary material, explicit resume instructions, and context continuity across multiple compaction cycles. It covers the continuation message anatomy, the `resume directly` behavioral directive, the shape of the generated summary, and how existing summaries are merged when compaction runs repeatedly. For trigger mechanics and tool-pair boundary preservation, see `analysis/compaction-trigger-tail.md`.

**Evidence sources:**
- `references/claw-code/rust/crates/runtime/src/compact.rs` — continuation message construction, summary shape, merge logic
- `references/claw-code/rust/crates/runtime/src/conversation.rs` — auto-compaction integration and health probe

---

## The Synthetic Continuation Message

When `compact_session()` finishes, it replaces the removed message block with a single **system message** that carries everything the model needs to continue coherently. This message is assembled in `get_compact_continuation_message()` and has a consistent anatomy:

```rust
const COMPACT_CONTINUATION_PREAMBLE: &str =
    "This session is being continued from a previous conversation that ran out of context. \
    The summary below covers the earlier portion of the conversation.\n\n";

const COMPACT_RECENT_MESSAGES_NOTE: &str = "Recent messages are preserved verbatim.";

const COMPACT_DIRECT_RESUME_INSTRUCTION: &str =
    "Continue the conversation from where it left off without asking the user any \
    further questions. Resume directly — do not acknowledge the summary, do not recap \
    what was happening, and do not preface with continuation text.";
```

The message is assembled from three pieces:

1. **Preamble** — identifies that this session is a continuation and that a summary follows
2. **Formatted summary** — the actual summary content (see Summary Shape below)
3. **Resume directive** — an explicit instruction to skip recap and continue directly

If recent messages were preserved (the tail is non-empty), a `Recent messages are preserved verbatim.` note is appended so the model knows those messages are live context, not summarized-away history.

---

## The `resume directly` Instruction

The resume directive is the most behaviorally significant part of the continuation message. It tells the model:

> Resume directly — do not acknowledge the summary, do not recap what was happening, and do not preface with continuation text.

This is not cosmetic. In prior dogfooding sessions (Jobdori, 2026-04-09), the bot would spend turns acknowledging the summary and re-explaining context instead of continuing work — because without an explicit directive, models treat summaries as topics to address. The `COMPACT_DIRECT_RESUME_INSTRUCTION` overrides that tendency.

The instruction is unconditional in the continuation message: it fires every time compaction occurs, regardless of whether a tail was preserved. When `suppress_follow_up_questions` is `true`, the instruction is included. In practice `compact_session()` always passes `suppress_follow_up_questions: true`, so the directive is always present.

---

## Summary Shape

The summary injected into the continuation message is not a raw transcript or a freeform paragraph — it is a structured, tagged block assembled by `summarize_messages()`. The structure preserves specific information categories:

```rust
lines.push("<summary>".to_string());
lines.push("Conversation summary:".to_string());
// - Scope: N earlier messages compacted (user=N, assistant=N, tool=N)
// - Tools mentioned: tool-a, tool-b, ...
// - Recent user requests:
//   - request 1
//   - request 2
// - Pending work:
//   - pending item 1
// - Key files referenced: file-a.rs, file-b.md, ...
// - Current work: description of most recent work
// - Key timeline:
//   - role: message content
//   - ...
"</summary>")
```

Each section has a defined purpose:

| Field | Source | Purpose |
|---|---|---|
| **Scope** | Message count by role | Tells the model how much history was compressed and what kinds of turns were involved |
| **Tools mentioned** | ToolUse and ToolResult block names, deduplicated | Shows which tools were used so the model knows the available surface |
| **Recent user requests** | Up to 3 user message texts, truncated to 160 chars | Captures what the human asked most recently before compaction |
| **Pending work** | User/assistant messages containing "todo", "next", "pending", "follow up", "remaining" | Preserves forward-looking intent even if the model was mid-task |
| **Key files** | File paths with interesting extensions extracted from all message content | References code under active work |
| **Current work** | Most recent non-empty text block | What was happening right before compaction |
| **Key timeline** | One line per message (role: content) | A stripped chronological record for grounding without full transcript |

This shape is designed to be comprehensive enough for the model to understand the full context, while staying compact enough to fit within the context window alongside the preserved tail. The `format_compact_summary()` function strips `<analysis>` tags and normalizes whitespace before injecting the summary, so the model sees clean prose rather than internal tagging artifacts.

---

## Repeated Compaction: Summary Merge Behavior

When compaction fires on a session that has already been compacted once, the new summary is not simply appended or replacing — it is **merged** with the existing summary. The `merge_compact_summaries()` function handles this.

### First compaction

On first compaction, there is no existing summary, so `summarize_messages()` produces the full summary and it is used as-is.

### Subsequent compactions

When `compact_session()` finds an existing summary in the system message prefix (detected via `extract_existing_compacted_summary()`), it:

1. Extracts highlights from the existing summary (everything except the timeline)
2. Formats the newly generated summary
3. Extracts highlights from the new summary
4. Extracts the new timeline
5. Assembles a merged block:

```rust
// Previously compacted context:
//   (highlights from existing summary)
// - Newly compacted context:
//   (highlights from new summary)
// - Key timeline:
//   (timeline from new summary)
```

The key property is that **previously compacted context is never overwritten** — highlights from earlier summaries accumulate under `Previously compacted context:`, while the most recent compaction's highlights and timeline are added as `Newly compacted context:`. This means that after N compaction cycles, the model has a cascading view of what happened across all cycles, not just the most recent one.

The test `keeps_previous_compacted_context_when_compacting_again` in `compact.rs` directly validates this:

```rust
assert!(second.formatted_summary.contains("Previously compacted context:"));
assert!(second.formatted_summary.contains("Newly compacted context:"));
assert!(second.formatted_summary.contains("Scope: 2 earlier messages compacted"));
```

And the continuation message in the compacted session reflects both layers:

```rust
assert!(matches!(
    &second.compacted_session.messages[0].blocks[0],
    ContentBlock::Text { text }
        if text.contains("Previously compacted context:")
            && text.contains("Newly compacted context:")
));
```

### Timeline preservation across compactions

Only the **newest** compaction's timeline is included in the merged summary — the timeline from earlier compactions is dropped during merge, because that history is already implicit in the `Previously compacted context:` highlights. Keeping all timelines would cause the summary to grow unbounded with each compaction cycle. The tradeoff is deliberate: highlights preserve what, timeline preserves order, and highlights from older cycles are enough to reconstruct intent.

---

## The `compacted_summary_prefix_len` Check

The `compact_session()` function uses `compacted_summary_prefix_len(session)` to detect whether the session has already been compacted. This is the number of messages at the start of the session that are part of the synthetic continuation block — either 0 (first compaction) or 1 (subsequent compactions, the system message itself).

```rust
fn compacted_summary_prefix_len(session: &Session) -> usize {
    usize::from(
        session
            .messages
            .first()
            .and_then(extract_existing_compacted_summary)
            .is_some(),
    )
}
```

This affects both boundary calculation and re-compaction behavior. The boundary walker that prevents tool-pair splitting accounts for the summary prefix, so it never treats the synthetic system message as a candidate for boundary adjustment. And `should_compact()` uses this to skip the existing summary when deciding whether additional compaction is needed — an already-compacted session with a small amount of new activity below the threshold will not re-compact.

---

## Builder Lessons

1. **The continuation message is a protocol artifact, not prose.** Its structure — preamble, summary, directive — is engineered to control model behavior (skip recap, continue work). A freeform summary would not reliably produce the same result.

2. **The resume directive must be explicit.** Without `COMPACT_DIRECT_RESUME_INSTRUCTION`, models naturally tend to acknowledge and recap summarized context rather than continue. The instruction overrides that tendency directly.

3. **Summary shape should be declarative and field-rich.** The eight-field structure (scope, tools, requests, pending, files, current, timeline) is specific enough that the model can reconstruct intent from it. Generic "conversation summary" prose would lose the distinction between what was asked, what was pending, and what was the current focus.

4. **Repeated compaction requires merge, not replace.** Simply overwriting the existing summary would lose historical context. The cumulative highlight approach (`Previously compacted context:` → `Newly compacted context:`) preserves all cycles without unbounded growth, because older timelines are dropped in favor of highlights.

5. **Summary extraction is structural, not regex-based.** `extract_existing_compacted_summary()` parses the synthetic message by finding the known preamble, splitting on the `Recent messages are preserved verbatim.` marker and the resume instruction marker. This is stable as long as the constants are stable — changing `COMPACT_CONTINUATION_PREAMBLE` or `COMPACT_DIRECT_RESUME_INSTRUCTION` would break extraction and must be done together.

6. **The compacted prefix must be accounted for in boundary walking.** The tool-pair guard walks back from `raw_keep_from` but stops early if `k <= compacted_prefix_len`, preventing the boundary walker from treating the synthetic system message as a potential tool-result or tool-use block. This invariant is tested by `compaction_does_not_split_tool_use_tool_result_pair` which goes through full compaction and verifies no orphaned tool results appear in the resulting session.
