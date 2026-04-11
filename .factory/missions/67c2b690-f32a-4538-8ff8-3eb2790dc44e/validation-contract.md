# SWELL MVP Validation Contract

This document defines the behavioral assertions that must pass for the SWELL MVP to be considered complete.

---

## Area: CLI Commands

### VAL-CLI-001: Task creation
User runs `swell task "description"` and the daemon creates a task, returning a task ID.
Tool: agent-browser (manual CLI testing via terminal)
Evidence: terminal output showing task ID, `swell list` shows the task

### VAL-CLI-002: Task listing
User runs `swell list` and sees all tasks with their states.
Tool: agent-browser (manual CLI testing)
Evidence: terminal output showing task list with states (CREATED, EXECUTING, etc.)

### VAL-CLI-003: Task watch
User runs `swell watch <task-id>` and receives progress updates.
Tool: agent-browser (manual CLI testing)
Evidence: terminal output showing state transitions as task progresses

### VAL-CLI-004: Task approve/cancel
User can approve or cancel a task via CLI commands.
Tool: agent-browser (manual CLI testing)
Evidence: terminal output confirming action, task state changes

---

## Area: Daemon Server

### VAL-DAEMON-001: Unix socket communication
CLI connects to daemon via Unix socket and exchanges JSON messages.
Tool: terminal (manual)
Evidence: successful command/response cycle

### VAL-DAEMON-002: Graceful shutdown
Daemon responds to SIGTERM and cleanly shuts down.
Tool: terminal
Evidence: daemon logs show clean shutdown, no zombie processes

### VAL-DAEMON-003: Error handling
Daemon returns structured error responses for invalid commands.
Tool: terminal
Evidence: JSON error response with message field

---

## Area: Orchestrator

### VAL-ORCH-001: Task lifecycle states
Task progresses through all states: CREATED → ENRICHED → READY → ASSIGNED → EXECUTING → VALIDATING → ACCEPTED.
Tool: cargo test (unit tests for state machine)
Evidence: all state transition tests pass

### VAL-ORCH-002: Invalid state transitions rejected
Attempting invalid transitions returns error and does not change state.
Tool: cargo test
Evidence: tests for invalid transitions return SwellError

### VAL-ORCH-003: Task plan attachment
Planner output is attached to task as a Plan.
Tool: cargo test
Evidence: task.plan is Some after planning

### VAL-ORCH-004: Agent pool management
Agents can be registered, reserved, and released.
Tool: cargo test
Evidence: agent pool tests pass

### VAL-ORCH-005: Task completion with validation result
Task transitions to ACCEPTED when validation passes, REJECTED when it fails.
Tool: cargo test
Evidence: complete_task tests verify correct state

---

## Area: Agents (Planner, Generator, Evaluator)

### VAL-AGENTS-001: Planner creates structured plan
Planner agent produces JSON with steps, affected_files, risk_level.
Tool: cargo test
Evidence: planner output parses to valid Plan structure

### VAL-AGENTS-002: Generator produces output
Generator agent produces output describing what was generated.
Tool: cargo test
Evidence: generator.execute returns success=true

### VAL-AGENTS-003: Evaluator produces result
Evaluator agent produces evaluation output.
Tool: cargo test
Evidence: evaluator.execute returns success=true

### VAL-AGENTS-004: Agents use correct role
Each agent type returns its correct AgentRole.
Tool: cargo test
Evidence: role() method returns expected role

---

## Area: Tool Execution

### VAL-TOOLS-001: File read tool
FileReadTool reads file contents from filesystem.
Tool: cargo test
Evidence: test creates temp file, reads it back

### VAL-TOOLS-002: File write tool
FileWriteTool writes content to filesystem.
Tool: cargo test
Evidence: test writes file, verifies content

### VAL-TOOLS-003: Shell execution
ShellTool executes commands and returns output.
Tool: cargo test
Evidence: shell tool test runs echo, captures output

### VAL-TOOLS-004: Git status
GitTool returns git repository status.
Tool: cargo test (with test repo)
Evidence: git status returns expected fields

### VAL-TOOLS-005: Tool registry
Tools can be registered and retrieved by name.
Tool: cargo test
Evidence: registry.list() shows registered tools

---

## Area: LLM Integration

### VAL-LLM-001: Anthropic backend creation
AnthropicBackend initializes with model name and API key.
Tool: cargo test
Evidence: backend.model() returns configured model

### VAL-LLM-002: OpenAI backend creation
OpenAIBackend initializes with model name and API key.
Tool: cargo test
Evidence: backend.model() returns configured model

### VAL-LLM-003: Mock backend for testing
MockLlm returns predictable responses for tests.
Tool: cargo test
Evidence: mock.chat returns expected content

### VAL-LLM-004: Backend health check
All backends implement health_check returning bool.
Tool: cargo test
Evidence: health_check returns true for healthy backend

---

## Area: Validation Gates

### VAL-VALIDATION-001: LintGate runs checks
LintGate executes and reports pass/fail.
Tool: cargo test (with valid/invalid Rust code)
Evidence: LintGate.validate returns ValidationOutcome

### VAL-VALIDATION-002: TestGate runs tests
TestGate executes test suite and reports results.
Tool: cargo test
Evidence: TestGate.validate returns ValidationOutcome

### VAL-VALIDATION-003: Validation pipeline
ValidationPipeline runs multiple gates in order.
Tool: cargo test
Evidence: pipeline.run aggregates results

### VAL-VALIDATION-004: Validation context
ValidationContext contains task_id, workspace_path, changed_files.
Tool: cargo test
Evidence: ValidationContext fields accessible

---

## Area: State Management

### VAL-STATE-001: SQLite checkpoint store
SqliteCheckpointStore implements CheckpointStore trait.
Tool: cargo test
Evidence: save/load operations work with SQLite

### VAL-STATE-002: In-memory store for testing
InMemoryCheckpointStore works for tests without database.
Tool: cargo test
Evidence: in-memory store tests pass

### VAL-STATE-003: Task checkpointing
Tasks can be checkpointed and restored.
Tool: cargo test
Evidence: checkpoint save/load preserves task state

---

## Area: Memory System

### VAL-MEMORY-001: SQLite memory store
SqliteMemoryStore implements MemoryStore trait.
Tool: cargo test
Evidence: memory store CRUD operations work

### VAL-MEMORY-002: Memory entry storage
MemoryEntry can be stored and retrieved by ID.
Tool: cargo test
Evidence: store/get roundtrip works

### VAL-MEMORY-003: Memory search
MemoryQuery returns matching entries.
Tool: cargo test
Evidence: search returns relevant results

### VAL-MEMORY-004: Memory by type/label
get_by_type and get_by_label return filtered entries.
Tool: cargo test
Evidence: filtered queries work correctly

---

## Area: Agent Skills

### VAL-SKILLS-001: Skill discovery
Skills are discovered from .swell/skills/ directory by scanning for SKILL.md files.
Tool: cargo test
Evidence: skills_discovered() returns all skills in directory

### VAL-SKILLS-002: YAML frontmatter parsing
SKILL.md files with YAML frontmatter (name, description) are parsed correctly.
Tool: cargo test
Evidence: parse_frontmatter extracts name and description

### VAL-SKILLS-003: Skill catalog
Catalog contains name, description, and location for each discovered skill.
Tool: cargo test
Evidence: get_catalog() returns vector of SkillCatalogEntry

### VAL-SKILLS-004: Progressive disclosure - Tier 1
At startup, only name and description (~50-100 tokens) are loaded for catalog.
Tool: cargo test
Evidence: catalog entries have description_length within expected range

### VAL-SKILLS-005: Progressive disclosure - Tier 2
On skill activation, full SKILL.md body is loaded.
Tool: cargo test
Evidence: load_skill_content() returns full markdown body

### VAL-SKILLS-006: On-demand resources
scripts/, references/, assets/ directories loaded only when explicitly requested.
Tool: cargo test
Evidence: load_skill_resources() only loads on-demand

### VAL-SKILLS-007: Model-driven activation
LLM can read catalog and match skills to tasks based on description keywords.
Tool: integration test
Evidence: skill_matches_task() returns relevant skills for keywords

### VAL-SKILLS-008: Lenient validation
Skills with missing optional frontmatter fields are accepted.
Tool: cargo test
Evidence: parsing succeeds with missing optional fields

## Area: Safety Controls

### VAL-SAFETY-001: CostGuard budget tracking
CostGuard tracks token usage against budget.
Tool: cargo test
Evidence: add_cost updates spent, is_warning_threshold works

### VAL-SAFETY-002: Doom loop detection
Iteration count is tracked and escalated after threshold.
Tool: cargo test
Evidence: task.iteration_count increments on rejection

---

## Cross-Area Flows

### VAL-CROSS-001: End-to-end task execution
User creates task via CLI → daemon creates task → orchestrator runs Planner → Generator → Evaluator → validation → task completed.
Tool: integration test (requires daemon running)
Evidence: full task lifecycle completes successfully

### VAL-CROSS-002: Task survives daemon restart
Task state persists in SQLite, survives daemon restart.
Tool: integration test
Evidence: task continues from checkpoint after restart

### VAL-CROSS-003: Multiple concurrent tasks
Multiple tasks can be queued and processed.
Tool: cargo test (execution controller batch test)
Evidence: execute_batch handles multiple tasks

---

## Area: Research Agent

### VAL-RESEARCH-001: Researcher agent registration
ResearcherAgent can be registered with the orchestrator and spawned when external information is needed.
Tool: cargo test
Evidence: researcher agent executes and returns research results

### VAL-RESEARCH-002: Web search tool
WebSearchTool executes searches and returns structured results with title, URL, snippet.
Tool: cargo test
Evidence: search returns results with required fields

### VAL-RESEARCH-003: Fetch page tool
FetchPageTool fetches web pages, extracts main content, and returns cleaned text with provenance.
Tool: cargo test
Evidence: page content extracted with URL and title

### VAL-RESEARCH-004: Search depth routing
Search router classifies queries into quick/deep search and routes accordingly.
Tool: cargo test
Evidence: routing decisions match expected complexity levels

### VAL-RESEARCH-005: Provenance tracking
All retrieved content includes source URL, title, fetch timestamp.
Tool: cargo test
Evidence: provenance fields present in all research results

### VAL-RESEARCH-006: Orchestrator triggers research
Orchestrator can spawn ResearcherAgent when task needs external information.
Tool: cargo test
Evidence: orchestrator spawns researcher on research-needed signal
