# Testing and Validation Research Spec for an Autonomous Coding Orchestrator

**Testing is the autonomous coding engine's primary mechanism for building confidence, steering execution, and proving correctness.** Without the ability to plan, generate, select, run, and interpret tests, an orchestrator cannot distinguish working code from plausible code — and plausible code produced by LLMs is the most dangerous failure mode in autonomous development. This spec defines how an orchestrator should reason about validation across the full lifecycle: from deriving test intent out of specs and code changes, through generation and execution, to interpreting results as planning signals that drive accept, reject, revise, or decompose decisions. The architecture is language-agnostic, CI-agnostic, and optimized for long-running autonomous execution with minimal human intervention.

The research synthesized here draws on the current state of the art (2024–2026) across autonomous coding agents (SWE-Agent, Devin, OpenHands, Codex, Claude Code, Cursor, Aider, AutoCodeRover, Amazon Q Developer, Copilot Workspace), academic literature on LLM-based testing (IEEE TSE surveys, ICSE/FSE/ISSTA papers, NeurIPS workshops), and industrial testing infrastructure from Google, Meta, Netflix, and Atlassian. A central finding is that **every successful autonomous coding system uses an edit→test→fix loop as its core control structure**, but most depend heavily on pre-existing test suites and do not yet reason deeply about what *should* be tested. The gap between running tests and reasoning about testing is precisely where an orchestrator must invest.

---

## 1. Why testing is foundational for autonomous development

Testing in an autonomous coding orchestrator serves three functions that go well beyond quality assurance: it is a **planning mechanism**, a **confidence signal**, and a **proof-of-work system**. Without testing, an orchestrator has no way to distinguish between code that implements a spec and code that merely compiles. With LLM-generated code, this distinction is critical — LLMs produce syntactically valid, plausible-looking code that may satisfy surface-level inspection while harboring deep behavioral defects.

**Testing as a planning mechanism.** Every autonomous coding agent that achieves meaningful results on benchmarks like SWE-bench uses an iterative test→fix loop as its primary control structure. Codex (OpenAI) was trained via reinforcement learning specifically to "iteratively run tests until passing." Aider's `--auto-test` flag runs the test suite after every LLM edit and feeds failures back into the model. Cursor's Composer model was trained via RL inside real codebases where it learned to run tests, fix linter errors, and navigate projects. The pattern is universal: **test results are the signal that drives iteration**. An orchestrator that cannot reason about test results cannot steer implementation.

More profoundly, test results enable the orchestrator to make the four critical planning decisions for any work unit: **accept** (all validation evidence confirms the goal is met), **reject** (failures indicate a fundamentally wrong approach), **revise** (partial passage reveals specific fixable defects), or **decompose** (complex failures reveal the task was too large or underspecified). This maps directly to ATDD (Acceptance Test-Driven Development), where failing acceptance tests become the task queue itself — each red test is a work item, and progress is objectively measurable as the ratio of green to total tests.

**Testing as confidence signal.** Autonomous systems need calibrated confidence to make merge decisions without human intervention. A single signal (e.g., "unit tests pass") is insufficient. Robust confidence requires aggregating multiple validation signals: unit test passage, integration test passage, static analysis cleanliness, type-check success, security scan results, mutation testing scores, and regression test stability. Netflix's "Test Confidence" system demonstrates this principle — by re-running failing tests against the destination branch to filter noise, they boosted availability of actionable confidence data from **35% to 74%**. An orchestrator should compute composite confidence scores and use graduated thresholds to determine whether to auto-merge, request review, or block.

**Testing as proof of progress.** In long autonomous runs — Codex has demonstrated 7+ hour continuous sessions, and Cursor has run 100+ agents for weeks producing over 1M lines of code — the orchestrator needs evidence that work actually satisfies its goals. Tests provide this evidence in a machine-verifiable form. Without it, the system accumulates "plausible but unverified" artifacts that may require complete rework. The relationship between tests, acceptance criteria, and proof of progress should be explicit and traceable: every task should have associated acceptance criteria, every acceptance criterion should have at least one test, and every test result should be recorded as evidence in the task's audit trail.

---

## 2. The testing model: what the orchestrator must understand

The orchestrator must maintain a taxonomy of test types and understand when each is appropriate. This is not merely a classification exercise — the choice of test type directly affects the quality of validation signal, the cost of execution, and the risk of false confidence.

### Test categories and their roles

**Unit tests** verify isolated logic in milliseconds and provide the fastest feedback loop. They are ideal for the inner iteration cycle of an autonomous agent but provide low confidence about system behavior because they test components in isolation. **Integration tests** verify component interactions and data flow, running in seconds to minutes. Kent C. Dodds' Testing Trophy elevates integration tests to the primary layer, arguing that "the more your tests resemble the way your software is used, the more confidence they can give you." **End-to-end tests** validate critical user journeys against a running system, providing the highest confidence but at the highest cost. **Regression tests** guard against re-breaking previously fixed behavior — and this is a critical category for autonomous agents, since TDAD research found that **TDD prompting alone actually increased regressions** (from 6.08% to 9.94%) with smaller models when contextual information about which tests to verify was missing.

**Contract tests** (e.g., Pact) verify service interface compatibility without deploying the full system, replacing many expensive E2E tests in distributed architectures. **Property-based tests** (Hypothesis, Hegel) verify invariants across randomly generated inputs rather than specific examples — Anthropic's agentic property-based testing research found real bugs in NumPy, SciPy, and AWS Lambda Powertools that example-based tests missed, with top-scoring bug reports being **86% valid and 81% reportable**. **Smoke tests** provide rapid post-deployment sanity checks. **Performance tests** detect latency and throughput regressions. **Security tests** scan for vulnerabilities, secrets, and injection vectors. **Migration tests** verify data integrity across schema or system changes. **Repo-specific validations** include custom linters, build checks, formatting rules, and domain-specific invariants defined per repository.

### How the orchestrator decides which tests are appropriate

Test strategy should vary along four dimensions:

**By task type.** Bug fixes require regression tests that reproduce the reported failure plus pass-to-pass tests ensuring no new breakage — this is precisely the SWE-bench validation model. New features require acceptance tests derived from the spec, plus integration tests for any new component interactions. Refactors require comprehensive regression testing with minimal new test creation, since the behavior should not change. Performance work requires benchmark tests with statistical comparison to baselines.

**By risk level.** Changes to authentication, payment processing, data persistence, or security boundaries demand comprehensive multi-layer validation including E2E tests, contract tests, and security scans. Changes to documentation, formatting, or internal comments need only smoke tests and linting. The orchestrator should compute a **risk score per change** based on files modified, their historical defect density, subsystem criticality, and change complexity. Meta's predictive test selection system uses gradient-boosted decision trees trained on exactly these features, catching **>95% of individual test failures while running only one-third of transitively dependent tests**.

**By subsystem maturity.** Mature, well-tested subsystems with high existing coverage need targeted regression tests on modified paths. New or poorly-tested subsystems need broader test generation including property-based tests to explore the input space. Subsystems with known flakiness need quarantine-aware execution strategies.

**By confidence requirement.** Pre-merge validation for auto-mergeable changes needs the highest confidence. In-progress validation during implementation needs fast feedback, not comprehensive coverage. Exploratory changes during planning need only smoke tests to verify feasibility.

---

## 3. Test planning: from source documents to validation strategy

Test planning is the process of deriving test intent — what must be validated and how — from the materials that define the work: specs, feature briefs, bug reports, architecture notes, and code changes themselves. The orchestrator must perform test planning at three stages: before implementation (to define success criteria), during implementation (to provide fast feedback), and before merge (to prove completion).

### Deriving test intent from source documents

The orchestrator should extract or infer acceptance criteria from whatever input defines the task. Academic tools demonstrate this is feasible at scale: CiRA extracts conditional statements from natural-language requirements using NLP and generates test cases via Cause-Effect Graphs, automatically generating **71.8% of manually-created test cases** in studies. SPECMATE extracts acceptance criteria from user stories using model-based testing, generating **56% of test cases** plus additional negative cases humans missed. AWS's VEW system reduces test case creation time by **80%** using LLM-based requirement classification and test generation.

For an orchestrator, the practical pipeline is:

1. **Parse the source document** (issue, spec, feature brief, bug report) to identify actions, conditions, expected outcomes, and edge cases. Given-When-Then extraction is the most effective format — tools like Zenhub already auto-generate BDD-format acceptance criteria from issue descriptions.

2. **Infer implicit requirements** the document doesn't state. Every bug fix implies "the bug should not recur" (regression test). Every API change implies "existing consumers should not break" (contract test). Every data mutation implies "data integrity is preserved" (property test). The orchestrator should maintain a library of **requirement archetypes** that trigger standard test categories.

3. **Analyze code changes** to identify what behavior changed. TDAD's approach is instructive: it builds AST-based code-test graphs with weighted impact analysis, identifying which existing tests are most likely affected by proposed changes. This reduced test-level regressions by **70%** on SWE-bench Verified.

4. **Detect missing coverage** by comparing the set of behaviors implied by the spec against the set of behaviors verified by existing tests. This gap analysis should use both static analysis (which code paths lack test coverage) and semantic analysis (which specified behaviors lack corresponding test assertions). Mutation testing is the strongest signal for test quality — Meta's research found that a test suite with **93% line coverage can have only 59% mutation score**, revealing 34 percentage points of "phantom coverage" where code is executed but bugs would not be caught.

### Choosing minimal vs. comprehensive validation

The orchestrator should maintain a **validation budget** that scales with risk and confidence requirements. For low-risk, high-confidence changes (e.g., renaming a variable in a well-tested module), running only affected unit tests and linting is sufficient. For high-risk, low-confidence changes (e.g., rewriting authentication logic), the orchestrator should run the full validation stack including unit, integration, E2E, contract, security, and property-based tests.

The key insight from Block's AI Agent Testing Pyramid (January 2026) is that validation layers should be organized by **uncertainty tolerance**, not just by test type. Deterministic validations (unit tests, type checking, linting) run on every change. Reproducible validations (record-and-replay integration tests) run on every PR. Probabilistic validations (benchmarks, statistical tests) run on demand. Subjective validations (LLM-as-judge evaluations) run for capability assessment only.

---

## 4. Test generation and maintenance

The orchestrator should treat test generation as an integral part of implementation, not a post-hoc activity. The most effective pattern, validated across multiple agents and research systems, is **spec-first test generation**: write tests before or alongside implementation so they constrain the solution rather than merely documenting it.

### When to create new tests

New tests should be created when: (a) no existing test covers a specified behavior, (b) a code change introduces new behavior not tested by existing tests, (c) a bug fix requires a regression test that reproduces the original failure, (d) mutation testing reveals surviving mutants indicating weak assertions, or (e) property-based testing identifies invariants that should hold but aren't verified.

Robert C. Martin's observation about the ATDD plugin for Claude Code is directly relevant: "The two different streams of tests cause Claude to think much more deeply about the structure of the code. It can't just willy-nilly plop code around and write a unit test for it. It is also constrained by the structure of the acceptance tests." Dual-stream testing — acceptance tests from specs plus unit tests from implementation — produces measurably better autonomous output than either stream alone.

### When to update existing tests

Existing tests should be updated when: (a) the spec changes and tests now encode outdated expectations, (b) refactoring changes interfaces without changing behavior (tests should be updated to use new interfaces while verifying the same behavior), or (c) tests are identified as flaky, brittle, or low-value. The orchestrator should **never silently delete or weaken a failing test** to make it pass — this is the single most dangerous failure mode for autonomous testing. Instead, if a test fails and the orchestrator believes the test rather than the implementation is wrong, it should flag this for explicit review, providing evidence for why the test's expectations are incorrect relative to the spec.

### Avoiding low-value and brittle generated tests

Research reveals five patterns of useless AI-generated tests that the orchestrator must actively avoid:

- **Mirror tests** that restate the implementation rather than verifying behavior (the test asserts what the code does, not what it should do)
- **Happy-path-only tests** that verify success cases but miss errors, timeouts, and edge cases
- **Over-mocked tests** that mock so aggressively nothing real is tested
- **Snapshot traps** that break on any output format change
- **Weak assertions** like `expect(result).toBeDefined()` that pass for any return value

Industrial data quantifies the danger: in one study, AI-generated tests achieved **91% code coverage but only 34% mutation score**, compared to human-written tests with 76% coverage and 68% mutation score. The AI tests execute more code but detect fewer bugs. The orchestrator must use **mutation testing as a quality gate for generated tests**. If a test suite has high coverage but low mutation score, the orchestrator should feed surviving mutant reports back to the test generator to strengthen assertions — Meta's ACH system and Atlassian's mutation coverage assistant both demonstrate this feedback loop working at scale.

Property-based testing is a powerful countermeasure to weak test generation. Instead of generating specific input-output examples (which LLMs tend to copy from the implementation), property-based tests define invariants that must hold across all inputs. The PGS (Property-Generated Solver) framework demonstrates a **15.7% average improvement in repair success rate** over traditional TDD approaches by using two independent LLM agents — one generating code, one generating properties — with property-based testing as the arbiter.

### Connecting tests to requirements and tasks

Every generated test should carry metadata linking it to: (a) the requirement or acceptance criterion it validates, (b) the task that triggered its creation, (c) the specific code paths or behaviors it exercises, and (d) the confidence signal it provides (e.g., "verifies error handling for null input per AC-3 of FEAT-1234"). This traceability is not just bookkeeping — it enables the orchestrator to answer "is this task complete?" by checking whether all acceptance criteria have corresponding passing tests.

---

## 5. Test execution strategy

Execution strategy determines how the orchestrator balances speed, cost, coverage, and confidence. The goal is progressive confidence-building: fast, cheap validations first, escalating to slower, more comprehensive validations only as confidence warrants.

### Progressive validation stages

The orchestrator should implement a staged execution pipeline:

**Stage 0 — Instant checks (seconds).** Static analysis, linting, type checking, syntax validation. These run on every edit during the implementation loop. Aider's tree-sitter-based AST linting catches fatal syntax errors without external tooling. These checks provide immediate signal and should never be skipped.

**Stage 1 — Fast unit tests (seconds to minutes).** Run affected unit tests identified through dependency analysis or change-impact mapping. This is the inner feedback loop that drives the edit→test→fix cycle. Use test impact analysis to avoid running unrelated tests — Google's TAP system processes over 4 billion test cases per day by batching related changes and using dependency-based selection.

**Stage 2 — Integration and contract tests (minutes).** Run integration tests for affected component boundaries and contract tests for any modified service interfaces. This validates that components work together correctly.

**Stage 3 — Comprehensive validation (minutes to hours).** Full regression suite, E2E tests for critical user journeys, security scans, performance benchmarks. This runs before merge and should include mutation testing for new or modified test code. Not every change triggers this stage — the orchestrator should use risk-based routing to determine when comprehensive validation is warranted.

**Stage 4 — Periodic deep validation (hours, scheduled).** Full suite runs across all test layers, comprehensive mutation testing, extended property-based test campaigns, and cross-browser/cross-platform testing. This catches issues that targeted selection misses and keeps predictive models honest. Google and Meta both maintain periodic full runs alongside their predictive selection systems.

### Risk-based test routing

Each change should be scored for risk and routed to the appropriate validation depth. The risk score should incorporate: files changed and their criticality classification, historical defect density of affected modules, complexity of the change (lines modified, AST nodes changed), whether the change touches shared infrastructure or isolated components, and the author's track record (for autonomous agents, this means the model's historical accuracy on similar tasks). Meta's predictive test selection system uses gradient-boosted decision trees on similar features and achieves **>99.9% detection of faulty changes** while halving infrastructure cost.

### Parallelization and environment management

The orchestrator should parallelize test execution across isolated environments. Each test run needs a reproducible environment — containerized (Docker) execution is the standard across all major autonomous coding agents (SWE-Agent, Devin, OpenHands, Codex all use Docker containers). Tests with no dependencies should execute simultaneously. Resource-intensive tests should be distributed to prevent overload. Netflix achieves **10x test execution reduction** using Develocity's predictive test selection combined with test distribution.

For long-running or expensive validations (E2E suites, performance benchmarks), the orchestrator should support asynchronous execution: kick off the validation, continue with other work, and process results when they arrive. This prevents expensive tests from blocking the agent's primary feedback loop.

---

## 6. Interpreting results: from test output to planning decisions

Result interpretation is where the orchestrator's reasoning capability matters most. Raw test output — pass, fail, error, skip — is insufficient for planning decisions. The orchestrator must classify failures, assess their significance, and generate appropriate follow-up actions.

### Failure classification taxonomy

The orchestrator should classify every test failure into one of five categories, each requiring different follow-up:

**Implementation defect.** The code under test is incorrect. Signal: test assertion fails on a behavioral check, error traces to application code, and the test matches the spec. Action: generate a fix task targeting the specific code path, including the failing test output as context.

**Test defect.** The test itself is incorrect. Signal: test makes assertions inconsistent with the spec, or has setup/teardown errors. This is particularly common with AI-generated tests. Action: regenerate or fix the test, explicitly verifying against the spec before retrying. **The orchestrator must never assume a failing test is wrong simply because the implementation looks correct** — this bias toward the implementation is a primary source of false confidence.

**Environment defect.** Infrastructure or configuration problems. Signal: tests pass locally but fail in CI, errors relate to network/permissions/missing services, or identical tests fail inconsistently across environments. Action: fix environment configuration; flag for infrastructure attention; do not modify code or tests.

**Flaky test.** Non-deterministic failure unrelated to the current change. Signal: same test passes and fails on identical code across multiple runs, or the test has a known flakiness history. Action: quarantine the test, retry with a cap (3x), and create a maintenance task to stabilize it. Google's data shows **~84% of observed pass-to-fail transitions in post-submit testing are flaky**, making robust flaky-test handling essential.

**Missing behavior.** Functionality not yet implemented. Signal: tests reference functions or endpoints that don't exist, or test errors indicate "not found"/"undefined" results. Action: generate implementation tasks for the missing components. This is a normal state during test-first development.

### Building confidence from multiple signals

The orchestrator should aggregate validation signals into a composite confidence score rather than treating test results as binary pass/fail. A practical scoring framework:

Mandatory signals that block merge if failing: build compilation, all existing tests continue passing (no regressions), new fail-to-pass tests pass, no critical security vulnerabilities. Weighted signals that contribute to the confidence score: code coverage on new/modified code (threshold: ≥80%), mutation testing score (threshold: ≥50%), static analysis cleanliness, type-check passage, performance within baseline bounds. The composite score should drive graduated decisions: confidence ≥0.95 enables auto-merge, 0.80–0.95 enables auto-merge with notification, 0.60–0.80 requires human review, and below 0.60 blocks the merge.

### Generating follow-up work from test results

When tests reveal issues, the orchestrator should generate structured follow-up tasks rather than simply reporting failure. A single test failure can generate multiple sub-tasks: diagnose root cause, implement fix, add regression test for the specific failure, and verify the fix doesn't introduce new regressions. Pattern matching across failures — identifying that multiple test failures share a common root cause — reduces redundant work. Launchable's approach of grouping related failures under common issues using AI summarization is a practical model.

---

## 7. Traceability and evidence: proving that work satisfies goals

Traceability is the mechanism by which the orchestrator demonstrates — to itself and to human reviewers — that completed work actually satisfies its goals. Without traceability, the orchestrator accumulates unverified claims of completion.

### The traceability chain

Every work unit should maintain a complete traceability chain: **Goal/Spec → Acceptance Criteria → Tests → Test Results → Code Changes → Evidence Pack**. This chain should be bidirectional — from any requirement, you can find its tests and their results; from any test, you can find the requirement it validates and the code it exercises.

The orchestrator should automatically generate traceability links by: parsing task descriptions and commit messages for requirement identifiers, analyzing test names and docstrings for requirement references, using code coverage data to map tests to exercised code paths, and recording test execution results with timestamps, environment details, and artifact references (logs, screenshots, coverage reports).

### Evidence for merge decisions

Before promoting any change, the orchestrator should assemble an evidence pack containing: the list of acceptance criteria derived from the spec, the mapping from each criterion to its validating tests, the results of those tests (with execution logs), any additional validation signals (coverage, mutation score, static analysis), and a confidence score with its contributing factors. This evidence pack should be attached to the PR or merge request and stored immutably for audit purposes.

### Evidence in long autonomous runs

During extended autonomous sessions — where the orchestrator may work for hours without human interaction — evidence accumulation is critical for post-hoc review. The orchestrator should maintain a running evidence log that records: every test execution (what ran, what passed, what failed, and why), every planning decision driven by test results (what was accepted, rejected, revised, or decomposed), every test generated or modified and why, and a cumulative confidence trajectory showing how confidence evolved over the session. This log enables a human reviewer to understand not just what the orchestrator produced, but why it believed the output was correct.

---

## 8. Failure modes and mitigations

Autonomous testing introduces failure modes that don't exist in human-driven development. The orchestrator must be designed to detect and mitigate these.

### False confidence from weak tests

This is the **most dangerous failure mode**. Industrial data shows AI-generated test suites can achieve 91% code coverage while catching only 34% of injected mutations, compared to human-written tests with 76% coverage and 68% mutation score. The orchestrator produces tests that execute code without actually verifying behavior, creating an illusion of thoroughness.

Mitigation: **Mutation testing as a mandatory quality gate.** The orchestrator should run mutation testing on all generated test code and reject tests that fall below a mutation score threshold (≥50% for standard code, ≥70% for critical paths). Surviving mutants should be fed back to the test generator as prompts for stronger assertions. Anthropic's property-based testing agent demonstrates that LLMs can write meaningful property tests when specifically prompted to think about invariants rather than examples.

### Brittle generated tests

AI-generated tests tend to assert exact outputs rather than structural properties, coupling tests to implementation details. These tests break on every refactoring, generating noise that obscures real failures.

Mitigation: Prefer behavioral assertions over value assertions. Use semantic locators (roles, labels) over structural selectors (CSS classes, XPaths) in UI tests. Validate state and outcomes rather than representation details. The Claude Code Playwright "Healer" agent demonstrates an effective pattern: when a test fails due to locator staleness, it inspects the current UI, suggests locator updates, and re-runs — automatically maintaining brittle tests rather than letting them accumulate.

### Validation blind spots in long autonomous runs

Over extended sessions, the orchestrator may develop systematic blind spots — categories of behavior it consistently fails to test. Common blind spots include: concurrency and race conditions, security boundaries, state leaks between operations, error handling under resource exhaustion, and backward compatibility with existing consumers.

Mitigation: Maintain a **validation checklist** that the orchestrator explicitly evaluates for every task. This checklist should include categories of behavior that LLMs systematically undertests (security, concurrency, error handling). Property-based testing with random input generation helps explore the input space beyond what the orchestrator would naturally consider. Periodic comprehensive validation (Stage 4) catches blind spots that accumulate during targeted testing.

### Tests encoding incorrect assumptions

When the orchestrator generates tests from a misunderstanding of the spec, those tests "protect" the wrong behavior — they pass for buggy code and fail for correct code. This is particularly insidious because it inverts the signal: the orchestrator becomes more confident in incorrect implementations.

Mitigation: Separate the test-specification step from the test-implementation step. The orchestrator should first derive test descriptions in natural language from the spec (what should be tested and why), then implement those descriptions as executable tests (how to test it). Human review of test descriptions is far more efficient than review of test code. The ATDD pattern — where acceptance tests are written in Given/When/Then domain language before implementation — provides a natural checkpoint.

### Over-testing and wasted runtime

Excessive test generation creates maintenance burden and wastes execution time. An orchestrator that generates 500 unit tests for a simple utility function is producing negative value.

Mitigation: Apply a **testing budget** proportional to the risk and complexity of the component. Use risk-based selection to determine test depth. Track the marginal value of each additional test (measured by mutation score improvement) and stop generating when marginal value drops below threshold. Google's data shows **91.3% of test targets pass and never fail** — most tests provide background assurance rather than active failure detection. Focus generation on the 8.7% that actually provide signal.

### Flaky tests undermining autonomous decisions

Flaky tests are catastrophic for autonomous systems because they generate noise that the orchestrator cannot distinguish from real failures without sophisticated analysis. Google reports ~16% of individual tests exhibit flakiness; at that rate, an autonomous system processing hundreds of tests per iteration will encounter flaky failures constantly.

Mitigation: Implement a **flakiness scoring system** based on historical pass/fail patterns. Use differential coverage analysis (DeFlaker approach): if a test fails but none of its covered code has changed, it's likely flaky. Maintain a quarantine pool for tests above a flakiness threshold. Retry suspected flaky tests up to 3x before treating failure as real. Require new tests to pass a stability loop (multiple runs with no code changes) before entering the critical CI path.

---

## 9. Recommended testing architecture

### v1: practical near-term design

The v1 architecture focuses on capabilities that are implementable today with proven techniques, providing immediate value for autonomous execution.

**Test planning engine.** Accepts task descriptions and code diffs as input. Extracts acceptance criteria using Given/When/Then parsing. Maps code changes to affected tests using static dependency analysis (build graph + import graph). Computes a risk score per change. Outputs a test plan specifying which test layers to execute and what new tests to generate. This engine should be configurable per repository — teams specify their test commands, framework conventions, and criticality mappings in a configuration file (analogous to Codex's AGENTS.md or Aider's `--test-cmd`).

**Test generator.** Generates tests from acceptance criteria and code changes. Uses the repository's existing test patterns as style templates (few-shot learning from the project's own tests). Generates both example-based tests (unit/integration) and property-based tests (using Hypothesis or equivalent). Runs mutation testing on generated tests and iterates until mutation score exceeds threshold. Connects every generated test to its source requirement via metadata comments or test naming conventions.

**Test executor.** Progressive staged execution: Stage 0 (static checks) → Stage 1 (affected unit tests) → Stage 2 (integration/contract) → Stage 3 (comprehensive, risk-gated). Runs in containerized environments for reproducibility. Supports parallel execution with configurable concurrency. Implements flakiness detection via historical tracking and differential coverage. Retries suspected flaky tests up to 3x. Collects structured results (pass/fail/skip/flaky, duration, coverage delta, error output).

**Result interpreter.** Classifies failures into the five-category taxonomy (implementation defect, test defect, environment defect, flaky test, missing behavior). Computes composite confidence score from all validation signals. Generates structured follow-up tasks for each failure category. Presents evidence packs for merge decisions.

**Traceability store.** Maintains bidirectional links between requirements, tests, code changes, and results. Records all test executions with immutable timestamps. Provides evidence packs on demand for any task or merge request. Tracks confidence trajectories over time.

### What should come later: advanced capabilities

**Predictive test selection.** Train ML models (gradient-boosted decision trees, following Meta's approach) on historical {change → test outcome} data to predict which tests are most likely to fail for a given change. This requires sufficient historical data to train effectively and should be introduced once the v1 system has accumulated execution history.

**Autonomous coverage gap detection.** Use mutation testing results and static analysis to automatically identify undertested behaviors and generate targeted tests. Feed surviving mutant reports back to the test generator in a closed loop. This requires robust mutation testing infrastructure and should be introduced once the basic generation pipeline is stable.

**Self-improving test suites.** Track which tests actually catch bugs versus which tests only provide background assurance. Retire tests with zero signal value over extended periods. Strengthen tests that catch important failures. This requires longitudinal tracking of test value and should be data-driven rather than heuristic.

**Calibrated confidence scoring.** Train a model to predict merge safety from validation signals, calibrated against actual post-merge defect rates. This requires ground truth data about post-merge outcomes and should be introduced once the system has sufficient deployment history.

**Multi-agent test specialization.** Deploy specialized testing agents — a planner that designs test strategy, a generator that writes tests, a healer that fixes broken tests, a reviewer that audits test quality — analogous to Claude Code's Playwright architecture (which reduced flaky tests by 85% at OpenObserve). This requires a mature agent orchestration framework and should be introduced when the single-agent loop is well-understood.

### What should remain configurable per repo or organization

The following should never be hardcoded: test commands and framework conventions (every repo is different), risk classification of subsystems (only the team knows what's critical), confidence thresholds for auto-merge vs. human review, mutation score targets (varies by domain — safety-critical systems need higher thresholds), flakiness tolerance and quarantine policies, and which test layers are available and relevant (not every repo has E2E tests or contract tests). The orchestrator should provide sensible defaults but allow override at the repository, organization, and task level.

---

## Open questions and implementation implications

Several significant questions remain unresolved in the current state of the art and will require experimentation and iteration:

**How should the orchestrator handle repos with no existing tests?** Most autonomous agents depend heavily on pre-existing test suites. When starting from zero, the orchestrator must generate the initial test suite from specs alone — a cold-start problem that current tools handle poorly. A practical approach is to begin with smoke tests and property-based tests (which require less domain-specific knowledge) and build up to comprehensive suites as the codebase matures.

**What is the right balance between test generation cost and test value?** Generating high-quality tests with mutation testing validation is expensive in both LLM tokens and execution time. The orchestrator needs a cost model that tracks the marginal value of additional testing and stops when returns diminish. Current research does not provide clear guidance on where this threshold lies across different domains.

**How should the orchestrator handle tests for non-deterministic behavior?** LLM-based features, probabilistic algorithms, and systems with intentional randomness resist traditional assertion-based testing. Block's AI Agent Testing Pyramid suggests statistical benchmarks (run multiple times, track success rate distributions) and LLM-as-judge evaluations for these cases, but the infrastructure for this is immature.

**How much should the orchestrator trust its own test quality assessment?** Mutation testing is the best available proxy for test effectiveness, but it has limitations — equivalent mutants, high computational cost, and incomplete coverage of all defect types. The orchestrator should treat mutation scores as a useful signal, not an oracle, and maintain epistemic humility about the completeness of its validation.

**When should the orchestrator escalate to human review?** The graduated confidence framework provides a structure, but the thresholds need calibration per organization. Too-low thresholds waste human attention; too-high thresholds let defects through. This requires monitoring post-merge defect rates and adjusting thresholds based on observed outcomes — a feedback loop that takes months to calibrate.

The fundamental insight underlying this entire spec is that **testing for an autonomous coding orchestrator is not a quality assurance function — it is the primary mechanism by which the system reasons about whether it is making progress toward its goals**. An orchestrator that cannot plan, generate, execute, and interpret tests with sophistication is an orchestrator that cannot work autonomously for any meaningful duration. Testing is not the last step before shipping. It is the continuous process by which the system proves, to itself and to its operators, that it knows what it is doing.