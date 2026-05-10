# MiniMax M2.7 end-to-end shakedown

Live test log of running real swell tasks against `MiniMax-M2.7` via the
Anthropic-compatible endpoint. Intent is to exercise every capability the
provider claims to support and watch how the swell stack behaves.

## Setup

- Model: `MiniMax-M2.7` (alias `minimax-m2.7` in `.swell/settings.json`)
- Endpoint: `https://api.minimax.io/anthropic`
- Auth: `MINIMAX_API_KEY`
- Socket: `/tmp/swell-daemon.sock`
- Daemon log: `/tmp/swell-daemon.log`

## What MiniMax-M2.7 supports (from provider docs)

Supported: `messages` (text + tool_use + tool_result + thinking), `max_tokens`,
`stream`, `system`, `temperature` ((0,1]), `top_p`, `tool_choice`, `tools`,
`metadata`, `thinking`.

Ignored: `top_k`, `stop_sequences`, `service_tier`, `mcp_servers`,
`context_management`, `container`.

Not supported in messages: `image`, `document`.

> Note: capability matrix is per-model. M2.5/M2.1 may differ — when we test
> those, refresh the table.

> **MCP**: MiniMax ignores the `mcp_servers` request field. So if a user
> registers MCP servers in swell, swell itself must act as the MCP client
> (stdio/http), surface those tools as regular Anthropic `tools`, execute
> the calls locally, and feed `tool_result` blocks back to the model. That
> is something to verify here.

## Test matrix

| # | Task | Expected swell behavior | Status |
|---|------|-------------------------|--------|
| 1 | Trivial — single-shot text answer (planner only) | Plan succeeds, no tool calls | |
| 2 | Easy — read a file, summarize it | `read_file` tool invoked, summary returned | |
| 3 | Medium — add a tiny new function in a sandbox | Plan→Generate→Evaluate, file written | |
| 4 | Medium — modify an existing file with a constraint | Edit tool used, validation passes | |
| 5 | Hard — multi-file refactor with tests | Multi-turn generator loop, tests run | |
| 6 | Streaming check — observe `thinking` deltas in logs | Thinking blocks captured & logged | |
| 7 | Cache control — second turn within 5min hits cache | `cache_read_input_tokens > 0` | |
| 8 | MCP via swell — register a stdio MCP server, ensure tools surface | Tools listed in catalog, calls round-trip | |
| 9 | Provider-cap respect — confirm `top_k`/`stop_sequences` not on the wire | Inspect daemon log; no warnings from provider | |

## Findings

### Post-fix verification run (2026-05-10, after I-1/I-2/I-3 landed)

- **I-1 verified live**: daemon startup now logs
  `INFO swell_llm::anthropic: skipping /v1/models listing — provider declared listing unsupported provider=minimax`
  instead of the prior `JSON error` line.
- **I-2 verified live**: planner produced a valid plan on the file-read task in ~9s end-to-end. Generator started ReAct without ever hitting the `Failed to parse plan JSON` path. The `emit_plan` tool round-tripped through MiniMax cleanly.
- **I-3 verified live**: ReAct loop fired the loop detector after **3 identical `read_file` calls** (vs 27 before) and aborted with
  `WARN ... Loop detector tripped during ReAct loop; aborting step pattern=escalation reason=Same-tool retry loop detected: Same tool 'read_file' executed 3 times consecutively without success`
  Task transitioned `EXECUTING → FAILED` with `SwellError::LoopDetected`. Cost containment on a misbehaving model is now real.

### New observation (worth a follow-up)

- **O-1 — MiniMax-M2.7 keeps re-reading instead of advancing**: even though the file content was returned successfully on the first call, the model issued the same `read_file({"path":"…/greet.txt"})` three turns in a row. It never attempted the obvious next step (`write_file` to `SUMMARY.md`). The loop detector is doing its job, but the *underlying* problem is that MiniMax doesn't aggregate prior `tool_result` content into forward action the way Sonnet does on the same prompt.
- **Severity**: medium for product UX (the task fails even though the data is in hand). Not blocking — loop detector now contains the cost.
- **Possible fixes worth exploring** (not committing to one yet):
  1. **Synthetic nudge on detector trip**: instead of failing the task outright, on `LoopIntervention::Escalation` push a synthetic user message back into the conversation (`"You already executed read_file successfully and got <content>. Take the next step: write the result to <output_path>."`) and continue, only failing if the next iteration also loops.
  2. **Stronger generator system prompt for MiniMax**: explicitly instruct "do not call the same tool with the same args twice; if a previous tool_result satisfies the immediate question, proceed to the next plan step."
  3. **Per-step planner output**: have the planner enumerate tool calls per step so the generator has less freedom to oscillate.
- **Status**: open, tracked here. Will revisit after rows 3-5 of the matrix (more complex tasks may either confirm the pattern or reveal it's a quirk of summarize-style prompts).

### Test matrix update

- Row 1 (trivial PONG) — **planner pass, generator overshoots**: planner now produces a 1-step plan via `emit_plan` (no JSON-parse crash). Generator interprets it as an engineering task and starts running `find` to recon the workspace; loop detector trips at 3 consecutive `shell` calls (same pattern as O-1, different tool). Task ends `EXECUTING → FAILED` with `LoopDetected(escalation)`. The cost guardrail works; the underlying UX issue is again "MiniMax doesn't move past the first observation."
- Row 2 (easy file-read summarize) — **partial pass**: planner+plan round-tripped, loop detector engaged at 3 `read_file` calls. Task ended `Failed` for the right reason. See O-1.
- Row 2 **rerun post-O-1-fix** (task d163ae4a, 17:19) — **agent-loop pass / gate rejection**: read_file → write_file in 2 calls, 891 tokens, no looping. SUMMARY.md correct. Validator rejected on 1 warning (see O-2).
- Row 3 (medium add-function — `Create math.py with add() and multiply()`, task cbfd3bd8, 17:25) — **agent-loop pass / gate rejection**: planner produced 1-step plan in 6s; generator wrote a complete typed Python module with Google-style docstrings in **a single `write_file` call**, 864 tokens. Validator again rejected on 1 warning (see O-2). Loop detector never tripped, rescue never needed.

### Generalized form of O-1

After two runs the symptom isn't tool-specific — MiniMax-M2.7, on swell's stock generator system prompt, tends to repeat its first reconnaissance tool until something nudges it forward. The loop detector now reliably catches this in ≤3 calls, but to make tasks actually *succeed* rather than just *fail safely* we likely want option (1) from the I-3 follow-up: **on first detector trip, inject a synthetic user message** (`"You already have the result of <tool>(<args>); proceed to the next plan step."`) and continue the ReAct loop, only failing on the *second* trip. That converts "fail-fast" into "fail-after-one-rescue."

### O-1 — **resolved** (2026-05-10, post-refactor)

Root cause was deeper than "MiniMax is stubborn": the generator's `execute_step_with_react` was rebuilding a fresh single-message prompt on every iteration, never echoing prior `tool_use` / `tool_result` / `thinking` blocks back to the model. With no memory of what it had already done, MiniMax (correctly) retried its only known move. Anthropic Sonnet papered over this by inferring history from the prompt template; MiniMax did not.

**Fix shipped**:
1. `GeneratorAgent::execute_step_with_react` now maintains a `Vec<LlmMessage>` conversation across the per-step loop. After each tool execution we append (a) an `Assistant` turn carrying `content + tool_calls + thinking_blocks` and (b) a `User` turn carrying the `tool_result` (with `tool_call_id` and `tool_result_is_error`).
2. Generator advertises four tools (`read_file`, `write_file`, `edit_file`, `shell`) as `LlmToolDefinition`s with proper JSON schemas, and sets `tool_choice = Any` so MiniMax must pick one.
3. Short, opinionated `GENERATOR_REACT_SYSTEM_PROMPT` ("call exactly one tool per turn, never repeat the same call, prefer write_file for new files").
4. Rescue-once: on the first `LoopIntervention` trip we clear the detector for the task, push a synthetic nudge, and continue. Only the second trip fails the task.

**Live verification (task d163ae4a, 2026-05-10 17:19)**: same file-read→summarize prompt that previously looped 27× and then 3× now completes in **2 tool calls**: `read_file({"path":"/tmp/swell-sandbox/greet.txt"})` → `write_file({"path":"/tmp/swell-sandbox/SUMMARY.md", "content":"The greet.txt file contains the message: \"hello swell, fourth time around\""})`. Total generator cost: 891 tokens. Loop detector never tripped; rescue path never needed. SUMMARY.md written correctly.

(Task ended in `REJECTED` state from a downstream validation gate with 0 errors / 1 warning — that's a separate validator concern, not an agent-loop problem. Filing as O-2 below.)

### O-2 — Validation gate rejects file-read-summary task with 0 errors / 1 warning

- **Symptom**: task d163ae4a above ended `VALIDATING → REJECTED` with `errors=0 warnings=1`. A 23s validation pass (probably running cargo / lint over the workspace) treats a single warning as fatal even though the task only touches a sandbox markdown file.
- **Severity**: low for the shakedown (the agent did the right thing; the gate is the issue) but visible to users.
- **Status**: open. To investigate after row 3.



### Issues

#### I-1 — `discover_models` fails on MiniMax response shape
- **Symptom**: daemon startup logs `discover_models unavailable; skipping startup model check error=LLM error: Anthropic SDK error: JSON error`.
- **Root cause**: `GET /v1/models` on MiniMax returns `{"data":null,"has_more":false}` (note `null`, not `[]`). The community SDK's `ModelListResponse.data` is a non-nullable `Vec<ModelInfo>`, so deserialization fails. `GET /v1/models/{id}` returns all-empty-string fields the same day, so even single-model lookup is uninformative.
- **Severity**: low — we already soft-fail and continue.
- **Suggested fix**: extend `ProviderCaps` with `supports_models_listing: bool` (default true; MiniMax → false) and skip the call entirely when false, instead of letting it error each startup. Bonus: add an info-log noting the provider reported the endpoint as unsupported.
- **Status**: **resolved**. Added `ProviderCaps::supports_models_listing` (default `true`, `false` for MiniMax) plus matching `ProviderCapsOverride` field. `warn_if_unknown_model` short-circuits with a clear `info!` line when the cap is off, instead of swallowing a JSON deserialize error.

#### I-2 — Planner agent doesn't use `chat_typed` → MiniMax escapes the plan schema
- **Symptom**: task `Reply with the single word PONG and nothing else.` failed with `Failed to parse plan JSON: expected value at line 1 column 1. Raw content: PONG`.
- **Root cause**: `crates/swell-orchestrator/src/agents.rs:629` runs free-form `chat()` then `serde_json::from_str` on the body. MiniMax obeyed the user instruction (return literal "PONG") instead of the planner's "return JSON" system prompt. Anthropic Sonnet would likely have respected the system prompt; MiniMax weights user > system more aggressively.
- **Severity**: medium — any user prompt that looks like a final answer (or that asks the model to "just respond with X") will crash the planner stage on MiniMax.
- **Suggested fix**: migrate the planner call to `AnthropicBackend::chat_typed::<PlanShape>(...)` (already implemented in `swell-llm/src/anthropic.rs`). This routes through the SDK's `messages().create_and_parse_with::<T>`, which forces a JSON-schema-constrained response. This is the "Structured-output convenience" item already on the implementation plan but not yet wired into agents.
- **Status**: **resolved**. Planner now passes a single `emit_plan` tool definition with the plan JSON schema and sets `tool_choice = Tool { name: "emit_plan" }`. Response is read from `response.tool_calls[0].arguments`; free-form `extract_json_object` path remains as a fallback for `MockLlm` and any backend that ignores `tool_choice`.

#### I-3 — Tool-call loop detector is decorative; agent loop never consults it
- **Symptom**: MiniMax called `read_file("/tmp/swell-sandbox/greet.txt")` 27 times in a row before we cancelled. Same args every iteration. Task never advanced past the initial read despite `no_progress_iterations: 5` and `max_tool_call_iterations: 15` in `.swell/settings.json`.
- **Root cause**: `crates/swell-orchestrator/src/lib.rs:445` constructs a `NonNovelRetryDetector` and stores it on the orchestrator, and `crates/swell-orchestrator/src/loop_detection.rs` defines an `OrchestratorLoopDetector` with `LoopIntervention` enum. Both ship through the wiring-report panel as "enabled". But `grep` for `non_novel_detector|register_tool_call|OrchestratorLoopDetector|LoopIntervention` across `agents.rs` and `execution.rs` returns **zero hits**. The detectors are never called from the ReAct loop, so no intervention is ever raised. `max_tool_call_iterations: 15` is similarly unenforced — no `MAX_TOOL_CALL_ITERATIONS` constant referenced from the agent loop either.
- **Severity**: HIGH. Critical for cost containment and for non-Anthropic models (Claude tends to self-correct after seeing identical tool_results; MiniMax demonstrably does not).
- **Suggested fix**:
  1. In the generator's tool-call iteration in `agents.rs`, after each `tool_use` block call `non_novel_detector.record(task_id, tool_name, args_hash)` and break the loop on `NonNovelRetryResult::ForceStrategyChange` (or whatever the enum exposes), feeding back a synthetic user message that tells the model to stop repeating itself.
  2. Hard cap on iterations: read `execution.max_tool_call_iterations` from settings and break with an explicit `LoopLimitExceeded` failure class when exceeded. Don't quietly run forever.
  3. Add a regression test using `MockLlm` that always emits the same `tool_use` block; assert the generator terminates within N iterations.
- **Status**: **resolved**. `GeneratorAgent` now takes an `Option<Arc<RwLock<OrchestratorLoopDetector>>>` (wired in via `OrchestratorBuilder` → `execution.rs` production agent construction). After every `execute_action` in `execute_step_with_react`, the detector's `record_tool_call` + `check` are consulted, and on `LoopIntervention::Halt | Escalation | StrategyChange` the ReAct loop returns `SwellError::LoopDetected { reason, pattern }`. The existing `ToolLoopTracker::detect_same_tool_retry` already counts identical tool names regardless of `success`, so the 27-call read_file scenario is caught by the default threshold.
- **Wiring-report bug-adjacent**: the wiring report says `NonNovelRetryDetector ... enabled` but "enabled" here just means "constructed". The wiring spec should distinguish "constructed" from "consumed by the hot path" — otherwise the panel gives false confidence.